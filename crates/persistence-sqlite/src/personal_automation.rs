//! 同规则占用、计划推进和模型准备共享一个事务；历史触发不授予新的执行许可。
use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{
    ModelExecutionLease, ModelExecutionOwner, ModelPurpose, PersonalAutomationAdmitParams,
    PersonalAutomationAdmitResult, PersonalAutomationBindParams, PersonalAutomationBindResult,
    PersonalAutomationClaim, PersonalAutomationFinishOutcome, PersonalAutomationFinishParams,
    PersonalAutomationFinishResult, PersonalAutomationListTriggersParams,
    PersonalAutomationPrepareExecutionParams, PersonalAutomationResolveInterruptedParams,
    PersonalAutomationRuleRecord, PersonalAutomationTriggerItem, PersonalAutomationTriggerOutcome,
    PersonalAutomationTriggerPage, PersonalAutomationTriggerRecord, PersonalWriterLease,
    PersonalWriterLeaseParams, PersonalWriterMode, PersonalWriterPrepareParams,
    PersonalWriterState,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[path = "personal_automation/actions.rs"]
mod actions;
#[cfg(test)]
#[path = "personal_automation/tests.rs"]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExecutionPreparation {
    client_operation_id: String,
    fingerprint: String,
    lease_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredTrigger {
    request_fingerprint: String,
    instance_fence: String,
    admission_rule_record: PersonalAutomationRuleRecord,
    rule_record: PersonalAutomationRuleRecord,
    claim: Option<PersonalAutomationClaim>,
    trigger: PersonalAutomationTriggerRecord,
    preparations: Vec<ExecutionPreparation>,
}

fn reject(code: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, code, code)
}
fn opaque(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn uuid32(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .is_ok_and(|id| id.get_version_num() == 4 && id.simple().to_string() == value)
}
fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn encode(value: &impl Serialize) -> Result<Value, PersistenceError> {
    serde_json::to_value(value).map_err(|_| reject("automation_claim_record_invalid"))
}
fn decode<T: DeserializeOwned>(value: Value) -> Result<T, PersistenceError> {
    serde_json::from_value(value).map_err(|_| reject("automation_claim_request_invalid"))
}
fn digest(value: &impl Serialize) -> Result<String, PersistenceError> {
    Ok(crate::ai_business_ledger::digest(&encode(value)?))
}
fn process_alive(pid: u32, fingerprint: &str) -> Result<bool, PersistenceError> {
    if pid == 0 || fingerprint.is_empty() {
        return Err(reject("automation_claim_process_unconfirmed"));
    }
    contexthub_platform_adapter::instance::process_start_fingerprint(pid)
        .map(|value| value.as_deref() == Some(fingerprint))
        .map_err(|_| reject("automation_claim_process_unconfirmed"))
}
fn owner_alive(owner: &ModelExecutionOwner) -> Result<bool, PersistenceError> {
    if !opaque(&owner.instance_id) {
        return Err(reject("automation_claim_owner_invalid"));
    }
    process_alive(owner.pid, &owner.start_fingerprint)
}
fn instance(tx: &Transaction<'_>, profile: &str) -> Result<String, PersistenceError> {
    tx.query_row("SELECT instance_fence FROM personal_profile_compatibility WHERE profile_id=?1 AND state='ready' AND data_version=2", [profile], |row| row.get(0))
        .map_err(database_error("read automation profile readiness"))
}
fn read(tx: &Transaction<'_>, id: &str) -> Result<Option<StoredTrigger>, PersistenceError> {
    let raw: Option<String> = tx.query_row("SELECT payload_json FROM durable_events WHERE object_type='personal_automation_trigger' AND object_id=?1 ORDER BY global_sequence DESC LIMIT 1", [id], |row| row.get(0))
        .optional().map_err(database_error("read automation trigger"))?;
    raw.map(|raw| {
        let record: StoredTrigger =
            serde_json::from_str(&raw).map_err(|_| reject("automation_claim_record_invalid"))?;
        if record.trigger.trigger_id != id
            || !uuid32(id)
            || record.trigger.read_only_after_restore
            || record.claim.as_ref().is_some_and(|claim| {
                claim.claim_id != id
                    || claim.trigger_id != id
                    || claim.profile_id != record.trigger.profile_id
                    || claim.rule_id != record.trigger.rule_id
            })
        {
            return Err(reject("automation_claim_record_invalid"));
        }
        Ok(record)
    })
    .transpose()
}
fn append(tx: &Transaction<'_>, record: &StoredTrigger, now: i64) -> Result<(), PersistenceError> {
    tx.execute("INSERT INTO durable_events(event_id,event_type,object_type,object_id,payload_json,correlation_id,occurred_at,created_at) VALUES(?1,'personal_automation.trigger','personal_automation_trigger',?2,?3,?2,?4,?4)",
        params![format!("personal-automation:{}",uuid::Uuid::new_v4().simple()),record.trigger.trigger_id,encode(record)?.to_string(),now])
        .map_err(database_error("append automation trigger"))?;
    Ok(())
}
fn rule(
    tx: &Transaction<'_>,
    id: &str,
) -> Result<Option<PersonalAutomationRuleRecord>, PersistenceError> {
    let key = format!("automationRules:{id}");
    let row: Option<(i64, String, String)> = tx
        .query_row(
            "SELECT position,record_json,digest FROM ai_runtime_entries WHERE record_id=?1",
            [&key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(database_error("read automation rule"))?;
    row.map(|(position, raw, expected)| {
        let value: Value =
            serde_json::from_str(&raw).map_err(|_| reject("automation_rule_record_invalid"))?;
        if position < 0
            || value["entryId"] != key
            || value["field"] != "automationRules"
            || value["member"] != id
            || value["shape"] != "array"
            || value["value"]["automationRuleId"] != id
            || digest(&json!({"position":position,"value":value}))? != expected
        {
            return Err(reject("automation_rule_record_invalid"));
        }
        Ok(PersonalAutomationRuleRecord {
            id: key,
            position,
            digest: expected,
            value,
        })
    })
    .transpose()
}
fn save_rule(
    tx: &Transaction<'_>,
    record: &PersonalAutomationRuleRecord,
    now: i64,
) -> Result<PersonalAutomationRuleRecord, PersistenceError> {
    crate::ai_business_ledger::apply_batch(
        tx,
        &json!({"commandId":format!("automation-claim:{}",uuid::Uuid::new_v4().simple()),"records":[{"collection":"entries","id":record.id,"position":record.position,"expectedDigest":record.digest,"value":record.value}]}),
        now,
    )?;
    Ok(PersonalAutomationRuleRecord {
        digest: digest(&json!({"position":record.position,"value":record.value}))?,
        ..record.clone()
    })
}
fn resource_id(rule_id: &str) -> String {
    format!("automation:{:x}", Sha256::digest(rule_id.as_bytes()))
}
fn active_claim(
    tx: &Transaction<'_>,
    profile: &str,
    rule_id: &str,
) -> Result<bool, PersistenceError> {
    tx.query_row("WITH latest AS (SELECT MAX(global_sequence) n FROM durable_events WHERE object_type='personal_automation_trigger' GROUP BY object_id) SELECT EXISTS(SELECT 1 FROM latest JOIN durable_events e ON e.global_sequence=latest.n WHERE json_extract(e.payload_json,'$.claim.profileId')=?1 AND json_extract(e.payload_json,'$.claim.ruleId')=?2 AND json_extract(e.payload_json,'$.claim.released')=0)",params![profile,rule_id],|row|row.get(0))
        .map_err(database_error("read active automation claims"))
}
fn current_claim<'a>(
    record: &'a StoredTrigger,
    profile: &str,
    fence: &str,
) -> Result<&'a PersonalAutomationClaim, PersistenceError> {
    if record.trigger.profile_id != profile {
        return Err(reject("automation_claim_restored_read_only"));
    }
    if record.instance_fence != fence {
        return Err(reject("automation_claim_instance_changed"));
    }
    record
        .claim
        .as_ref()
        .ok_or_else(|| reject("automation_claim_not_found"))
}
fn owned(
    tx: &Transaction<'_>,
    record: &StoredTrigger,
    profile: &str,
    fence: &str,
    owner: &ModelExecutionOwner,
) -> Result<(), PersistenceError> {
    let claim = current_claim(record, profile, fence)?;
    if &claim.owner != owner || !owner_alive(owner)? {
        return Err(reject("automation_claim_owner_unconfirmed"));
    }
    if claim.released {
        return Err(reject("automation_claim_released"));
    }
    let raw: String = tx
        .query_row(
            "SELECT record_json FROM personal_profile_writers WHERE profile_id=?1 AND lease_id=?2",
            params![profile, claim.writer_lease_id],
            |row| row.get(0),
        )
        .map_err(database_error("read automation claim writer"))?;
    let writer: PersonalWriterLease =
        serde_json::from_str(&raw).map_err(|_| reject("automation_claim_writer_invalid"))?;
    if writer.owner != claim.owner
        || writer.resource_id != resource_id(&claim.rule_id)
        || writer.state != PersonalWriterState::Active
        || writer.process_mode != PersonalWriterMode::RemoteRequest
        || writer.local_transport_finished
    {
        return Err(reject("automation_claim_writer_unconfirmed"));
    }
    Ok(())
}
fn model_lease(tx: &Transaction<'_>, id: &str) -> Result<ModelExecutionLease, PersistenceError> {
    let raw: String = tx
        .query_row(
            "SELECT json_extract(record_json,'$.value') FROM ai_runtime_entries WHERE record_id=?1",
            [format!("personalModel:lease:{id}")],
            |row| row.get(0),
        )
        .map_err(database_error("read automation model lease"))?;
    let lease: ModelExecutionLease =
        serde_json::from_str(&raw).map_err(|_| reject("automation_claim_lease_invalid"))?;
    if lease.lease_id != id {
        return Err(reject("automation_claim_lease_invalid"));
    }
    Ok(lease)
}
fn model_leases_ended(
    tx: &Transaction<'_>,
    claim: &PersonalAutomationClaim,
) -> Result<(), PersistenceError> {
    for id in &claim.model_lease_ids {
        let lease = model_lease(tx, id)?;
        if lease.snapshot.profile_id != claim.profile_id
            || lease.owner != claim.owner
            || !lease.released
            || (lease.execution_started && lease.processes.is_empty())
        {
            return Err(reject("automation_claim_execution_unconfirmed"));
        }
        for process in &lease.processes {
            if process_alive(process.pid, &process.start_fingerprint)? {
                return Err(reject("automation_claim_process_alive"));
            }
        }
    }
    Ok(())
}

impl SqlitePersistence {
    /// # Errors
    /// 资料、规则修订、占用或实际执行退出不能核实时拒绝；未知状态不重新执行。
    pub fn personal_automation_rpc(
        &mut self,
        method: &str,
        params: Value,
        profile: &str,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("begin personal automation operation"))?;
        let fence = instance(&tx, profile)?;
        let result = match method {
            "personalAutomation/admit" => {
                encode(&actions::admit(&tx, decode(params)?, profile, &fence, now)?)?
            }
            "personalAutomation/prepareExecution" => encode(&actions::prepare_execution(
                &tx,
                decode(params)?,
                profile,
                &fence,
                now,
            )?)?,
            "personalAutomation/bind" => {
                encode(&actions::bind(&tx, decode(params)?, profile, &fence, now)?)?
            }
            "personalAutomation/finish" => encode(&actions::finish(
                &tx,
                decode(params)?,
                profile,
                &fence,
                now,
            )?)?,
            "personalAutomation/listTriggers" => {
                encode(&actions::list(&tx, &decode(params)?, profile)?)?
            }
            "personalAutomation/resolveInterrupted" => encode(&actions::resolve(
                &tx,
                &decode(params)?,
                profile,
                &fence,
                now,
            )?)?,
            _ => return Err(reject("automation_claim_method_unknown")),
        };
        tx.commit()
            .map_err(database_error("commit personal automation operation"))?;
        Ok(result)
    }
}
