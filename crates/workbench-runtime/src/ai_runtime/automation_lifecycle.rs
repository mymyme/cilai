use super::{AiRuntimeManager, AutomationRule, lock, persist_locked_or_restore, unix_epoch_millis};
use contexthub_platform_adapter::power_epoch::{PowerEpoch, PowerEpochSnapshot};
use std::sync::{Arc, Mutex};
#[cfg(test)]
mod tests;

type Sample = Arc<dyn Fn() -> Result<PowerEpochSnapshot, String> + Send + Sync>;
pub(super) const MISSED_SLEEP: &str = "automation_missed_during_sleep";

pub(super) struct AutomationPowerState {
    sample: Sample,
    observed: Option<PowerEpochSnapshot>,
    retiring: Vec<AutomationTicket>,
}

impl Default for AutomationPowerState {
    fn default() -> Self {
        Self {
            sample: Arc::new(|| {
                contexthub_platform_adapter::power_epoch::snapshot()
                    .map_err(|_| "automation_power_epoch_unconfirmed".into())
            }),
            observed: None,
            retiring: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub(super) struct AutomationTicket(Arc<TicketState>);
struct TicketState {
    sample: Sample,
    epoch: PowerEpoch,
    phase: Mutex<AdmissionPhase>,
    settlement: Mutex<Settlement>,
}

struct Settlement {
    writer: Option<AutomationWriter>,
    claim: Option<super::automation_claim::RuleClaim>,
    run_id: Option<String>,
    failure: Option<String>,
    preparation: PreparationState,
    synchronous_finished: bool,
    terminal_finished: bool,
    native_exit_confirmed: bool,
    phase: SettlementPhase,
    executions: Vec<Arc<crate::model_connections::ExecutionScope>>,
}

impl Default for Settlement {
    fn default() -> Self {
        Self {
            writer: None,
            claim: None,
            run_id: None,
            failure: None,
            preparation: PreparationState::Known,
            synchronous_finished: false,
            terminal_finished: false,
            native_exit_confirmed: true,
            phase: SettlementPhase::Pending,
            executions: Vec::new(),
        }
    }
}

enum SettlementPhase {
    Pending,
    Finishing,
}

enum PreparationState {
    Known,
    Unconfirmed,
}

enum AdmissionPhase {
    Pending,
    Started,
    Failed(String),
}

impl AutomationTicket {
    #[cfg(test)]
    pub(super) fn attach_writer(&self, writer: AutomationWriter) {
        lock(&self.0.settlement).writer = Some(writer);
    }

    pub(super) fn attach_claim(&self, claim: super::automation_claim::RuleClaim) {
        lock(&self.0.settlement).claim = Some(claim);
    }

    pub(super) fn prepare_execution(
        &self,
        manager: &AiRuntimeManager,
        params: contexthub_protocol::ModelExecutionPrepareParams,
    ) -> Result<Arc<crate::model_connections::ExecutionScope>, String> {
        self.validate_pending()?;
        let claim = {
            let mut state = lock(&self.0.settlement);
            let claim = state.claim.clone().ok_or("automation_claim_required")?;
            state.preparation = PreparationState::Unconfirmed;
            claim
        };
        let execution = claim.prepare_execution(manager, params).inspect_err(|_| {
            if claim
                .report_unconfirmed(manager, "automation_model_preparation_unconfirmed")
                .is_err()
            {
                eprintln!("automation_unknown_record_unconfirmed");
            }
        })?;
        let mut state = lock(&self.0.settlement);
        state.executions.push(Arc::clone(&execution));
        state.preparation = PreparationState::Known;
        Ok(execution)
    }

    pub(super) fn bind_run(&self, manager: &AiRuntimeManager, run_id: &str) -> Result<(), String> {
        let claim = {
            let mut state = lock(&self.0.settlement);
            let claim = state.claim.clone().ok_or("automation_claim_required")?;
            state.preparation = PreparationState::Unconfirmed;
            claim
        };
        claim.bind(manager, run_id).inspect_err(|_| {
            if claim
                .report_unconfirmed(manager, "automation_run_binding_unconfirmed")
                .is_err()
            {
                eprintln!("automation_unknown_record_unconfirmed");
            }
        })?;
        let mut state = lock(&self.0.settlement);
        state.run_id = Some(run_id.into());
        state.preparation = PreparationState::Known;
        Ok(())
    }

    pub(super) fn begin_native_process(&self) {
        lock(&self.0.settlement).native_exit_confirmed = false;
    }

    pub(super) fn finish_native_process(&self, manager: &AiRuntimeManager) -> Result<(), String> {
        lock(&self.0.settlement).native_exit_confirmed = true;
        self.release_finished_writer(manager)
    }

    pub(super) fn finish_synchronous(
        &self,
        manager: &AiRuntimeManager,
        error: Option<&str>,
    ) -> Result<(), String> {
        {
            let mut settlement = lock(&self.0.settlement);
            settlement.synchronous_finished = true;
            settlement.terminal_finished |= error.is_some();
            settlement.failure = error.map(str::to_owned);
        }
        self.release_finished_writer(manager)
    }

    pub(super) fn finish_terminal(&self, manager: &AiRuntimeManager) -> Result<(), String> {
        lock(&self.0.settlement).terminal_finished = true;
        self.release_finished_writer(manager)
    }

    fn release_finished_writer(&self, manager: &AiRuntimeManager) -> Result<(), String> {
        let pending = {
            let mut settlement = lock(&self.0.settlement);
            if !settlement.synchronous_finished
                || !settlement.terminal_finished
                || !settlement.native_exit_confirmed
                || matches!(settlement.phase, SettlementPhase::Finishing)
                || matches!(settlement.preparation, PreparationState::Unconfirmed)
            {
                return Ok(());
            }
            if settlement.writer.is_none() && settlement.claim.is_none() {
                return Ok(());
            }
            settlement.phase = SettlementPhase::Finishing;
            (
                settlement.writer.clone(),
                settlement.executions.clone(),
                settlement.claim.clone(),
                settlement.run_id.clone(),
                settlement.failure.clone(),
            )
        };
        let unknown_claim = pending.2.clone();
        let result = pending
            .1
            .iter()
            .try_for_each(|execution| execution.finish_release())
            .and_then(|()| {
                if let Some(claim) = pending.2 {
                    let (outcome, reason) =
                        claim_outcome(manager, pending.3.as_deref(), pending.4)?;
                    claim.finish(manager, outcome, reason.as_deref())?;
                }
                pending.0.map_or(Ok(()), |writer| writer.finish(manager))
            });
        if result.is_err()
            && let Some(claim) = unknown_claim
            && claim
                .report_unconfirmed(manager, "automation_previous_run_outcome_unconfirmed")
                .is_err()
        {
            eprintln!("automation_unknown_record_unconfirmed");
        }
        let mut settlement = lock(&self.0.settlement);
        settlement.phase = SettlementPhase::Pending;
        if result.is_ok() {
            settlement.writer = None;
            settlement.claim = None;
            settlement.executions.clear();
        } else {
            settlement.preparation = PreparationState::Unconfirmed;
        }
        result
    }
    pub(super) fn validate_pending(&self) -> Result<(), String> {
        self.validate(&mut lock(&self.0.phase))
    }

    fn validate(&self, phase: &mut AdmissionPhase) -> Result<(), String> {
        match phase {
            AdmissionPhase::Failed(error) => return Err(error.clone()),
            AdmissionPhase::Started => return Ok(()),
            AdmissionPhase::Pending => {}
        }
        let result = (self.0.sample)().and_then(|snapshot| {
            (snapshot.epoch == self.0.epoch)
                .then_some(())
                .ok_or_else(|| MISSED_SLEEP.to_owned())
        });
        if let Err(error) = &result {
            *phase = AdmissionPhase::Failed(error.clone());
        }
        result
    }

    pub(super) fn begin_model_request(&self) -> Result<(), String> {
        let mut phase = lock(&self.0.phase);
        self.validate(&mut phase)?;
        *phase = AdmissionPhase::Started;
        Ok(())
    }

    pub(super) fn failure(&self) -> Option<String> {
        match &*lock(&self.0.phase) {
            AdmissionPhase::Failed(error) => Some(error.clone()),
            AdmissionPhase::Pending | AdmissionPhase::Started => None,
        }
    }
}

fn claim_outcome(
    manager: &AiRuntimeManager,
    run_id: Option<&str>,
    failure: Option<String>,
) -> Result<
    (
        contexthub_protocol::PersonalAutomationFinishOutcome,
        Option<String>,
    ),
    String,
> {
    use contexthub_protocol::PersonalAutomationFinishOutcome as Outcome;
    let Some(run_id) = run_id else {
        return Ok((Outcome::Failed, failure));
    };
    let state = lock(&manager.inner);
    if super::persistence_outcome_unknown(&state)
        || state.persisted.permission_requests.iter().any(|request| {
            request.run_id == run_id
                && request.application_write.as_ref().is_some_and(|write| {
                    matches!(
                        write.execution_status.as_deref(),
                        Some("outcome_unknown" | "submitting")
                    )
                })
        })
    {
        return Err("automation_previous_run_outcome_unconfirmed".into());
    }
    let run = state
        .persisted
        .runs
        .iter()
        .find(|run| run.run_id == run_id)
        .ok_or("automation_run_missing")?;
    if run.error_reason.as_deref().is_some_and(|reason| {
        [
            "unconfirmed",
            "uncertain",
            "outcome_unknown",
            "response_invalid",
            "response_missing",
        ]
        .iter()
        .any(|part| reason.contains(part))
    }) {
        return Err("automation_previous_run_outcome_unconfirmed".into());
    }
    let outcome = match run.status.as_str() {
        "completed" => Outcome::Completed,
        "failed" => Outcome::Failed,
        "cancelled" | "terminated" => Outcome::Cancelled,
        _ => return Err("automation_run_not_terminal".into()),
    };
    Ok((outcome, run.error_reason.clone().or(failure)))
}

pub(super) struct AutomationInvocation<'a> {
    pub rule: &'a AutomationRule,
    pub ticket: &'a AutomationTicket,
}

#[derive(Clone)]
pub(super) struct ScheduledAutomation {
    pub rule_id: String,
    pub due_at: i64,
    pub rule_version: u32,
}

#[derive(Clone)]
pub(super) struct AutomationWriter {
    server: crate::supervisor::AppServerSupervisor,
    lease: contexthub_protocol::PersonalWriterLease,
}

impl AutomationWriter {
    pub(super) fn prepare(
        server: crate::supervisor::AppServerSupervisor,
        stage: &str,
    ) -> Result<Self, String> {
        let lease: contexthub_protocol::PersonalWriterLease = serde_json::from_value(server.request_result(
            "profileWriter/prepare", &serde_json::json!({
                "clientOperationId":format!("automation-{}",uuid::Uuid::new_v4().simple()),
                "kind":"local_operation", "resourceId":crate::persistent_threads::digest(&serde_json::json!({"automationStage":stage})),
                "resourceRevision":"1", "owner":crate::model_connections::execution_owner()?,
                "launchProtocol":1, "processMode":"remote_request"
            }))?).map_err(|_| "automation_writer_unconfirmed")?;
        if lease.state != contexthub_protocol::PersonalWriterState::Active
            || lease.process_mode != contexthub_protocol::PersonalWriterMode::RemoteRequest
        {
            return Err("automation_writer_unconfirmed".into());
        }
        Ok(Self { server, lease })
    }

    pub(super) fn finish(self, manager: &AiRuntimeManager) -> Result<(), String> {
        if super::persistence_outcome_unknown(&lock(&manager.inner)) {
            return Err("automation_persistence_unconfirmed".into());
        }
        let released: contexthub_protocol::PersonalWriterLease = serde_json::from_value(self.server.request_result(
            "profileWriter/release", &serde_json::json!({"leaseId":self.lease.lease_id,
                "owner":self.lease.owner,"localTransportFinished":true,"remoteOutcomeUnconfirmed":false}))?)
            .map_err(|_| "automation_writer_unconfirmed")?;
        if released.state != contexthub_protocol::PersonalWriterState::Released {
            return Err("automation_writer_unconfirmed".into());
        }
        Ok(())
    }
}

impl AiRuntimeManager {
    #[cfg(test)]
    pub(crate) fn set_automation_power_test_source(
        &self,
        sample: impl Fn() -> Result<PowerEpochSnapshot, String> + Send + Sync + 'static,
    ) {
        *lock(&self.automation_power) = AutomationPowerState {
            sample: Arc::new(sample),
            observed: None,
            retiring: Vec::new(),
        };
    }

    pub(super) fn reconcile_automation_power(&self) -> Result<AutomationTicket, String> {
        let _operation = self.begin_capability_operation()?;
        let mut power = lock(&self.automation_power);
        let mut snapshot = (power.sample)().inspect_err(|error| {
            self.record_automation_power_failure(error);
        })?;
        if power.observed.map(|observed| observed.epoch) != Some(snapshot.epoch) {
            if let Some(anchor) = power.observed.and_then(|observed| observed.clock_anchor)
                && snapshot.clock_anchor.is_some()
            {
                snapshot.wake_boundary = Some(
                    anchor
                        .subsequent_wake_boundary(snapshot.epoch.last_wake)
                        .map_err(|_| {
                            self.record_automation_power_failure(
                                "automation_wake_boundary_unconfirmed",
                            );
                            "automation_wake_boundary_unconfirmed"
                        })?,
                );
            }
            let mut state = lock(&self.inner);
            super::ensure_profile_admission(&state)?;
            let before = state.persisted.clone();
            let mut rules = state.persisted.automation_rules.clone();
            let changed = match skip_sleep_deadlines(&mut rules, snapshot) {
                Ok(changed) => changed,
                Err(error) => {
                    drop(state);
                    self.record_automation_power_failure(&error);
                    return Err(error);
                }
            };
            if changed {
                let writer = AutomationWriter::prepare(
                    state.server.clone().ok_or("ai_runtime_not_initialized")?,
                    "sleep_reconciliation",
                )?;
                state.persisted.automation_rules = rules;
                let result = persist_locked_or_restore(&mut state, before);
                drop(state);
                writer.finish(self)?;
                result?;
            }
        }
        power.observed = Some(snapshot);
        Ok(AutomationTicket(Arc::new(TicketState {
            sample: Arc::clone(&power.sample),
            epoch: snapshot.epoch,
            phase: Mutex::new(AdmissionPhase::Pending),
            settlement: Mutex::new(Settlement::default()),
        })))
    }

    fn record_automation_power_failure(&self, reason: &str) {
        let Ok(server) = self.app_server() else {
            return;
        };
        let Ok(writer) = AutomationWriter::prepare(server, "power_status") else {
            return;
        };
        let mut state = lock(&self.inner);
        if super::ensure_profile_admission(&state).is_err() {
            drop(state);
            let _ = writer.finish(self);
            return;
        }
        let before = state.persisted.clone();
        let mut changed = false;
        for rule in &mut state.persisted.automation_rules {
            if rule.enabled && rule.last_error.as_deref() != Some(reason) {
                rule.last_error = Some(reason.into());
                changed = true;
            }
        }
        if changed && let Err(error) = persist_locked_or_restore(&mut state, before) {
            eprintln!("automation_power_status_unconfirmed error={error}");
        }
        drop(state);
        if let Err(error) = writer.finish(self) {
            eprintln!("automation_power_status_unconfirmed error={error}");
        }
    }

    pub(super) fn automation_due(&self) -> Result<Vec<ScheduledAutomation>, String> {
        let _serial = lock(&self.automation_lifecycle);
        self.reconcile_automation_power()?;
        let now = unix_epoch_millis()?;
        let state = lock(&self.inner);
        super::ensure_profile_admission(&state)?;
        Ok(state
            .persisted
            .automation_rules
            .iter()
            .filter_map(|rule| {
                let due_at = rule.next_run_at?;
                (rule.enabled && rule.model_id.is_some() && due_at <= now).then(|| {
                    ScheduledAutomation {
                        rule_id: rule.automation_rule_id.clone(),
                        due_at,
                        rule_version: rule.rule_version,
                    }
                })
            })
            .collect())
    }

    pub(super) fn settle_automation_not_started(&self, run_id: &str, reason: &str) {
        if reason != MISSED_SLEEP && reason != "automation_power_epoch_unconfirmed" {
            return;
        }
        let mut state = lock(&self.inner);
        let rule_id = state
            .persisted
            .runs
            .iter_mut()
            .find(|run| run.run_id == run_id)
            .and_then(|run| {
                "cancelled".clone_into(&mut run.status);
                run.error_reason = Some(reason.into());
                run.automation_rule_id.clone()
            });
        if let Some(rule) = rule_id.and_then(|id| {
            state
                .persisted
                .automation_rules
                .iter_mut()
                .find(|rule| rule.automation_rule_id == id)
        }) {
            rule.last_error = Some(reason.into());
        }
        super::persist_or_log(&mut state, "automation_not_started");
    }

    pub(super) fn retain_automation_shutdown_tickets(&self, tickets: Vec<AutomationTicket>) {
        lock(&self.automation_power).retiring.extend(tickets);
    }

    pub(super) fn finish_automation_shutdown_tickets(&self) {
        if lock(&self.inner).harness_retirement_error.is_some() {
            return;
        }
        let tickets = std::mem::take(&mut lock(&self.automation_power).retiring);
        for ticket in tickets {
            if let Err(error) = ticket.finish_terminal(self) {
                lock(&self.inner).harness_retirement_error = Some(error);
            }
        }
    }
}

fn skip_sleep_deadlines(
    rules: &mut [AutomationRule],
    snapshot: PowerEpochSnapshot,
) -> Result<bool, String> {
    let Some(wake) = snapshot.wake_boundary else {
        return Ok(false);
    };
    if wake.earliest_millis > wake.latest_millis || wake.latest_millis > unix_epoch_millis()? {
        return Err("automation_wake_boundary_unconfirmed".into());
    }
    if rules.iter().any(|rule| {
        rule.enabled
            && rule
                .next_run_at
                .is_some_and(|due| due > wake.earliest_millis && due <= wake.latest_millis)
    }) {
        return Err("automation_wake_boundary_unconfirmed".into());
    }
    let mut changed = false;
    for rule in rules {
        if rule.enabled
            && rule
                .next_run_at
                .is_some_and(|due| due <= wake.earliest_millis)
        {
            rule.next_run_at = super::next_run_after(
                &rule.schedule_kind,
                rule.interval_minutes,
                rule.time_of_day_minutes,
                &super::effective_weekdays(&rule.weekdays, rule.weekday),
                wake.latest_millis,
            );
            rule.last_error = Some(MISSED_SLEEP.into());
            rule.updated_at = wake.latest_millis;
            changed = true;
        }
    }
    Ok(changed)
}
