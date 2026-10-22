use super::internal_error;
use super::invalid_request;
use super::thread_input::ensure_direct_input_allowed;
use super::thread_lifecycle::ThreadShutdownResult;
use super::thread_lifecycle::wait_for_thread_shutdown;
use super::thread_processor::ThreadRequestProcessor;
use codex_app_server_protocol::ClientResponsePayload;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::ThreadHistoryImportParams;
use codex_app_server_protocol::ThreadHistoryImportResponse;
use codex_app_server_protocol::ThreadHistoryImportRole;
use codex_protocol::ThreadId;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::AgentMessageItem;
use codex_protocol::items::PlanItem;
use codex_protocol::items::TurnItem;
use codex_protocol::items::UserMessageItem;
use codex_protocol::models::ContentItem;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::TurnCompleteEvent;
use codex_protocol::protocol::TurnStartedEvent;
use codex_protocol::user_input::UserInput;
use codex_rollout::RolloutItem;
use codex_thread_store::PersistContext;
use std::collections::HashSet;

impl ThreadRequestProcessor {
    pub(crate) async fn thread_history_import(
        &self,
        params: ThreadHistoryImportParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let thread_id = ThreadId::from_string(&params.thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))?;
        let expected = history_items(thread_id, &params)?;
        let thread = self
            .thread_manager
            .get_thread(thread_id)
            .await
            .map_err(|_| invalid_request("start or resume the dedicated import thread first"))?;
        ensure_direct_input_allowed(&thread).await?;
        if matches!(thread.agent_status().await, AgentStatus::Running) {
            return Err(invalid_request("cannot import into an active thread"));
        }
        self.thread_store
            .persist_thread(thread_id, PersistContext::Standard)
            .await
            .map_err(|err| internal_error(format!("cannot persist import thread: {err}")))?;
        thread
            .flush_rollout()
            .await
            .map_err(|err| internal_error(format!("cannot flush import thread: {err}")))?;
        let stored = self.thread_store
            .load_latest_model_context(codex_thread_store::LoadThreadHistoryParams { thread_id, include_archived: false })
            .await
            .map_err(|err| internal_error(format!("cannot read import history: {err}")))?;
        let offset = imported_prefix(&stored.items, &expected)?;
        if offset < expected.len() {
            thread
                .append_rollout_items(&expected[offset..])
                .await
                .map_err(|err| internal_error(format!("cannot append import history: {err}")))?;
            thread
                .flush_rollout()
                .await
                .map_err(|err| internal_error(format!("cannot flush imported history: {err}")))?;
        }
        // The next resume rebuilds model context from the same canonical history used by reads.
        match wait_for_thread_shutdown(&thread).await {
            ThreadShutdownResult::Complete => {}
            ThreadShutdownResult::SubmitFailed => {
                return Err(internal_error("import thread shutdown failed"));
            }
            ThreadShutdownResult::TimedOut => {
                return Err(internal_error("import thread shutdown timed out"));
            }
        }
        self.prepare_thread_for_removal(thread_id, "history import")
            .await;
        Ok(Some(
            ThreadHistoryImportResponse {
                thread_id: params.thread_id,
                turn_count: params.turns.len() as u32,
                message_count: params
                    .turns
                    .iter()
                    .map(|turn| turn.messages.len() as u32)
                    .sum(),
            }
            .into(),
        ))
    }
}

fn history_items(
    thread_id: ThreadId,
    params: &ThreadHistoryImportParams,
) -> Result<Vec<RolloutItem>, JSONRPCErrorError> {
    let mut turn_ids = HashSet::new();
    let mut message_ids = HashSet::new();
    let mut result = Vec::new();
    let mut total_bytes = 0usize;
    if params.turns.len() > 10_000 {
        return Err(invalid_request(
            "import must contain at most 10000 turns",
        ));
    }
    for turn in &params.turns {
        if turn.id.is_empty()
            || turn.id.len() > 256
            || !turn_ids.insert(&turn.id)
            || turn.started_at < 0
            || turn.completed_at < turn.started_at
            || (turn.messages.is_empty() && turn.display_notes.is_none())
        {
            return Err(invalid_request("invalid or duplicate import turn"));
        }
        result.push(RolloutItem::EventMsg(EventMsg::TurnStarted(
            TurnStartedEvent {
                turn_id: turn.id.clone(),
                trace_id: None,
                started_at: Some(turn.started_at),
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            },
        )));
        if let Some(context) = &turn.model_context {
            total_bytes = total_bytes.saturating_add(context.len());
            if total_bytes > 64 * 1024 * 1024 { return Err(invalid_request("import exceeds 64 MiB")); }
            for fragment in model_fragments(context) {
                result.push(RolloutItem::ResponseItem(ResponseItem::Message {
                    id: Some(codex_protocol::ResponseItemId::from_server(format!("history-context:{}",turn.id))),
                    role: "developer".into(), content: vec![ContentItem::InputText {text:fragment.into()}], phase:None,
                    internal_chat_message_metadata_passthrough:None,
                }.into()));
            }
        }
        let mut last_agent_message = None;
        for message in &turn.messages {
            total_bytes = total_bytes.saturating_add(message.text.len());
            if message.id.is_empty()
                || message.id.len() > 256
                || !message_ids.insert(&message.id)
                || total_bytes > 64 * 1024 * 1024
                || message_ids.len() > 20_000
            {
                return Err(invalid_request(
                    "invalid, duplicate or oversized import message",
                ));
            }
            let (role, phase, item) = match message.role {
                ThreadHistoryImportRole::User => (
                    "user",
                    None,
                    TurnItem::UserMessage(UserMessageItem {
                        id: message.id.clone(),
                        client_id: Some(message.id.clone()),
                        content: vec![UserInput::Text {
                            text: message.text.clone(),
                            text_elements: vec![],
                        }],
                    }),
                ),
                ThreadHistoryImportRole::Assistant => {
                    last_agent_message = Some(message.text.clone());
                    (
                        "assistant",
                        Some(MessagePhase::FinalAnswer),
                        TurnItem::AgentMessage(AgentMessageItem {
                            id: message.id.clone(),
                            content: vec![AgentMessageContent::Text {
                                text: message.text.clone(),
                            }],
                            phase: Some(MessagePhase::FinalAnswer),
                            memory_citation: None,
                            delivery: None,
                            questions: None,
                        }),
                    )
                }
            };
            for fragment in model_fragments(&message.text) {
                let content = match message.role {
                    ThreadHistoryImportRole::User => ContentItem::InputText { text: fragment.into() },
                    ThreadHistoryImportRole::Assistant => ContentItem::OutputText { text: fragment.into() },
                };
                result.push(RolloutItem::ResponseItem(ResponseItem::Message {
                    id: Some(codex_protocol::ResponseItemId::from_server(message.id.clone())),
                    role: role.into(), content: vec![content], phase: phase.clone(),
                    internal_chat_message_metadata_passthrough: None,
                }.into()));
            }
            result.push(RolloutItem::EventMsg(EventMsg::ItemCompleted(
                ItemCompletedEvent {
                    thread_id,
                    turn_id: turn.id.clone(),
                    item,
                    started_at_ms: Some(turn.started_at.saturating_mul(1000)),
                    completed_at_ms: turn.completed_at.saturating_mul(1000),
                },
            )));
        }
        if let Some(text) = &turn.display_notes {
            total_bytes = total_bytes.saturating_add(text.len());
            if total_bytes > 64 * 1024 * 1024 {
                return Err(invalid_request("oversized import notes"));
            }
            result.push(RolloutItem::EventMsg(EventMsg::ItemCompleted(
                ItemCompletedEvent {
                    thread_id,
                    turn_id: turn.id.clone(),
                    item: TurnItem::Plan(PlanItem {
                        id: format!("history-notes:{}", turn.id),
                        text: text.clone(),
                    }),
                    started_at_ms: Some(turn.started_at.saturating_mul(1000)),
                    completed_at_ms: turn.completed_at.saturating_mul(1000),
                },
            )));
        }
        result.push(RolloutItem::EventMsg(EventMsg::TurnComplete(
            TurnCompleteEvent {
                turn_id: turn.id.clone(),
                last_agent_message,
                error: turn.error.as_ref().map(|message| ErrorEvent {
                    message: message.clone(),
                    codex_error_info: None,
                    misalignment: None,
                }),
                started_at: Some(turn.started_at),
                completed_at: Some(turn.completed_at),
                duration_ms: Some((turn.completed_at - turn.started_at).saturating_mul(1000)),
                time_to_first_token_ms: None,
            },
        )));
    }
    Ok(result)
}

fn model_fragments(mut text: &str) -> Vec<&str> {
    let mut fragments = Vec::new();
    while text.len() > 8000 {
        let mut end = 8000;
        while !text.is_char_boundary(end) { end -= 1; }
        fragments.push(&text[..end]);
        text = &text[end..];
    }
    fragments.push(text);
    fragments
}

fn imported_prefix(
    stored: &[RolloutItem],
    expected: &[RolloutItem],
) -> Result<usize, JSONRPCErrorError> {
    let mut found = Vec::new();
    for item in stored {
        match item {
            RolloutItem::EventMsg(
                EventMsg::TurnStarted(_) | EventMsg::TurnComplete(_) | EventMsg::ItemCompleted(_),
            ) => found.push(item),
            RolloutItem::ResponseItem(response) if matches!(&response.item, ResponseItem::Message { role, id, .. } if role == "user" || role == "assistant" || (role == "developer" && id.as_ref().is_some_and(|id| id.as_str().starts_with("history-context:")))) => {
                found.push(item)
            }
            RolloutItem::SessionMeta(_)
            | RolloutItem::TurnContext(_)
            | RolloutItem::EventMsg(EventMsg::ThreadSettingsApplied(_)) => {}
            RolloutItem::ResponseItem(response) if matches!(&response.item, ResponseItem::Message { role, .. } if role == "developer" || role == "system") =>
                {}
            _ => {
                return Err(invalid_request(
                    "thread contains history outside this import",
                ));
            }
        }
    }
    if found.len() > expected.len() {
        return Err(invalid_request(
            "thread contains history outside this import",
        ));
    }
    for (actual, expected) in found.iter().zip(expected) {
        if serde_json::to_value(actual).map_err(|err| internal_error(err.to_string()))?
            != serde_json::to_value(expected).map_err(|err| internal_error(err.to_string()))?
        {
            return Err(invalid_request(
                "import content differs from persisted history",
            ));
        }
    }
    Ok(found.len())
}

#[cfg(test)]
#[path = "thread_history_import_tests.rs"]
mod tests;
