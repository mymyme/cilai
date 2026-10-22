use super::{PersistedAiState, ledger, native_history};
use crate::{
    codex_threads::CodexThreadService, persistent_threads, supervisor::AppServerSupervisor,
};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;

fn execute(server: &AppServerSupervisor, action: &str, payload: &Value) -> Result<Value, String> {
    let started = std::time::Instant::now();
    let result = server.request_result(
        "aiHistory/execute",
        &json!({"action":action,"payload":payload}),
    );
    eprintln!(
        "ai_history_operation action={action} elapsed_ms={} status={}",
        started.elapsed().as_millis(),
        if result.is_ok() { "ok" } else { "failed" },
    );
    let operation = match action {
        "source" => "读取历史会话",
        "backup" => "备份历史会话",
        "record" => "保存会话迁移进度",
        "activate" => "完成历史会话迁移",
        _ => "恢复历史会话",
    };
    result
        .map_err(|error| format!("{operation}失败：{error}"))?
        .get("value")
        .cloned()
        .ok_or_else(|| "ai_history_response_invalid".into())
}

#[expect(clippy::too_many_lines, reason = "迁移顺序与切换门槛在同一流程中核对")]
pub(super) fn load(
    server: &AppServerSupervisor,
    service: &CodexThreadService,
    ai_dir: &Path,
) -> Result<(ledger::Baseline, PersistedAiState), String> {
    let source = execute(server, "source", &Value::Null)?;
    if source["active"] == true {
        return native_history::read(&source["ledger"], service);
    }
    let state: PersistedAiState = serde_json::from_value(source["state"].clone())
        .map_err(|e| format!("ai_history_source_invalid:{e}"))?;
    let snapshot = serde_json::to_value(&state).map_err(|e| e.to_string())?;
    let backup = execute(server, "backup", &Value::Null)?;
    let backup_dir = backup["databasePath"]
        .as_str()
        .map(Path::new)
        .ok_or("ai_history_backup_invalid")?
        .with_extension("native");
    let native_home = ai_dir.join("cilai-harness");
    if native_home.exists() {
        copy_directory(&native_home, &backup_dir)?;
    }
    let mut reports = Vec::new();
    let catalog = service.catalog()?;
    for conversation in &state.conversations {
        let id = &conversation.conversation_id;
        let history = plan(&state, id, ai_dir)?;
        let hash = persistent_threads::digest(&history);
        let existing = source["migrations"]
            .as_array()
            .ok_or("ai_history_source_invalid")?
            .iter()
            .find(|r| r["conversationId"] == *id);
        if existing.is_some_and(|r| r["sourceDigest"] != hash) {
            return Err(format!("ai_history_source_changed:{id}"));
        }
        let journal = server.request_result("aiThread/get", &json!({"conversationId":id}))?;
        let source_thread = journal["binding"]["nativeThreadId"]
            .as_str()
            .map(str::to_owned);
        let reuse_required =
            existing.is_some_and(|record| record["receipt"]["reuseExisting"] == true);
        if (existing.is_none() || reuse_required)
            && let Some(thread) = source_thread.as_deref()
        {
            if catalog.contains_key(thread) {
                let turns = service.turns(thread)?;
                match super::history_reuse::verify(&state, id, &journal, &turns) {
                    Ok(mut receipt) => {
                        receipt["verifiedDigest"] = json!(hash);
                        receipt["sourceThreadId"] = json!(thread);
                        if catalog[thread]["name"] != conversation.title {
                            service.request(
                                "thread/name/set",
                                &json!({"threadId":thread,"name":conversation.title}),
                            )?;
                        }
                        if catalog[thread]["archived"] != conversation.archived {
                            service.request(
                                if conversation.archived {
                                    "thread/archive"
                                } else {
                                    "thread/unarchive"
                                },
                                &json!({"threadId":thread}),
                            )?;
                        }
                        execute(
                            server,
                            "record",
                            &json!({"conversationId":id,"sourceDigest":hash,"nativeThreadId":thread,"status":"verified","messageCount":history["messageCount"],"receipt":receipt}),
                        )?;
                        reports.push(json!({"conversationId":id,"nativeThreadId":thread,"sourceThreadId":thread,"messageCount":history["messageCount"],"verifiedDigest":hash,"status":"verified","reuseExisting":true,"messages":receipt["messages"],"runs":receipt["runs"]}));
                        continue;
                    }
                    Err(error) if reuse_required => {
                        return Err(format!("ai_history_reused_source_changed:{id}:{error}"));
                    }
                    Err(error) => {
                        eprintln!("ai_history_import_required conversation={id} reason={error}");
                    }
                }
            } else if reuse_required {
                return Err(format!("ai_history_reused_thread_missing:{id}"));
            }
        }
        let thread_id = if let Some(thread) = existing.and_then(|r| r["nativeThreadId"].as_str()) {
            thread.to_owned()
        } else {
            execute(
                server,
                "record",
                &json!({"conversationId":id,"sourceDigest":hash,"nativeThreadId":null,"status":"prepared","messageCount":history["messageCount"],"receipt":{}}),
            )?;
            let created = service.request("thread/start",&json!({"cwd":ai_dir,"approvalPolicy":"never","sandbox":"read-only","ephemeral":false}))?;
            let thread = created["thread"]["id"]
                .as_str()
                .ok_or("ai_history_thread_create_invalid")?
                .to_owned();
            execute(
                server,
                "record",
                &json!({"conversationId":id,"sourceDigest":hash,"nativeThreadId":thread,"status":"importing","messageCount":history["messageCount"],"receipt":{"sourceThreadId":source_thread}}),
            )?;
            thread
        };
        if existing.is_some_and(|r| r["status"] == "verified") {
            verify(&history, &service.turns(&thread_id)?)?;
        } else {
            if existing
                .and_then(|r| r["nativeThreadId"].as_str())
                .is_some()
            {
                service.request("thread/resume",&json!({"threadId":thread_id,"excludeTurns":true,"approvalPolicy":"never","sandbox":"read-only"}))?;
            }
            service.import_history(
                &json!({"threadId":thread_id,"name":conversation.title,"turns":history["turns"]}),
            )?;
            verify(&history, &service.turns(&thread_id)?)?;
            service.request(
                "thread/name/set",
                &json!({"threadId":thread_id,"name":conversation.title}),
            )?;
            if conversation.archived {
                service.request("thread/archive", &json!({"threadId":thread_id}))?;
            }
            execute(
                server,
                "record",
                &json!({"conversationId":id,"sourceDigest":hash,"nativeThreadId":thread_id,"status":"verified","messageCount":history["messageCount"],"receipt":{"verifiedDigest":hash,"messages":history["messages"],"runs":history["runs"],"sourceThreadId":source_thread}}),
            )?;
        }
        reports.push(json!({"conversationId":id,"nativeThreadId":thread_id,"sourceThreadId":source_thread,"messageCount":history["messageCount"],"verifiedDigest":hash,"status":"verified","messages":history["messages"],"runs":history["runs"]}));
    }
    let pending = ledger::Baseline::default().prepare(&snapshot)?;
    let preview = preview_ledger(&pending.request, &reports)?;
    let (_, hydrated) = native_history::read(&preview, service)?;
    let mut restored = serde_json::to_value(&hydrated).map_err(|error| error.to_string())?;
    let mut expected = snapshot.clone();
    let reused: HashSet<_> = reports
        .iter()
        .filter(|report| report["reuseExisting"] == true)
        .filter_map(|report| report["conversationId"].as_str())
        .collect();
    for value in [&mut restored, &mut expected] {
        for run in value["runs"]
            .as_array_mut()
            .ok_or("ai_history_state_invalid")?
        {
            if reused.contains(
                run["conversationId"]
                    .as_str()
                    .ok_or("ai_history_state_invalid")?,
            ) {
                let object = run.as_object_mut().ok_or("ai_history_state_invalid")?;
                for field in ["events", "logTail", "partialOutput"] {
                    object.remove(field);
                }
            }
        }
    }
    if restored != expected {
        return Err("ai_history_restored_state_mismatch".into());
    }
    let report = json!({"schemaVersion":1,"databaseBackup":backup["databasePath"],"nativeBackup":backup_dir,"conversations":reports});
    std::fs::write(
        ai_dir.join("history-migration-report.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("ai_history_report_failed:{e}"))?;
    execute(
        server,
        "activate",
        &json!({"sourceDigest":source["sourceDigest"],"ledger":pending.request}),
    )?;
    let active = execute(server, "source", &Value::Null)?;
    native_history::read(&active["ledger"], service)
}

fn preview_ledger(request: &Value, reports: &[Value]) -> Result<Value, String> {
    let mut ledger = json!({"conversations":[],"messages":[],"runs":[],"entries":[],"native":[],"outbox":[],"runRefs":[],"messageRefs":[]});
    for record in request["records"]
        .as_array()
        .ok_or("ai_history_ledger_invalid")?
    {
        let collection = record["collection"]
            .as_str()
            .ok_or("ai_history_ledger_invalid")?;
        ledger[collection].as_array_mut().ok_or("ai_history_ledger_invalid")?.push(json!({"id":record["id"],"position":record["position"],"value":record["value"],"digest":"preflight"}));
    }
    for report in reports {
        let thread = &report["nativeThreadId"];
        let messages = report["messages"]
            .as_array()
            .ok_or("ai_history_report_invalid")?;
        let mappings=messages.iter().filter(|message| message["role"]=="user").map(|message|json!({"messageId":message["messageId"],"nativeTurnId":message["nativeTurnId"],"clientMessageId":message["messageId"]})).collect::<Vec<_>>();
        ledger["native"].as_array_mut().ok_or("ai_history_ledger_invalid")?.push(json!({"conversationId":report["conversationId"],"journal":{"binding":{"nativeThreadId":thread},"messages":mappings,"operations":[]}}));
        for message in messages {
            ledger["messageRefs"].as_array_mut().ok_or("ai_history_ledger_invalid")?.push(json!({"messageId":message["messageId"],"nativeThreadId":thread,"nativeTurnId":message["nativeTurnId"],"nativeItemId":message.get("nativeItemId").unwrap_or(&message["messageId"]),"bodyFormat":message.get("bodyFormat").unwrap_or(&json!("plain"))}));
        }
        for run in report["runs"]
            .as_array()
            .ok_or("ai_history_report_invalid")?
        {
            ledger["runRefs"].as_array_mut().ok_or("ai_history_ledger_invalid")?.push(json!({"runId":run["runId"],"nativeThreadId":thread,"nativeTurnId":run["nativeTurnId"]}));
        }
    }
    for collection in ["conversations", "messages", "runs", "entries"] {
        ledger[collection]
            .as_array_mut()
            .ok_or("ai_history_ledger_invalid")?
            .sort_by_key(|r| r["position"].as_u64().unwrap_or_default());
    }
    Ok(ledger)
}

fn copy_directory(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| format!("ai_history_backup_failed:{e}"))?;
    for entry in std::fs::read_dir(source).map_err(|e| format!("ai_history_backup_failed:{e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            contexthub_platform_adapter::backup_link::copy_symbolic_link(
                &entry.path(),
                &target.join(entry.file_name()),
            )
            .map_err(|error| format!("ai_history_backup_failed:{error}"))?;
        } else if kind.is_dir() {
            copy_directory(&entry.path(), &target.join(entry.file_name()))?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target.join(entry.file_name()))
                .map_err(|e| format!("ai_history_backup_failed:{e}"))?;
        } else {
            return Err(format!(
                "ai_history_backup_special_file:{}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

pub(super) fn plan(
    state: &PersistedAiState,
    conversation_id: &str,
    ai_dir: &Path,
) -> Result<Value, String> {
    let mut turns = Vec::new();
    let mut mappings = Vec::new();
    let mut runs = Vec::new();
    let mut included = HashSet::new();
    for run in state
        .runs
        .iter()
        .filter(|r| r.conversation_id == conversation_id)
    {
        let messages = state
            .messages
            .iter()
            .filter(|m| {
                m.conversation_id == conversation_id && m.run_id.as_deref() == Some(&run.run_id)
            })
            .collect::<Vec<_>>();
        let mut notes = json!({"schemaVersion":1,"run":{"output":run.output,"partialOutput":run.partial_output,"events":run.events,"logTail":run.log_tail,"previousRevisions":run.previous_revisions,"regenerationDraft":run.regeneration_draft},"details":{}});
        let detail_ids = run
            .events
            .iter()
            .filter(|e| e.detail_available)
            .filter_map(|e| e.item_id.as_deref())
            .collect::<HashSet<_>>();
        if !detail_ids.is_empty() {
            if !run
                .run_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err("ai_history_run_id_invalid".into());
            }
            let path = ai_dir
                .join("runs")
                .join(format!("{}.harness.jsonl", run.run_id));
            match std::fs::read_to_string(path) {
                Ok(log) => {
                    for line in log.lines() {
                        if let Ok(value) = serde_json::from_str::<Value>(line) {
                            let item = &value["params"]["item"];
                            if let Some(id) =
                                item["id"].as_str().filter(|id| detail_ids.contains(*id))
                            {
                                notes["details"][id] = item.clone();
                            }
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    notes["detailsUnavailable"] = json!(true);
                }
                Err(error) => return Err(format!("ai_history_detail_read_failed:{error}")),
            }
        }
        let model_context = messages
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| migration_context(message))
            .collect::<Vec<_>>()
            .join("\n");
        let imported = messages.iter().map(|message| {
            included.insert(message.message_id.clone());
            mappings.push(json!({"messageId":message.message_id,"role":message.role,"runId":run.run_id,"nativeTurnId":run.run_id,"contentDigest":super::sha256_hex(&message.content)}));
            json!({"id":message.message_id,"role":message.role,"text":message.content})
        }).collect::<Vec<_>>();
        turns.push(json!({"id":run.run_id,"startedAt":run.started_at.max(0)/1000,"completedAt":run.completed_at.unwrap_or(run.started_at).max(run.started_at).max(0)/1000,"error":run.error_reason,"messages":imported,"displayNotes":notes.to_string(),"modelContext":(!model_context.is_empty()).then_some(model_context)}));
        runs.push(json!({"runId":run.run_id,"nativeTurnId":run.run_id}));
    }
    for message in state
        .messages
        .iter()
        .filter(|m| m.conversation_id == conversation_id && !included.contains(&m.message_id))
    {
        let id = format!("history:{}", message.message_id);
        let source = state
            .conversations
            .iter()
            .find(|conversation| conversation.conversation_id == conversation_id)
            .and_then(|conversation| conversation.application_handoff.as_ref())
            .filter(|source| {
                source["assistantMessageId"] == message.message_id
                    && source["runSnapshot"].is_object()
            });
        let notes = source.map(|source| json!({"schemaVersion":1,"applicationHandoff":{"runId":source["runId"],"events":source["runSnapshot"]["events"]}}).to_string());
        turns.push(json!({"id":id,"startedAt":message.created_at.max(0)/1000,"completedAt":message.created_at.max(0)/1000,"error":null,"displayNotes":notes,"modelContext":migration_context(message),"messages":[{"id":message.message_id,"role":message.role,"text":message.content}]}));
        mappings.push(json!({"messageId":message.message_id,"role":message.role,"runId":id,"nativeTurnId":id,"contentDigest":super::sha256_hex(&message.content)}));
    }
    turns.sort_by_key(|turn| turn["startedAt"].as_i64().unwrap_or_default());
    Ok(json!({"turns":turns,"messages":mappings,"runs":runs,"messageCount":mappings.len()}))
}

pub(super) fn verify(plan: &Value, turns: &[Value]) -> Result<(), String> {
    let expected = plan["turns"].as_array().ok_or("ai_history_plan_invalid")?;
    if expected.len() != turns.len() {
        return Err("ai_history_turn_count_mismatch".into());
    }
    let mut count = 0;
    for source in expected {
        let actual = turns
            .iter()
            .find(|turn| turn["id"] == source["id"])
            .ok_or("ai_history_turn_missing")?;
        let items = actual["items"]
            .as_array()
            .ok_or("ai_history_items_invalid")?;
        let messages = source["messages"]
            .as_array()
            .ok_or("ai_history_plan_invalid")?;
        if items
            .iter()
            .filter(|item| matches!(item["type"].as_str(), Some("userMessage" | "agentMessage")))
            .count()
            != messages.len()
        {
            return Err("ai_history_message_count_mismatch".into());
        }
        for message in messages {
            let matches = items
                .iter()
                .filter(|item| item["id"] == message["id"])
                .collect::<Vec<_>>();
            let [item] = matches.as_slice() else {
                return Err("ai_history_message_missing_or_duplicate".into());
            };
            let text = match message["role"].as_str() {
                Some("user") if item["type"] == "userMessage" => {
                    crate::codex_threads::user_text(item)
                }
                Some("assistant") if item["type"] == "agentMessage" => item["text"]
                    .as_str()
                    .ok_or("ai_history_reply_invalid")?
                    .to_owned(),
                _ => return Err("ai_history_role_mismatch".into()),
            };
            if message["text"] != text {
                return Err("ai_history_content_mismatch".into());
            }
            count += 1;
        }
        if let Some(notes) = source["displayNotes"].as_str() {
            let id = format!(
                "history-notes:{}",
                source["id"].as_str().ok_or("ai_history_plan_invalid")?
            );
            if items
                .iter()
                .filter(|item| item["id"] == id && item["text"] == notes)
                .count()
                != 1
            {
                return Err("ai_history_notes_mismatch".into());
            }
        }
    }
    if plan["messageCount"] != count {
        return Err("ai_history_message_count_mismatch".into());
    }
    Ok(())
}

fn migration_context(message: &super::ConversationMessage) -> String {
    let mut context = super::model_turn_context(message);
    for attachment in message
        .attachments
        .iter()
        .filter(|attachment| attachment.media_type != "document")
    {
        write!(
            &mut context,
            "\n历史附件 {}，本机只读路径 {}。需要时读取，未读取前不得声称已看过。",
            serde_json::json!(attachment.file_name),
            serde_json::json!(attachment.local_path)
        )
        .expect("writing to a String cannot fail");
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_rejects_missing_extra_or_changed_visible_history() {
        let plan = json!({"messageCount":2,"turns":[{"id":"r","messages":[{"id":"u","role":"user","text":"原文"},{"id":"a","role":"assistant","text":"回答"}],"displayNotes":"完整活动"}]});
        let turn = json!({"id":"r","items":[{"id":"u","type":"userMessage","content":[{"type":"text","text":"原文"}]},{"id":"a","type":"agentMessage","text":"回答"},{"id":"history-notes:r","type":"plan","text":"完整活动"}]});
        verify(&plan, std::slice::from_ref(&turn)).unwrap();
        let mut changed = turn.clone();
        changed["items"][0]["content"][0]["text"] = json!("原文和内部材料");
        assert!(verify(&plan, &[changed]).is_err());
        assert!(verify(&plan, &[]).is_err());
        let mut changed = turn;
        changed["items"].as_array_mut().unwrap().pop();
        assert!(verify(&plan, &[changed]).is_err());
    }
}
