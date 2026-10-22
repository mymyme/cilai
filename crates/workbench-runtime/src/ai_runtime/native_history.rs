use super::{AgentRunEvent, PersistedAiState, ledger};
use crate::{codex_harness, codex_threads, work_classification};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

fn array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], String> {
    value[field]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("native_history_invalid:{field}"))
}
fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value[field]
        .as_str()
        .ok_or_else(|| format!("native_history_invalid:{field}"))
}

pub(super) fn read(
    ledger: &Value,
    service: &codex_threads::CodexThreadService,
) -> Result<(ledger::Baseline, PersistedAiState), String> {
    let (baseline, mut state) = ledger::Baseline::read(ledger)?;
    let mut services = BTreeMap::new();
    let mut bindings = Vec::new();
    for native in array(ledger, "native")? {
        let journal = &native["journal"];
        let binding = &journal["binding"];
        let thread = text(binding, "nativeThreadId")?;
        bindings.push(thread);
        if !binding["runtimeScope"].is_null() {
            services.insert(
                thread.to_owned(),
                service.for_runtime_scope(&binding["runtimeScope"])?,
            );
        }
        for operation in array(journal, "operations")? {
            if operation["state"] != "acknowledged"
                || !matches!(
                    operation["request"]["kind"].as_str(),
                    Some("create" | "resume" | "fork")
                )
            {
                continue;
            }
            let scope = &operation["request"]["input"]["runtimeScope"];
            let Some(thread) = operation["receipt"]["nativeThreadId"].as_str() else {
                continue;
            };
            if !scope.is_null() {
                let current = service.for_runtime_scope(scope)?;
                if services.get(thread).is_some_and(
                    |previous: &Arc<codex_threads::CodexThreadService>| {
                        previous.home() != current.home()
                    },
                ) {
                    return Err("model_thread_scope_conflict".into());
                }
                services.insert(thread.to_owned(), current);
            }
        }
    }
    let mut catalog = BTreeMap::new();
    for thread in bindings {
        let source = services.get(thread).map_or(service, Arc::as_ref);
        let current = source.catalog_for_threads([thread])?;
        catalog.insert(
            thread.to_owned(),
            current
                .get(thread)
                .cloned()
                .ok_or("native_history_thread_missing")?,
        );
    }
    let mut turns = BTreeMap::new();
    for reference in array(ledger, "runRefs")?
        .iter()
        .chain(array(ledger, "messageRefs")?)
    {
        let thread = text(reference, "nativeThreadId")?;
        if !turns.contains_key(thread) {
            turns.insert(
                thread.to_owned(),
                services
                    .get(thread)
                    .map_or(service, Arc::as_ref)
                    .turns(thread)?,
            );
        }
    }
    hydrate(&mut state, ledger, &catalog, &turns)?;
    let state = serde_json::from_value(state)
        .map_err(|error| format!("native_history_state_invalid:{error}"))?;
    Ok((baseline, state))
}

fn turn_for_run<'a>(
    run: &str,
    ledger: &Value,
    turns: &'a BTreeMap<String, Vec<Value>>,
) -> Result<Option<&'a Value>, String> {
    let Some(reference) = array(ledger, "runRefs")?.iter().find(|r| r["runId"] == run) else {
        return Ok(None);
    };
    let thread = text(reference, "nativeThreadId")?;
    turns
        .get(thread)
        .and_then(|items| {
            items
                .iter()
                .find(|turn| turn["id"] == reference["nativeTurnId"])
        })
        .map(Some)
        .ok_or_else(|| format!("native_history_turn_missing:{run}"))
}

fn import_notes(turn: &Value) -> Result<Option<Value>, String> {
    let expected = format!("history-notes:{}", text(turn, "id")?);
    let Some(item) = array(turn, "items")?
        .iter()
        .find(|item| item["id"] == expected)
    else {
        return Ok(None);
    };
    let notes: Value =
        serde_json::from_str(text(item, "text")?).map_err(|_| "native_history_notes_invalid")?;
    if notes["schemaVersion"] != 1 {
        return Err("native_history_notes_version_invalid".into());
    }
    Ok(Some(notes))
}

fn hydrate_handoff(
    conversation: &mut Value,
    ledger: &Value,
    turns: &BTreeMap<String, Vec<Value>>,
) -> Result<(), String> {
    let Some(source) = conversation
        .get_mut("applicationHandoff")
        .filter(|source| source["runSnapshot"].is_object())
    else {
        return Ok(());
    };
    let message_id = text(source, "assistantMessageId")?;
    let reference = array(ledger, "messageRefs")?
        .iter()
        .find(|reference| reference["messageId"] == message_id)
        .ok_or("native_history_handoff_message_missing")?;
    let turn = turns
        .get(text(reference, "nativeThreadId")?)
        .and_then(|turns| {
            turns
                .iter()
                .find(|turn| turn["id"] == reference["nativeTurnId"])
        })
        .ok_or("native_history_handoff_turn_missing")?;
    let notes = import_notes(turn)?.ok_or("native_history_handoff_notes_missing")?;
    if notes["applicationHandoff"]["runId"] != source["runId"]
        || !notes["applicationHandoff"]["events"].is_array()
    {
        return Err("native_history_handoff_notes_invalid".into());
    }
    source["runSnapshot"]["events"] = notes["applicationHandoff"]["events"].clone();
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "消息和轮次共同还原后才能校验整个会话"
)]
pub(super) fn hydrate(
    state: &mut Value,
    ledger: &Value,
    catalog: &BTreeMap<String, Value>,
    turns: &BTreeMap<String, Vec<Value>>,
) -> Result<(), String> {
    for conversation in state["conversations"]
        .as_array_mut()
        .ok_or("native_history_state_invalid")?
    {
        let id = text(conversation, "conversationId")?;
        let binding = array(ledger, "native")?
            .iter()
            .find(|record| record["conversationId"] == id)
            .map(|record| &record["journal"]["binding"])
            .ok_or("native_history_binding_missing")?;
        let thread = catalog
            .get(text(binding, "nativeThreadId")?)
            .ok_or_else(|| format!("native_history_thread_missing:{id}"))?;
        conversation["title"] = thread["name"]
            .as_str()
            .ok_or("native_history_title_missing")?
            .into();
        conversation["archived"] = thread["archived"]
            .as_bool()
            .ok_or("native_history_archive_missing")?
            .into();
        hydrate_handoff(conversation, ledger, turns)?;
    }
    recover_accepted_inputs(state, ledger, turns)?;
    let mut run_metadata = array(state, "runs")?.to_vec();
    for run in &mut run_metadata {
        let conversation = array(state, "conversations")?
            .iter()
            .find(|conversation| conversation["conversationId"] == run["conversationId"])
            .ok_or("native_history_conversation_missing")?;
        run["formalResultRequired"] =
            json!(conversation["context"]["projectId"].is_string() && run["planning"] != true);
    }
    for message in state["messages"]
        .as_array_mut()
        .ok_or("native_history_state_invalid")?
    {
        let id = text(message, "messageId")?;
        let run_id = message["runId"].as_str();
        let outbox = array(ledger, "outbox")?
            .iter()
            .find(|input| input["messageId"] == id);
        let body = if let Some(input) = outbox {
            if message["role"] != "user" {
                return Err("native_history_outbox_role_invalid".into());
            }
            text(input, "content")?.to_owned()
        } else if let Some(reference) = array(ledger, "messageRefs")?
            .iter()
            .find(|r| r["messageId"] == id)
        {
            let turn = turns
                .get(text(reference, "nativeThreadId")?)
                .and_then(|turns| {
                    turns
                        .iter()
                        .find(|turn| turn["id"] == reference["nativeTurnId"])
                })
                .ok_or("native_history_turn_missing")?;
            let item = array(turn, "items")?
                .iter()
                .find(|item| item["id"] == reference["nativeItemId"])
                .ok_or("native_history_message_item_missing")?;
            match message["role"].as_str() {
                Some("user") if item["type"] == "userMessage" => codex_threads::user_text(item),
                Some("assistant") if item["type"] == "agentMessage" => {
                    if reference["bodyFormat"] == "native" {
                        normalized_reply(text(item, "text")?, message, &run_metadata, turn)?
                    } else {
                        text(item, "text")?.into()
                    }
                }
                _ => return Err("native_history_message_role_invalid".into()),
            }
        } else {
            let run_id = run_id.ok_or("native_history_message_run_missing")?;
            let turn = turn_for_run(run_id, ledger, turns)?
                .ok_or_else(|| format!("native_history_message_turn_missing:{id}"))?;
            message_body(message, turn, ledger, &run_metadata)?
        };
        if message["contentDigest"] != super::sha256_hex(&body) {
            return Err(format!("native_history_content_mismatch:{id}"));
        }
        message["content"] = body.into();
    }
    let mut messages = array(state, "messages")?.to_vec();
    for run in state["runs"]
        .as_array_mut()
        .ok_or("native_history_state_invalid")?
    {
        let id = text(run, "runId")?.to_owned();
        run["output"] = Value::Null;
        run["partialOutput"] = Value::Null;
        run["events"] = json!([]);
        run["logTail"] = json!([]);
        run["previousRevisions"] = json!([]);
        run["regenerationDraft"] = Value::Null;
        if let Some(turn) = turn_for_run(&id, ledger, turns)? {
            if let Some(notes) = import_notes(turn)? {
                for field in [
                    "output",
                    "partialOutput",
                    "events",
                    "logTail",
                    "previousRevisions",
                    "regenerationDraft",
                ] {
                    run[field] = notes["run"][field].clone();
                }
            } else {
                run["events"] = serde_json::to_value(events(
                    turn,
                    run["startedAt"].as_i64().unwrap_or_default(),
                )?)
                .map_err(|e| e.to_string())?;
                if let Some(raw) = codex_threads::turn_final_reply(turn).or_else(|| {
                    (run["harnessProtocolVersion"]
                        .as_u64()
                        .is_some_and(|version| version >= 7)
                        && turn["items"].as_array().is_some_and(|items| {
                            items.iter().any(|item| {
                                item["type"] == "dynamicToolCall"
                                    && item["tool"] == crate::workbench_result::RESULT_TOOL
                                    && item["status"] == "completed"
                                    && item["success"] == true
                            })
                        }))
                    .then(String::new)
                }) {
                    // The message has already been reconstructed and digest-checked. Reuse
                    // that exact historical body, including the pre-nonce parser format.
                    let content = if let Some(message) = messages
                        .iter()
                        .find(|message| message["runId"] == id && message["role"] == "assistant")
                    {
                        text(message, "content")?.to_owned()
                    } else {
                        normalized_reply(&raw, &json!({"runId":id}), &run_metadata, turn)?
                    };
                    if turn["status"] == "completed"
                        && matches!(
                            run["status"].as_str(),
                            Some(
                                "queued"
                                    | "running"
                                    | "paused"
                                    | "terminating"
                                    | "waiting_permission"
                                    | "interrupted"
                            )
                        )
                    {
                        if !messages
                            .iter()
                            .any(|message| message["runId"] == id && message["role"] == "assistant")
                        {
                            messages.push(json!({"messageId":format!("message_recovered_{id}"),"conversationId":run["conversationId"],"runId":id,"role":"assistant","content":content,"attachments":[],"referencedMaterials":[],"applicationContext":null,"createdAt":turn["completedAt"].as_i64().unwrap_or_default().saturating_mul(1000)}));
                        }
                        run["status"] = json!("completed");
                        run["pid"] = Value::Null;
                        run["threadWriterActive"] = json!(false);
                        run["errorReason"] = Value::Null;
                        run["completedAt"] = json!(
                            turn["completedAt"]
                                .as_i64()
                                .unwrap_or_default()
                                .saturating_mul(1000)
                        );
                    }
                    if turn["status"] == "completed"
                        && run["status"] == "completed"
                        && run["classificationStatus"] == "pending"
                    {
                        run["classificationStatus"] = json!("failed");
                        run["classificationError"] =
                            json!("回复已恢复，业务分类的提交结果尚未确认。");
                    }
                    if run["status"] == "completed" {
                        run["output"] = json!(content);
                    } else {
                        run["partialOutput"] = json!(content);
                    }
                }
            }
        } else if messages
            .iter()
            .any(|m| m["runId"] == id && m["role"] == "assistant")
        {
            return Err(format!("native_history_run_missing:{id}"));
        }
    }
    state["messages"] = json!(messages);
    Ok(())
}

fn recover_accepted_inputs(
    state: &mut Value,
    ledger: &Value,
    turns: &BTreeMap<String, Vec<Value>>,
) -> Result<(), String> {
    let mut missing = Vec::new();
    for native in array(ledger, "native")? {
        let journal = &native["journal"];
        for operation in array(journal, "operations")? {
            if operation["state"] != "acknowledged"
                || operation["request"]["kind"] != "steer"
                || operation["generation"] != journal["binding"]["generation"]
            {
                continue;
            }
            let request = &operation["request"];
            let receipt = &operation["receipt"];
            let id = text(&request["input"], "messageId")?;
            if array(state, "messages")?
                .iter()
                .any(|message| message["messageId"] == id)
            {
                continue;
            }
            if !array(state, "runs")?
                .iter()
                .any(|run| run["runId"] == request["runId"] && run["superseded"] != true)
            {
                continue;
            }
            let turn = turns
                .get(text(receipt, "nativeThreadId")?)
                .and_then(|items| {
                    items
                        .iter()
                        .find(|turn| turn["id"] == receipt["nativeTurnId"])
                })
                .ok_or("native_history_turn_missing")?;
            let item = array(turn, "items")?
                .iter()
                .find(|item| {
                    item["type"] == "userMessage" && item["clientId"] == receipt["clientMessageId"]
                })
                .ok_or("native_history_user_item_missing")?;
            let content = codex_threads::user_text(item);
            missing.push(json!({"messageId":id,"conversationId":native["conversationId"],"role":"user","runId":request["runId"],"contentDigest":super::sha256_hex(&content),"attachments":[],"referencedMaterials":[],"applicationContext":null,"createdAt":request["input"]["createdAt"]}));
        }
    }
    if !missing.is_empty() {
        state["messages"]
            .as_array_mut()
            .ok_or("native_history_state_invalid")?
            .extend(missing);
        state["messages"]
            .as_array_mut()
            .ok_or("native_history_state_invalid")?
            .sort_by_key(|message| message["createdAt"].as_i64().unwrap_or_default());
    }
    Ok(())
}

fn message_body(
    message: &Value,
    turn: &Value,
    ledger: &Value,
    runs: &[Value],
) -> Result<String, String> {
    let items = array(turn, "items")?;
    let id = text(message, "messageId")?;
    if message["role"] == "user" {
        let conversation = array(ledger, "native")?
            .iter()
            .find(|v| v["conversationId"] == message["conversationId"])
            .ok_or("native_history_binding_missing")?;
        let link = array(&conversation["journal"], "messages")?
            .iter()
            .rev()
            .find(|link| link["messageId"] == id && link["nativeTurnId"] == turn["id"])
            .ok_or("native_history_message_link_missing")?;
        let item = items
            .iter()
            .find(|item| {
                item["type"] == "userMessage" && item["clientId"] == link["clientMessageId"]
            })
            .ok_or("native_history_user_item_missing")?;
        return Ok(codex_threads::user_text(item));
    }
    if message["role"] != "assistant" {
        return Err("native_history_message_role_invalid".into());
    }
    if let Some(item) = items
        .iter()
        .find(|item| item["type"] == "agentMessage" && item["id"] == id)
    {
        return Ok(text(item, "text")?.into());
    }
    let formal = runs.iter().any(|run| {
        run["runId"] == message["runId"]
            && run["harnessProtocolVersion"]
                .as_u64()
                .is_some_and(|version| version >= 7)
            && run["classificationNonce"].is_string()
    });
    let output = if formal {
        codex_threads::turn_final_reply(turn).unwrap_or_default()
    } else {
        codex_threads::turn_final_reply(turn).ok_or("native_history_reply_missing")?
    };
    normalized_reply(&output, message, runs, turn)
}

fn normalized_reply(
    output: &str,
    message: &Value,
    runs: &[Value],
    turn: &Value,
) -> Result<String, String> {
    let run = runs
        .iter()
        .find(|r| r["runId"] == message["runId"])
        .ok_or("native_history_run_missing")?;
    let nonce = run["classificationNonce"].as_str();
    if run["harnessProtocolVersion"]
        .as_u64()
        .is_some_and(|version| version >= 7)
        && let Some(nonce) = nonce
    {
        let submitted = array(turn, "items")?.iter().any(|item| {
            item["type"] == "dynamicToolCall"
                && item["tool"] == crate::workbench_result::RESULT_TOOL
                && item["status"] == "completed"
                && item["success"] == true
        });
        if submitted
            || (run["classificationStatus"] == "applied" && run["formalResultRequired"] != false)
        {
            let result = crate::workbench_result::from_native_turn(turn, nonce)?;
            return crate::workbench_result::reply(&result).map(|reply| reply.trim().to_owned());
        }
        return Ok(output.trim().to_owned());
    }

    // Match codex_harness live completion: ordinary JSON is user content;
    // only an envelope carrying this run's nonce can be unwrapped.
    let current = if let Some(nonce) = nonce {
        work_classification::visible_reply(output, nonce, false)?
            .ok_or("ai_response_envelope_content_invalid")?
    } else {
        output.to_owned()
    };
    let current = current.trim().to_owned();
    if let Some(expected) = message["contentDigest"].as_str()
        && super::sha256_hex(&current) != expected
    {
        // Older releases unwrapped any `reply` property. Their stored digest
        // identifies that historical representation; never change the digest
        // or accept a legacy interpretation without an exact match.
        let legacy_source = nonce.map_or(output, |nonce| {
            work_classification::without_appended_metadata(output, nonce, false)
        });
        if let Ok(legacy) = work_classification::reply(legacy_source)
            && super::sha256_hex(legacy.trim()) == expected
        {
            return Ok(legacy.trim().to_owned());
        }
        // 显示规则后来会隐藏照抄的历史信封；此前保存的正文按当时规则还原，摘要才一致。
        if let Some(nonce) = nonce
            && let Ok(Some(saved)) = work_classification::saved_visible_reply(output, nonce)
            && super::sha256_hex(saved.trim()) == expected
        {
            return Ok(saved.trim().to_owned());
        }
    }
    Ok(current)
}

pub(super) fn events(turn: &Value, created_at: i64) -> Result<Vec<AgentRunEvent>, String> {
    let collected = Arc::new(Mutex::new(Vec::new()));
    let sink = collected.clone();
    let callback: codex_harness::ProgressCallback = Arc::new(move |event| {
        let mut events = super::lock(&sink);
        let sequence = u32::try_from(events.len())
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        events.push(AgentRunEvent {
            sequence,
            event_type: event.event_type,
            item_id: event.item_id,
            phase: event.phase,
            kind: event.kind,
            status: event.status,
            summary: event.summary,
            duration_ms: event.duration_ms,
            detail_available: event.detail_available,
            read_paths: event.read_paths,
            created_at,
        });
    });
    for item in array(turn, "items")? {
        if item["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("history-notes:"))
        {
            continue;
        }
        codex_harness::handle_notification(
            &json!({"method":"item/completed","params":{"item":item}}),
            &callback,
            "reply",
        );
    }
    drop(callback);
    let result = super::lock(&collected).clone();
    Ok(result)
}

pub(super) fn detail(turn: &Value, item_id: &str) -> Result<Value, String> {
    if let Some(item) = array(turn, "items")?
        .iter()
        .find(|item| item["id"] == item_id)
    {
        return Ok(item.clone());
    }
    if let Some(notes) = import_notes(turn)?
        && let Some(item) = notes["details"]
            .as_object()
            .and_then(|details| details.get(item_id))
    {
        return Ok(item.clone());
    }
    Err("ai_run_event_detail_not_found".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn formal_main_reply_recovers_from_native_success_receipt_and_keeps_legacy_reading() {
        use serde_json::json;
        let result = json!({"schemaVersion":1,"nonce":"current","reply":"正式回复","decision":"unclassified"});
        let accepted = json!({"id":"receipt","type":"dynamicToolCall","tool":crate::workbench_result::RESULT_TOOL,
            "status":"completed","success":true,"arguments":result});
        let runs = vec![
            json!({"runId":"run","harnessProtocolVersion":7,"classificationNonce":"current","classificationStatus":"applied"}),
        ];
        let message = json!({"runId":"run","role":"assistant","contentDigest":super::super::sha256_hex("正式回复")});
        assert_eq!(
            super::normalized_reply(
                "普通最终文字",
                &message,
                &runs,
                &json!({"items":[accepted]})
            )
            .unwrap(),
            "正式回复"
        );
        assert!(
            super::normalized_reply("口头完成", &message, &runs, &json!({"items":[]})).is_err()
        );
        let old_runs = vec![
            json!({"runId":"run","harnessProtocolVersion":6,"classificationNonce":"current","classificationStatus":"applied"}),
        ];
        assert_eq!(
            super::normalized_reply(
                r#"{"schemaVersion":1,"nonce":"current","reply":"正式回复","decision":"unclassified"}"#,
                &message,
                &old_runs,
                &json!({"items":[]})
            )
            .unwrap(),
            "正式回复"
        );
        let failed_runs = vec![
            json!({"runId":"run","harnessProtocolVersion":7,"classificationNonce":"current","classificationStatus":"failed"}),
        ];
        assert_eq!(
            super::normalized_reply(
                "普通失败回复",
                &json!({"runId":"run"}),
                &failed_runs,
                &json!({"items":[]})
            )
            .unwrap(),
            "普通失败回复"
        );
    }

    #[test]
    fn recovered_events_keep_native_tool_outcomes_and_reject_missing_history() {
        let turn = serde_json::json!({"items":[
            {"id":"history-notes:turn","type":"agentMessage","text":"内部导入说明"},
            {"id":"read-success","type":"mcpToolCall","tool":"read_source","status":"completed","durationMs":33},
            {"id":"read-failure","type":"mcpToolCall","tool":"read_other_source","status":"failed","durationMs":44}
        ]});
        let events = super::events(&turn, 1234).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].item_id.as_deref(), Some("read-success"));
        assert_eq!(events[0].kind, "mcp");
        assert_eq!(events[0].status, "completed");
        assert_eq!(events[0].duration_ms, Some(33));
        assert!(events[0].detail_available);
        assert_eq!(events[1].item_id.as_deref(), Some("read-failure"));
        assert_eq!(events[1].status, "failed");
        assert_eq!(events[1].duration_ms, Some(44));
        assert!(events.iter().all(|event| event.created_at == 1234));
        assert!(super::events(&serde_json::json!({"items":"invalid"}), 1234).is_err());
    }
}
