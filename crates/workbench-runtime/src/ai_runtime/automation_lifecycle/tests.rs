use super::*;
use crate::ai_runtime::{
    persist_locked,
    tests::{AutomationTestRuntime, automation_fixture},
};
use contexthub_platform_adapter::power_epoch::WakeBoundary;
use serde_json::json;

type Source = Arc<Mutex<Result<PowerEpochSnapshot, String>>>;

fn snapshot(generation: u64, wake: i64) -> PowerEpochSnapshot {
    PowerEpochSnapshot {
        epoch: PowerEpoch {
            last_sleep: generation * 2 + 1,
            last_wake: generation * 2 + 2,
        },
        wake_boundary: Some(WakeBoundary {
            earliest_millis: wake,
            latest_millis: wake,
        }),
        clock_anchor: None,
    }
}

fn setup(runtime: &AutomationTestRuntime, initial: PowerEpochSnapshot) -> Source {
    let source = Arc::new(Mutex::new(Ok(initial)));
    let supplied = Arc::clone(&source);
    *lock(&runtime.manager.automation_power) = AutomationPowerState {
        sample: Arc::new(move || lock(&supplied).clone()),
        observed: None,
        retiring: Vec::new(),
    };
    runtime.manager.reconcile_automation_power().unwrap();
    source
}

fn rule(runtime: &AutomationTestRuntime, id: &str, due: i64) {
    let mut state = lock(&runtime.manager.inner);
    let mut rule = automation_fixture(id);
    rule.next_run_at = Some(due);
    rule.model_id = Some("no-request-may-reach-this-model".into());
    rule.last_run_id = Some("previous-confirmed-run".into());
    rule.last_run_at = Some(7);
    state.persisted.automation_rules.push(rule);
    persist_locked(&mut state).unwrap();
}

#[test]
fn wake_before_shell_resume_skips_only_sleep_deadlines_and_persists_the_receipt() {
    let _guard = crate::supervisor::app_server_test_guard();
    let runtime = AutomationTestRuntime::new();
    let now = unix_epoch_millis().unwrap();
    let source = setup(&runtime, snapshot(1, now - 120_000));
    rule(&runtime, "missed", now - 60_000);
    rule(&runtime, "after-wake", now - 1);
    *lock(&source) = Ok(snapshot(2, now - 30_000));
    let due = runtime.manager.automation_due().unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].rule_id, "after-wake");
    let state = lock(&runtime.manager.inner);
    let skipped = &state.persisted.automation_rules[0];
    assert!(skipped.next_run_at.unwrap() > now);
    assert_eq!(skipped.last_error.as_deref(), Some(MISSED_SLEEP));
    assert_eq!(
        skipped.last_run_id.as_deref(),
        Some("previous-confirmed-run")
    );
    assert_eq!(skipped.last_run_at, Some(7));
    assert!(state.persisted.runs.is_empty());
    drop(state);
    let stored = runtime
        .server
        .request_result("aiLedger/get", &json!({}))
        .unwrap();
    let (_, actual) = crate::ai_runtime::ledger::Baseline::read(&stored["ledger"]).unwrap();
    assert_eq!(actual["automationRules"][0]["lastError"], MISSED_SLEEP);
}

#[test]
fn a_collected_schedule_is_rechecked_after_wake_and_cannot_create_a_model_run() {
    let _guard = crate::supervisor::app_server_test_guard();
    let runtime = AutomationTestRuntime::new();
    let now = unix_epoch_millis().unwrap();
    let source = setup(&runtime, snapshot(1, now - 120_000));
    rule(&runtime, "collected", now - 60_000);
    let due = runtime.manager.automation_due().unwrap();
    *lock(&source) = Ok(snapshot(2, now - 30_000));
    assert_eq!(
        runtime
            .manager
            .execute_automation_candidate(&due[0].rule_id, Some(&due[0]))
            .unwrap_err(),
        "automation_schedule_changed"
    );
    let state = lock(&runtime.manager.inner);
    assert!(state.persisted.runs.is_empty());
    assert!(state.persisted.conversations.is_empty());
    assert_eq!(
        state.persisted.automation_rules[0].last_error.as_deref(),
        Some(MISSED_SLEEP)
    );
}

#[test]
fn ordinary_awake_delay_does_not_turn_a_due_schedule_into_a_sleep_skip() {
    let _guard = crate::supervisor::app_server_test_guard();
    let runtime = AutomationTestRuntime::new();
    let now = unix_epoch_millis().unwrap();
    setup(&runtime, snapshot(1, now - 86_400_000));
    rule(&runtime, "awake", now - 3_600_000);
    assert_eq!(
        runtime.manager.automation_due().unwrap()[0].rule_id,
        "awake"
    );
    let state = lock(&runtime.manager.inner);
    assert_eq!(
        state.persisted.automation_rules[0].next_run_at,
        Some(now - 3_600_000)
    );
    assert_ne!(
        state.persisted.automation_rules[0].last_error.as_deref(),
        Some(MISSED_SLEEP)
    );
}

#[test]
fn unknown_power_records_do_not_advance_the_plan_or_start_a_model() {
    let _guard = crate::supervisor::app_server_test_guard();
    let runtime = AutomationTestRuntime::new();
    let now = unix_epoch_millis().unwrap();
    let source = setup(&runtime, snapshot(1, now - 120_000));
    rule(&runtime, "unknown", now - 60_000);
    *lock(&source) = Err("automation_power_epoch_unconfirmed".into());
    assert_eq!(
        runtime.manager.automation_due().err().unwrap(),
        "automation_power_epoch_unconfirmed"
    );
    let state = lock(&runtime.manager.inner);
    assert!(state.persisted.runs.is_empty());
    assert_eq!(
        state.persisted.automation_rules[0].next_run_at,
        Some(now - 60_000)
    );
    assert_eq!(
        state.persisted.automation_rules[0].last_error.as_deref(),
        Some("automation_power_epoch_unconfirmed")
    );
}

#[test]
fn queued_admission_expires_on_sleep_but_started_work_is_never_reissued() {
    let source = Arc::new(Mutex::new(Ok(snapshot(1, 10))));
    let supplied = Arc::clone(&source);
    let ticket = AutomationTicket(Arc::new(TicketState {
        sample: Arc::new(move || lock(&supplied).clone()),
        epoch: snapshot(1, 10).epoch,
        phase: Mutex::new(AdmissionPhase::Pending),
        settlement: Mutex::new(Settlement::default()),
    }));
    let queued = ticket.clone();
    *lock(&source) = Ok(snapshot(2, 20));
    assert_eq!(queued.validate_pending().unwrap_err(), MISSED_SLEEP);
    assert_eq!(queued.begin_model_request().unwrap_err(), MISSED_SLEEP);
    assert!(matches!(*lock(&queued.0.phase), AdmissionPhase::Failed(_)));
    *lock(&source) = Ok(snapshot(1, 10));
    assert_eq!(queued.validate_pending().unwrap_err(), MISSED_SLEEP);
    let active = AutomationTicket(Arc::new(TicketState {
        sample: Arc::clone(&ticket.0.sample),
        epoch: snapshot(1, 10).epoch,
        phase: Mutex::new(AdmissionPhase::Pending),
        settlement: Mutex::new(Settlement::default()),
    }));
    active.begin_model_request().unwrap();
    *lock(&source) = Ok(snapshot(3, 30));
    active.validate_pending().unwrap();
    assert!(matches!(*lock(&active.0.phase), AdmissionPhase::Started));
}

#[test]
fn wake_reconciliation_cannot_overwrite_another_hosts_rule_revision() {
    let _guard = crate::supervisor::app_server_test_guard();
    let runtime = AutomationTestRuntime::new();
    let now = unix_epoch_millis().unwrap();
    let initial = snapshot(1, now - 120_000);
    let source = setup(&runtime, initial);
    rule(&runtime, "shared", now - 60_000);
    let stored = runtime
        .server
        .request_result("aiLedger/get", &json!({}))
        .unwrap();
    let record = stored["ledger"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == "automationRules:shared")
        .unwrap();
    let mut changed = record["value"].clone();
    changed["value"]["name"] = json!("另一宿主已修改");
    runtime.server.request_result("aiLedger/apply", &json!({
        "commandId":"other-host-rule-edit", "records":[{"collection":"entries", "id":record["id"],
        "position":record["position"], "expectedDigest":record["digest"], "value":changed}]
    })).unwrap();
    *lock(&source) = Ok(snapshot(2, now - 30_000));
    assert!(
        runtime
            .manager
            .automation_due()
            .err()
            .unwrap()
            .contains("ai_ledger_record_conflict")
    );
    assert_eq!(
        lock(&runtime.manager.automation_power).observed,
        Some(initial)
    );
    let state = lock(&runtime.manager.inner);
    assert!(state.persisted.runs.is_empty());
    assert_eq!(
        state.persisted.automation_rules[0].next_run_at,
        Some(now - 60_000)
    );
    drop(state);
    let stored = runtime
        .server
        .request_result("aiLedger/get", &json!({}))
        .unwrap();
    let (_, actual) = crate::ai_runtime::ledger::Baseline::read(&stored["ledger"]).unwrap();
    assert_eq!(actual["automationRules"][0]["name"], "另一宿主已修改");
    assert_eq!(actual["automationRules"][0]["nextRunAt"], now - 60_000);
}

#[test]
fn changing_the_clock_after_wake_does_not_skip_a_normal_post_wake_plan() {
    use contexthub_platform_adapter::power_epoch::PowerClockAnchor;
    let _guard = crate::supervisor::app_server_test_guard();
    let runtime = AutomationTestRuntime::new();
    let now = unix_epoch_millis().unwrap();
    let anchor_wall = now - 120_000;
    let mut initial = snapshot(1, now - 180_000);
    initial.clock_anchor = Some(PowerClockAnchor {
        interrupt_before: 100,
        interrupt_after: 100,
        wall_unix_100ns: u64::try_from(anchor_wall).unwrap() * 10_000,
    });
    let source = setup(&runtime, initial);
    rule(&runtime, "normal-after-wake", now - 30_000);
    let mut resumed = snapshot(2, now - 1);
    resumed.epoch = PowerEpoch {
        last_sleep: 200,
        last_wake: 600_000_100,
    };
    resumed.clock_anchor = Some(PowerClockAnchor {
        interrupt_before: 1_200_000_100,
        interrupt_after: 1_200_000_100,
        wall_unix_100ns: u64::try_from(now).unwrap() * 10_000,
    });
    *lock(&source) = Ok(resumed);
    assert_eq!(
        runtime.manager.automation_due().unwrap()[0].rule_id,
        "normal-after-wake"
    );
    assert_ne!(
        lock(&runtime.manager.inner).persisted.automation_rules[0]
            .last_error
            .as_deref(),
        Some(MISSED_SLEEP)
    );
}

#[test]
fn maintenance_keeps_the_scheduler_closed_until_its_actual_guard_is_released() {
    let _guard = crate::supervisor::app_server_test_guard();
    let runtime = AutomationTestRuntime::new();
    let now = unix_epoch_millis().unwrap();
    let source = setup(&runtime, snapshot(1, now - 120_000));
    rule(&runtime, "during-maintenance", now - 60_000);
    let prepared = runtime.server.request_result("personalData/backupPrepare", &json!({
        "selection":{"includeWorkspaceFiles":[]}, "clientOperationId":"sleep-maintenance-fixture"
    })).unwrap();
    let guard = runtime
        .manager
        .begin_profile_maintenance(
            &runtime.root,
            prepared["operationId"].as_str().unwrap(),
            prepared["previewDigest"].as_str().unwrap(),
        )
        .unwrap();
    *lock(&source) = Ok(snapshot(2, now - 30_000));
    assert!(runtime.manager.automation_due().is_err());
    assert!(
        AutomationWriter::prepare(runtime.server.clone(), "foreign_host_scheduler")
            .err()
            .unwrap()
            .contains("personal_data_maintenance_active")
    );
    assert!(lock(&runtime.manager.inner).persisted.runs.is_empty());
    guard.finish().unwrap();
    assert!(runtime.manager.automation_due().unwrap().is_empty());
    let state = lock(&runtime.manager.inner);
    assert_eq!(
        state.persisted.automation_rules[0].last_error.as_deref(),
        Some("资料维护期间错过本次任务，未补跑。")
    );
}

#[test]
fn an_ambiguous_wake_boundary_keeps_the_original_plan() {
    let mut rule = automation_fixture("boundary");
    let now = unix_epoch_millis().unwrap();
    rule.next_run_at = Some(now - 30_000);
    let before = serde_json::to_value(&rule).unwrap();
    let mut sample = snapshot(1, now - 30_001);
    sample.wake_boundary.as_mut().unwrap().latest_millis = now - 29_999;
    assert_eq!(
        skip_sleep_deadlines(std::slice::from_mut(&mut rule), sample).unwrap_err(),
        "automation_wake_boundary_unconfirmed"
    );
    assert_eq!(serde_json::to_value(rule).unwrap(), before);
}

#[test]
fn another_hosts_maintenance_observes_the_writer_until_both_execution_receipts_are_saved() {
    let _guard = crate::supervisor::app_server_test_guard();
    let runtime = AutomationTestRuntime::new();
    let now = unix_epoch_millis().unwrap();
    setup(&runtime, snapshot(1, now - 120_000));
    let ticket = runtime.manager.reconcile_automation_power().unwrap();
    let writer = AutomationWriter::prepare(runtime.server.clone(), "async_settlement").unwrap();
    let lease_id = writer.lease.lease_id.clone();
    ticket.attach_writer(writer);
    ticket.begin_native_process();
    ticket.finish_synchronous(&runtime.manager, None).unwrap();
    let operation = runtime
        .server
        .request_result(
            "personalData/backupPrepare",
            &json!({
                "selection":{"includeWorkspaceFiles":[]},"clientOperationId":"sleep-async-writer"
            }),
        )
        .unwrap();
    let owner = crate::model_connections::execution_owner().unwrap();
    let fence = runtime.server.request_result("personalMaintenance/acquire", &json!({
        "operationId":operation["operationId"], "previewDigest":operation["previewDigest"], "owner":owner
    })).unwrap();
    let params = json!({"fenceId":fence["fenceId"],"owner":owner});
    let pending = runtime
        .server
        .request_result("personalMaintenance/drainStatus", &params)
        .unwrap();
    assert!(
        pending["activeWriters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["leaseId"] == lease_id)
    );
    ticket.finish_terminal(&runtime.manager).unwrap();
    let awaiting_exit = runtime
        .server
        .request_result("personalMaintenance/drainStatus", &params)
        .unwrap();
    assert!(
        awaiting_exit["activeWriters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["leaseId"] == lease_id)
    );
    ticket.finish_native_process(&runtime.manager).unwrap();
    let settled = runtime
        .server
        .request_result("personalMaintenance/drainStatus", &params)
        .unwrap();
    let leases = |value: &serde_json::Value| {
        value["activeWriters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["leaseId"].as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let mut expected = leases(&pending);
    assert!(expected.remove(&lease_id));
    assert_eq!(leases(&settled), expected);
    runtime
        .server
        .request_result("personalMaintenance/release", &params)
        .unwrap();
}
