//! Explicit, evidence-checked maintenance of previously damaged history.
use super::{PersistedAiState, history_migration, ledger, native_history, sha256_hex};
use crate::{codex_threads::CodexThreadService, supervisor::AppServerSupervisor};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("text_recovery_missing:{key}"))
}

fn evidence_contains(value: &Value, target: &str) -> bool {
    let target = target.trim_end_matches('…');
    match value {
        Value::String(value) => {
            value.contains(target)
                || value
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .contains(target)
                || serde_json::from_str::<Value>(value)
                    .is_ok_and(|nested| evidence_contains(&nested, target))
        }
        Value::Object(value) => value.values().any(|v| evidence_contains(v, target)),
        Value::Array(value) => value.iter().any(|v| evidence_contains(v, target)),
        _ => false,
    }
}

fn restoration_matches(before: &str, after: &str) -> bool {
    if !before.contains('\u{fffd}') || after.contains('\u{fffd}') {
        return false;
    }
    let mut pattern = String::from("\\A");
    let mut damaged = false;
    for ch in before.chars() {
        if ch == '\u{fffd}' {
            if !damaged {
                pattern.push_str("[^\\x{fffd}]");
            }
            damaged = true;
        } else {
            damaged = false;
            pattern.push_str(&regex::escape(&ch.to_string()));
        }
    }
    pattern.push_str("\\z");
    regex::Regex::new(&pattern).is_ok_and(|pattern| pattern.is_match(after))
}

fn load(
    server: &AppServerSupervisor,
    service: &CodexThreadService,
) -> Result<(Value, ledger::Baseline, Value), String> {
    let ledger = server.request_result("aiLedger/get", &json!({}))?["ledger"].clone();
    let (baseline, state) = native_history::read(&ledger, service)?;
    let state = serde_json::to_value(state).map_err(|e| e.to_string())?;
    Ok((ledger, baseline, state))
}

/// Export canonical current history without starting models or automation workers.
/// # Errors
/// Returns an error when native history or its ledger cannot be verified.
pub fn export(
    server: &AppServerSupervisor,
    profile: &Path,
    harness: &Path,
) -> Result<Value, String> {
    let service = CodexThreadService::new(
        harness.to_path_buf(),
        crate::codex_upgrade::official_home(&profile.join("ai-runtime/cilai-harness")),
    )
    .with_history_server(server.clone());
    let (ledger, _, state) = load(server, &service)?;
    Ok(
        json!({"ledgerDigest":sha256_hex(&ledger.to_string()),"stateDigest":sha256_hex(&state.to_string()),"state":state}),
    )
}

fn verify_patches(
    state: &mut Value,
    patches: &[Value],
    root: &Path,
) -> Result<BTreeSet<String>, String> {
    let mut affected = BTreeSet::new();
    let mut pointers = BTreeSet::new();
    for patch in patches {
        let pointer = text(patch, "pointer")?;
        if !pointers.insert(pointer.to_owned()) {
            return Err("text_recovery_duplicate_patch".into());
        }
        let before = text(patch, "before")?;
        let after = text(patch, "after")?;
        if state.pointer(pointer).and_then(Value::as_str) != Some(before)
            || !restoration_matches(before, after)
        {
            return Err("text_recovery_invalid_patch".into());
        }
        let path = Path::new(text(&patch["evidence"], "file")?)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !path.starts_with(root.join("runs"))
            && !path.starts_with(root.join("cilai-harness/sessions"))
            && !path.starts_with(root.join("cilai-harness/official-runtime/sessions"))
        {
            return Err("text_recovery_evidence_scope_invalid".into());
        }
        let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if patch["evidence"]["digest"] != sha256_hex(&content) {
            return Err("text_recovery_evidence_changed".into());
        }
        let line = patch["evidence"]["line"]
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or("text_recovery_evidence_invalid")?;
        let value: Value = serde_json::from_str(
            content
                .lines()
                .nth(line - 1)
                .ok_or("text_recovery_evidence_missing")?,
        )
        .map_err(|e| e.to_string())?;
        if !evidence_contains(&value, after) {
            return Err("text_recovery_evidence_mismatch".into());
        }
        let parts = pointer.split('/').collect::<Vec<_>>();
        let collection = *parts.get(1).ok_or("text_recovery_invalid_pointer")?;
        let index: usize = parts
            .get(2)
            .ok_or("text_recovery_invalid_pointer")?
            .parse()
            .map_err(|_| "text_recovery_invalid_pointer")?;
        if !matches!(collection, "conversations" | "messages" | "runs") {
            return Err("text_recovery_invalid_pointer".into());
        }
        affected.insert(text(&state[collection][index], "conversationId")?.to_owned());
        *state
            .pointer_mut(pointer)
            .ok_or("text_recovery_invalid_pointer")? = json!(after);
    }
    Ok(affected)
}

fn verify_rebound_history(
    ledger: &Value,
    request: &Value,
    transitions: &[Value],
    service: &CodexThreadService,
    state: &Value,
) -> Result<(), String> {
    let mut preview = ledger.clone();
    for record in request["records"]
        .as_array()
        .ok_or("text_recovery_ledger_invalid")?
    {
        let collection = text(record, "collection")?;
        let stored = preview[collection]
            .as_array_mut()
            .ok_or("text_recovery_ledger_invalid")?
            .iter_mut()
            .find(|r| r["id"] == record["id"])
            .ok_or("text_recovery_record_missing")?;
        stored["value"] = record["value"].clone();
    }
    for change in transitions {
        let id = &change["conversationId"];
        let thread = &change["nativeThreadId"];
        let binding = preview["native"]
            .as_array_mut()
            .ok_or("text_recovery_ledger_invalid")?
            .iter_mut()
            .find(|r| r["conversationId"] == *id)
            .ok_or("text_recovery_binding_missing")?;
        binding["journal"]["binding"]["nativeThreadId"] = thread.clone();
        for run in change["runs"]
            .as_array()
            .ok_or("text_recovery_ledger_invalid")?
        {
            let reference = preview["runRefs"]
                .as_array_mut()
                .ok_or("text_recovery_ledger_invalid")?
                .iter_mut()
                .find(|r| r["runId"] == run["runId"])
                .ok_or("text_recovery_reference_missing")?;
            reference["nativeThreadId"] = thread.clone();
            reference["nativeTurnId"] = run["nativeTurnId"].clone();
        }
        for message in change["messages"]
            .as_array()
            .ok_or("text_recovery_ledger_invalid")?
        {
            let references = preview["messageRefs"]
                .as_array_mut()
                .ok_or("text_recovery_ledger_invalid")?;
            references.retain(|r| r["messageId"] != message["messageId"]);
            references.push(json!({"messageId":message["messageId"],"nativeThreadId":thread,"nativeTurnId":message["nativeTurnId"],"nativeItemId":message["messageId"],"bodyFormat":"plain"}));
        }
    }
    let (_, verified) = native_history::read(&preview, service)?;
    if serde_json::to_value(verified).map_err(|e| e.to_string())? != *state {
        return Err("text_recovery_precheck_mismatch".into());
    }
    Ok(())
}

fn verify_idle(ledger: &Value, state: &Value, affected: &BTreeSet<String>) -> Result<(), String> {
    for id in affected {
        if ledger["native"]
            .as_array()
            .ok_or("text_recovery_ledger_invalid")?
            .iter()
            .any(|r| {
                r["conversationId"] == *id
                    && (r["journal"]["binding"]["activeRunId"].is_string()
                        || r["journal"]["operations"].as_array().is_some_and(|ops| {
                            ops.iter().any(|op| {
                                matches!(
                                    op["state"].as_str(),
                                    Some("prepared" | "dispatched" | "uncertain")
                                )
                            })
                        }))
            })
        {
            return Err("text_recovery_active_writer".into());
        }
        if ledger["outbox"]
            .as_array()
            .ok_or("text_recovery_ledger_invalid")?
            .iter()
            .any(|r| {
                state["messages"].as_array().is_some_and(|messages| {
                    messages
                        .iter()
                        .any(|m| m["messageId"] == r["messageId"] && m["conversationId"] == *id)
                })
            })
        {
            return Err("text_recovery_pending_input".into());
        }
    }
    Ok(())
}

/// Restore only documented damaged characters, preserving business identities and old threads.
/// # Errors
/// Rejects stale plans, absent evidence, active writers and non-restoration edits.
pub fn apply(
    server: &AppServerSupervisor,
    profile: &Path,
    harness: &Path,
    plan: &Value,
) -> Result<Value, String> {
    let ai_dir = profile.join("ai-runtime");
    let root = ai_dir.canonicalize().map_err(|e| e.to_string())?;
    let service = CodexThreadService::new(
        harness.to_path_buf(),
        crate::codex_upgrade::official_home(&ai_dir.join("cilai-harness")),
    )
    .with_history_server(server.clone());
    let (ledger, baseline, mut state) = load(server, &service)?;
    if plan["stateDigest"] != sha256_hex(&state.to_string())
        || plan["ledgerDigest"] != sha256_hex(&ledger.to_string())
    {
        return Err("text_recovery_source_changed".into());
    }
    let patches = plan["patches"]
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("text_recovery_empty")?;
    let affected = verify_patches(&mut state, patches, &root)?;
    verify_idle(&ledger, &state, &affected)?;
    let operation = uuid::Uuid::new_v4().to_string();
    let backup = server.request_result(
        "aiHistory/execute",
        &json!({"action":"recoveryBackup","payload":{}}),
    )?;
    let backup_path =
        Path::new(text(&backup["value"], "databasePath")?).with_extension("recovery.json");
    fs::write(
        &backup_path,
        serde_json::to_vec(
            &json!({"operationId":operation,"plan":plan,"before":load(server,&service)?.2}),
        )
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let restored: PersistedAiState =
        serde_json::from_value(state.clone()).map_err(|e| e.to_string())?;
    let mut transitions = Vec::new();
    for id in &affected {
        let conversation = restored
            .conversations
            .iter()
            .find(|c| c.conversation_id == *id)
            .ok_or("text_recovery_conversation_missing")?;
        let history = history_migration::plan(&restored, id, &ai_dir)?;
        let created = service.request(
            "thread/start",
            &json!({"cwd":ai_dir,"approvalPolicy":"never","sandbox":"read-only","ephemeral":false}),
        )?;
        let thread = text(&created["thread"], "id")?;
        service.import_history(
            &json!({"threadId":thread,"name":conversation.title,"turns":history["turns"]}),
        )?;
        history_migration::verify(&history, &service.turns(thread)?)?;
        service.request(
            "thread/name/set",
            &json!({"threadId":thread,"name":conversation.title}),
        )?;
        if conversation.archived {
            service.request("thread/archive", &json!({"threadId":thread}))?;
        }
        let binding = ledger["native"]
            .as_array()
            .ok_or("text_recovery_ledger_invalid")?
            .iter()
            .find(|r| r["conversationId"] == *id)
            .ok_or("text_recovery_binding_missing")?;
        transitions.push(json!({"conversationId":id,"previousThreadId":binding["journal"]["binding"]["nativeThreadId"],"nativeThreadId":thread,"title":conversation.title,"archived":conversation.archived,"runs":history["runs"],"messages":history["messages"]}));
    }
    let mut pending = baseline.prepare(&state)?;
    pending.request["pendingInputs"] = json!([]);
    pending.request["deliveryInputs"] = json!([]);
    verify_rebound_history(&ledger, &pending.request, &transitions, &service, &state)?;
    let receipt = server.request_result("aiHistory/execute", &json!({"action":"recoverText","payload":{"operationId":operation,"expectedLedgerDigest":plan["ledgerDigest"],"ledger":pending.request,"transitions":transitions}}))?;
    let (_, _, actual) = load(server, &service)?;
    if actual != state {
        return Err("text_recovery_postcheck_mismatch".into());
    }
    Ok(
        json!({"operationId":operation,"fields":patches.len(),"conversations":affected.len(),"backup":backup["value"]["databasePath"],"verified":true,"receipt":receipt}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restoration_preserves_every_intact_character_and_requires_evidence() {
        assert!(restoration_matches(
            "测试��能/蓝图.png",
            "测试技能/蓝图.png"
        ));
        assert!(!restoration_matches(
            "测试��能/蓝图.png",
            "其他目录/蓝图.png"
        ));
        assert!(!restoration_matches("测试技能", "测试技能"));
        assert!(evidence_contains(
            &json!({"arguments":"{\"path\":\"测试技能/蓝图.png\"}"}),
            "测试技能/蓝图.png"
        ));
        assert!(!evidence_contains(
            &json!({"path":"其他图片.png"}),
            "测试技能/蓝图.png"
        ));
    }

    #[test]
    fn restores_native_history_and_artifact_metadata_then_reopens() {
        use super::super::{AgentRun, ConversationMessage, tests as fixtures};
        let _guard = crate::supervisor::app_server_test_guard();
        let root = super::super::model_test_fixture::fixture_root("text-recovery");
        let profile = super::super::model_test_fixture::profile_data_root(&root);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &profile,
                &profile.join("run"),
            )
            .unwrap();
        let mut conversation = fixtures::fixture_conversation("agent");
        conversation.title = "图��验收".into();
        conversation.context.workspace_root = root.to_string_lossy().into_owned();
        let user = ConversationMessage {
            content: "核对原始图片".into(),
            run_id: Some("old-run".into()),
            ..fixtures::fixture_message(&conversation.conversation_id)
        };
        let reply = ConversationMessage {
            message_id: "reply".into(),
            role: "assistant".into(),
            content: "完整中��原文".into(),
            ..user.clone()
        };
        let run: AgentRun = serde_json::from_value(json!({"runId":"old-run","conversationId":conversation.conversation_id,"agentId":"agent","inputMessageId":user.message_id,"provider":"codex","status":"completed","startedAt":1000,"completedAt":2000,"output":reply.content,"artifacts":[{"artifactId":"image","artifactType":"image","label":"蓝图.png","path":"测试��能/蓝图.png"}]})).unwrap();
        let saved = PersistedAiState {
            conversations: vec![conversation.clone()],
            messages: vec![user, reply],
            runs: vec![run],
            ..Default::default()
        };
        server
            .request_result(
                "aiState/replace",
                &json!({"expectedRevision":0,"state":saved}),
            )
            .unwrap();
        let ai_dir = profile.join("ai-runtime");
        fs::create_dir_all(ai_dir.join("runs")).unwrap();
        let harness =
            std::path::PathBuf::from(super::super::bundled_codex_harness_executable().unwrap());
        let service = super::super::codex_thread_service(&ai_dir, &server).unwrap();
        history_migration::load(&server, &service, &ai_dir).unwrap();
        service.shutdown_checked().unwrap();
        let before = export(&server, &profile, &harness).unwrap();
        let evidence = ai_dir.join("runs/original.harness.jsonl");
        let original =
            json!({"title":"图像验收","reply":"完整中文原文","path":"测试技能/蓝图.png"})
                .to_string();
        fs::write(&evidence, &original).unwrap();
        let patches = [
            ("/conversations/0/title","图��验收","图像验收"),
            ("/messages/1/content","完整中��原文","完整中文原文"),
            ("/runs/0/output","完整中��原文","完整中文原文"),
            ("/runs/0/artifacts/0/path","测试��能/蓝图.png","测试技能/蓝图.png"),
        ].iter().map(|(pointer,before,after)|json!({"pointer":pointer,"before":before,"after":after,"evidence":{"file":evidence,"line":1,"digest":sha256_hex(&original)}})).collect::<Vec<_>>();
        let plan = json!({"stateDigest":before["stateDigest"],"ledgerDigest":before["ledgerDigest"],"patches":patches});
        let result = apply(&server, &profile, &harness, &plan).unwrap();
        assert_eq!(result["verified"], true);
        let ledger = server.request_result("aiLedger/get", &json!({})).unwrap();
        assert_eq!(ledger["ledger"]["messageRefs"].as_array().unwrap().len(), 2);
        let after = export(&server, &profile, &harness).unwrap();
        let mut expected = before["state"].clone();
        for patch in plan["patches"].as_array().unwrap() {
            *expected
                .pointer_mut(patch["pointer"].as_str().unwrap())
                .unwrap() = patch["after"].clone();
        }
        assert_eq!(after["state"], expected);
        assert_eq!(
            apply(&server, &profile, &harness, &plan).unwrap_err(),
            "text_recovery_source_changed"
        );
        server.shutdown().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
