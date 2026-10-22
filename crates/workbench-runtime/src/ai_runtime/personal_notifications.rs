//! 当前个人资料的通知候选和耐久投递回执；操作系统通知仅由可信桌面主进程执行。
use super::{
    AiRuntimeManager, ManagerState, data_dir_for_state, ensure_persistence_ready,
    flush_pending_ledger_write, flush_pending_turn_finalization, lock, maintenance,
    restore_after_save_rejection, sha256_hex, unix_epoch_millis,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

const MAX_DELIVERIES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct NotificationPreferences {
    pub enabled: bool,
    pub completed: bool,
    pub failed: bool,
    pub action_required: bool,
    pub sound: bool,
    pub revision: u32,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            enabled: false,
            completed: true,
            failed: true,
            action_required: true,
            sound: false,
            revision: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Completed,
    Failed,
    ActionRequired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NotificationTarget {
    Run {
        #[serde(rename = "runId")]
        run_id: String,
        #[serde(rename = "conversationId")]
        conversation_id: String,
    },
    Automation {
        #[serde(rename = "automationRuleId")]
        automation_rule_id: String,
    },
    Application {
        #[serde(rename = "applicationId")]
        application_id: String,
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "runId")]
        run_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationEvent {
    pub event_id: String,
    pub kind: NotificationKind,
    pub occurred_at: i64,
    pub target: NotificationTarget,
    pub permission_id: Option<String>,
    pub confirmation_id: Option<String>,
    pub valid_until: Option<i64>,
    pub enabled_for_delivery: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryOutcome {
    Claimed,
    Shown,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationDelivery {
    pub notification_id: String,
    pub profile_id: String,
    pub source_epoch: String,
    pub event: NotificationEvent,
    pub outcome: DeliveryOutcome,
    pub claimed_at: i64,
    pub reported_at: Option<i64>,
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NotificationLedger {
    pub preferences: NotificationPreferences,
    pub deliveries: BTreeMap<String, NotificationDelivery>,
}
impl NotificationLedger {
    pub(super) fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSnapshot {
    pub profile_id: String,
    pub source_epoch: String,
    pub observed_at: i64,
    pub preferences: NotificationPreferences,
    pub events: Vec<NotificationEvent>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationClaimParams {
    pub profile_id: String,
    pub source_epoch: String,
    pub event_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationClaim {
    pub may_deliver: bool,
    pub delivery: NotificationDelivery,
    pub sound: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationReportParams {
    pub profile_id: String,
    pub notification_id: String,
    pub outcome: DeliveryOutcome,
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct NotificationPreferencesSave {
    pub profile_id: String,
    pub expected_revision: u32,
    pub enabled: bool,
    pub completed: bool,
    pub failed: bool,
    pub action_required: bool,
    pub sound: bool,
}

fn profile(state: &ManagerState) -> Result<String, String> {
    let directory = data_dir_for_state(state)?;
    let id = directory
        .parent()
        .and_then(std::path::Path::file_name)
        .and_then(std::ffi::OsStr::to_str)
        .ok_or("personal_notification_profile_invalid")?;
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("personal_notification_profile_invalid".into());
    }
    Ok(id.into())
}

fn check_profile(state: &ManagerState, expected: &str) -> Result<(), String> {
    if profile(state)? != expected {
        return Err("personal_notification_profile_changed".into());
    }
    Ok(())
}

fn enabled(preferences: &NotificationPreferences, kind: &NotificationKind) -> bool {
    preferences.enabled
        && match kind {
            NotificationKind::Completed => preferences.completed,
            NotificationKind::Failed => preferences.failed,
            NotificationKind::ActionRequired => preferences.action_required,
        }
}

#[derive(Default)]
struct ApprovalReference {
    permission_id: Option<String>,
    confirmation_id: Option<String>,
    valid_until: Option<i64>,
}

fn event(
    profile_id: &str,
    kind: NotificationKind,
    target: NotificationTarget,
    occurred_at: i64,
    approval: ApprovalReference,
    allowed: bool,
) -> NotificationEvent {
    let ApprovalReference {
        permission_id,
        confirmation_id,
        valid_until,
    } = approval;
    let event_id = sha256_hex(
        &json!({"profileId":profile_id,"kind":kind,"target":target,"at":occurred_at,
        "permissionId":permission_id,"confirmationId":confirmation_id,"validUntil":valid_until})
        .to_string(),
    );
    NotificationEvent {
        event_id,
        kind,
        target,
        occurred_at,
        permission_id,
        confirmation_id,
        valid_until,
        enabled_for_delivery: allowed,
    }
}

fn terminal(status: &str) -> Option<NotificationKind> {
    match status {
        "completed" => Some(NotificationKind::Completed),
        "failed" | "interrupted" => Some(NotificationKind::Failed),
        _ => None,
    }
}

fn run_events(state: &ManagerState, id: &str, now: i64) -> Vec<NotificationEvent> {
    let preferences = &state.persisted.personal_notifications.preferences;
    let mut result = Vec::new();
    for run in &state.persisted.runs {
        let target = NotificationTarget::Run {
            run_id: run.run_id.clone(),
            conversation_id: run.conversation_id.clone(),
        };
        if let (Some(kind), Some(at)) = (terminal(&run.status), run.completed_at) {
            let automation_all = run.automation_rule_id.as_ref().is_none_or(|rule_id| {
                state
                    .persisted
                    .automation_rules
                    .iter()
                    .any(|rule| &rule.automation_rule_id == rule_id && rule.notify_on_every_run)
            });
            let allowed = enabled(preferences, &kind)
                && (kind != NotificationKind::Completed || automation_all);
            result.push(event(
                id,
                kind,
                target.clone(),
                at,
                ApprovalReference::default(),
                allowed,
            ));
        }
        for request in state
            .persisted
            .permission_requests
            .iter()
            .filter(|request| request.run_id == run.run_id)
        {
            let live = if request.application_write.is_some() {
                state
                    .application_write_controls
                    .contains_key(&request.permission_request_id)
            } else {
                request.protocol_request_id.is_some()
                    && state.harness_controls.contains_key(&run.run_id)
            };
            if request.status != "pending"
                || request.valid_until <= now
                || !live
                || !matches!(run.status.as_str(), "running" | "waiting_permission")
            {
                continue;
            }
            let confirmation = request
                .application_write
                .as_ref()
                .filter(|write| write.execution_status.as_deref() == Some("awaiting_confirmation"));
            if request.application_write.is_some() && confirmation.is_none() {
                continue;
            }
            result.push(event(
                id,
                NotificationKind::ActionRequired,
                target.clone(),
                request.requested_at,
                ApprovalReference {
                    permission_id: Some(request.permission_request_id.clone()),
                    confirmation_id: confirmation.map(|write| write.confirmation_id.clone()),
                    valid_until: Some(request.valid_until),
                },
                enabled(preferences, &NotificationKind::ActionRequired),
            ));
        }
    }
    result
}

fn application_events(state: &ManagerState, id: &str, now: i64) -> Vec<NotificationEvent> {
    let preferences = &state.persisted.personal_notifications.preferences;
    let mut result = Vec::new();
    for (run_id, record) in &state.application_model_runs {
        let Some(session_id) = &record.persistent_session_id else {
            continue;
        };
        let snapshot = &record.snapshot;
        let target = NotificationTarget::Application {
            application_id: snapshot.application_id.clone(),
            session_id: session_id.clone(),
            run_id: run_id.clone(),
        };
        if let (Some(kind), Some(at)) = (terminal(&snapshot.status), snapshot.completed_at) {
            let allowed = enabled(preferences, &kind);
            result.push(event(
                id,
                kind,
                target.clone(),
                at,
                ApprovalReference::default(),
                allowed,
            ));
        }
        if record.cancel_requested || snapshot.status != "waiting_confirmation" {
            continue;
        }
        let Some(request) = &snapshot.write_request else {
            continue;
        };
        if request.valid_until <= now
            || request.approval.execution_status.as_deref() != Some("awaiting_confirmation")
            || !state
                .application_model_write_controls
                .contains_key(&request.approval.confirmation_id)
        {
            continue;
        }
        result.push(event(
            id,
            NotificationKind::ActionRequired,
            target,
            request.requested_at,
            ApprovalReference {
                permission_id: None,
                confirmation_id: Some(request.approval.confirmation_id.clone()),
                valid_until: Some(request.valid_until),
            },
            enabled(preferences, &NotificationKind::ActionRequired),
        ));
    }
    result
}

fn events(state: &ManagerState, id: &str, now: i64) -> Vec<NotificationEvent> {
    let mut result = run_events(state, id, now);
    result.extend(application_events(state, id, now));
    let preferences = &state.persisted.personal_notifications.preferences;
    for rule in &state.persisted.automation_rules {
        if rule.last_error.is_none() {
            continue;
        }
        let Some(at) = rule.last_run_at else {
            continue;
        };
        if rule.last_run_id.as_ref().is_some_and(|run_id| {
            state
                .persisted
                .runs
                .iter()
                .any(|run| &run.run_id == run_id && run.started_at >= at)
        }) {
            continue;
        }
        result.push(event(
            id,
            NotificationKind::Failed,
            NotificationTarget::Automation {
                automation_rule_id: rule.automation_rule_id.clone(),
            },
            at,
            ApprovalReference::default(),
            enabled(preferences, &NotificationKind::Failed),
        ));
    }
    result.sort_by(|a, b| a.event_id.cmp(&b.event_id));
    result
}

fn prepare_write(state: &mut ManagerState) -> Result<(), String> {
    super::application_handoff::flush(state)?;
    flush_pending_turn_finalization(state)?;
    flush_pending_ledger_write(state)?;
    ensure_persistence_ready(state)
}

fn persist(state: &mut ManagerState, before: NotificationLedger) -> Result<(), String> {
    let value = serde_json::to_value(&state.persisted.personal_notifications)
        .map_err(|_| "personal_notification_state_invalid")?;
    state.pending_ledger_write = Some(
        state
            .ledger_baseline
            .prepare_personal_notifications(&value)?,
    );
    let result = flush_pending_ledger_write(state);
    if result.is_err() {
        restore_after_save_rejection(state, |saved| saved.personal_notifications = before);
    }
    result
}

impl AiRuntimeManager {
    fn notification_write<T>(
        &self,
        write: impl FnOnce(&mut ManagerState) -> Result<T, String>,
    ) -> Result<T, String> {
        let _operation = self.begin_capability_operation()?;
        let (server, root) = {
            let state = lock(&self.inner);
            let directory = data_dir_for_state(&state)?;
            (
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
                directory
                    .parent()
                    .ok_or("personal_notification_profile_invalid")?
                    .to_path_buf(),
            )
        };
        let mut writer = maintenance::InitializationWriter::prepare(&server, &root)?;
        let result = {
            let mut state = lock(&self.inner);
            write(&mut state)
        };
        writer.finish()?;
        result
    }

    /// # Errors
    /// 资料不可用或处于维护中时不产生通知候选；不读取原生目录或启动模型。
    pub fn personal_notification_snapshot(&self) -> Result<NotificationSnapshot, String> {
        let state = lock(&self.inner);
        let id = profile(&state)?;
        let now = unix_epoch_millis()?;
        Ok(NotificationSnapshot {
            profile_id: id.clone(),
            source_epoch: state.conversation_session_id.clone(),
            observed_at: now,
            preferences: state.persisted.personal_notifications.preferences.clone(),
            events: events(&state, &id, now),
        })
    }

    /// # Errors
    /// 资料或偏好修订不匹配、维护中或持久化未确认时拒绝保存。
    pub fn save_personal_notification_preferences(
        &self,
        request: &NotificationPreferencesSave,
    ) -> Result<NotificationPreferences, String> {
        self.notification_write(|state| {
            check_profile(state, &request.profile_id)?;
            prepare_write(state)?;
            let before = state.persisted.personal_notifications.clone();
            if before.preferences.revision != request.expected_revision {
                return Err("personal_notification_preferences_changed".into());
            }
            let revision = request
                .expected_revision
                .checked_add(1)
                .ok_or("personal_notification_revision_exhausted")?;
            let preferences = NotificationPreferences {
                enabled: request.enabled,
                completed: request.completed,
                failed: request.failed,
                action_required: request.action_required,
                sound: request.sound,
                revision,
            };
            state.persisted.personal_notifications.preferences = preferences.clone();
            persist(state, before)?;
            Ok(preferences)
        })
    }

    /// # Errors
    /// 候选失效、未授权通知、资料变化或回执未确认时禁止调用系统通知。
    pub fn claim_personal_notification(
        &self,
        request: NotificationClaimParams,
    ) -> Result<NotificationClaim, String> {
        self.notification_write(|state| {
            check_profile(state, &request.profile_id)?;
            prepare_write(state)?;
            if state.conversation_session_id != request.source_epoch {
                return Err("personal_notification_epoch_changed".into());
            }
            let preferences = &state.persisted.personal_notifications.preferences;
            let sound = preferences.sound;
            if let Some(delivery) = state
                .persisted
                .personal_notifications
                .deliveries
                .get(&request.event_id)
            {
                if delivery.profile_id != request.profile_id {
                    return Err("personal_notification_profile_changed".into());
                }
                return Ok(NotificationClaim {
                    may_deliver: false,
                    delivery: delivery.clone(),
                    sound,
                });
            }
            let now = unix_epoch_millis()?;
            let candidate = events(state, &request.profile_id, now)
                .into_iter()
                .find(|e| e.event_id == request.event_id)
                .ok_or("personal_notification_event_expired")?;
            if !candidate.enabled_for_delivery {
                return Err("personal_notification_disabled".into());
            }
            if state.persisted.personal_notifications.deliveries.len() >= MAX_DELIVERIES {
                return Err("personal_notification_delivery_limit".into());
            }
            let delivery = NotificationDelivery {
                notification_id: request.event_id[..32].into(),
                profile_id: request.profile_id,
                source_epoch: request.source_epoch,
                event: candidate,
                outcome: DeliveryOutcome::Claimed,
                claimed_at: now,
                reported_at: None,
                reason_code: None,
            };
            let before = state.persisted.personal_notifications.clone();
            state
                .persisted
                .personal_notifications
                .deliveries
                .insert(request.event_id, delivery.clone());
            persist(state, before)?;
            Ok(NotificationClaim {
                may_deliver: true,
                delivery,
                sound,
            })
        })
    }

    /// # Errors
    /// 不接受其他资料、未登记投递或改写已确认的终态；未知结果不能重新投递。
    pub fn report_personal_notification(
        &self,
        request: NotificationReportParams,
    ) -> Result<NotificationDelivery, String> {
        if request.outcome == DeliveryOutcome::Claimed
            || request.reason_code.as_deref().is_some_and(|code| {
                !matches!(
                    code,
                    "notification_unsupported"
                        | "notification_native_failed"
                        | "notification_outcome_unconfirmed"
                        | "notification_profile_changed"
                )
            })
        {
            return Err("personal_notification_report_invalid".into());
        }
        self.notification_write(|state| {
            check_profile(state, &request.profile_id)?;
            prepare_write(state)?;
            let key = state
                .persisted
                .personal_notifications
                .deliveries
                .iter()
                .find(|(_, d)| d.notification_id == request.notification_id)
                .map(|(key, _)| key.clone())
                .ok_or("personal_notification_not_found")?;
            let before = state.persisted.personal_notifications.clone();
            let delivery = state
                .persisted
                .personal_notifications
                .deliveries
                .get_mut(&key)
                .ok_or("personal_notification_not_found")?;
            if delivery.profile_id != request.profile_id {
                return Err("personal_notification_profile_changed".into());
            }
            if delivery.outcome != DeliveryOutcome::Claimed {
                if delivery.outcome == request.outcome
                    && delivery.reason_code == request.reason_code
                {
                    return Ok(delivery.clone());
                }
                return Err("personal_notification_outcome_frozen".into());
            }
            delivery.outcome = request.outcome;
            delivery.reason_code = request.reason_code;
            delivery.reported_at = Some(unix_epoch_millis()?);
            let result = delivery.clone();
            persist(state, before)?;
            Ok(result)
        })
    }

    /// # Errors
    /// 点击仅核对定位对象；资料变化、对象不存在或确认已失效时拒绝，不执行任务。
    pub fn open_personal_notification(
        &self,
        expected_profile: &str,
        notification_id: &str,
    ) -> Result<NotificationTarget, String> {
        let state = lock(&self.inner);
        check_profile(&state, expected_profile)?;
        let delivery = state
            .persisted
            .personal_notifications
            .deliveries
            .values()
            .find(|d| d.notification_id == notification_id && d.profile_id == expected_profile)
            .ok_or("personal_notification_not_found")?;
        if delivery.event.kind == NotificationKind::ActionRequired
            && !events(&state, expected_profile, unix_epoch_millis()?)
                .iter()
                .any(|e| e.event_id == delivery.event.event_id)
        {
            return Err("personal_notification_event_expired".into());
        }
        let exists = match &delivery.event.target {
            NotificationTarget::Run {
                run_id,
                conversation_id,
            } => {
                state
                    .persisted
                    .runs
                    .iter()
                    .any(|r| &r.run_id == run_id && &r.conversation_id == conversation_id)
                    && state
                        .persisted
                        .conversations
                        .iter()
                        .any(|c| &c.conversation_id == conversation_id)
            }
            NotificationTarget::Automation { automation_rule_id } => state
                .persisted
                .automation_rules
                .iter()
                .any(|r| &r.automation_rule_id == automation_rule_id),
            NotificationTarget::Application {
                application_id,
                session_id,
                run_id,
            } => state.application_model_runs.get(run_id).is_some_and(|r| {
                &r.snapshot.application_id == application_id
                    && r.persistent_session_id.as_ref() == Some(session_id)
            }),
        };
        if !exists {
            return Err("personal_notification_target_missing".into());
        }
        Ok(delivery.event.target.clone())
    }
}

#[cfg(test)]
mod tests;
