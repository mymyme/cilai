use super::{AiRuntimeManager, AppServerSupervisor, ManagerState, codex_threads, lock};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

pub(super) struct Binding {
    pub conversation_id: String,
    pub thread_id: Option<String>,
}

pub(super) struct Group {
    pub service: Arc<codex_threads::CodexThreadService>,
    pub bindings: Vec<Binding>,
}

impl Group {
    pub fn catalog(&self) -> Result<BTreeMap<String, Value>, String> {
        let threads = self
            .bindings
            .iter()
            .filter_map(|binding| binding.thread_id.as_deref())
            .collect::<Vec<_>>();
        if threads.is_empty() {
            return Ok(BTreeMap::new());
        }
        self.service.catalog_for_threads(threads)
    }
}

pub(super) fn pending_conversations(state: &ManagerState) -> Vec<&str> {
    let active = state
        .persisted
        .runs
        .iter()
        .filter(|run| run.thread_writer_active)
        .map(|run| run.conversation_id.as_str())
        .collect::<HashSet<_>>();
    state
        .persisted
        .conversations
        .iter()
        .filter(|conversation| {
            !active.contains(conversation.conversation_id.as_str())
                && !state
                    .native_catalog
                    .get(&conversation.conversation_id)
                    .is_some_and(|(_, title, archived)| {
                        title == &conversation.title && *archived == conversation.archived
                    })
        })
        .map(|conversation| conversation.conversation_id.as_str())
        .collect()
}

pub(super) fn groups<'a>(
    server: &AppServerSupervisor,
    default_service: &Arc<codex_threads::CodexThreadService>,
    conversations: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<Group>, String> {
    let mut grouped = BTreeMap::<String, (Value, Vec<Binding>)>::new();
    for id in conversations {
        let journal =
            server.request_result("aiThread/get", &serde_json::json!({"conversationId":id}))?;
        let scope = journal["binding"]["runtimeScope"].clone();
        grouped
            .entry(scope.to_string())
            .or_insert_with(|| (scope, Vec::new()))
            .1
            .push(Binding {
                conversation_id: id.to_owned(),
                thread_id: journal["binding"]["nativeThreadId"]
                    .as_str()
                    .map(str::to_owned),
            });
    }
    grouped
        .into_values()
        .map(|(scope, bindings)| {
            let service = if scope.is_null() {
                Arc::clone(default_service)
            } else {
                default_service.for_runtime_scope(&scope)?
            };
            Ok(Group { service, bindings })
        })
        .collect()
}

impl AiRuntimeManager {
    pub(super) fn refresh_native_catalog(&self) -> Result<(), String> {
        let (service, server, profile, revision, bindings, conversations) = {
            let state = lock(&self.inner);
            if state.persisted.conversations.is_empty() {
                return Ok(());
            }
            (
                state
                    .codex_threads
                    .clone()
                    .ok_or("codex_thread_service_unavailable")?,
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
                state.data_dir.clone(),
                state.state_revision,
                state.native_catalog.keys().cloned().collect::<HashSet<_>>(),
                state
                    .persisted
                    .conversations
                    .iter()
                    .map(|conversation| {
                        (
                            conversation.conversation_id.clone(),
                            (conversation.title.clone(), conversation.archived),
                        )
                    })
                    .collect::<HashMap<_, _>>(),
            )
        };
        let mut updates = Vec::new();
        for group in groups(&server, &service, conversations.keys().map(String::as_str))? {
            let catalog = group.catalog()?;
            for binding in group.bindings {
                let id = binding.conversation_id;
                let thread = binding.thread_id.ok_or("native_history_binding_missing")?;
                let current = catalog
                    .get(&thread)
                    .ok_or("native_history_thread_missing")?;
                let title = current["name"]
                    .as_str()
                    .ok_or("native_history_title_missing")?;
                let archived = current["archived"]
                    .as_bool()
                    .ok_or("native_history_archive_missing")?;
                let (previous_title, previous_archived) = &conversations[&id];
                if title != previous_title
                    || archived != *previous_archived
                    || !bindings.contains(&id)
                {
                    server.request_result("aiHistory/execute", &serde_json::json!({"action":"index","payload":{"conversationId":id,"nativeThreadId":thread,"title":title,"archived":archived}}))?;
                    updates.push((id, thread, title.to_owned(), archived));
                }
            }
        }
        let mut state = lock(&self.inner);
        if state.data_dir != profile {
            return Err("ai_profile_changed".into());
        }
        if state.state_revision != revision {
            return Ok(());
        }
        for (id, thread, title, archived) in updates {
            if let Some(conversation) = state
                .persisted
                .conversations
                .iter_mut()
                .find(|conversation| conversation.conversation_id == id)
            {
                conversation.title.clone_from(&title);
                conversation.archived = archived;
                state.native_catalog.insert(id, (thread, title, archived));
                state.state_revision = state.state_revision.saturating_add(1);
            }
        }
        Ok(())
    }
}
