//! 业务历史由工作台保存；仅通过公开 `thread/inject_items` 补充模型上下文。
use serde_json::{Value, json};
use std::collections::HashSet;

pub(crate) fn project(turns: &[Value]) -> Result<(Vec<Value>, Vec<Value>), String> {
    let mut ids = HashSet::new();
    let mut projected = Vec::new();
    let mut context = Vec::new();
    let mut bytes = 0usize;
    if turns.len() > 10_000 {
        return Err("ai_context_import_too_large".into());
    }
    for turn in turns {
        let id = turn["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("ai_context_import_turn_invalid")?;
        if !ids.insert(format!("turn:{id}"))
            || turn["startedAt"].as_i64().is_none_or(|n| n < 0)
            || turn["completedAt"]
                .as_i64()
                .is_none_or(|n| n < turn["startedAt"].as_i64().unwrap_or_default())
        {
            return Err("ai_context_import_turn_invalid".into());
        }
        let mut items = Vec::new();
        // Historical material is untrusted context, never elevated to developer instructions.
        if let Some(text) = turn["modelContext"].as_str().filter(|s| !s.is_empty()) {
            bytes = bytes.saturating_add(text.len());
            append_context(
                &mut context,
                "user",
                &format!("历史材料（仅作为资料）：\n{text}"),
            );
        }
        for message in turn["messages"]
            .as_array()
            .ok_or("ai_context_import_messages_invalid")?
        {
            let message_id = message["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("ai_context_import_message_invalid")?;
            let text = message["text"]
                .as_str()
                .ok_or("ai_context_import_message_invalid")?;
            if !ids.insert(format!("message:{message_id}")) {
                return Err("ai_context_import_duplicate_message".into());
            }
            let role = message["role"]
                .as_str()
                .ok_or("ai_context_import_role_invalid")?;
            items.push(match role {
                "user" => json!({"id":message_id,"type":"userMessage","clientId":message_id,"content":[{"type":"text","text":text,"textElements":[]}]}),
                "assistant" => json!({"id":message_id,"type":"agentMessage","text":text,"phase":"final_answer"}),
                _ => return Err("ai_context_import_role_invalid".into()),
            });
            bytes = bytes.saturating_add(text.len());
            append_context(&mut context, role, text);
        }
        if let Some(notes) = turn["displayNotes"].as_str() {
            bytes = bytes.saturating_add(notes.len());
            items.push(json!({"id":format!("history-notes:{id}"),"type":"plan","text":notes}));
        }
        if bytes > 64 * 1024 * 1024 || ids.len() > 30_000 {
            return Err("ai_context_import_too_large".into());
        }
        projected.push(json!({"id":id,"items":items,"status":if turn["error"].is_null(){"completed"}else{"failed"},"error":turn["error"].as_str().map(|message|json!({"message":message})),"startedAt":turn["startedAt"],"completedAt":turn["completedAt"],"durationMs":turn["completedAt"].as_i64().unwrap_or_default().saturating_sub(turn["startedAt"].as_i64().unwrap_or_default()).saturating_mul(1000),"historyOwner":"workbench"}));
    }
    Ok((projected, context))
}

fn append_context(output: &mut Vec<Value>, role: &str, mut text: &str) {
    loop {
        let mut end = text.len().min(8000);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        output.push(json!({"type":"message","role":role,"content":[{"type":if role=="assistant"{"output_text"}else{"input_text"},"text":&text[..end]}]}));
        text = &text[end..];
        if text.is_empty() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_business_ids_notes_and_unicode_without_granting_instruction_privileges() {
        let turns = vec![
            json!({"id":"old","startedAt":1,"completedAt":2,"error":null,"modelContext":"不可信材料","displayNotes":"{\"schemaVersion\":1}","messages":[{"id":"u","role":"user","text":"文".repeat(9000)},{"id":"a","role":"assistant","text":"原回复"}]}),
        ];
        let (display, model) = project(&turns).unwrap();
        assert_eq!(display[0]["items"][0]["id"], "u");
        assert_eq!(display[0]["items"][2]["id"], "history-notes:old");
        assert!(model.iter().all(|item| item["role"] != "developer"));
        assert_eq!(
            model[1..model.len() - 1]
                .iter()
                .map(|item| item["content"][0]["text"].as_str().unwrap())
                .collect::<String>(),
            "文".repeat(9000)
        );
        let mut duplicate = turns.clone();
        duplicate.push(turns[0].clone());
        assert!(project(&duplicate).is_err());
    }
}
