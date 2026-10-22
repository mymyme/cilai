use super::{Arc, AtomicUsize, Duration, Fixture, Instant, Ordering, Value, json};
use crate::personal_page_ai::{PersonalPageAiManager, UiActionExecutor};
use contexthub_protocol::PersonalPageBindingParams;
use sha2::{Digest as _, Sha256};

pub(super) fn response_tool(body: &Value, has_result: bool) -> Option<Value> {
    if body.to_string().contains("PERSONAL_PAGE_EMPTY_FIXTURE_V1") {
        assert_page_tools(body, &[]);
        return None;
    }
    if body.to_string().contains("PERSONAL_PAGE_NODE_FIXTURE_V1") {
        assert_page_tools(body, &["cilai_personal_tools"]);
        return node_response(body);
    }
    if !body.to_string().contains("PERSONAL_PAGE_ACTION_FIXTURE_V1") || has_result {
        return None;
    }
    assert_page_tools(body, &["cilai_application_action"]);
    assert!(super::find_tool(&body["tools"], "cilai_application_action").is_some());
    Some(
        json!({"type":"response.output_item.done","item":{"type":"function_call",
        "call_id":"fixture-page-action","name":"cilai_application_action",
        "arguments":json!({"actionId":"note.update","arguments":{"text":"model draft"}}).to_string()}}),
    )
}

fn install_page(f: &Fixture) -> PersonalPageBindingParams {
    install_page_package(f, false)
}

fn install_page_package(f: &Fixture, node: bool) -> PersonalPageBindingParams {
    let mut permissions = json!([
        "page_ai.context",
        "page_ai.ui_actions_confirmed",
        "page_ai.model",
        "page_ai.conversation_handoff"
    ]);
    let mut manifest = json!({"schemaVersion":1,"contextTypes":[{"contextType":"note.current","schemaVersion":"1.0",
        "dataSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}}],
        "uiActions":[{"actionId":"note.update","title":"更新草稿","description":"仅更新当前页面草稿",
            "inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false},
            "riskLevel":"medium","preconditions":[],"expectedEffects":["页面草稿更新"],"confirmationPolicy":"always"}],
        "model":true,"conversationHandoff":true,"readTools":[],"writeTools":[]});
    if node {
        permissions
            .as_array_mut()
            .unwrap()
            .push(json!("page_ai.tools.read"));
        manifest["readTools"] = json!([{"name":"echo","title":"处理本页数据","description":"处理明确确认的数据",
            "inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}}]);
    }
    let mut fields = json!({"conversationId":null,"title":"隔离个人页面","businessGoal":"用户确认后仅操作当前页面",
        "targetType":if node{"web_mcp_application"}else{"web_application"},"requestedPermissions":permissions,
        "acceptanceExamples":[{"input":"更新页面","expected":"先人工确认"}],
        "sourceFiles":[{"path":"web/index.html","content":"<h1>个人页面</h1>"},{"path":"web/page-ai.json","content":manifest.to_string()}]});
    if node {
        fields["sourceFiles"]
            .as_array_mut()
            .unwrap()
            .push(json!({"path":"mcp/server.mjs","content":NODE_MCP}));
    }
    let mut request = fields.clone();
    request["clientOperationId"] = json!(format!("create-page-{}", uuid::Uuid::new_v4().simple()));
    request["requestFingerprint"] = json!(format!(
        "{:x}",
        Sha256::digest(fields.to_string().as_bytes())
    ));
    let created = f
        .server
        .request_result("capabilityCreation/create", &request)
        .unwrap();
    let project = &created["project"];
    let operation = format!("install-page-{}", uuid::Uuid::new_v4().simple());
    let prepared=f.server.request_result("personalApplicationData/prepare",&json!({
        "creationProjectId":project["creationProjectId"],"expectedRevision":project["stateRevision"],
        "packageDigest":project["packageDigest"],"clientOperationId":operation,"action":"upgrade","targetOperationId":null
    })).unwrap();
    let guard = f
        .manager
        .begin_profile_maintenance(
            &f.profile,
            prepared["operationId"].as_str().unwrap(),
            prepared["previewDigest"].as_str().unwrap(),
        )
        .unwrap();
    f.server.request_result("personalCapability/upgrade",&json!({
        "creationProjectId":project["creationProjectId"],"expectedRevision":project["stateRevision"],
        "packageDigest":project["packageDigest"],"clientOperationId":operation,"confirmedPermissions":permissions,
        "dataChange":{"operationId":prepared["operationId"],"previewDigest":prepared["previewDigest"],"quiescence":guard.quiescence()}
    })).unwrap();
    guard.finish().unwrap();
    PersonalPageBindingParams {
        creation_project_id: project["creationProjectId"].as_str().unwrap().into(),
        expected_package_digest: project["packageDigest"].as_str().unwrap().into(),
    }
}

fn dispatch(
    manager: &PersonalPageAiManager,
    doc: &str,
    method: &str,
    params: Value,
    ui: &UiActionExecutor,
) -> Result<Value, String> {
    manager.dispatch(
        doc,
        {
            let mut request =
                json!({"method":method,"requestId":uuid::Uuid::new_v4().simple().to_string()});
            request["params"] = params;
            request
        },
        Arc::clone(ui),
    )
}

fn publish(manager: &PersonalPageAiManager, doc: &str, ui: &UiActionExecutor) {
    dispatch(
        manager,
        doc,
        "context.publish",
        json!({"contextId":"current","contextType":"note.current","schemaVersion":"1.0",
        "title":"当前笔记","summary":"本页资料","data":{"text":"private-page-context-canary"}}),
        ui,
    )
    .unwrap();
}

#[test]
fn personal_page_context_is_document_bound_audited_and_cannot_approve_itself() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    let page = PersonalPageAiManager::new(f.manager.clone());
    let binding = install_page(&f);
    page.register_document("trusted_document_a", binding.clone())
        .unwrap();
    let ui: UiActionExecutor =
        Arc::new(|_, _| panic!("page must not execute without trusted approval"));
    publish(&page, "trusted_document_a", &ui);
    assert_eq!(
        dispatch(&page, "unknown_document", "session.read", json!({}), &ui).unwrap_err(),
        "personal_page_document_required"
    );
    assert_eq!(
        dispatch(
            &page,
            "trusted_document_a",
            "model.write.resolve",
            json!({"decision":"approved"}),
            &ui
        )
        .unwrap_err(),
        "personal_page_confirmation_host_required"
    );
    let repeated = json!({"method":"context.publish","requestId":"same-request","params":{"contextId":"repeated","contextType":"note.current","schemaVersion":"1.0","title":"重复请求","summary":"","data":{"text":"same-input"}}});
    let first = page
        .dispatch("trusted_document_a", repeated.clone(), Arc::clone(&ui))
        .unwrap();
    assert_eq!(
        first,
        page.dispatch("trusted_document_a", repeated, Arc::clone(&ui))
            .unwrap()
    );
    assert_eq!(page.dispatch("trusted_document_a",json!({"method":"context.revoke","requestId":"same-request","params":{"contextId":"current"}}),Arc::clone(&ui)).unwrap_err(),"personal_page_request_reused");
    assert!(dispatch(&page,"trusted_document_a","context.publish",json!({"contextId":"bad","contextType":"note.current","schemaVersion":"1.0","title":"","summary":"","data":{"unexpected":true}}),&ui).is_err());
    page.close_document("trusted_document_a").unwrap();
    assert_eq!(
        dispatch(&page, "trusted_document_a", "session.read", json!({}), &ui).unwrap_err(),
        "personal_page_scope_revoked"
    );
    let audit=f.server.request_result("personalPage/auditList",&json!({"creationProjectId":binding.creation_project_id,"limit":200,"beforeSequence":null})).unwrap();
    assert!(!audit.to_string().contains("private-page-context-canary"));
    assert!(audit["items"].as_array().unwrap().iter().any(
        |item| item["record"]["request"]["method"] == "session_close"
            && item["record"]["outcome"] == "succeeded"
    ));
    assert_eq!(
        f.server
            .request_result("profileWriter/list", &json!({}))
            .unwrap(),
        json!([])
    );
    f.manager.shutdown_checked().unwrap();
}

fn page_run(page: &PersonalPageAiManager, session: &str, ui: &UiActionExecutor) -> Value {
    dispatch(page,"model_document","model.run.create",json!({"modelSessionId":session,"modelId":"isolated-official-fixture",
        "prompt":"PERSONAL_PAGE_ACTION_FIXTURE_V1 请提议更新页面","clientMessageId":uuid::Uuid::new_v4().simple().to_string()}),ui).unwrap()
}

fn await_page_write(page: &PersonalPageAiManager, run: &str, ui: &UiActionExecutor) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = dispatch(
            page,
            "model_document",
            "model.run.get",
            json!({"runId":run}),
            ui,
        )
        .unwrap();
        if snapshot["writeRequest"]["approval"]["executionStatus"] == "awaiting_confirmation" {
            return snapshot["writeRequest"]["approval"].clone();
        }
        assert!(
            !matches!(snapshot["status"].as_str(), Some("failed" | "cancelled")),
            "{snapshot}"
        );
        assert!(Instant::now() < deadline, "{snapshot}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn await_page_end(page: &PersonalPageAiManager, run: &str, ui: &UiActionExecutor) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = dispatch(
            page,
            "model_document",
            "model.run.get",
            json!({"runId":run}),
            ui,
        )
        .unwrap();
        if snapshot["status"] == "completed" {
            return snapshot;
        }
        assert!(snapshot["status"] != "failed", "{snapshot}");
        assert!(Instant::now() < deadline, "{snapshot}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn configure_page(f: &Fixture) -> PersonalPageBindingParams {
    configure_page_target(f, false)
}

fn configure_page_target(f: &Fixture, node: bool) -> PersonalPageBindingParams {
    let validated=f.manager.personal_model_validate(serde_json::from_value(json!({
        "runtimeKind":"application","connectionId":f.connection.connection_id,"modelId":"isolated-official-fixture",
        "layer":"tools","expectedConfigRevision":f.connection.config_revision,"expectedAuthGeneration":f.connection.auth_generation
    })).unwrap()).unwrap();
    assert_eq!(
        validated.outcome,
        Some(contexthub_protocol::ModelValidationOutcome::Passed),
        "{validated:?}"
    );
    let binding = install_page_package(f, node);
    f.server.request_result("modelPurpose/save",&json!({"binding":{"bindingId":"fixture-page-model","purpose":"page_ai","scopeId":null,"policy":"explicit",
        "selection":{"connectionId":f.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},"enabled":true,"revision":0,
        "consent":{"serviceName":"本机Responses仿真服务","historyScope":"本页上下文和页面内消息","mayIncurCost":false,"confirmedAt":1}},"expectedRevision":null})).unwrap();
    binding
}

const NODE_MCP: &str = r"import readline from 'node:readline';
import fs from 'node:fs';
for await (const line of readline.createInterface({input:process.stdin})) {
 const m=JSON.parse(line); if(!m.id) continue;
 let result;
 if(m.method==='initialize') result={protocolVersion:'2024-11-05',capabilities:{tools:{}}};
 else if(m.method==='tools/list') result={tools:[{name:'echo',inputSchema:{type:'object',properties:{text:{type:'string'}},required:['text'],additionalProperties:false},annotations:{readOnlyHint:true}}]};
 else {fs.writeFileSync('page-node-call.txt',m.params.arguments.text);result={content:[{type:'text',text:m.params.arguments.text}]};}
 console.log(JSON.stringify({jsonrpc:'2.0',id:m.id,result}));
}";

fn catalog(value: &Value) -> Option<Value> {
    match value {
        Value::Object(object)
            if object.contains_key("creationProjectId")
                && object.contains_key("packageDigest")
                && object.contains_key("tools") =>
        {
            Some(value.clone())
        }
        Value::Object(object) => object.values().find_map(catalog),
        Value::Array(items) => items.iter().find_map(catalog),
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|v| catalog(&v)),
        _ => None,
    }
}

fn assert_page_tools(body: &Value, expected: &[&str]) {
    let tools = body["tools"].as_array().expect("explicit tool directory");
    assert_eq!(
        tools.len(),
        expected.len(),
        "unexpected page tools: {tools:?}"
    );
    for tool in tools {
        assert_eq!(tool["type"], "function");
        assert!(expected.contains(&tool["name"].as_str().unwrap()), "{tool}");
    }
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    eprintln!("personal_page_wire_tool_directory={names:?}");
}

fn node_response(body: &Value) -> Option<Value> {
    if body["input"].to_string().contains("page-node-human") {
        return None;
    }
    let tool = super::find_tool(&body["tools"], "cilai_personal_tools")
        .expect("only scoped personal tool protocol");
    let args = if let Some(catalog) = catalog(&body["input"]) {
        json!({"action":"call","creationProjectId":catalog["creationProjectId"],"packageDigest":catalog["packageDigest"],"tool":"echo","arguments":{"text":"model-proposed"}})
    } else {
        json!({"action":"list"})
    };
    Some(
        json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":if args["action"]=="list"{"page-node-list"}else{"page-node-call"},
        "name":tool["name"],"arguments":args.to_string()}}),
    )
}

#[test]
fn official_personal_page_node_tool_is_scoped_confirmed_and_has_a_real_writer() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    f.validate_tools();
    let binding = configure_page_target(&f, true);
    let snapshot = f
        .server
        .request_result(
            "personalPage/binding",
            &serde_json::to_value(&binding).unwrap(),
        )
        .unwrap();
    let data = std::path::PathBuf::from(snapshot["runtimeBinding"]["dataRoot"].as_str().unwrap());
    let page = PersonalPageAiManager::new(f.manager.clone());
    page.register_document("model_document", binding).unwrap();
    let ui: UiActionExecutor = Arc::new(|_, _| panic!("node tool must not invoke a page action"));
    publish(&page, "model_document", &ui);
    let session = dispatch(
        &page,
        "model_document",
        "model.session.open",
        json!({}),
        &ui,
    )
    .unwrap();
    let run=dispatch(&page,"model_document","model.run.create",json!({"modelSessionId":session["modelSessionId"],"modelId":"isolated-official-fixture",
        "prompt":"PERSONAL_PAGE_NODE_FIXTURE_V1 调用本页明确登记的个人工具","clientMessageId":uuid::Uuid::new_v4().simple().to_string()}),&ui).unwrap();
    let id = run["runId"].as_str().unwrap();
    let approval = await_page_write(&page, id, &ui);
    assert!(!data.join("page-node-call.txt").exists());
    assert_eq!(approval["toolName"], "echo");
    page.resolve_write(
        "model_document",
        id,
        approval["confirmationId"].as_str().unwrap(),
        "approved",
        Some(&json!({"text":"page-node-human"})),
    )
    .unwrap();
    await_page_end(&page, id, &ui);
    assert_eq!(
        std::fs::read_to_string(data.join("page-node-call.txt")).unwrap(),
        "page-node-human"
    );
    let writers = f
        .server
        .request_result("profileWriter/list", &json!({}))
        .unwrap();
    assert_eq!(writers, json!([]));
    assert_eq!(
        page.close_document("model_document").unwrap()["processesExited"],
        true
    );
    f.assert_released();
    f.manager.shutdown_checked().unwrap();
}

#[test]
fn context_revocation_cancels_the_real_page_run_and_maintenance_close_is_explicitly_deferred() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    f.validate_tools();
    let binding = configure_page(&f);
    let page = PersonalPageAiManager::new(f.manager.clone());
    page.register_document("model_document", binding).unwrap();
    let ui: UiActionExecutor = Arc::new(|_, _| panic!("revoked proposal must never execute"));
    publish(&page, "model_document", &ui);
    dispatch(
        &page,
        "model_document",
        "uiAction.register",
        json!({"actionIds":["note.update"]}),
        &ui,
    )
    .unwrap();
    let session = dispatch(
        &page,
        "model_document",
        "model.session.open",
        json!({}),
        &ui,
    )
    .unwrap();
    let session = session["modelSessionId"].as_str().unwrap();
    let run = page_run(&page, session, &ui);
    let id = run["runId"].as_str().unwrap();
    await_page_write(&page, id, &ui);
    publish(&page, "model_document", &ui);
    let cancelled = dispatch(
        &page,
        "model_document",
        "model.run.get",
        json!({"runId":id}),
        &ui,
    )
    .unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(dispatch(&page,"model_document","model.run.create",json!({"modelSessionId":session,"modelId":"isolated-official-fixture","prompt":"不重发旧范围","clientMessageId":"new-page-message"}),&ui).unwrap_err(),"personal_page_model_session_stale");
    let operation=f.server.request_result("personalData/backupPrepare",&json!({"selection":{"includeWorkspaceFiles":[]},"clientOperationId":"page-close-maintenance"})).unwrap();
    let guard = f
        .manager
        .begin_profile_maintenance(
            &f.profile,
            operation["operationId"].as_str().unwrap(),
            operation["previewDigest"].as_str().unwrap(),
        )
        .unwrap();
    let retired = page.close_document("model_document").unwrap();
    assert_eq!(
        retired,
        json!({"retired":true,"processesExited":true,"cleanupPending":true,"auditPending":true})
    );
    assert_eq!(page.close_document("model_document").unwrap(), retired);
    assert_eq!(
        f.server
            .request_result("profileWriter/list", &json!({}))
            .unwrap(),
        json!([])
    );
    guard.finish().unwrap();
    f.manager.shutdown_checked().unwrap();
}

#[test]
fn official_personal_page_action_requires_trusted_confirmation_and_uses_the_edited_arguments() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    f.validate_tools();
    let binding = configure_page(&f);
    let page = PersonalPageAiManager::new(f.manager.clone());
    page.register_document("model_document", binding.clone())
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let actual = Arc::clone(&calls);
    let ui: UiActionExecutor = Arc::new(move |name, args| {
        assert_eq!(name, "note.update");
        assert_eq!(args, json!({"text":"human edited"}));
        actual.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"applied":true}))
    });
    publish(&page, "model_document", &ui);
    dispatch(
        &page,
        "model_document",
        "uiAction.register",
        json!({"actionIds":["note.update"]}),
        &ui,
    )
    .unwrap();
    let session = dispatch(
        &page,
        "model_document",
        "model.session.open",
        json!({}),
        &ui,
    )
    .unwrap();
    let session = session["modelSessionId"].as_str().unwrap();
    let run = page_run(&page, session, &ui);
    let run = run["runId"].as_str().unwrap();
    let approval = await_page_write(&page, run, &ui);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        dispatch(
            &page,
            "model_document",
            "model.write.resolve",
            json!({"runId":run,"confirmationId":approval["confirmationId"],"decision":"approved"}),
            &ui
        )
        .unwrap_err(),
        "personal_page_confirmation_host_required"
    );
    page.resolve_write(
        "model_document",
        run,
        approval["confirmationId"].as_str().unwrap(),
        "approved",
        Some(&json!({"text":"human edited"})),
    )
    .unwrap();
    assert_eq!(
        await_page_end(&page, run, &ui)["output"],
        "fixture completed"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let history = dispatch(
        &page,
        "model_document",
        "model.session.read",
        json!({"modelSessionId":session}),
        &ui,
    )
    .unwrap();
    assert!(
        history
            .to_string()
            .contains("PERSONAL_PAGE_ACTION_FIXTURE_V1")
    );
    let second = page_run(&page, session, &ui);
    let second = second["runId"].as_str().unwrap();
    let approval = await_page_write(&page, second, &ui);
    page.resolve_write(
        "model_document",
        second,
        approval["confirmationId"].as_str().unwrap(),
        "rejected",
        None,
    )
    .unwrap();
    await_page_end(&page, second, &ui);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    page.close_document("model_document").unwrap();
    f.assert_released();
    assert_eq!(
        f.server
            .request_result("profileWriter/list", &json!({}))
            .unwrap(),
        json!([])
    );
    f.manager.shutdown_checked().unwrap();
}

#[test]
fn disabled_page_close_confirms_process_exit_and_reports_the_missing_audit() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    let binding = install_page(&f);
    let page = PersonalPageAiManager::new(f.manager.clone());
    page.register_document("disabled_document", binding.clone())
        .unwrap();
    let status = f
        .server
        .request_result(
            "personalCapability/status",
            &json!({"creationProjectId":binding.creation_project_id}),
        )
        .unwrap();
    f.server.request_result("personalCapability/manage", &json!({"creationProjectId":binding.creation_project_id,"packageDigest":binding.expected_package_digest,"expectedRevision":status["project"]["stateRevision"],"action":"disable","clientOperationId":"disable-page-close"})).unwrap();
    let ui: UiActionExecutor = Arc::new(|_, _| panic!("disabled page must not execute"));
    assert!(dispatch(&page, "disabled_document", "session.read", json!({}), &ui).is_err());
    let closed = page.close_document("disabled_document").unwrap();
    assert_eq!(closed["retired"], true);
    assert_eq!(closed["processesExited"], true);
    assert_eq!(closed["auditPending"], true);
    assert_eq!(page.close_document("disabled_document").unwrap(), closed);
    assert_eq!(
        f.server
            .request_result("profileWriter/list", &json!({}))
            .unwrap(),
        json!([])
    );
    f.manager.shutdown_checked().unwrap();
}

#[test]
fn official_plain_page_can_chat_with_no_native_or_package_tools() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    f.validate_tools();
    let binding = configure_page(&f);
    let page = PersonalPageAiManager::new(f.manager.clone());
    page.register_document("model_document", binding).unwrap();
    let ui: UiActionExecutor = Arc::new(|_, _| panic!("no action registered"));
    publish(&page, "model_document", &ui);
    let session = dispatch(
        &page,
        "model_document",
        "model.session.open",
        json!({}),
        &ui,
    )
    .unwrap();
    let run=dispatch(&page,"model_document","model.run.create",json!({"modelSessionId":session["modelSessionId"],"modelId":"isolated-official-fixture","prompt":"PERSONAL_PAGE_EMPTY_FIXTURE_V1 仅根据本页上下文回答","clientMessageId":uuid::Uuid::new_v4().simple().to_string()}),&ui).unwrap();
    let completed = await_page_end(&page, run["runId"].as_str().unwrap(), &ui);
    assert_eq!(completed["output"], "本机仿真正式结果");
    assert_eq!(
        completed["modelExecution"]["connectionId"],
        f.connection.connection_id
    );
    assert_eq!(
        completed["replyUsage"],
        json!({"inputTokens":1,"cachedInputTokens":0,"outputTokens":1})
    );
    let requests = f.count.load(Ordering::Acquire);
    let usage = f.manager.usage_view().unwrap();
    let rows = usage["rows"].as_array().unwrap();
    let row = rows
        .iter()
        .find(|row| row["runId"] == run["runId"])
        .unwrap();
    assert_eq!(row["purpose"], "page_ai");
    assert_eq!(row["connectionId"], f.connection.connection_id);
    assert_eq!(row["tokens"]["inputTokens"], 1);
    assert!(row["cost"].is_null());
    assert!(!usage.to_string().contains("private-page-context-canary"));
    assert_eq!(f.count.load(Ordering::Acquire), requests);
    assert_eq!(
        page.close_document("model_document").unwrap()["processesExited"],
        true
    );
    f.assert_released();
    f.manager.shutdown_checked().unwrap();
    let reopened = crate::ai_runtime::AiRuntimeManager::new();
    reopened.initialize(&f.profile, &f.server).unwrap();
    let usage = reopened.usage_view().unwrap();
    let row = usage["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["runId"] == run["runId"])
        .unwrap();
    assert_eq!(row["recordSource"], "persisted_page_metadata");
    assert_eq!(row["connectionId"], f.connection.connection_id);
    assert_eq!(
        row["tokens"],
        json!({"inputTokens":1,"cachedInputTokens":0,"outputTokens":1,"source":"official_runtime_usage"})
    );
    assert!(row["cost"].is_null());
    assert_eq!(f.count.load(Ordering::Acquire), requests);
    reopened.shutdown_checked().unwrap();
}
