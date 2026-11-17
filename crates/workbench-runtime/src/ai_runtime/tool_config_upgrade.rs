use super::{Conversation, ManagerState, SpawnModelHarnessInput, history_migration};
use crate::{context_import, persistent_threads, workbench_result};
use serde_json::json;
use std::collections::HashSet;
use std::path::Path;

pub(super) fn desired_version(conversation: &Conversation) -> i64 {
    if super::personal_assistant::is_conversation(conversation) {
        4
    } else {
        workbench_result::TOOL_CONFIG_VERSION
    }
}

pub(super) fn prepare(
    state: &ManagerState,
    data_dir: &Path,
    conversation: &Conversation,
    input: &mut SpawnModelHarnessInput,
) -> Result<(), String> {
    let journal = input.persistent_thread.journal.state()?;
    prepare_with_journal(state, data_dir, conversation, input, &journal)
}

pub(super) fn prepare_with_journal(
    state: &ManagerState,
    data_dir: &Path,
    conversation: &Conversation,
    input: &mut SpawnModelHarnessInput,
    journal: &serde_json::Value,
) -> Result<(), String> {
    let plan = &mut input.persistent_thread;
    let native = persistent_threads::validate_binding(journal, &plan.journal.owner_instance_id)?;
    if native != plan.native_thread_id {
        return Err("thread_binding_conflict".into());
    }
    let desired = desired_version(conversation);
    plan.tool_config_version = desired;
    if native.is_none() {
        return Ok(());
    }
    let version = journal["binding"]["toolConfigVersion"]
        .as_i64()
        .ok_or("ai_thread_tool_config_invalid")?;
    if version == desired {
        return Ok(());
    }
    if !(0..desired).contains(&version) || native != plan.native_thread_id {
        return Err("ai_thread_tool_config_invalid".into());
    }
    let generation = journal["binding"]["generation"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or("thread_binding_invalid")?;
    let revision = journal["binding"]["bindingRevision"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or("thread_binding_invalid")?;
    let source = prior_state(
        &state.persisted,
        &conversation.conversation_id,
        &input.run_id,
    )?;
    if !plan.workbench_history_fork {
        let history = history_migration::plan(&source, &conversation.conversation_id, data_dir)?;
        let mut turns = history["turns"]
            .as_array()
            .ok_or("ai_context_import_invalid")?
            .clone();
        if let Some(before) = &plan.fork_before_turn_id {
            let operations = journal["operations"]
                .as_array()
                .ok_or("thread_journal_invalid")?;
            let runs = operations
                .iter()
                .filter(|op| {
                    op["state"] == "acknowledged"
                        && op["generation"] == generation
                        && op["receipt"]["nativeThreadId"].as_str() == native.as_deref()
                        && op["receipt"]["nativeTurnId"] == *before
                        && matches!(op["request"]["kind"].as_str(), Some("send" | "steer"))
                })
                .filter_map(|op| op["request"]["runId"].as_str())
                .collect::<HashSet<_>>();
            if runs.len() != 1 {
                return Err("thread_edit_prefix_unavailable".into());
            }
            let position = turns
                .iter()
                .position(|turn| turn["id"].as_str().is_some_and(|id| runs.contains(id)))
                .ok_or("thread_edit_prefix_unavailable")?;
            turns.truncate(position);
        }
        let (_, items) = context_import::project(&turns)?;
        plan.workbench_history = turns;
        plan.history = items;
    }
    plan.workbench_history_fork = true;
    plan.tool_config_upgrade = Some(persistent_threads::ToolConfigUpgrade {
        source_generation: generation,
        source_binding_revision: revision,
        history_digest: persistent_threads::digest(
            &json!({"display":plan.workbench_history,"items":plan.history}),
        ),
    });
    Ok(())
}

fn prior_state(
    state: &super::PersistedAiState,
    conversation: &str,
    current_run: &str,
) -> Result<super::PersistedAiState, String> {
    let mut source = state.clone();
    let current = source
        .runs
        .iter()
        .position(|run| run.run_id == current_run && run.conversation_id == conversation)
        .ok_or("agent_run_not_found")?;
    let excluded = source.runs[current..]
        .iter()
        .filter(|run| run.conversation_id == conversation)
        .map(|run| run.run_id.clone())
        .collect::<HashSet<_>>();
    source.runs.retain(|run| !excluded.contains(&run.run_id));
    source.messages.retain(|message| {
        message.conversation_id != conversation
            || message
                .run_id
                .as_ref()
                .is_none_or(|id| !excluded.contains(id))
    });
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(id: &str, conversation: &str) -> super::super::AgentRun {
        serde_json::from_value(json!({"runId":id,"conversationId":conversation,"agentId":"agent","inputMessageId":format!("m-{id}"),"provider":"codex","executionTarget":"model","status":"completed","startedAt":1})).unwrap()
    }
    fn message(id: &str, conversation: &str) -> super::super::ConversationMessage {
        serde_json::from_value(json!({"messageId":format!("m-{id}"),"conversationId":conversation,"role":"user","content":format!("原文 {id}\nUnicode保持"),"runId":id,"createdAt":1})).unwrap()
    }
    #[test]
    fn queued_upgrade_uses_all_finished_predecessors_and_excludes_current_and_later_inputs_only_in_same_conversation()
     {
        let order = [
            ("a0", "a"),
            ("b0", "b"),
            ("a1", "a"),
            ("a2", "a"),
            ("b1", "b"),
        ];
        let state = super::super::PersistedAiState {
            runs: order.iter().map(|(i, c)| run(i, c)).collect(),
            messages: order.iter().map(|(i, c)| message(i, c)).collect(),
            ..Default::default()
        };
        let source = prior_state(&state, "a", "a1").unwrap();
        assert_eq!(
            source
                .runs
                .iter()
                .map(|r| r.run_id.as_str())
                .collect::<Vec<_>>(),
            ["a0", "b0", "b1"]
        );
        assert_eq!(source.messages[0], state.messages[0]);
        assert_eq!(source.messages[1], state.messages[1]);
        assert_eq!(source.messages[2], state.messages[4]);
        assert_eq!(state.runs.len(), 5);
        assert!(prior_state(&state, "b", "a1").is_err());
    }
    #[test]
    fn upgrade_history_preserves_original_message_ids_bytes_failures_and_valid_empty_final_phase() {
        let mut old = run("old", "a");
        old.status = "failed".into();
        old.error_reason = Some("original failure".into());
        let text = message("old", "a");
        let mut reply = text.clone();
        reply.role = "assistant".into();
        reply.message_id = "old-reply".into();
        reply.content = " 合法原文\n 结束 ".into();
        let state = super::super::PersistedAiState {
            runs: vec![old.clone(), run("new", "a")],
            messages: vec![text.clone(), reply.clone(), message("new", "a")],
            ..Default::default()
        };
        let source = prior_state(&state, "a", "new").unwrap();
        let history = history_migration::plan(&source, "a", Path::new("/synthetic")).unwrap();
        let (display, items) =
            context_import::project(history["turns"].as_array().unwrap()).unwrap();
        assert_eq!(display[0]["items"][0]["id"], text.message_id);
        assert_eq!(display[0]["items"][1]["id"], reply.message_id);
        assert_eq!(display[0]["items"][1]["text"], reply.content);
        assert!(items.iter().all(|item| item["role"] != "developer"));
        assert_eq!(source.runs[0], old);
    }
}
