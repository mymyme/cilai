use super::{
    AgentRun, AiRuntimeManager, AiRuntimeSnapshot, Conversation, ConversationMessage, ManagerState,
    PermissionRequest, ensure_profile_admission, lock, message_for_ui, run_for_ui, run_is_active,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const PAGE_SIZE: usize = 100;
const PREVIEW_LENGTH: usize = 512;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkbenchHistoryCursor {
    session_id: String,
    conversation_id: String,
    before_message_id: String,
    archived: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchHistoryPage {
    session_id: String,
    conversation_id: String,
    messages: Vec<ConversationMessage>,
    runs: Vec<AgentRun>,
    permission_requests: Vec<PermissionRequest>,
    previous_cursor: Option<WorkbenchHistoryCursor>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkbenchHistoryWindow {
    session_id: String,
    conversation_id: String,
    previous_cursor: Option<WorkbenchHistoryCursor>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogMessage {
    message_id: String,
    role: String,
    content: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CatalogPreview {
    first_prompt: Option<String>,
    latest: CatalogMessage,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbenchReadSnapshot {
    #[serde(flatten)]
    snapshot: AiRuntimeSnapshot,
    runtime_session_id: String,
    catalog_previews: HashMap<String, CatalogPreview>,
    history_window: Option<WorkbenchHistoryWindow>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 160
}

fn history(
    state: &ManagerState,
    conversation_id: &str,
    archived: bool,
    cursor: Option<&WorkbenchHistoryCursor>,
) -> Result<WorkbenchHistoryPage, String> {
    let session_id = state.session.id();
    if let Some(cursor) = cursor
        && (cursor.session_id != session_id
            || cursor.conversation_id != conversation_id
            || cursor.archived != archived
            || !valid_id(&cursor.before_message_id))
    {
        return Err("ai_history_cursor_scope_changed".into());
    }
    let mut ordered = state
        .persisted
        .messages
        .iter()
        .filter(|message| message.conversation_id == conversation_id)
        .collect::<Vec<_>>();
    // Stable order preserves ledger order when a user message and reply share a timestamp.
    ordered.sort_by_key(|message| message.created_at);
    let end = match cursor {
        None => ordered.len(),
        Some(cursor) => ordered
            .iter()
            .position(|message| message.message_id == cursor.before_message_id)
            .ok_or("ai_history_cursor_stale")?,
    };
    let start = end.saturating_sub(PAGE_SIZE);
    Ok(WorkbenchHistoryPage {
        session_id: session_id.to_owned(),
        conversation_id: conversation_id.to_owned(),
        messages: ordered[start..end]
            .iter()
            .map(|message| message_for_ui(message))
            .collect(),
        runs: Vec::new(),
        permission_requests: Vec::new(),
        previous_cursor: (start > 0).then(|| WorkbenchHistoryCursor {
            session_id: session_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            before_message_id: ordered[start].message_id.clone(),
            archived,
        }),
    })
}

fn preview(message: &ConversationMessage) -> String {
    message
        .content
        .trim_start()
        .chars()
        .take(PREVIEW_LENGTH)
        .collect()
}

fn catalog_previews(state: &ManagerState) -> HashMap<String, CatalogPreview> {
    let run_nonces = state
        .persisted
        .runs
        .iter()
        .filter_map(|run| Some((run.run_id.as_str(), run.classification_nonce.as_deref()?)))
        .collect::<HashMap<_, _>>();
    let mut pairs = HashMap::<&str, (Option<&ConversationMessage>, &ConversationMessage)>::new();
    for message in &state.persisted.messages {
        let (first, latest) = pairs
            .entry(&message.conversation_id)
            .or_insert((None, message));
        if message.role == "user"
            && first.is_none_or(|current| message.created_at < current.created_at)
        {
            *first = Some(message);
        }
        if message.created_at >= latest.created_at {
            *latest = message;
        }
    }
    pairs
        .into_iter()
        .map(|(id, (first, latest))| {
            let classification_nonce = latest
                .run_id
                .as_deref()
                .and_then(|id| run_nonces.get(id).copied());
            let reply = classification_nonce
                .filter(|_| latest.role == "assistant")
                .and_then(|nonce| {
                    crate::work_classification::saved_visible_reply(&latest.content, nonce)
                        .ok()
                        .flatten()
                });
            (
                id.to_owned(),
                CatalogPreview {
                    first_prompt: first.map(preview),
                    latest: CatalogMessage {
                        message_id: latest.message_id.clone(),
                        role: latest.role.clone(),
                        content: reply
                            .as_deref()
                            .unwrap_or(&latest.content)
                            .trim_start()
                            .chars()
                            .take(PREVIEW_LENGTH)
                            .collect(),
                    },
                },
            )
        })
        .collect()
}

fn directory_conversation(conversation: &Conversation, selected: Option<&str>) -> Conversation {
    let mut projected = conversation.clone();
    if selected != Some(conversation.conversation_id.as_str()) {
        projected.application_handoff = None;
        projected.context.referenced_materials.clear();
    }
    projected
}

fn history_runs(
    state: &ManagerState,
    selected: Option<&str>,
    messages: &[ConversationMessage],
    include_active: bool,
) -> (Vec<AgentRun>, Vec<PermissionRequest>) {
    let selected_runs = messages
        .iter()
        .filter_map(|m| m.run_id.as_deref())
        .collect::<HashSet<_>>();
    let message_ids = messages
        .iter()
        .map(|m| m.message_id.as_str())
        .collect::<HashSet<_>>();
    let latest = selected.filter(|_| include_active).and_then(|id| {
        state
            .persisted
            .runs
            .iter()
            .filter(|run| run.conversation_id == id)
            .max_by_key(|run| run.started_at)
            .map(|run| run.run_id.as_str())
    });
    let runs = state
        .persisted
        .runs
        .iter()
        .filter(|run| {
            (include_active && (run_is_active(&run.status) || run.thread_writer_active))
                || (selected == Some(run.conversation_id.as_str())
                    && (latest == Some(run.run_id.as_str())
                        || selected_runs.contains(run.run_id.as_str())
                        || message_ids.contains(run.input_message_id.as_str())))
        })
        .map(run_for_ui)
        .collect::<Vec<_>>();
    let run_ids = runs
        .iter()
        .map(|run| run.run_id.as_str())
        .collect::<HashSet<_>>();
    let approvals = state
        .persisted
        .permission_requests
        .iter()
        .filter(|request| run_ids.contains(request.run_id.as_str()))
        .cloned()
        .collect();
    (runs, approvals)
}

impl AiRuntimeManager {
    /// # Errors
    /// Returns an error if the profile or requested conversation cannot be read.
    pub fn workbench_snapshot(
        &self,
        refresh_native_catalog: bool,
        conversation_id: Option<&str>,
    ) -> Result<WorkbenchReadSnapshot, String> {
        if conversation_id.is_some_and(|id| !valid_id(id)) {
            return Err("ai_history_conversation_invalid".into());
        }
        if refresh_native_catalog {
            self.refresh_native_catalog()?;
        }
        let state = lock(&self.inner);
        ensure_profile_admission(&state)?;
        let selected =
            conversation_id.filter(|id| {
                state.persisted.conversations.iter().any(|conversation| {
                    !conversation.archived && conversation.conversation_id == *id
                })
            });
        let page = selected
            .map(|id| history(&state, id, false, None))
            .transpose()?;
        let (history_window, messages) = page.map_or((None, Vec::new()), |page| {
            (
                Some(WorkbenchHistoryWindow {
                    session_id: page.session_id,
                    conversation_id: page.conversation_id,
                    previous_cursor: page.previous_cursor,
                }),
                page.messages,
            )
        });
        let (runs, permission_requests) = history_runs(&state, selected, &messages, true);
        Ok(WorkbenchReadSnapshot {
            snapshot: AiRuntimeSnapshot {
                state_revision: state.state_revision,
                models: state.model_catalog.models().to_vec(),
                agents: state.persisted.agents.clone(),
                conversations: state
                    .persisted
                    .conversations
                    .iter()
                    .map(|conversation| directory_conversation(conversation, selected))
                    .collect(),
                messages,
                runs,
                permission_requests,
                inbox_read_item_ids: state.persisted.inbox_read_item_ids.clone(),
                automation_rules: state.persisted.automation_rules.clone(),
                message_submissions: Self::message_submissions_for_ui(&state),
            },
            runtime_session_id: state.session.id().to_owned(),
            catalog_previews: catalog_previews(&state),
            history_window,
        })
    }

    /// # Errors
    /// Refuses mismatched conversation, archive state or a stale page cursor.
    pub fn workbench_history_page(
        &self,
        conversation_id: &str,
        archived: bool,
        cursor: Option<&WorkbenchHistoryCursor>,
    ) -> Result<WorkbenchHistoryPage, String> {
        if !valid_id(conversation_id) {
            return Err("ai_history_conversation_invalid".into());
        }
        let state = lock(&self.inner);
        ensure_profile_admission(&state)?;
        if !state.persisted.conversations.iter().any(|conversation| {
            conversation.conversation_id == conversation_id && conversation.archived == archived
        }) {
            return Err("ai_history_conversation_changed".into());
        }
        let mut page = history(&state, conversation_id, archived, cursor)?;
        if !archived {
            (page.runs, page.permission_requests) =
                history_runs(&state, Some(conversation_id), &page.messages, false);
        }
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn history_pages_are_disjoint_across_new_messages_and_refuse_other_sessions() {
        let manager = AiRuntimeManager::new();
        {
            let mut state = lock(&manager.inner);
            state.session.reset();
            for id in ["one", "two"] {
                state.persisted.conversations.push(serde_json::from_value(json!({
                    "conversationId":id, "title":"新对话", "conversationType":"direct", "mode":"discuss",
                    "context":{"workspaceRoot":"/test"}, "archived":false, "createdAt":1, "updatedAt":1
                })).unwrap());
            }
            for index in 0..225 {
                state.persisted.messages.push(
                    serde_json::from_value(json!({
                        "messageId":format!("m-{index}"), "conversationId":"one", "role":"user",
                        "content": "字".repeat(1200), "createdAt":index/2
                    }))
                    .unwrap(),
                );
            }
            state.persisted.messages.push(
                serde_json::from_value(json!({
                    "messageId":"private-other", "conversationId":"two", "role":"assistant",
                    "content":"第二个对话".repeat(200), "createdAt":1
                }))
                .unwrap(),
            );
        }
        let initial = manager.workbench_snapshot(false, Some("one")).unwrap();
        assert_eq!(initial.snapshot.messages.len(), 100);
        assert_eq!(initial.snapshot.messages[0].message_id, "m-125");
        assert_eq!(initial.snapshot.messages[99].message_id, "m-224");
        assert_eq!(initial.snapshot.messages[0].content.chars().count(), 1200);
        assert_eq!(
            initial.catalog_previews["two"]
                .latest
                .content
                .chars()
                .count(),
            512
        );
        assert!(
            initial
                .snapshot
                .messages
                .iter()
                .all(|message| message.conversation_id == "one")
        );
        let cursor = initial.history_window.unwrap().previous_cursor.unwrap();
        {
            let mut state = lock(&manager.inner);
            state.persisted.messages.push(serde_json::from_value(json!({
                "messageId":"new", "conversationId":"one", "role":"assistant", "content":"新消息", "createdAt":200
            })).unwrap());
        }
        let earlier = manager
            .workbench_history_page("one", false, Some(&cursor))
            .unwrap();
        assert_eq!(earlier.messages[0].message_id, "m-25");
        assert_eq!(earlier.messages[99].message_id, "m-124");
        let earliest = manager
            .workbench_history_page("one", false, earlier.previous_cursor.as_ref())
            .unwrap();
        assert_eq!(earliest.messages.len(), 25);
        assert_eq!(earliest.messages[24].message_id, "m-24");
        assert!(earliest.previous_cursor.is_none());
        assert_eq!(
            manager
                .workbench_history_page("two", false, Some(&cursor))
                .unwrap_err(),
            "ai_history_cursor_scope_changed"
        );
        assert_eq!(
            manager
                .workbench_history_page("one", true, Some(&cursor))
                .unwrap_err(),
            "ai_history_conversation_changed"
        );
        lock(&manager.inner).session.reset();
        assert_eq!(
            manager
                .workbench_history_page("one", false, Some(&cursor))
                .unwrap_err(),
            "ai_history_cursor_scope_changed"
        );
    }
}
