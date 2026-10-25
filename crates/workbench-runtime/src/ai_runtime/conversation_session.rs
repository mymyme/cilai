use super::*;

const QUEUE: &str = "queuedFollowUps";
use contexthub_protocol::{
    CONVERSATION_SESSION_KEYS as KEYS, ConversationSessionChange as SessionChange,
};
pub use contexthub_protocol::{ConversationSessionPatch, ConversationSessionSnapshot};

pub struct ClaimedFollowUp {
    pub queue_id: String,
    pub request: MessageSendRequest,
    pub operation: LocalOperationGuard,
}

#[derive(Default)]
pub(super) struct SessionState {
    id: String,
    revision: u64,
}

impl SessionState {
    pub(super) fn reset(&mut self) {
        self.id = uuid::Uuid::new_v4().to_string();
        self.revision = 0;
    }
    pub(super) fn id(&self) -> &str {
        &self.id
    }
    pub(super) fn changed(&mut self) {
        self.revision += 1;
    }
    fn snapshot(&self, values: &BTreeMap<String, Value>) -> ConversationSessionSnapshot {
        ConversationSessionSnapshot {
            session_id: self.id.clone(),
            revision: self.revision,
            values: values.clone(),
        }
    }
}

fn snapshot(state: &ManagerState) -> ConversationSessionSnapshot {
    state
        .session
        .snapshot(&state.persisted.conversation_session)
}

fn entries(values: &BTreeMap<String, Value>) -> Vec<Value> {
    values
        .get(QUEUE)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn field<'a>(entry: &'a Value, key: &str) -> &'a str {
    entry.get(key).and_then(Value::as_str).unwrap_or("")
}

fn request(entry: &Value) -> Result<MessageSendRequest, String> {
    let attachments = entry
        .get("attachments")
        .and_then(Value::as_array)
        .ok_or_else(|| "conversation_queue_invalid".to_owned())?;
    let attachment_ids = attachments
        .iter()
        .map(|item| {
            item.pointer("/attachment/attachmentId")
                .and_then(Value::as_str)
                .filter(|id| safe_storage_id(id))
                .map(str::to_owned)
                .ok_or_else(|| "conversation_queue_attachment_invalid".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::from_value(serde_json::json!({
        "clientMessageId": entry["queueId"], "conversationId": entry["conversationId"],
        "content": entry["content"], "attachmentIds": attachment_ids,
        "referencedMaterials": entry.get("referencedMaterials").cloned().unwrap_or_else(|| serde_json::json!([])),
        "applicationContext": entry["applicationContext"], "selectedSkillId": entry["selectedSkillId"],
        "retryOfRunId": null, "modelOverride": entry["modelOverride"],
        "modelSelection":entry["modelSelection"], "historyConsent":entry["historyConsent"],
        "reasoningEffort": entry["reasoningEffort"], "executionTarget": entry["executionTarget"],
        "permissionMode": entry["permissionMode"],
        "planning": entry.get("planning").cloned().unwrap_or(Value::Bool(false)),
    })).map_err(|_| "conversation_queue_invalid".to_owned())
}

/// 派发器已领取、正在发送的后续指令同样占用对话。
pub(super) fn follow_up_sending(values: &BTreeMap<String, Value>, conversation_id: &str) -> bool {
    entries(values).iter().any(|entry| {
        field(entry, "conversationId") == conversation_id && field(entry, "status") == "sending"
    })
}

fn recorded(entry: &Value, messages: &[ConversationMessage]) -> bool {
    let id = entry
        .get("sentMessageId")
        .and_then(Value::as_str)
        .unwrap_or_else(|| field(entry, "queueId"));
    messages.iter().any(|message| {
        message.message_id == id && message.conversation_id == field(entry, "conversationId")
    })
}

fn failed(entry: &mut Value, error: &str) {
    entry["status"] = Value::from("failed");
    entry["error"] = Value::from(error);
    entry["transcriptCommitted"] = Value::from(false);
}

pub(super) fn recover(persisted: &mut PersistedAiState) -> bool {
    let previous = entries(&persisted.conversation_session);
    let mut queue = previous.clone();
    queue.retain(|entry| !recorded(entry, &persisted.messages));
    for entry in &mut queue {
        if field(entry, "status") != "failed" {
            failed(entry, "桌面已重启，请核对后重试。");
        }
    }
    let outgoing = persisted
        .conversation_session
        .get("pendingOutgoing")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut recovered = persisted
        .conversation_session
        .get("recoveredOutgoing")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for entry in &outgoing {
        let id = field(entry, "pendingId");
        if let Some(message) = persisted
            .messages
            .iter()
            .find(|message| message.message_id == id)
        {
            let draft_key = field(entry, "draftKey");
            let draft_empty = persisted
                .conversation_session
                .get("drafts")
                .and_then(|drafts| drafts.get(draft_key))
                .and_then(Value::as_str)
                .is_none_or(str::is_empty);
            if draft_empty
                && persisted
                    .conversation_session
                    .get("pendingCreate")
                    .is_some_and(|create| field(create, "draftKey") == draft_key)
            {
                persisted
                    .conversation_session
                    .insert("pendingCreate".to_owned(), Value::Null);
                persisted.conversation_session.insert(
                    "selectedConversationId".to_owned(),
                    Value::from(message.conversation_id.clone()),
                );
            }
            if let Some(group) = persisted
                .conversation_session
                .get_mut("draftAttachmentGroups")
                .and_then(|groups| groups.get_mut(draft_key))
                .and_then(Value::as_array_mut)
            {
                group.retain(|attachment| {
                    !message.attachments.iter().any(|sent| {
                        attachment
                            .pointer("/attachment/attachmentId")
                            .and_then(Value::as_str)
                            == Some(sent.attachment_id.as_str())
                    })
                });
            }
        } else if !recovered.iter().any(|item| field(item, "pendingId") == id) {
            recovered.push(entry.clone());
        }
    }
    if queue == previous && outgoing.is_empty() {
        return false;
    }
    if !outgoing.is_empty() {
        persisted
            .conversation_session
            .insert("pendingOutgoing".to_owned(), serde_json::json!([]));
        persisted
            .conversation_session
            .insert("recoveredOutgoing".to_owned(), Value::Array(recovered));
    }
    persisted
        .conversation_session
        .insert(QUEUE.to_owned(), Value::Array(queue));
    true
}

/// 以界面提交的顺序为准合并队列：界面未改动的条目取服务端当前版本（已送达而移除的不再带回），
/// 界面新增或修改的条目只在服务端未同时改动该条时采用；双方改了同一条返回冲突。
fn merge_queue(current: &Value, previous: &Value, value: &Value) -> Result<Value, String> {
    let as_entries = |value: &Value| -> Result<Vec<Value>, String> {
        match value {
            Value::Null => Ok(Vec::new()),
            Value::Array(items) => Ok(items.clone()),
            _ => Err("conversation_queue_invalid".to_owned()),
        }
    };
    let (current, previous, value) = (
        as_entries(current)?,
        as_entries(previous)?,
        as_entries(value)?,
    );
    let find = |items: &[Value], id: &str| {
        items
            .iter()
            .find(|entry| field(entry, "queueId") == id)
            .cloned()
    };
    let conflict = || Err("conversation_session_conflict".to_owned());
    let mut merged = Vec::with_capacity(value.len());
    for entry in &value {
        let id = field(entry, "queueId");
        let (before, now) = (find(&previous, id), find(&current, id));
        match (before.as_ref(), now) {
            (Some(before), now) if before == entry => merged.extend(now),
            (None, Some(now)) if &now != entry => return conflict(),
            (None, _) => merged.push(entry.clone()),
            (Some(before), Some(now)) if &now == before || &now == entry => {
                merged.push(entry.clone());
            }
            (Some(_), _) => return conflict(),
        }
    }
    for before in &previous {
        let id = field(before, "queueId");
        if find(&value, id).is_none()
            && let Some(now) = find(&current, id)
            && &now != before
        {
            return conflict();
        }
    }
    for now in &current {
        let id = field(now, "queueId");
        if find(&previous, id).is_none() && find(&value, id).is_none() {
            merged.push(now.clone());
        }
    }
    Ok(Value::Array(merged))
}

fn apply_changes(
    values: &BTreeMap<String, Value>,
    changes: Vec<SessionChange>,
) -> Result<BTreeMap<String, Value>, String> {
    if changes.len() > KEYS.len() {
        return Err("conversation_session_invalid".to_owned());
    }
    let mut result = values.clone();
    let mut seen = HashSet::new();
    for change in changes {
        if !KEYS.contains(&change.key.as_str()) || !seen.insert(change.key.clone()) {
            return Err("conversation_session_key_invalid".to_owned());
        }
        let current = values.get(&change.key).unwrap_or(&Value::Null);
        // 队列同时由界面（编辑、补充、删除）和派发器（领取、送达）修改，整份比对会让互不相干的
        // 两处改动互相冲突；按条目三方合并，只有双方改了同一条时才算冲突。
        let change =
            if change.key == QUEUE && current != &change.previous && current != &change.value {
                SessionChange {
                    value: merge_queue(current, &change.previous, &change.value)?,
                    previous: current.clone(),
                    key: change.key,
                }
            } else {
                change
            };
        if current != &change.previous && current != &change.value {
            return Err("conversation_session_conflict".to_owned());
        }
        if change.key == "applicationHandoffs" {
            let previous = current.as_array().cloned().unwrap_or_default();
            let next = change
                .value
                .as_array()
                .ok_or("application_handoff_invalid")?;
            if previous
                .iter()
                .any(|entry| entry.get("transfer").is_some() && !next.contains(entry))
            {
                return Err("application_handoff_reconciliation_required".into());
            }
            let mut ids = HashSet::new();
            if next
                .iter()
                .any(|entry| !previous.contains(entry) || !ids.insert(field(entry, "handoffId")))
            {
                return Err("application_handoff_invalid".to_owned());
            }
        }
        if change.key == QUEUE {
            let queue = change
                .value
                .as_array()
                .filter(|queue| queue.len() <= 100)
                .ok_or_else(|| "conversation_queue_invalid".to_owned())?;
            let previous = entries(values);
            let mut ids = HashSet::new();
            for entry in queue {
                let id = field(entry, "queueId");
                let req = request(entry)?;
                if !safe_storage_id(id)
                    || !ids.insert(id)
                    || req.content.chars().count() > MAX_MESSAGE_CHARS
                    || !matches!(
                        field(entry, "status"),
                        "queued" | "failed" | "steering" | "deleting" | "sending"
                    )
                    || !matches!(req.execution_target.as_str(), "model" | "agent")
                    || !matches!(
                        req.permission_mode.as_str(),
                        "read-only" | "workspace-write" | "request" | "auto-review" | "full-access"
                    )
                    || req.attachment_ids.len() > MAX_MESSAGE_ATTACHMENTS
                {
                    return Err("conversation_queue_invalid".to_owned());
                }
                if field(entry, "status") == "sending" && !previous.iter().any(|old| old == entry) {
                    return Err("conversation_queue_busy".to_owned());
                }
            }
            for old in previous
                .iter()
                .filter(|entry| field(entry, "status") == "sending")
            {
                if !queue.contains(old) {
                    return Err("conversation_queue_busy".to_owned());
                }
            }
        }
        result.insert(change.key, change.value);
    }
    if serde_json::to_vec(&result)
        .map_err(|error| error.to_string())?
        .len()
        > 2 * 1024 * 1024
    {
        return Err("conversation_session_too_large".to_owned());
    }
    Ok(result)
}

fn save_values(state: &mut ManagerState, values: BTreeMap<String, Value>) -> Result<(), String> {
    let changed = values != state.persisted.conversation_session;
    let previous = std::mem::replace(&mut state.persisted.conversation_session, values);
    if let Err(error) = persist_session_locked(state) {
        let retained = persistence_outcome_unknown(state);
        restore_after_save_rejection(state, |persisted| {
            persisted.conversation_session = previous;
        });
        if retained && changed {
            state.session.changed();
        }
        return Err(error);
    }
    if changed {
        state.session.changed();
    }
    Ok(())
}

impl AiRuntimeManager {
    #[must_use]
    pub fn recent_conversations(&self) -> Vec<Value> {
        let state = lock(&self.inner);
        let active: HashSet<_> = state
            .persisted
            .runs
            .iter()
            .filter(|run| run.thread_writer_active || run_is_active(&run.status))
            .map(|run| run.conversation_id.as_str())
            .collect();
        let mut conversations: Vec<_> = state
            .persisted
            .conversations
            .iter()
            .filter(|item| !item.archived)
            .collect();
        conversations.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.conversation_id.cmp(&right.conversation_id))
        });
        conversations.into_iter().take(4).map(|item| serde_json::json!({
            "conversationId": item.conversation_id, "title": item.title, "updatedAt": item.updated_at,
            "projectName": item.context.project_name, "running": active.contains(item.conversation_id.as_str())
        })).collect()
    }

    /// # Errors
    /// 当前本地服务尚未初始化时返回错误。
    pub fn conversation_session(&self) -> Result<ConversationSessionSnapshot, String> {
        let state = lock(&self.inner);
        data_dir_for_state(&state)?;
        Ok(snapshot(&state))
    }

    /// # Errors
    /// 会话已切换、并发修改、输入无效或持久化失败时返回错误。
    pub fn patch_conversation_session(
        &self,
        patch: ConversationSessionPatch,
    ) -> Result<ConversationSessionSnapshot, String> {
        let mut state = lock(&self.inner);
        ensure_profile_admission(&state)?;
        data_dir_for_state(&state)?;
        if patch.session_id != state.session.id() {
            return Err("ai_profile_changed".to_owned());
        }
        let values = apply_changes(&state.persisted.conversation_session, patch.changes)?;
        if values != state.persisted.conversation_session || persistence_outcome_unknown(&state) {
            save_values(&mut state, values)?;
        }
        Ok(snapshot(&state))
    }

    /// # Errors
    /// 队列无法解析或领取状态保存失败时返回错误。
    pub fn claim_follow_up(&self) -> Result<Option<ClaimedFollowUp>, String> {
        let mut state = lock(&self.inner);
        ensure_profile_admission(&state)?;
        if state.data_dir.is_none() {
            return Ok(None);
        }
        let mut queue = entries(&state.persisted.conversation_session);
        let original = queue.clone();
        queue.retain(|entry| !recorded(entry, &state.persisted.messages));
        let mut conversations = HashSet::new();
        let mut claimed = None;
        for entry in &mut queue {
            let conversation_id = field(entry, "conversationId").to_owned();
            if !conversations.insert(conversation_id.clone()) || field(entry, "status") != "queued"
            {
                continue;
            }
            if state.persisted.runs.iter().any(|run| {
                run.conversation_id == conversation_id
                    && (run.thread_writer_active || run_is_active(&run.status))
            }) || super::message_submission::conversation_preparing(&state, &conversation_id)
            {
                continue;
            }
            if !state.persisted.conversations.iter().any(|conversation| {
                conversation.conversation_id == conversation_id && !conversation.archived
            }) {
                failed(entry, "对话已归档或不可用。");
                continue;
            }
            if entry
                .get("requiresApplicationResolution")
                .and_then(Value::as_bool)
                == Some(true)
                || entry
                    .get("applicationContext")
                    .is_some_and(|context| !context.is_null())
            {
                failed(entry, "旧企业应用指令已停用，请移除应用引用后重试。");
                continue;
            }
            let req = request(entry)?;
            if req.model_selection.is_none() {
                failed(entry, "请选择模型连接后重试。");
                continue;
            }
            if req.execution_target == "agent" {
                let conversation = state
                    .persisted
                    .conversations
                    .iter()
                    .find(|item| item.conversation_id == conversation_id)
                    .ok_or("conversation_not_found")?;
                if let Some(agent) =
                    state.persisted.agents.iter().find(|agent| {
                        Some(agent.agent_id.as_str()) == conversation.agent_id.as_deref()
                    })
                    && state
                        .persisted
                        .runs
                        .iter()
                        .filter(|run| {
                            run.agent_id == agent.agent_id
                                && (run.thread_writer_active || run_is_active(&run.status))
                        })
                        .count()
                        >= agent.concurrency_limit as usize
                {
                    continue;
                }
            }
            entry["status"] = Value::from("sending");
            entry
                .as_object_mut()
                .ok_or("ai_conversation_session_invalid")?
                .remove("error");
            claimed = Some((field(entry, "queueId").to_owned(), req));
            break;
        }
        if queue != original {
            let mut values = state.persisted.conversation_session.clone();
            values.insert(QUEUE.to_owned(), Value::Array(queue));
            save_values(&mut state, values)?;
        }
        Ok(claimed.map(|(queue_id, request)| {
            state.local_operations += 1;
            ClaimedFollowUp {
                queue_id,
                request,
                operation: LocalOperationGuard {
                    manager: self.clone(),
                },
            }
        }))
    }

    /// # Errors
    /// 投递结果无法保存时返回错误。
    pub fn finish_follow_up(&self, queue_id: &str, error: Option<&str>) -> Result<(), String> {
        let mut state = lock(&self.inner);
        let mut queue = entries(&state.persisted.conversation_session);
        if let Some(index) = queue
            .iter()
            .position(|entry| field(entry, "queueId") == queue_id)
        {
            if recorded(&queue[index], &state.persisted.messages) {
                queue.remove(index);
            } else {
                failed(
                    &mut queue[index],
                    error.unwrap_or("投递结果尚未确认，请核对后重试。"),
                );
            }
            let mut values = state.persisted.conversation_session.clone();
            values.insert(QUEUE.to_owned(), Value::Array(queue));
            save_values(&mut state, values)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(id: &str, status: &str) -> Value {
        serde_json::json!({ "queueId":id, "conversationId":"conv_1", "content":"材料", "attachments":[],
            "applicationContext":null, "modelOverride":"model_1", "reasoningEffort":"high", "selectedSkillId":"skill_1",
            "modelSelection":{"connectionId":"frozen-connection","modelId":"model_1","reasoningEffort":"high"},
            "historyConsent":{"connectionId":"frozen-connection","configRevision":3,"authGeneration":7,"modelId":"model_1","runtimeKind":"workbench","snapshotDigest":"frozen-snapshot","historyDigest":"frozen-history","confirmedAt":1},
            "executionTarget":"model", "permissionMode":"auto-review", "status":status })
    }
    #[test]
    fn handoff_patches_can_remove_but_cannot_forge_or_change_source() {
        let entry = serde_json::json!({"handoffId":"handoff_one","output":"原始回答"});
        let values =
            BTreeMap::from([("applicationHandoffs".to_owned(), serde_json::json!([entry]))]);
        let patch = |value| {
            vec![SessionChange {
                key: "applicationHandoffs".to_owned(),
                previous: values["applicationHandoffs"].clone(),
                value,
            }]
        };
        assert!(apply_changes(&values, patch(serde_json::json!([]))).is_ok());
        assert!(
            apply_changes(
                &values,
                patch(serde_json::json!([{"handoffId":"handoff_one","output":"篡改"}]))
            )
            .is_err()
        );
        assert!(apply_changes(&values, patch(serde_json::json!([entry, entry]))).is_err());
    }

    #[test]
    fn queue_and_draft_keep_message_materials_separate_from_conversation_context() {
        let materials = serde_json::json!([{"localFileId":"file_1","relativePath":"材料.md","fileRevision":3,"workspaceId":"workspace_1"}]);
        let mut queued = entry("q_material", "queued");
        queued["content"] = Value::String(String::new());
        queued["referencedMaterials"] = materials.clone();
        let parsed = request(&queued).unwrap();
        assert!(parsed.content.is_empty());
        assert_eq!(
            serde_json::to_value(parsed.referenced_materials).unwrap(),
            materials
        );
        let values = apply_changes(
            &BTreeMap::new(),
            vec![SessionChange {
                key: "draftMaterialGroups".to_owned(),
                previous: Value::Null,
                value: serde_json::json!({"conversation:conv_1":materials}),
            }],
        )
        .unwrap();
        assert_eq!(
            values["draftMaterialGroups"]["conversation:conv_1"],
            materials
        );
    }

    #[test]
    fn freezes_send_parameters() {
        let req = request(&entry("q_1", "queued")).unwrap();
        assert_eq!(req.client_message_id.as_deref(), Some("q_1"));
        assert_eq!(req.selected_skill_id.as_deref(), Some("skill_1"));
        assert_eq!(req.model_override.as_deref(), Some("model_1"));
        assert_eq!(req.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(
            serde_json::to_value(req.model_selection).unwrap(),
            entry("q_1", "queued")["modelSelection"]
        );
        assert_eq!(
            serde_json::to_value(req.history_consent).unwrap(),
            entry("q_1", "queued")["historyConsent"]
        );
    }
    #[test]
    fn queue_patches_merge_with_dispatcher_changes_to_other_entries() {
        let previous = Value::Array(vec![entry("q_1", "queued"), entry("q_2", "queued")]);
        // 派发器已把 q_1 送达并移除，界面同时把 q_2 标为补充中
        let current =
            BTreeMap::from([(QUEUE.to_owned(), Value::Array(vec![entry("q_2", "queued")]))]);
        let mut steering = entry("q_2", "steering");
        steering["transcriptCommitted"] = Value::Bool(true);
        let change = SessionChange {
            key: QUEUE.to_owned(),
            previous: previous.clone(),
            value: Value::Array(vec![entry("q_1", "queued"), steering.clone()]),
        };
        let merged = apply_changes(&current, vec![change]).unwrap();
        assert_eq!(merged[QUEUE], Value::Array(vec![steering]));
        // 新排队的条目追加在界面给出的位置
        let change = SessionChange {
            key: QUEUE.to_owned(),
            previous,
            value: Value::Array(vec![
                entry("q_1", "queued"),
                entry("q_2", "queued"),
                entry("q_3", "queued"),
            ]),
        };
        let merged = apply_changes(&current, vec![change]).unwrap();
        assert_eq!(
            merged[QUEUE],
            Value::Array(vec![entry("q_2", "queued"), entry("q_3", "queued")])
        );
    }
    #[test]
    fn queue_patch_conflicts_only_when_both_sides_change_the_same_entry() {
        let previous = Value::Array(vec![entry("q_1", "queued")]);
        let current = BTreeMap::from([(
            QUEUE.to_owned(),
            Value::Array(vec![entry("q_1", "sending")]),
        )]);
        let change = SessionChange {
            key: QUEUE.to_owned(),
            previous,
            value: Value::Array(vec![entry("q_1", "steering")]),
        };
        assert_eq!(
            apply_changes(&current, vec![change]).unwrap_err(),
            "conversation_session_conflict"
        );
        assert!(follow_up_sending(&current, "conv_1"));
        assert!(!follow_up_sending(&current, "conv_2"));
    }
    #[test]
    fn concurrent_claim_cannot_be_removed_or_replayed_by_ui() {
        let before = Value::Array(vec![entry("q_1", "queued")]);
        let current = BTreeMap::from([(
            QUEUE.to_owned(),
            Value::Array(vec![entry("q_1", "sending")]),
        )]);
        let change = SessionChange {
            key: QUEUE.to_owned(),
            previous: before,
            value: Value::Array(vec![]),
        };
        assert_eq!(
            apply_changes(&current, vec![change]).unwrap_err(),
            "conversation_session_conflict"
        );
        let change = SessionChange {
            key: QUEUE.to_owned(),
            previous: current[QUEUE].clone(),
            value: Value::Array(vec![]),
        };
        assert_eq!(
            apply_changes(&current, vec![change]).unwrap_err(),
            "conversation_queue_busy"
        );
    }
    #[test]
    fn restart_requires_confirmation_and_keeps_frozen_content() {
        let mut state = PersistedAiState::default();
        state.conversation_session.insert(
            QUEUE.to_owned(),
            Value::Array(vec![entry("q_1", "sending"), entry("q_2", "queued")]),
        );
        assert!(recover(&mut state));
        for entry in entries(&state.conversation_session) {
            assert_eq!(field(&entry, "status"), "failed");
            assert_eq!(field(&entry, "content"), "材料");
        }
        assert!(!recover(&mut state));
    }
    #[test]
    fn interrupted_direct_send_is_recovered_without_replay_or_draft_overwrite() {
        let mut state = PersistedAiState::default();
        state.messages.push(serde_json::from_value(serde_json::json!({
            "messageId":"accepted", "conversationId":"conv_1", "role":"user", "content":"相同内容", "createdAt":1
        })).unwrap());
        state
            .conversation_session
            .insert("drafts".to_owned(), serde_json::json!({"new":"后写的草稿"}));
        state.conversation_session.insert("pendingOutgoing".to_owned(), serde_json::json!([
            {"pendingId":"accepted", "content":"相同内容", "attachments":[]},
            {"pendingId":"unknown", "content":"相同内容", "attachments":[], "recovery":{"permissionMode":"request"}}
        ]));
        assert!(recover(&mut state));
        assert_eq!(
            state.conversation_session["pendingOutgoing"],
            serde_json::json!([])
        );
        let recovered = state.conversation_session["recoveredOutgoing"]
            .as_array()
            .unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0]["pendingId"], "unknown");
        assert_eq!(state.conversation_session["drafts"]["new"], "后写的草稿");
        assert!(!recover(&mut state));
    }

    #[test]
    fn rejects_unknown_keys_and_duplicate_queue_ids() {
        let change = SessionChange {
            key: "harness_controls".to_owned(),
            previous: Value::Null,
            value: Value::Null,
        };
        assert!(apply_changes(&BTreeMap::new(), vec![change]).is_err());
        let change = SessionChange {
            key: QUEUE.to_owned(),
            previous: Value::Null,
            value: Value::Array(vec![entry("q_1", "queued"), entry("q_1", "queued")]),
        };
        assert!(apply_changes(&BTreeMap::new(), vec![change]).is_err());
    }
    #[test]
    #[allow(clippy::too_many_lines)]
    fn durable_session_reloads_and_claims_once_with_profile_fence() {
        let _guard = crate::supervisor::app_server_test_guard();
        let owner = model_test_fixture::fixture_root("conversation-session");
        let root = model_test_fixture::profile_data_root(&owner);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &root,
                &root.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&root, &server).unwrap();
        let selection = model_test_fixture::seed_simulated_connection(&server, "model_1");
        model_test_fixture::bind_purpose(
            &server,
            &selection,
            contexthub_protocol::ModelPurpose::Foreground,
        );
        let folder = owner.join("original-workspace");
        fs::create_dir_all(&folder).unwrap();
        let workspace = server.request_result("workspace/create", &serde_json::json!({"localRoot":folder,"displayName":"原工作空间","clientOperationId":"original-workspace"})).unwrap();
        let project = server.request_result("project/create", &serde_json::json!({
            "workspaceId":workspace["workspace"]["workspaceId"], "workspaceIds":[], "displayName":"未连接文件夹的项目", "goal":"先讨论", "clientOperationId":"create-project", "requestFingerprint":"create-project"
        })).unwrap();
        server.request_result("project/detachWorkspace", &serde_json::json!({"projectId":project["project"]["projectId"],"workspaceId":workspace["workspace"]["workspaceId"],"clientOperationId":"detach-workspace","requestFingerprint":"detach-workspace"})).unwrap();
        let conversation = manager.create_conversation(serde_json::from_value(serde_json::json!({
            "title":"无文件夹讨论", "conversationType":"project", "mode":"discuss", "agentId":null,
            "context":{"workspaceRoot":"", "projectId":project["project"]["projectId"]}
        })).unwrap()).unwrap();
        assert_eq!(
            conversation.context.project_name.as_deref(),
            Some("未连接文件夹的项目")
        );
        assert!(conversation.context.workspace_id.is_none());
        assert!(Path::new(&conversation.context.workspace_root).is_dir());
        assert_eq!(
            manager.recent_conversations()[0]["conversationId"],
            conversation.conversation_id
        );
        let original = manager.conversation_session().unwrap();
        let patch = |session_id: String, previous: Value, value: Value| ConversationSessionPatch {
            session_id,
            changes: vec![SessionChange {
                key: QUEUE.to_owned(),
                previous,
                value,
            }],
        };
        let queued_conversation = manager.create_conversation(serde_json::from_value(serde_json::json!({
            "title":"队列测试", "conversationType":"direct", "mode":"discuss", "agentId":null,
            "context":{"workspaceRoot":""}
        })).unwrap()).unwrap();
        let attachment = manager
            .create_attachment(super::super::AttachmentCreateRequest {
                file_name: "材料.txt".to_owned(),
                mime_type: "text/plain".to_owned(),
                size_bytes: 3,
            })
            .unwrap();
        manager
            .append_attachment_chunk(super::super::AttachmentChunkRequest {
                attachment_id: attachment.attachment_id.clone(),
                offset: 0,
                bytes: b"123".to_vec(),
                complete: true,
            })
            .unwrap();
        let mut queued_entry = entry("q_1", "queued");
        queued_entry["conversationId"] = serde_json::json!(queued_conversation.conversation_id);
        queued_entry["modelSelection"] = serde_json::to_value(selection).unwrap();
        queued_entry["historyConsent"] = Value::Null;
        let queued_before = queued_entry.clone();
        let unselected_conversation = manager.create_conversation(serde_json::from_value(serde_json::json!({
            "title":"缺少连接的队列", "conversationType":"direct", "mode":"discuss", "agentId":null,
            "context":{"workspaceRoot":""}
        })).unwrap()).unwrap();
        let mut unselected = entry("missing-selection", "queued");
        unselected["conversationId"] = serde_json::json!(unselected_conversation.conversation_id);
        unselected["modelSelection"] = Value::Null;
        unselected["content"] = Value::from("保留未选择连接的正文");
        unselected["attachments"] = serde_json::json!([{"attachment":attachment}]);
        let unselected_before = unselected.clone();
        let mut legacy = entry("legacy_application", "queued");
        legacy["conversationId"] = serde_json::json!(conversation.conversation_id);
        legacy["content"] = Value::from("保留旧应用消息正文");
        legacy["attachments"] = serde_json::json!([{"attachment":attachment}]);
        legacy["applicationContext"] = serde_json::json!({"applicationId":"retired.enterprise","contextIds":[],"toolNames":["read"]});
        legacy["requiresApplicationResolution"] = Value::Bool(false);
        let legacy_before = legacy.clone();
        manager
            .patch_conversation_session(patch(
                original.session_id.clone(),
                Value::Null,
                Value::Array(vec![legacy, unselected, queued_entry]),
            ))
            .unwrap();
        let claimed = manager.claim_follow_up().unwrap().unwrap();
        assert_eq!(claimed.queue_id, "q_1");
        assert_eq!(
            serde_json::to_value(&claimed.request.model_selection).unwrap(),
            queued_before["modelSelection"]
        );
        assert_eq!(
            serde_json::to_value(&claimed.request.history_consent).unwrap(),
            queued_before["historyConsent"]
        );
        let missing = entries(&manager.conversation_session().unwrap().values)[1].clone();
        assert_eq!(missing["status"], "failed");
        assert_eq!(missing["error"], "请选择模型连接后重试。");
        for key in [
            "content",
            "attachments",
            "conversationId",
            "modelSelection",
            "historyConsent",
        ] {
            assert_eq!(missing[key], unselected_before[key]);
        }
        let rejected = entries(&manager.conversation_session().unwrap().values)[0].clone();
        assert_eq!(rejected["status"], "failed");
        assert!(rejected["error"].as_str().unwrap().contains("旧企业应用"));
        for key in [
            "conversationId",
            "content",
            "attachments",
            "applicationContext",
            "requiresApplicationResolution",
        ] {
            assert_eq!(rejected[key], legacy_before[key]);
        }
        assert!(manager.claim_follow_up().unwrap().is_none());
        assert!(
            manager
                .tenant_switch_blockers()
                .contains(&"ai_run_active".to_owned())
        );
        manager
            .finish_follow_up(&claimed.queue_id, Some("连接中断"))
            .unwrap();
        drop(claimed);
        manager.shutdown();
        let reloaded = AiRuntimeManager::new();
        reloaded.initialize(&root, &server).unwrap();
        let current = reloaded.conversation_session().unwrap();
        assert_ne!(current.session_id, original.session_id);
        assert_eq!(field(&entries(&current.values)[0], "status"), "failed");
        for (index, before) in [(1, &unselected_before), (2, &queued_before)] {
            let restored = entries(&current.values)[index].clone();
            assert_eq!(field(&restored, "status"), "failed");
            for key in [
                "conversationId",
                "content",
                "attachments",
                "modelSelection",
                "historyConsent",
            ] {
                assert_eq!(restored[key], before[key]);
            }
        }
        let restored = entries(&current.values)[0].clone();
        for key in [
            "conversationId",
            "content",
            "attachments",
            "applicationContext",
        ] {
            assert_eq!(restored[key], legacy_before[key]);
        }
        assert_eq!(
            reloaded.read_attachment(&attachment.attachment_id).unwrap(),
            b"123".to_vec()
        );
        assert!(
            !reloaded
                .tenant_switch_blockers()
                .contains(&"attachment_upload_pending".to_owned())
        );
        assert_eq!(
            reloaded
                .patch_conversation_session(patch(
                    original.session_id,
                    Value::Null,
                    Value::Array(vec![])
                ))
                .unwrap_err(),
            "ai_profile_changed"
        );
        assert!(reloaded.claim_follow_up().unwrap().is_none());
        reloaded.shutdown();
        server.shutdown().unwrap();
        fs::remove_dir_all(owner).unwrap();
    }
}
