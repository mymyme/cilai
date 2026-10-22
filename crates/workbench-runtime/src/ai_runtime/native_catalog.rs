use super::{AiRuntimeManager, lock};

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
                state.native_catalog.clone(),
                state.persisted.conversations.clone(),
            )
        };
        let mut updates = Vec::new();
        for conversation in conversations {
            let id = conversation.conversation_id;
            let journal =
                server.request_result("aiThread/get", &serde_json::json!({"conversationId":id}))?;
            let scoped_service = (!journal["binding"]["runtimeScope"].is_null())
                .then(|| service.for_runtime_scope(&journal["binding"]["runtimeScope"]))
                .transpose()?;
            let thread = if let Some((thread, _, _)) = bindings.get(&id) {
                thread.clone()
            } else {
                let journal = server
                    .request_result("aiThread/get", &serde_json::json!({"conversationId":id}))?;
                journal["binding"]["nativeThreadId"]
                    .as_str()
                    .ok_or("native_history_binding_missing")?
                    .to_owned()
            };
            let catalog = scoped_service
                .as_ref()
                .unwrap_or(&service)
                .catalog_for_threads([thread.as_str()])?;
            let current = catalog
                .get(&thread)
                .ok_or("native_history_thread_missing")?;
            let title = current["name"]
                .as_str()
                .ok_or("native_history_title_missing")?;
            let archived = current["archived"]
                .as_bool()
                .ok_or("native_history_archive_missing")?;
            if title != conversation.title
                || archived != conversation.archived
                || !bindings.contains_key(&id)
            {
                server.request_result("aiHistory/execute",&serde_json::json!({"action":"index","payload":{"conversationId":id,"nativeThreadId":thread,"title":title,"archived":archived}}))?;
                updates.push((id, thread, title.to_owned(), archived));
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
