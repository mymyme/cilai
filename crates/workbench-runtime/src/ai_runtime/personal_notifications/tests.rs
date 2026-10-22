use super::*;
use crate::ai_runtime::{AgentRun, PermissionRequest, persist_locked};
use crate::supervisor::AppServerSupervisor;
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    manager: AiRuntimeManager,
    server: AppServerSupervisor,
    root: PathBuf,
    data: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl Fixture {
    fn new() -> Self {
        let guard = crate::supervisor::app_server_test_guard();
        let root = crate::ai_runtime::model_test_fixture::fixture_root("personal-notifications");
        let data = crate::ai_runtime::model_test_fixture::profile_data_root(&root);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap_or_else(|error| {
                panic!("notification fixture start: {error}; {:?}", server.status())
            });
        let manager = AiRuntimeManager::new();
        manager.initialize(&data, &server).unwrap();
        Self {
            manager,
            server,
            root,
            data,
            _guard: guard,
        }
    }
    fn enable(&self) -> NotificationPreferences {
        let snapshot = self.manager.personal_notification_snapshot().unwrap();
        self.manager
            .save_personal_notification_preferences(&NotificationPreferencesSave {
                profile_id: snapshot.profile_id,
                expected_revision: snapshot.preferences.revision,
                enabled: true,
                completed: true,
                failed: true,
                action_required: true,
                sound: false,
            })
            .unwrap()
    }
    fn seed(&self) {
        self.manager
            .notification_write(|state| {
                let mut conversation = crate::ai_runtime::tests::fixture_conversation("agent_1");
                "conversation_1".clone_into(&mut conversation.conversation_id);
                state.persisted.conversations.push(conversation);
                state.persisted.runs.push(run("completed"));
                persist_locked(state)
            })
            .unwrap();
    }
    fn claim(&self) -> NotificationClaim {
        let snapshot = self.manager.personal_notification_snapshot().unwrap();
        self.manager
            .claim_personal_notification(NotificationClaimParams {
                profile_id: snapshot.profile_id,
                source_epoch: snapshot.source_epoch,
                event_id: snapshot.events[0].event_id.clone(),
            })
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.manager.shutdown();
        self.server.shutdown().unwrap();
        if std::thread::panicking() {
            eprintln!("notification_fixture_retained={}", self.root.display());
        } else {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }
}
fn run(status: &str) -> AgentRun {
    serde_json::from_value(json!({
        "runId":"run_1","conversationId":"conversation_1","agentId":"agent_1",
        "inputMessageId":"input_1","provider":"codex","model":null,"status":status,
        "pid":null,"startedAt":1,"completedAt":2,"retryOfRunId":null,
        "output":"private-model-body","errorReason":"private-error","logTail":["private-command"]
    }))
    .unwrap()
}
fn report(claim: &NotificationClaim, outcome: DeliveryOutcome) -> NotificationReportParams {
    NotificationReportParams {
        profile_id: claim.delivery.profile_id.clone(),
        notification_id: claim.delivery.notification_id.clone(),
        outcome,
        reason_code: Some("notification_outcome_unconfirmed".into()),
    }
}

#[test]
fn preferences_default_off_and_claim_is_durable_across_restart_without_replay() {
    let fixture = Fixture::new();
    fixture.seed();
    let snapshot = fixture.manager.personal_notification_snapshot().unwrap();
    assert!(!snapshot.preferences.enabled);
    assert!(!snapshot.events[0].enabled_for_delivery);
    assert_eq!(
        fixture
            .manager
            .claim_personal_notification(NotificationClaimParams {
                profile_id: snapshot.profile_id.clone(),
                source_epoch: snapshot.source_epoch.clone(),
                event_id: snapshot.events[0].event_id.clone(),
            })
            .unwrap_err(),
        "personal_notification_disabled"
    );
    assert!(
        lock(&fixture.manager.inner)
            .persisted
            .personal_notifications
            .deliveries
            .is_empty()
    );
    let metadata = serde_json::to_string(&snapshot).unwrap();
    assert!(!metadata.contains("private-model-body"));
    assert!(!metadata.contains("private-error"));
    assert!(!metadata.contains("private-command"));
    fixture.enable();
    let claimed = fixture.claim();
    assert!(claimed.may_deliver);
    assert_eq!(claimed.delivery.outcome, DeliveryOutcome::Claimed);
    assert!(!fixture.claim().may_deliver);
    fixture
        .manager
        .report_personal_notification(report(&claimed, DeliveryOutcome::Unknown))
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .open_personal_notification(
                &claimed.delivery.profile_id,
                &claimed.delivery.notification_id
            )
            .unwrap(),
        claimed.delivery.event.target
    );
    fixture.manager.shutdown();
    fixture
        .manager
        .initialize(&fixture.data, &fixture.server)
        .unwrap();
    let recovered = fixture.claim();
    assert!(!recovered.may_deliver);
    assert_eq!(recovered.delivery.outcome, DeliveryOutcome::Unknown);
    assert!(
        fixture
            .manager
            .report_personal_notification(report(&claimed, DeliveryOutcome::Shown))
            .unwrap_err()
            .contains("outcome_frozen")
    );
    assert_eq!(
        lock(&fixture.manager.inner)
            .persisted
            .personal_notifications
            .deliveries
            .len(),
        1
    );
}

#[test]
fn foreign_profile_old_revision_and_unconfirmed_reason_cannot_write_or_open() {
    let fixture = Fixture::new();
    fixture.seed();
    fixture.enable();
    let snapshot = fixture.manager.personal_notification_snapshot().unwrap();
    let prior_revision = NotificationPreferencesSave {
        profile_id: snapshot.profile_id.clone(),
        expected_revision: 0,
        enabled: false,
        completed: false,
        failed: false,
        action_required: false,
        sound: false,
    };
    assert!(
        fixture
            .manager
            .save_personal_notification_preferences(&prior_revision)
            .unwrap_err()
            .contains("preferences_changed")
    );
    let claimed = fixture.claim();
    let mut request = report(&claimed, DeliveryOutcome::Failed);
    request.reason_code = Some("private-native-error".into());
    assert_eq!(
        fixture
            .manager
            .report_personal_notification(request)
            .unwrap_err(),
        "personal_notification_report_invalid"
    );
    let mut foreign = report(&claimed, DeliveryOutcome::Unknown);
    foreign.profile_id = "0".repeat(32);
    assert!(
        fixture
            .manager
            .report_personal_notification(foreign)
            .unwrap_err()
            .contains("profile_changed")
    );
    assert!(
        fixture
            .manager
            .open_personal_notification(&"0".repeat(32), &claimed.delivery.notification_id)
            .is_err()
    );
    let mut state = lock(&fixture.manager.inner);
    state
        .persisted
        .personal_notifications
        .deliveries
        .get_mut(&snapshot.events[0].event_id)
        .unwrap()
        .profile_id = "0".repeat(32);
    drop(state);
    let snapshot = fixture.manager.personal_notification_snapshot().unwrap();
    assert!(
        fixture
            .manager
            .claim_personal_notification(NotificationClaimParams {
                profile_id: snapshot.profile_id,
                source_epoch: snapshot.source_epoch,
                event_id: snapshot.events[0].event_id.clone()
            })
            .unwrap_err()
            .contains("profile_changed")
    );
}

#[test]
fn maintenance_blocks_new_claim_without_registering_a_delivery() {
    let fixture = Fixture::new();
    fixture.seed();
    fixture.enable();
    let snapshot = fixture.manager.personal_notification_snapshot().unwrap();
    lock(&fixture.manager.inner).maintenance_fence =
        Some(contexthub_protocol::PersonalMaintenanceFence {
            fence_id: "maintenance_".into(),
            operation_id: "notification-admission-component".into(),
            profile_id: snapshot.profile_id.clone(),
            instance_fence: "fixture".into(),
            owner: crate::model_connections::execution_owner().unwrap(),
            state: contexthub_protocol::PersonalMaintenanceState::Draining,
            created_at: 1,
            updated_at: 1,
        });
    let result = fixture
        .manager
        .claim_personal_notification(NotificationClaimParams {
            profile_id: snapshot.profile_id,
            source_epoch: snapshot.source_epoch,
            event_id: snapshot.events[0].event_id.clone(),
        });
    lock(&fixture.manager.inner).maintenance_fence = None;
    assert!(result.unwrap_err().contains("maintenance"));
    assert!(
        lock(&fixture.manager.inner)
            .persisted
            .personal_notifications
            .deliveries
            .is_empty()
    );
}

#[test]
fn automation_preference_filters_success_but_keeps_failures_without_exposing_prompts() {
    let manager = AiRuntimeManager::new();
    let mut state = lock(&manager.inner);
    state.persisted.personal_notifications.preferences.enabled = true;
    let mut rule = crate::ai_runtime::tests::automation_fixture("rule_1");
    rule.notify_on_every_run = false;
    rule.last_run_id = Some("run_1".into());
    rule.last_run_at = Some(1);
    rule.last_error = Some("private-automation-error".into());
    state.persisted.automation_rules.push(rule);
    let mut automatic = run("completed");
    automatic.automation_rule_id = Some("rule_1".into());
    state.persisted.runs.push(automatic);
    let first = events(&state, &"a".repeat(32), 100);
    assert_eq!(first.len(), 1);
    assert!(!first[0].enabled_for_delivery);
    state.persisted.automation_rules[0].notify_on_every_run = true;
    assert!(events(&state, &"a".repeat(32), 100)[0].enabled_for_delivery);
    "failed".clone_into(&mut state.persisted.runs[0].status);
    state.persisted.automation_rules[0].notify_on_every_run = false;
    let failed = events(&state, &"a".repeat(32), 100);
    assert_eq!(failed.len(), 1);
    assert!(failed[0].enabled_for_delivery);
    state.persisted.automation_rules[0].last_run_at = Some(200);
    let preflight = events(&state, &"a".repeat(32), 100);
    assert_eq!(preflight.len(), 2);
    assert!(
        preflight
            .iter()
            .any(|e| matches!(e.target, NotificationTarget::Automation { .. }))
    );
    assert!(
        !serde_json::to_string(&preflight)
            .unwrap()
            .contains("private-automation-error")
    );
}

fn permission() -> PermissionRequest {
    serde_json::from_value::<PermissionRequest>(json!({
        "permissionRequestId":"permission_1","runId":"run_1","agentId":"agent_1",
        "workspaceRoot":"/private/owned","directoryScope":"owned","tools":["write"],
        "networkAccess":false,"credentialLabels":[],"reason":"private-reason","riskLevel":"high",
        "status":"pending","requestedAt":10,"validUntil":200,"decidedAt":null,"decidedBy":null,
        "protocolRequestId":null,"applicationWrite":{
            "confirmationId":"confirmation_1","applicationId":"app_1","applicationName":"private-title",
            "toolName":"write","toolTitle":"private-title","description":"private-description",
            "riskLevel":"high","arguments":{"secret":"private-arguments"},"executionStatus":"awaiting_confirmation"
        }
    })).unwrap()
}

#[test]
fn pending_requires_live_control_and_expiry_and_disappears_after_revocation() {
    let manager = AiRuntimeManager::new();
    let mut state = lock(&manager.inner);
    state.persisted.personal_notifications.preferences.enabled = true;
    state.persisted.runs.push(run("waiting_permission"));
    state.persisted.permission_requests.push(permission());
    assert!(events(&state, &"a".repeat(32), 100).is_empty());
    let (sender, _receiver) = std::sync::mpsc::channel();
    state
        .application_write_controls
        .insert("permission_1".into(), sender);
    let live = events(&state, &"a".repeat(32), 100);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].confirmation_id.as_deref(), Some("confirmation_1"));
    let value: Value = serde_json::to_value(&live).unwrap();
    assert!(!value.to_string().contains("private-"));
    assert!(events(&state, &"a".repeat(32), 200).is_empty());
    state.application_write_controls.clear();
    assert!(events(&state, &"a".repeat(32), 100).is_empty());
}
