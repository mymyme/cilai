use super::{AiRuntimeManager, AutomationRule, automation_lifecycle::ScheduledAutomation, lock};
use crate::{model_connections::ExecutionScope, supervisor::AppServerSupervisor};
use contexthub_protocol::{
    ModelExecutionLease, ModelExecutionPrepareParams, PersonalAutomationAdmitParams,
    PersonalAutomationAdmitResult, PersonalAutomationBindParams, PersonalAutomationBindResult,
    PersonalAutomationClaim, PersonalAutomationFinishOutcome, PersonalAutomationFinishParams,
    PersonalAutomationFinishResult, PersonalAutomationListTriggersParams,
    PersonalAutomationPrepareExecutionParams, PersonalAutomationResolveInterruptedParams,
    PersonalAutomationRuleRecord, PersonalAutomationTriggerPage,
};
use serde::{Serialize, de::DeserializeOwned};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct RuleClaim {
    server: AppServerSupervisor,
    record: PersonalAutomationClaim,
}

fn request<T: Serialize, R: DeserializeOwned>(
    server: &AppServerSupervisor,
    method: &str,
    params: &T,
) -> Result<R, String> {
    serde_json::from_value(server.request_result(
        method,
        &serde_json::to_value(params).map_err(|_| "automation_request_invalid")?,
    )?)
    .map_err(|_| "automation_receipt_unconfirmed".into())
}

fn accept_rule(
    manager: &AiRuntimeManager,
    record: &PersonalAutomationRuleRecord,
) -> Result<AutomationRule, String> {
    let rule: AutomationRule = serde_json::from_value(record.value["value"].clone())
        .map_err(|_| "automation_rule_record_invalid")?;
    if record.id != format!("automationRules:{}", rule.automation_rule_id)
        || record.value["entryId"] != record.id
        || record.value["field"] != "automationRules"
        || record.value["member"] != rule.automation_rule_id
        || record.value["shape"] != "array"
    {
        return Err("automation_rule_record_invalid".into());
    }
    let mut state = lock(&manager.inner);
    if super::persistence_outcome_unknown(&state) {
        return Err("automation_persistence_unconfirmed".into());
    }
    if state.persisted.automation_rules.iter().any(|current| {
        current.automation_rule_id == rule.automation_rule_id
            && (current.rule_version > rule.rule_version
                || (current.rule_version == rule.rule_version
                    && current.updated_at > rule.updated_at))
    }) {
        return Ok(rule);
    }
    state.ledger_baseline.accept_automation_record(record)?;
    if let Some(existing) = state
        .persisted
        .automation_rules
        .iter_mut()
        .find(|existing| existing.automation_rule_id == rule.automation_rule_id)
    {
        existing.clone_from(&rule);
    } else {
        state.persisted.automation_rules.push(rule.clone());
    }
    Ok(rule)
}

fn profile_id(manager: &AiRuntimeManager) -> Result<String, String> {
    let directory = manager.gateway_data_dir()?;
    if directory.file_name().and_then(|name| name.to_str()) != Some("ai-runtime") {
        return Err("automation_profile_identity_unconfirmed".into());
    }
    directory
        .parent()
        .and_then(|root| root.file_name())
        .and_then(|name| name.to_str())
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "automation_profile_identity_unconfirmed".into())
}

fn current_rule(
    manager: &AiRuntimeManager,
    rule_id: &str,
) -> Result<PersonalAutomationRuleRecord, String> {
    let server = manager.app_server()?;
    let value = server.request_result("aiLedger/get", &serde_json::json!({}))?;
    let record = value["ledger"]["entries"]
        .as_array()
        .ok_or("automation_rule_record_invalid")?
        .iter()
        .find(|record| record["id"] == format!("automationRules:{rule_id}"))
        .ok_or("automation_rule_not_found")?;
    let record =
        serde_json::from_value(record.clone()).map_err(|_| "automation_rule_record_invalid")?;
    accept_rule(manager, &record)?;
    Ok(record)
}

impl RuleClaim {
    pub(super) fn admit(
        manager: &AiRuntimeManager,
        rule_id: &str,
        scheduled: Option<&ScheduledAutomation>,
    ) -> Result<(Self, AutomationRule), String> {
        let record = current_rule(manager, rule_id)?;
        let rule: AutomationRule = serde_json::from_value(record.value["value"].clone())
            .map_err(|_| "automation_rule_record_invalid")?;
        let now = super::unix_epoch_millis()?;
        if let Some(candidate) = scheduled
            && (!rule.enabled
                || rule.rule_version != candidate.rule_version
                || rule.next_run_at != Some(candidate.due_at)
                || candidate.due_at > now)
        {
            return Err("automation_schedule_changed".into());
        }
        let next_run_at = rule
            .enabled
            .then(|| {
                super::next_run_after(
                    &rule.schedule_kind,
                    rule.interval_minutes,
                    rule.time_of_day_minutes,
                    &super::effective_weekdays(&rule.weekdays, rule.weekday),
                    now,
                )
            })
            .flatten();
        let server = manager.app_server()?;
        let result: PersonalAutomationAdmitResult = request(
            &server,
            "personalAutomation/admit",
            &PersonalAutomationAdmitParams {
                trigger_id: uuid::Uuid::new_v4().simple().to_string(),
                rule_id: rule_id.into(),
                expected_rule_digest: record.digest,
                scheduled_due_at: scheduled.map(|candidate| candidate.due_at),
                next_run_at,
                owner: crate::model_connections::execution_owner()?,
            },
        )?;
        if result.replayed {
            return Err("automation_trigger_already_accepted".into());
        }
        accept_rule(manager, &result.rule_record)?;
        let rule: AutomationRule =
            serde_json::from_value(result.admission_rule_record.value["value"].clone())
                .map_err(|_| "automation_rule_record_invalid")?;
        let record = result.claim.ok_or("automation_previous_run_active")?;
        if record.released || record.rule_id != rule_id || record.profile_id != profile_id(manager)?
        {
            return Err("automation_claim_unconfirmed".into());
        }
        Ok((Self { server, record }, rule))
    }

    pub(super) fn prepare_execution(
        &self,
        manager: &AiRuntimeManager,
        mut execution: ModelExecutionPrepareParams,
    ) -> Result<Arc<ExecutionScope>, String> {
        execution.launch_protocol = 1;
        execution.owner = Some(self.record.owner.clone());
        let lease: ModelExecutionLease = request(
            &self.server,
            "personalAutomation/prepareExecution",
            &PersonalAutomationPrepareExecutionParams {
                claim_id: self.record.claim_id.clone(),
                owner: self.record.owner.clone(),
                client_operation_id: uuid::Uuid::new_v4().simple().to_string(),
                execution,
            },
        )?;
        ExecutionScope::from_lease(
            manager,
            self.server.clone(),
            manager.gateway_data_dir()?,
            lease,
        )
    }

    pub(super) fn bind(&self, manager: &AiRuntimeManager, run_id: &str) -> Result<(), String> {
        let result: PersonalAutomationBindResult = request(
            &self.server,
            "personalAutomation/bind",
            &PersonalAutomationBindParams {
                claim_id: self.record.claim_id.clone(),
                owner: self.record.owner.clone(),
                run_id: run_id.into(),
            },
        )?;
        if let Some(record) = result.rule_record {
            accept_rule(manager, &record)?;
        }
        Ok(())
    }

    pub(super) fn finish(
        &self,
        manager: &AiRuntimeManager,
        outcome: PersonalAutomationFinishOutcome,
        reason: Option<&str>,
    ) -> Result<(), String> {
        let result: PersonalAutomationFinishResult = request(
            &self.server,
            "personalAutomation/finish",
            &PersonalAutomationFinishParams {
                claim_id: self.record.claim_id.clone(),
                owner: self.record.owner.clone(),
                outcome,
                reason_code: reason.map(reason_code),
            },
        )?;
        if !result.claim.released {
            return Err("automation_claim_release_unconfirmed".into());
        }
        if let Some(record) = result.rule_record {
            accept_rule(manager, &record)?;
        }
        Ok(())
    }

    pub(super) fn report_unconfirmed(
        &self,
        manager: &AiRuntimeManager,
        reason: &str,
    ) -> Result<(), String> {
        let result: PersonalAutomationFinishResult = request(
            &self.server,
            "personalAutomation/finish",
            &PersonalAutomationFinishParams {
                claim_id: self.record.claim_id.clone(),
                owner: self.record.owner.clone(),
                outcome: PersonalAutomationFinishOutcome::OutcomeUnconfirmed,
                reason_code: Some(reason_code(reason)),
            },
        )?;
        if result.claim.released || result.claim.profile_id != profile_id(manager)? {
            return Err("automation_claim_unknown_receipt_invalid".into());
        }
        Ok(())
    }
}

fn reason_code(reason: &str) -> String {
    if !reason.is_empty()
        && reason.len() <= 100
        && reason
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        reason.into()
    } else {
        "automation_run_failed".into()
    }
}

impl AiRuntimeManager {
    /// # Errors
    /// 当前资料不可用、分页无效或持久触发回执无法读取时拒绝。
    pub fn automation_trigger_history(
        &self,
        params: &PersonalAutomationListTriggersParams,
    ) -> Result<PersonalAutomationTriggerPage, String> {
        request(
            &self.app_server()?,
            "personalAutomation/listTriggers",
            params,
        )
    }

    /// # Errors
    /// 原拥有者仍在运行、历史来源不符或真实退出无法证明时保留占用。
    pub fn reconcile_automation_claim(
        &self,
        claim_id: &str,
    ) -> Result<PersonalAutomationFinishResult, String> {
        let _automation = lock(&self.automation_lifecycle);
        let result: PersonalAutomationFinishResult = request(
            &self.app_server()?,
            "personalAutomation/resolveInterrupted",
            &PersonalAutomationResolveInterruptedParams {
                claim_id: claim_id.into(),
            },
        )?;
        if result.claim.profile_id != profile_id(self)?
            || result.trigger.profile_id != result.claim.profile_id
            || result.trigger.read_only_after_restore
            || !result.claim.released
        {
            return Err("automation_claim_reconcile_unconfirmed".into());
        }
        if let Some(record) = &result.rule_record {
            accept_rule(self, record)?;
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::{
        persist_locked,
        tests::{AutomationTestRuntime, automation_fixture},
    };

    #[test]
    fn a_delayed_trigger_receipt_cannot_replace_a_newer_saved_rule_configuration() {
        let _guard = crate::supervisor::app_server_test_guard();
        let runtime = AutomationTestRuntime::new();
        {
            let mut state = lock(&runtime.manager.inner);
            state
                .persisted
                .automation_rules
                .push(automation_fixture("shared-rule"));
            persist_locked(&mut state).unwrap();
        }
        let prior = current_rule(&runtime.manager, "shared-rule").unwrap();
        {
            let mut state = lock(&runtime.manager.inner);
            let rule = &mut state.persisted.automation_rules[0];
            rule.rule_version += 1;
            rule.updated_at += 1;
            rule.name = "已经保存的新配置".into();
            persist_locked(&mut state).unwrap();
        }
        accept_rule(&runtime.manager, &prior).unwrap();
        let mut state = lock(&runtime.manager.inner);
        assert_eq!(state.persisted.automation_rules[0].name, "已经保存的新配置");
        assert_eq!(state.persisted.automation_rules[0].rule_version, 2);
        persist_locked(&mut state).unwrap();
        drop(state);
        let stored = current_rule(&runtime.manager, "shared-rule").unwrap();
        assert_eq!(stored.value["value"]["name"], "已经保存的新配置");
        assert_eq!(stored.value["value"]["ruleVersion"], 2);
    }
}
