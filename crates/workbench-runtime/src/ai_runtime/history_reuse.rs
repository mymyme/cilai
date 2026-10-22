use super::PersistedAiState;
use crate::{codex_threads, work_classification};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

#[expect(
    clippy::too_many_lines,
    reason = "复用必须在同一核对中检查全部原生轮次与业务映射"
)]
pub(super) fn verify(
    state: &PersistedAiState,
    conversation: &str,
    journal: &Value,
    turns: &[Value],
) -> Result<Value, String> {
    let thread = journal["binding"]["nativeThreadId"]
        .as_str()
        .ok_or("history_binding_missing")?;
    let operations = journal["operations"]
        .as_array()
        .ok_or("history_journal_invalid")?;
    let mut run_turns = HashMap::new();
    let mut run_refs = Vec::new();
    for run in state
        .runs
        .iter()
        .filter(|run| run.conversation_id == conversation)
    {
        if run.regeneration_draft.is_some()
            || run.execution_target != "model"
            || run.status != "completed"
            || !run.previous_revisions.is_empty()
            || run.partial_output.as_ref().is_some_and(|s| !s.is_empty())
        {
            return Err("history_run_requires_import".into());
        }
        let accepted = operations
            .iter()
            .filter(|op| {
                op["state"] == "acknowledged"
                    && op["request"]["kind"] == "send"
                    && op["request"]["runId"] == run.run_id
            })
            .collect::<Vec<_>>();
        let [op] = accepted.as_slice() else {
            return Err("history_run_mapping_incomplete".into());
        };
        if op["receipt"]["nativeThreadId"] != thread {
            return Err("history_run_in_previous_generation".into());
        }
        let turn = turns
            .iter()
            .find(|turn| turn["id"] == op["receipt"]["nativeTurnId"])
            .ok_or("history_run_turn_missing")?;
        if turn["status"] != "completed" {
            return Err("history_run_status_mismatch".into());
        }
        let raw = codex_threads::turn_final_reply(turn).ok_or("history_reply_missing")?;
        let clean = run
            .classification_nonce
            .as_deref()
            .map_or(raw.as_str(), |nonce| {
                work_classification::without_appended_metadata(&raw, nonce, false)
            });
        if run.output.as_deref() != Some(work_classification::reply(clean)?.as_str()) {
            return Err("history_run_output_mismatch".into());
        }
        run_turns.insert(run.run_id.as_str(), turn);
        run_refs.push(json!({"runId":run.run_id,"nativeTurnId":turn["id"]}));
    }
    let messages = state
        .messages
        .iter()
        .filter(|message| message.conversation_id == conversation)
        .collect::<Vec<_>>();
    let expected_users = messages.iter().filter(|m| m.role == "user").count();
    let actual_users = turns
        .iter()
        .flat_map(|turn| turn["items"].as_array().into_iter().flatten())
        .filter(|item| item["type"] == "userMessage")
        .count();
    if expected_users != actual_users {
        return Err("history_user_count_mismatch".into());
    }
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for message in messages {
        let run_id = message
            .run_id
            .as_deref()
            .ok_or("history_message_run_missing")?;
        let turn = run_turns
            .get(run_id)
            .ok_or("history_message_turn_missing")?;
        let items = turn["items"].as_array().ok_or("history_items_invalid")?;
        let (item, client, body) = if message.role == "user" {
            let link = journal["messages"]
                .as_array()
                .ok_or("history_journal_invalid")?
                .iter()
                .rev()
                .find(|link| {
                    link["messageId"] == message.message_id
                        && link["nativeThreadId"] == thread
                        && link["nativeTurnId"] == turn["id"]
                })
                .ok_or("history_message_link_missing")?;
            let item = items
                .iter()
                .find(|item| {
                    item["type"] == "userMessage" && item["clientId"] == link["clientMessageId"]
                })
                .ok_or("history_user_item_missing")?;
            (
                item,
                link["clientMessageId"].clone(),
                codex_threads::user_text(item),
            )
        } else if message.role == "assistant" {
            let item = items
                .iter()
                .rev()
                .find(|item| {
                    item["type"] == "agentMessage"
                        && (item["phase"].is_null() || item["phase"] == "final_answer")
                })
                .ok_or("history_reply_missing")?;
            let raw = item["text"].as_str().ok_or("history_reply_invalid")?;
            let run = state
                .runs
                .iter()
                .find(|run| run.run_id == run_id)
                .ok_or("history_run_missing")?;
            let cleaned = run.classification_nonce.as_deref().map_or(raw, |nonce| {
                work_classification::without_appended_metadata(raw, nonce, false)
            });
            (item, Value::Null, work_classification::reply(cleaned)?)
        } else {
            return Err("history_message_role_invalid".into());
        };
        if body != message.content || !seen.insert(item["id"].to_string()) {
            return Err("history_message_content_mismatch".into());
        }
        refs.push(json!({"messageId":message.message_id,"role":message.role,"runId":run_id,"nativeTurnId":turn["id"],"nativeItemId":item["id"],"clientMessageId":client,"bodyFormat":"native","contentDigest":super::sha256_hex(&body)}));
    }
    Ok(json!({"reuseExisting":true,"messages":refs,"runs":run_refs}))
}
