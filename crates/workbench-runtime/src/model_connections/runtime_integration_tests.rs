use super::*;
use crate::ai_runtime::model_test_fixture::{fixture_root, profile_data_root};
use std::fs;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
mod automation_overlap;
mod automations;
mod native_bindings;
mod network_scopes;
mod personal_pages;
mod request_shapes;
mod service_errors;
mod usage;

struct Fixture {
    owner: PathBuf,
    profile: PathBuf,
    manager: AiRuntimeManager,
    server: AppServerSupervisor,
    connection: ModelConnection,
    stop: Arc<AtomicBool>,
    count: Arc<AtomicUsize>,
    failure_status: Arc<AtomicU16>,
    hold: Arc<AtomicBool>,
    closed_requests: Arc<AtomicUsize>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    fn new() -> Self {
        let owner = fixture_root("official-responses");
        let profile = profile_data_root(&owner);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let count = Arc::new(AtomicUsize::new(0));
        let worker_stop = Arc::clone(&stop);
        let worker_count = Arc::clone(&count);
        let failure_status = Arc::new(AtomicU16::new(0));
        let worker_failure_status = Arc::clone(&failure_status);
        let hold = Arc::new(AtomicBool::new(false));
        let worker_hold = Arc::clone(&hold);
        let closed_requests = Arc::new(AtomicUsize::new(0));
        let worker_closed = Arc::clone(&closed_requests);
        let wire_root = owner.clone();
        let worker = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        let Some((body, subagent)) = request_body(&mut socket) else {
                            continue;
                        };
                        let index = worker_count.fetch_add(1, Ordering::AcqRel);
                        record_request(&wire_root, index, &body, subagent.as_deref());
                        assert!(matches!(
                            body["model"].as_str(),
                            Some("isolated-official-fixture" | "isolated-review-fixture")
                        ));
                        assert!(body.get("models").is_none());
                        assert!(body.get("route").is_none());
                        if peer_closed_while_held(&socket, &worker_hold, &worker_stop) {
                            worker_closed.fetch_add(1, Ordering::AcqRel);
                            continue;
                        }
                        if worker_stop.load(Ordering::Acquire) {
                            break;
                        }
                        if !service_errors::respond_failure(
                            &mut socket,
                            worker_failure_status.load(Ordering::Acquire),
                        ) {
                            respond(&mut socket, &body, subagent.as_deref());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("{error}"),
                }
            }
        });
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &profile,
                &profile.join("run"),
            )
            .unwrap();
        let prepared = server.request_result("modelConnection/savePrepare", &json!({
            "operationId":"official-fixture-connection","connectionId":null,"expectedConfigRevision":null,
            "config":{"name":"本机Responses仿真服务","kind":"local","baseUrl":format!("http://{address}/v1"),
                "localRuntime":"ollama","authMode":"none","enabled":true,"network":{"mode":"direct"}},
            "credentialAction":"clear"
        })).unwrap();
        let connection: ModelConnection = serde_json::from_value(
            server
                .request_result(
                    "modelConnection/saveCommit",
                    &json!({
                        "operationId":prepared["operationId"],"credentialRef":null
                    }),
                )
                .unwrap(),
        )
        .unwrap();
        server.request_result("modelDescriptor/save", &json!({
            "connectionId":connection.connection_id,"modelId":"isolated-official-fixture","displayName":"本机Responses仿真模型",
            "expectedRevision":null,"contextWindowTokens":null,"inputModalities":["text"],
            "defaultReasoningEffort":null,"supportedReasoningEfforts":[]
        })).unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&profile, &server).unwrap();
        Self {
            owner,
            profile,
            manager,
            server,
            connection,
            stop,
            count,
            failure_status,
            hold,
            closed_requests,
            worker: Some(worker),
        }
    }
}

impl Fixture {
    fn validate_tools(&self) {
        self.validate_tools_for(&self.connection);
    }
    fn validate_tools_for(&self, connection: &ModelConnection) {
        self.validate_model(connection, "isolated-official-fixture");
    }
    fn validate_model(&self, connection: &ModelConnection, model: &str) {
        let result = self
            .manager
            .personal_model_validate(contexthub_protocol::ModelValidationPrepareParams {
                runtime_kind: ModelRuntimeKind::Workbench,
                connection_id: connection.connection_id.clone(),
                model_id: Some(model.into()),
                layer: contexthub_protocol::ModelValidationLayer::Tools,
                expected_config_revision: connection.config_revision,
                expected_auth_generation: connection.auth_generation,
            })
            .unwrap();
        assert_eq!(
            result.outcome,
            Some(contexthub_protocol::ModelValidationOutcome::Passed),
            "{result:?}"
        );
    }
    fn start(&self) -> crate::ai_runtime::ConversationStartResult {
        self.start_with_permission("request")
    }
    fn start_with_permission(
        &self,
        permission: &str,
    ) -> crate::ai_runtime::ConversationStartResult {
        let snapshot = serde_json::to_value(self.manager.live_snapshot()).unwrap();
        self.manager.start_conversation_with_application_context(serde_json::from_value(json!({
            "title":"本机协议闭环","conversationType":"task","mode":"edit","agentId":snapshot["agents"][0]["agentId"],
            "context":{"workspaceRoot":""},"content":"仅验证本机协议","clientMessageId":"official-fixture-main",
            "modelSelection":{"connectionId":self.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},
            "modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":permission,"planning":false
        })).unwrap(), None, None, None).unwrap()
    }
    fn assert_released(&self) {
        self.assert_released_via(&self.server);
    }
    fn assert_released_via(&self, server: &AppServerSupervisor) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let preflight = server
                .request_result(
                    "modelConnection/preflight",
                    &json!({
                        "connectionId":self.connection.connection_id,"action":"disable"
                    }),
                )
                .unwrap();
            if preflight["activeLeaseIds"]
                .as_array()
                .is_some_and(Vec::is_empty)
            {
                break;
            }
            assert!(Instant::now() < deadline, "{preflight}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn completed_run(manager: &AiRuntimeManager, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = serde_json::to_value(manager.live_snapshot()).unwrap();
        let run = snapshot["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|run| run["runId"] == id)
            .unwrap();
        assert_ne!(run["status"], "failed", "{run}");
        if run["status"] == "completed" && run["threadWriterActive"] == false {
            return run.clone();
        }
        assert!(Instant::now() < deadline, "{run}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.manager.shutdown();
        let stopped = self.server.shutdown();
        if !std::thread::panicking() {
            stopped.unwrap();
        } else if stopped.is_err() {
            eprintln!("isolated_fixture_server_exit_unconfirmed");
        }
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let joined = worker.join();
            if !std::thread::panicking() {
                joined.unwrap();
            }
        }
        if std::thread::panicking() {
            eprintln!("isolated_fixture_failure_evidence={}", self.owner.display());
        } else {
            fs::remove_dir_all(&self.owner).unwrap();
        }
    }
}

fn record_request(root: &Path, index: usize, body: &Value, subagent: Option<&str>) {
    fs::write(
        root.join(format!("fixture-request-{index}.json")),
        serde_json::to_vec_pretty(body).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join(format!("fixture-metadata-{index}.json")),
        serde_json::to_vec(&json!({"subagent":subagent})).unwrap(),
    )
    .unwrap();
}

fn request_body(socket: &mut TcpStream) -> Option<(Value, Option<String>)> {
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let (header_end, length, subagent) = loop {
        let size = socket.read(&mut chunk).unwrap();
        if size == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..size]);
        if let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            assert!(headers.starts_with("POST /v1/responses HTTP/1.1\r\n"));
            assert!(!headers.to_ascii_lowercase().contains("authorization:"));
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            let subagent = headers.lines().find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("x-openai-subagent"))
                    .map(|(_, value)| value.trim().to_owned())
            });
            break (end + 4, length, subagent);
        }
    };
    while bytes.len() < header_end + length {
        let size = socket.read(&mut chunk).unwrap();
        if size == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..size]);
    }
    Some((
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap(),
        subagent,
    ))
}

fn peer_closed_while_held(socket: &TcpStream, hold: &AtomicBool, stop: &AtomicBool) -> bool {
    socket
        .set_read_timeout(Some(Duration::from_millis(10)))
        .unwrap();
    while hold.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
        match socket.peek(&mut [0u8; 1]) {
            Ok(0) => return true,
            Ok(_) => panic!("unexpected pipelined fixture request"),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("{error}"),
        }
    }
    socket
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    false
}

fn respond(socket: &mut TcpStream, body: &Value, subagent: Option<&str>) {
    let mut events = vec![json!({"type":"response.created","response":{"id":"fixture-response"}})];
    let inputs = body["input"].as_array().unwrap();
    let latest_user = inputs
        .iter()
        .rposition(|item| item["type"] == "message" && item["role"] == "user")
        .unwrap_or(0);
    let has_result = inputs[latest_user..]
        .iter()
        .any(|item| item["type"] == "function_call_output");
    let tool = find_tool(&body["tools"], crate::application_result::RESULT_TOOL);
    let workbench = find_tool(&body["tools"], crate::workbench_result::RESULT_TOOL);
    if body["model"] == "isolated-review-fixture" && subagent == Some("guardian") {
        push_text_events(&mut events,&json!({"risk_level":"high","user_authorization":"unknown","outcome":"deny","rationale":"isolated explicit reviewer denial"}).to_string());
    } else if let Some(item) = personal_pages::response_tool(body, has_result) {
        events.push(item);
    } else if body.to_string().contains("GUARDIAN_REVIEW_FIXTURE_V1") && !has_result {
        assert!(find_tool(&body["tools"], "exec_command").is_some());
        let args = json!({"cmd":"printf 'must-not-run' > \"$HOME/review-denied-canary\"","sandbox_permissions":"require_escalated","justification":"isolated explicit review probe","yield_time_ms":1000,"max_output_tokens":1024});
        events.push(json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"fixture-guardian-command","name":"exec_command","arguments":args.to_string()}}));
    } else if body.to_string().contains("USER_MCP_FIXTURE_V1") && !has_result {
        let text = body.to_string();
        let nonce = text
            .split("USER_MCP_FIXTURE_V1 ")
            .nth(1)
            .unwrap()
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect::<String>();
        assert_eq!(nonce.len(), 32);
        let tool = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| {
                tool["name"]
                    .as_str()
                    .is_some_and(|name| name.contains("personal_user"))
            })
            .expect("official runtime must publish only this turn's selected MCP namespace");
        events.push(json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"fixture-user-mcp-call","namespace":tool["name"],"name":tool["tools"][0]["name"],"arguments":json!({"message":nonce}).to_string()}}));
    } else if body.to_string().contains("TOOL_HOME_FIXTURE_V1") && !has_result {
        assert!(find_tool(&body["tools"], "exec_command").is_some());
        let args = json!({"cmd":r#"printf 'TOOL_HOME=%s\nTOOL_CODEX_HOME=%s\nTOOL_TMPDIR=%s\n' "$HOME" "$CODEX_HOME" "$TMPDIR""#, "yield_time_ms":1000, "max_output_tokens":1024});
        events.push(json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"fixture-env-call",
            "name":"exec_command","arguments":args.to_string()}}));
    } else if let Some(tool) = workbench.filter(|_| !has_result) {
        let args = json!({"schemaVersion":1,"nonce":tool["parameters"]["properties"]["nonce"]["enum"][0],
            "reply":"本机仿真正式结果","decision":"unclassified","taskId":"","taskTitle":"","objective":"",
            "expectedOutput":"","completionCriteria":"","conversationStatus":"","stage":"","outcomeTitle":"",
            "outcomeSummary":"","reason":"隔离协议验证","confidence":1.0});
        events.push(json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"fixture-main-call",
            "name":crate::workbench_result::RESULT_TOOL,"arguments":args.to_string()}}));
    } else if let Some(tool) = tool.filter(|_| !has_result) {
        let prompt = find_tool_prompt(&body["input"]).unwrap();
        let task = prompt
            .split("taskId=")
            .nth(1)
            .unwrap()
            .split('，')
            .next()
            .unwrap();
        let digest = prompt
            .split("inputDigest=")
            .nth(1)
            .unwrap()
            .split('，')
            .next()
            .unwrap();
        let nonce = prompt
            .split("\"nonce\":\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let args = json!({"taskId":task,"inputDigest":digest,"result":{"nonce":nonce}});
        assert_eq!(tool["name"], crate::application_result::RESULT_TOOL);
        events.push(json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"fixture-call",
            "name":crate::application_result::RESULT_TOOL,"arguments":args.to_string()}}));
    } else {
        let text = if has_result && body.to_string().contains("AUTOMATION_SLEEP_FIXTURE_V1") {
            "本机仿真正式结果".to_owned()
        } else if has_result {
            "fixture completed".to_owned()
        } else {
            find_nonce(body).unwrap_or_else(|| "本机仿真正式结果".into())
        };
        push_text_events(&mut events, &text);
    }
    events.push(json!({"type":"response.completed","response":{"id":"fixture-response","usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}));
    write_sse(socket, &events);
}

fn write_sse(socket: &mut TcpStream, events: &[Value]) {
    let mut sse = String::new();
    for event in events {
        use std::fmt::Write as _;
        write!(
            sse,
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap()
        )
        .unwrap();
    }
    let _ = write!(
        socket,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{sse}",
        sse.len()
    );
}

fn push_text_events(events: &mut Vec<Value>, text: &str) {
    events.push(json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"fixture-message","role":"assistant","content":[]}}));
    events.push(json!({"type":"response.content_part.added","output_index":0,"content_index":0,"item_id":"fixture-message","part":{"type":"output_text","text":""}}));
    events.push(json!({"type":"response.output_text.delta","delta":text,"output_index":0,"content_index":0,"item_id":"fixture-message"}));
    events.push(json!({"type":"response.output_item.done","item":{"type":"message","id":"fixture-message","role":"assistant","content":[{"type":"output_text","text":text}]}}));
}

fn find_tool<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Array(values) => values.iter().find_map(|value| find_tool(value, name)),
        Value::Object(values) if values.get("name").and_then(Value::as_str) == Some(name) => {
            Some(value)
        }
        Value::Object(values) => values.values().find_map(|value| find_tool(value, name)),
        _ => None,
    }
}

fn find_tool_prompt(value: &Value) -> Option<&str> {
    match value {
        Value::String(text)
            if text.contains("只调用 cilai_submit_application_result 一次。taskId=") =>
        {
            Some(text)
        }
        Value::Array(values) => values.iter().find_map(find_tool_prompt),
        Value::Object(values) => values.values().find_map(find_tool_prompt),
        _ => None,
    }
}

fn find_nonce(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if text.contains("只回复下面这串校验文本") => text
            .split(|character: char| !character.is_ascii_hexdigit())
            .rfind(|part| part.len() == 32)
            .map(str::to_owned),
        Value::Array(values) => values.iter().find_map(find_nonce),
        Value::Object(values) => values.values().find_map(find_nonce),
        _ => None,
    }
}

#[test]
fn official_gate_streaming_and_tool_probe_reaps_process_and_releases_lease() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    for layer in [
        contexthub_protocol::ModelValidationLayer::Streaming,
        contexthub_protocol::ModelValidationLayer::Tools,
    ] {
        let record = fixture
            .manager
            .personal_model_validate(contexthub_protocol::ModelValidationPrepareParams {
                runtime_kind: ModelRuntimeKind::Workbench,
                connection_id: fixture.connection.connection_id.clone(),
                model_id: Some("isolated-official-fixture".into()),
                layer,
                expected_config_revision: fixture.connection.config_revision,
                expected_auth_generation: fixture.connection.auth_generation,
            })
            .unwrap();
        assert_eq!(
            record.outcome,
            Some(contexthub_protocol::ModelValidationOutcome::Passed),
            "{record:?}"
        );
        assert_eq!(record.runtime_version.as_deref(), Some("0.163.0"));
    }
    assert_eq!(fixture.count.load(Ordering::Acquire), 3);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let preflight = fixture
            .server
            .request_result(
                "modelConnection/preflight",
                &json!({"connectionId":fixture.connection.connection_id,"action":"disable"}),
            )
            .unwrap();
        if preflight["activeLeaseIds"]
            .as_array()
            .is_some_and(Vec::is_empty)
        {
            break;
        }
        assert!(Instant::now() < deadline, "{preflight}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn official_main_first_send_restores_in_the_same_connection_and_releases_idle_process() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    let first_main_request = fixture.count.load(Ordering::Acquire);
    let started = fixture.start();
    let first = completed_run(&fixture.manager, &started.run.run_id);
    let wire = fs::read_to_string(
        fixture
            .owner
            .join(format!("fixture-request-{first_main_request}.json")),
    )
    .unwrap();
    assert!(wire.contains("仅使用本轮实际提供并明确授权的个人应用或 MCP 工具"));
    for removed in [
        "当前员工",
        "本轮员工",
        "已授权的企业应用工具",
        "需要企业能力时必须使用 tool_search",
    ] {
        assert!(!wire.contains(removed));
    }
    assert_eq!(first["output"], "本机仿真正式结果");
    assert_eq!(
        first["modelSelection"]["connectionId"],
        fixture.connection.connection_id
    );
    fixture.assert_released();
    fixture.manager.shutdown();
    let restored = AiRuntimeManager::new();
    restored
        .initialize(&fixture.profile, &fixture.server)
        .unwrap();
    let history = serde_json::to_value(restored.snapshot().unwrap()).unwrap();
    assert!(
        history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["content"] == "本机仿真正式结果")
    );
    let material = Path::new(&started.conversation.context.workspace_root).join("本地资料.md");
    fs::write(&material, "本地引用正文保持为数据").unwrap();
    let second_main_request = fixture.count.load(Ordering::Acquire);
    let run = restored.send_message_with_application_context(serde_json::from_value(json!({
        "conversationId":started.conversation.conversation_id,"content":"继续同一连接","clientMessageId":"official-fixture-second",
        "referencedMaterials":[{"localFileId":"local-material-wire","relativePath":"本地资料.md","fileRevision":1}],
        "modelSelection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},
        "modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":"request"
    })).unwrap(), None, None, None).unwrap();
    let second = completed_run(&restored, &run.run_id);
    let material_wire = fs::read_to_string(
        fixture
            .owner
            .join(format!("fixture-request-{second_main_request}.json")),
    )
    .unwrap();
    assert!(material_wire.contains("仅在本轮正式授权范围内从已校验的 resolvedPath 读取本地材料"));
    assert!(material_wire.contains(material.to_str().unwrap()));
    for removed in [
        "协作MCP",
        "云材料仅按",
        "修改云材料必须",
        "企业应用工具搜索",
    ] {
        assert!(!material_wire.contains(removed));
    }
    assert_eq!(
        fs::read_to_string(material).unwrap(),
        "本地引用正文保持为数据"
    );
    assert_eq!(first["sessionId"], second["sessionId"]);
    assert_eq!(second["output"], "本机仿真正式结果");
    fixture.assert_released();
    restored.shutdown();
    assert_eq!(fixture.count.load(Ordering::Acquire), 4);
}

fn wait_for_requests(fixture: &Fixture, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while fixture.count.load(Ordering::Acquire) < expected {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn terminal_run(manager: &AiRuntimeManager, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let snapshot = serde_json::to_value(manager.live_snapshot()).unwrap();
        let run = snapshot["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|run| run["runId"] == id)
            .unwrap();
        if matches!(
            run["status"].as_str(),
            Some("failed" | "terminated" | "interrupted")
        ) && run["threadWriterActive"] == false
        {
            return run.clone();
        }
        assert!(Instant::now() < deadline, "{run}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn official_main_cancel_shutdown_and_review_revocation_stop_before_lease_release() {
    let _guard = crate::supervisor::app_server_test_guard();
    for action in ["cancel", "revoke", "shutdown"] {
        let fixture = Fixture::new();
        fixture.validate_tools();
        let binding = json!({"bindingId":"fixture-review","purpose":"review","scopeId":null,"policy":"explicit",
            "selection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},
            "enabled":true,"revision":0,"consent":{"serviceName":"本机Responses仿真服务","historyScope":"仅隔离测试资料","mayIncurCost":false,"confirmedAt":1}});
        if action == "revoke" {
            fixture
                .server
                .request_result(
                    "modelPurpose/save",
                    &json!({"binding":binding,"expectedRevision":null}),
                )
                .unwrap();
        }
        fixture.hold.store(true, Ordering::Release);
        let started = fixture.start_with_permission(if action == "revoke" {
            "auto-review"
        } else {
            "request"
        });
        wait_for_requests(&fixture, 3);
        match action {
            "cancel" => {
                fixture
                    .manager
                    .control_run(&crate::ai_runtime::RunControlRequest {
                        run_id: started.run.run_id.clone(),
                        action: "terminate".into(),
                    })
                    .unwrap();
            }
            "revoke" => {
                let mut disabled = binding;
                disabled["enabled"] = json!(false);
                disabled["revision"] = json!(1);
                fixture
                    .server
                    .request_result(
                        "modelPurpose/save",
                        &json!({"binding":disabled,"expectedRevision":1}),
                    )
                    .unwrap();
            }
            "shutdown" => fixture.manager.shutdown(),
            _ => unreachable!(),
        }
        let run = terminal_run(&fixture.manager, &started.run.run_id);
        fixture.hold.store(false, Ordering::Release);
        fixture.assert_released();
        assert_eq!(run["output"], Value::Null);
        if action == "cancel" {
            assert_eq!(run["status"], "terminated");
        }
        if action == "revoke" {
            assert!(
                run["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|event| event["eventType"] == "model.execution_revoked")
            );
        }
        assert_eq!(fixture.count.load(Ordering::Acquire), 3);
    }
}

#[test]
fn another_host_cannot_signal_a_live_process_and_its_owner_can_pause_resume_and_cancel() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    fixture.hold.store(true, Ordering::Release);
    let started = fixture.start();
    wait_for_requests(&fixture, 3);
    let snapshot = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    let pid = u32::try_from(
        snapshot["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|run| run["runId"] == started.run.run_id)
            .unwrap()["pid"]
            .as_u64()
            .unwrap(),
    )
    .unwrap();
    let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
        .unwrap()
        .unwrap();
    let other = AiRuntimeManager::new();
    other.initialize(&fixture.profile, &fixture.server).unwrap();
    for action in ["pause", "resume", "terminate"] {
        assert_eq!(
            other
                .control_run(&crate::ai_runtime::RunControlRequest {
                    run_id: started.run.run_id.clone(),
                    action: action.into(),
                })
                .unwrap_err(),
            "agent_run_owner_required"
        );
        assert_eq!(
            contexthub_platform_adapter::instance::process_start_fingerprint(pid)
                .unwrap()
                .as_deref(),
            Some(fingerprint.as_str())
        );
    }
    for (action, expected) in [("pause", "paused"), ("resume", "running")] {
        assert_eq!(
            fixture
                .manager
                .control_run(&crate::ai_runtime::RunControlRequest {
                    run_id: started.run.run_id.clone(),
                    action: action.into(),
                })
                .unwrap()
                .status,
            expected
        );
        assert_eq!(
            contexthub_platform_adapter::instance::process_start_fingerprint(pid)
                .unwrap()
                .as_deref(),
            Some(fingerprint.as_str())
        );
    }
    other.shutdown_checked().unwrap();
    fixture
        .manager
        .control_run(&crate::ai_runtime::RunControlRequest {
            run_id: started.run.run_id.clone(),
            action: "terminate".into(),
        })
        .unwrap();
    let terminal = terminal_run(&fixture.manager, &started.run.run_id);
    assert_eq!(terminal["status"], "terminated");
    fixture.hold.store(false, Ordering::Release);
    fixture.assert_released();
    assert_eq!(fixture.count.load(Ordering::Acquire), 3);
    fixture.manager.shutdown_checked().unwrap();
}

#[test]
fn proxy_connect_authentication_challenge_has_a_fixed_safe_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        use std::io::BufRead as _;
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
        let mut start = String::new();
        reader.read_line(&mut start).unwrap();
        assert!(start.starts_with("CONNECT fixture.invalid:443 HTTP/1.1"));
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
        }
        let body = "Authorization: echoed-fixture-secret";
        write!(socket, "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let network = contexthub_protocol::ModelNetworkConfig {
        mode: contexthub_protocol::ModelNetworkMode::CustomProxy,
        proxy_url: Some(format!("http://{address}")),
        proxy_secret_ref: None,
        ca_certificate_pem: None,
        ..Default::default()
    };
    let client = connection_client(&network, &std::env::temp_dir().join("credentials")).unwrap();
    let error = crate::async_support::block_on(async {
        client
            .get("https://fixture.invalid/v1/models")
            .send()
            .await
            .unwrap_err()
    });
    assert_eq!(
        network_failure_reason(&error, "unmapped"),
        "model_proxy_authentication_rejected"
    );
    worker.join().unwrap();
}

fn second_connection(fixture: &Fixture) -> ModelConnection {
    let operation = uuid::Uuid::new_v4().simple().to_string();
    let prepared = fixture.server.request_result("modelConnection/savePrepare", &json!({
        "operationId":operation,"connectionId":null,"expectedConfigRevision":null,
        "config":{"name":"另一显式连接","kind":"local","baseUrl":fixture.connection.config.base_url,
            "localRuntime":"ollama","authMode":"none","enabled":true,"network":{"mode":"direct"}},
        "credentialAction":"clear"
    })).unwrap();
    let connection: ModelConnection = serde_json::from_value(
        fixture
            .server
            .request_result(
                "modelConnection/saveCommit",
                &json!({
                    "operationId":prepared["operationId"],"credentialRef":null
                }),
            )
            .unwrap(),
    )
    .unwrap();
    fixture.server.request_result("modelDescriptor/save", &json!({
        "connectionId":connection.connection_id,"modelId":"isolated-official-fixture","displayName":"同名模型",
        "expectedRevision":null,"contextWindowTokens":null,"inputModalities":["text"],
        "defaultReasoningEffort":null,"supportedReasoningEfforts":[]
    })).unwrap();
    fixture.validate_tools_for(&connection);
    connection
}

#[test]
fn official_connection_switch_requires_exact_history_consent_and_starts_a_new_native_thread() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    let started = fixture.start();
    let first = completed_run(&fixture.manager, &started.run.run_id);
    fixture.assert_released();
    let other = second_connection(&fixture);
    let selection = contexthub_protocol::ModelSelection {
        connection_id: other.connection_id.clone(),
        model_id: "isolated-official-fixture".into(),
        reasoning_effort: None,
    };
    let mut message: crate::ai_runtime::MessageSendRequest = serde_json::from_value(json!({
        "conversationId":started.conversation.conversation_id,"content":"明确继续到另一连接","clientMessageId":"official-fixture-switch",
        "modelSelection":selection,"modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":"request"
    })).unwrap();
    let before = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    assert_eq!(
        fixture
            .manager
            .send_message_with_application_context(message.clone(), None, None, None)
            .unwrap_err(),
        "model_thread_history_consent_required"
    );
    assert_eq!(fixture.count.load(Ordering::Acquire), 5);
    let after = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    assert_eq!(before["messages"], after["messages"]);
    let preflight = fixture
        .manager
        .personal_model_execution_preflight(
            contexthub_protocol::ModelExecutionPrepareParams {
                owner: None,
                launch_protocol: 1,
                purpose: contexthub_protocol::ModelPurpose::Foreground,
                binding_id: None,
                selection: Some(selection),
                conversation_id: Some(started.conversation.conversation_id),
            },
            ModelRuntimeKind::Workbench,
        )
        .unwrap();
    let transfer = preflight.history_transfer.unwrap();
    assert_eq!(transfer.message_count, 2);
    message.history_consent = Some(ModelHistoryTransferConsent {
        connection_id: transfer.connection_id,
        config_revision: transfer.config_revision,
        auth_generation: transfer.auth_generation,
        model_id: transfer.model_id,
        runtime_kind: transfer.runtime_kind,
        snapshot_digest: transfer.snapshot_digest,
        history_digest: transfer.history_digest,
        confirmed_at: 1,
    });
    let run = fixture
        .manager
        .send_message_with_application_context(message, None, None, None)
        .unwrap();
    let second = completed_run(&fixture.manager, &run.run_id);
    assert_ne!(first["sessionId"], second["sessionId"]);
    assert_eq!(
        second["modelSelection"]["connectionId"],
        other.connection_id
    );
    assert_eq!(fixture.count.load(Ordering::Acquire), 6);
    fixture.manager.shutdown();
    let restored = AiRuntimeManager::new();
    restored
        .initialize(&fixture.profile, &fixture.server)
        .unwrap();
    let snapshot = serde_json::to_value(restored.snapshot().unwrap()).unwrap();
    assert_eq!(snapshot["messages"].as_array().unwrap().len(), 4);
    assert_eq!(snapshot["runs"].as_array().unwrap().len(), 2);
    restored.shutdown();
}

#[test]
fn official_native_shell_uses_only_the_private_home_and_tmp_environment() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    let snapshot = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    let started = fixture.manager.start_conversation_with_application_context(serde_json::from_value(json!({
        "title":"工具环境核验","conversationType":"task","mode":"edit","agentId":snapshot["agents"][0]["agentId"],
        "context":{"workspaceRoot":""},"content":"TOOL_HOME_FIXTURE_V1","clientMessageId":"official-fixture-env",
        "modelSelection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},
        "modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":"request","planning":false
    })).unwrap(), None, None, None).unwrap();
    let run = completed_run(&fixture.manager, &started.run.run_id);
    assert_eq!(run["output"], "fixture completed");
    let execution = started.run.model_execution.as_ref().unwrap();
    let home = scoped_home(
        &fixture.manager.gateway_data_dir().unwrap(),
        &execution.runtime_home_scope,
        ModelRuntimeKind::Workbench,
    );
    let output = (0..fixture.count.load(Ordering::Acquire))
        .flat_map(|index| {
            let request: Value = serde_json::from_slice(
                &fs::read(fixture.owner.join(format!("fixture-request-{index}.json"))).unwrap(),
            )
            .unwrap();
            request["input"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| {
                    item["type"] == "function_call_output" && item["call_id"] == "fixture-env-call"
                })
                .map(|item| item["output"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        output.contains(&format!(
            "TOOL_HOME={}\n",
            home.join("process-home").display()
        )),
        "{output}"
    );
    assert!(
        output.contains(&format!("TOOL_CODEX_HOME={}\n", home.display())),
        "{output}"
    );
    assert!(
        output.contains(&format!(
            "TOOL_TMPDIR={}\n",
            home.join("process-tmp").display()
        )),
        "{output}"
    );
    fixture.assert_released();
    assert_eq!(fixture.count.load(Ordering::Acquire), 4);
}

#[test]
fn official_user_mcp_requires_real_confirmation_and_never_calls_a_rejected_tool() {
    let _guard = crate::supervisor::app_server_test_guard();
    for decision in ["approved", "rejected"] {
        let service = crate::mcp_connections::tests::Server::new(true);
        let fixture = Fixture::new();
        fixture.validate_tools();
        let granted = grant_user_mcp(&fixture, &service.url);
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let snapshot = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
        let started=fixture.manager.start_conversation_with_application_context(serde_json::from_value(json!({"title":"用户MCP正式执行","conversationType":"task","mode":"edit","agentId":snapshot["agents"][0]["agentId"],"context":{"workspaceRoot":""},"content":format!("USER_MCP_FIXTURE_V1 {nonce}"),"clientMessageId":"official-user-mcp-fixture","modelSelection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},"modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":"request","planning":false})).unwrap(),None,None,None).unwrap();
        let permission = await_user_mcp_permission(&fixture, &started.run.run_id);
        assert_eq!(service.calls.load(Ordering::SeqCst), 0);
        assert_eq!(permission["networkAccess"], true);
        let revised = format!("{nonce}-confirmed");
        fixture.manager.resolve_permission_request(serde_json::from_value(json!({"permissionRequestId":permission["permissionRequestId"],"decision":decision,"decidedBy":"isolated-fixture-human","applicationWriteArguments":if decision=="approved" {json!({"message":revised})} else {Value::Null}})).unwrap()).unwrap();
        let run = completed_run(&fixture.manager, &started.run.run_id);
        assert_eq!(run["output"], "fixture completed");
        assert_personal_confirmation_summary(&run, decision);
        fixture.assert_released();
        assert_eq!(
            service.calls.load(Ordering::SeqCst),
            usize::from(decision == "approved")
        );
        if decision == "approved" {
            let calls = service
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m["method"] == "tools/call")
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0]["params"]["arguments"]["message"], revised);
        }
        let history = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
        let recorded = history["permissionRequests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["permissionRequestId"] == permission["permissionRequestId"])
            .unwrap();
        assert_eq!(
            recorded["applicationWrite"]["executionStatus"],
            if decision == "approved" {
                "succeeded"
            } else {
                "cancelled"
            }
        );
        let leases: Vec<contexthub_protocol::McpExecutionLease> = serde_json::from_value(
            fixture
                .server
                .request_result("mcpExecution/list", &json!({}))
                .unwrap(),
        )
        .unwrap();
        assert!(
            leases
                .iter()
                .all(|lease| lease.state == contexthub_protocol::McpExecutionState::Finished),
            "{leases:?}"
        );
        fixture.server.request_result("mcpConnection/disable",&json!({"connectionId":granted["connectionId"],"expectedRevision":granted["revision"]})).unwrap();
        assert!(
            fixture
                .manager
                .personal_mcp_selected_snapshot(contexthub_protocol::McpPurpose::Foreground)
                .unwrap()
                .connections
                .is_empty()
        );
    }
}

fn assert_personal_confirmation_summary(run: &Value, decision: &str) {
    let (event, summary) = if decision == "approved" {
        ("application.write.confirmed", "已允许本次应用写入")
    } else {
        ("application.write.rejected", "已拒绝本次应用写入")
    };
    assert!(
        run["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| { item["eventType"] == event && item["summary"] == summary })
    );
}

fn grant_user_mcp(fixture: &Fixture, url: &str) -> Value {
    let operation = uuid::Uuid::new_v4().simple().to_string();
    let prepared=fixture.server.request_result("mcpConnection/savePrepare",&json!({"operationId":operation,"connectionId":null,"expectedRevision":null,"config":{"name":"本机MCP真实确认","source":{"kind":"remote_http","url":url},"authMode":"none","environmentVariables":[],"network":{"mode":"direct","proxyUrl":null}},"credentialAction":"clear"})).unwrap();
    let connection: contexthub_protocol::McpConnection = serde_json::from_value(
        fixture
            .server
            .request_result(
                "mcpConnection/saveCommit",
                &json!({"operationId":prepared["operationId"],"credentialRef":null}),
            )
            .unwrap(),
    )
    .unwrap();
    let validation = fixture
        .manager
        .personal_mcp_validate(contexthub_protocol::McpValidateParams {
            connection_id: connection.connection_id.clone(),
            expected_revision: connection.revision,
            source_consent: true,
        })
        .unwrap();
    assert_eq!(
        validation.state,
        contexthub_protocol::McpValidationState::Passed,
        "{:?}",
        validation.reason_code
    );
    fixture.server.request_result("mcpConnection/grant",&json!({"connectionId":connection.connection_id,"expectedRevision":connection.revision,"validationId":validation.validation_id,"approvedToolNames":["echo"],"purposes":["foreground"],"sourceConsent":true})).unwrap()
}

#[test]
fn personal_memory_confirmation_uses_the_bound_service_and_is_invalidated_by_revision() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    let saved = bind_fixture_memory(&fixture);
    let settings = fixture.manager.memory_settings().unwrap();
    let service = settings.service.unwrap();
    assert_eq!(service.connection_id, fixture.connection.connection_id);
    assert_eq!(
        settings.model_label.as_deref(),
        Some("isolated-official-fixture")
    );
    assert!(!settings.enabled);
    let request = |confirmation| crate::memory::MemorySettingsSaveRequest {
        enabled: true,
        from_tool_chats: false,
        confirmation,
    };
    assert_eq!(
        fixture
            .manager
            .save_personal_memory_settings(request(None))
            .unwrap_err(),
        "ai_memory_enable_consent_required"
    );
    let confirmation = crate::memory::MemoryEnableConfirmation {
        consent_digest: service.consent_digest,
        from_tool_chats: false,
        confirmed_at: 1,
    };
    assert!(
        fixture
            .manager
            .save_personal_memory_settings(request(Some(confirmation)))
            .unwrap()
            .enabled
    );
    fixture
        .manager
        .validate_personal_memory_scope(crate::memory::MemoryPreferences {
            enabled: true,
            from_tool_chats: false,
        })
        .unwrap();
    let mut changed = saved;
    changed["consent"]["historyScope"] = json!("explicitly_reconfirmed_scope");
    fixture
        .server
        .request_result(
            "modelPurpose/save",
            &json!({"binding":changed,"expectedRevision":changed["revision"]}),
        )
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .validate_personal_memory_scope(crate::memory::MemoryPreferences {
                enabled: true,
                from_tool_chats: false
            })
            .unwrap_err(),
        "ai_memory_enable_consent_stale"
    );
    let changed = fixture.manager.memory_settings().unwrap();
    assert_eq!(changed.phase, "blocked");
    assert_eq!(
        changed.service_error.as_deref(),
        Some("ai_memory_enable_consent_stale")
    );
    let disabled = fixture
        .manager
        .save_personal_memory_settings(crate::memory::MemorySettingsSaveRequest {
            enabled: false,
            from_tool_chats: false,
            confirmation: None,
        })
        .unwrap();
    assert!(!disabled.enabled);
    assert_eq!(
        fixture
            .manager
            .validate_personal_memory_scope(crate::memory::MemoryPreferences {
                enabled: true,
                from_tool_chats: false
            })
            .unwrap_err(),
        "ai_memory_preferences_changed"
    );
    assert_eq!(fixture.count.load(Ordering::Acquire), 2);
}

fn bind_fixture_memory(fixture: &Fixture) -> Value {
    fixture.server.request_result("modelPurpose/save", &json!({"binding":{
        "bindingId":"profile:memory","purpose":"memory","scopeId":null,"policy":"explicit",
        "selection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},
        "enabled":true,"consent":{"serviceName":"本机Responses仿真服务","historyScope":"native_scope_history","mayIncurCost":false,"confirmedAt":1},"revision":0
    },"expectedRevision":null})).unwrap()
}

fn await_user_mcp_permission(fixture: &Fixture, run_id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
        let run = state["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["runId"] == run_id)
            .unwrap();
        assert_ne!(run["status"], "failed", "{run}");
        if let Some(permission) = state["permissionRequests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| {
                p["runId"] == run_id
                    && p["status"] == "pending"
                    && p["protocolMethod"] == "personal/mcp/tool"
            })
        {
            return permission.clone();
        }
        assert!(Instant::now() < deadline, "{run}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn shutdown_releases_a_real_pending_user_mcp_confirmation_without_executing_the_tool() {
    let _guard = crate::supervisor::app_server_test_guard();
    let service = crate::mcp_connections::tests::Server::new(false);
    let fixture = Fixture::new();
    fixture.validate_tools();
    grant_user_mcp(&fixture, &service.url);
    let snapshot = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    let started=fixture.manager.start_conversation_with_application_context(serde_json::from_value(json!({"title":"退出人工确认","conversationType":"task","mode":"edit","agentId":snapshot["agents"][0]["agentId"],"context":{"workspaceRoot":""},"content":format!("USER_MCP_FIXTURE_V1 {}",uuid::Uuid::new_v4().simple()),"clientMessageId":"official-exit-mcp","modelSelection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},"modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":"request","planning":false})).unwrap(),None,None,None).unwrap();
    let permission = await_user_mcp_permission(&fixture, &started.run.run_id);
    assert_eq!(service.calls.load(Ordering::SeqCst), 0);
    let started_exit = Instant::now();
    fixture.manager.shutdown();
    assert!(started_exit.elapsed() < Duration::from_secs(25));
    fixture.assert_released();
    assert_eq!(service.calls.load(Ordering::SeqCst), 0);
    let history = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    let request = history["permissionRequests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["permissionRequestId"] == permission["permissionRequestId"])
        .unwrap();
    assert_eq!(request["status"], "cancelled");
    assert_eq!(request["applicationWrite"]["executionStatus"], "cancelled");
}

#[test]
fn another_host_disabling_memory_stops_the_real_owned_native_process_and_keeps_the_input() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    bind_fixture_memory(&fixture);
    let service = fixture.manager.memory_settings().unwrap().service.unwrap();
    fixture
        .manager
        .save_personal_memory_settings(crate::memory::MemorySettingsSaveRequest {
            enabled: true,
            from_tool_chats: false,
            confirmation: Some(crate::memory::MemoryEnableConfirmation {
                consent_digest: service.consent_digest,
                from_tool_chats: false,
                confirmed_at: 1,
            }),
        })
        .unwrap();
    fixture.hold.store(true, Ordering::Release);
    let started = fixture.start();
    let deadline = Instant::now() + Duration::from_secs(30);
    while fixture.count.load(Ordering::Acquire) < 3 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let other = AiRuntimeManager::new();
    other.initialize(&fixture.profile, &fixture.server).unwrap();
    other
        .save_personal_memory_settings(crate::memory::MemorySettingsSaveRequest {
            enabled: false,
            from_tool_chats: false,
            confirmation: None,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let history = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
        let run = history["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|run| run["runId"] == started.run.run_id)
            .unwrap();
        if run["threadWriterActive"] == false && run["status"] == "failed" {
            break;
        }
        assert!(Instant::now() < deadline, "{run}");
        std::thread::sleep(Duration::from_millis(20));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while fixture.closed_requests.load(Ordering::Acquire) == 0 {
        assert!(
            Instant::now() < deadline,
            "upstream connection was not cancelled"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    fixture.assert_released();
    fixture.hold.store(false, Ordering::Release);
    fixture.assert_released();
    let history = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    assert!(
        history["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["runId"] == started.run.run_id && message["role"] == "user")
    );
    assert!(!fixture.manager.memory_settings().unwrap().enabled);
    other.shutdown_checked().unwrap();
    fixture.manager.shutdown_checked().unwrap();
    let ledger = fixture
        .server
        .request_result("aiLedger/get", &json!({}))
        .unwrap();
    let persisted = ledger["ledger"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == started.run.run_id)
        .unwrap();
    assert_eq!(persisted["value"]["status"], "failed");
    assert_eq!(persisted["value"]["threadWriterActive"], false);
}

#[test]
fn opening_another_host_preserves_the_live_run_and_its_pending_tool_confirmation() {
    let _guard = crate::supervisor::app_server_test_guard();
    let service = crate::mcp_connections::tests::Server::new(false);
    let fixture = Fixture::new();
    fixture.validate_tools();
    grant_user_mcp(&fixture, &service.url);
    let snapshot = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    let started=fixture.manager.start_conversation_with_application_context(serde_json::from_value(json!({"title":"多宿主待确认","conversationType":"task","mode":"edit","agentId":snapshot["agents"][0]["agentId"],"context":{"workspaceRoot":""},"content":format!("USER_MCP_FIXTURE_V1 {}",uuid::Uuid::new_v4().simple()),"clientMessageId":"official-other-host-mcp","modelSelection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},"modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":"request","planning":false})).unwrap(),None,None,None).unwrap();
    let permission = await_user_mcp_permission(&fixture, &started.run.run_id);
    let other = AiRuntimeManager::new();
    other.initialize(&fixture.profile, &fixture.server).unwrap();
    let opened = serde_json::to_value(other.live_snapshot()).unwrap();
    let retained = opened["permissionRequests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["permissionRequestId"] == permission["permissionRequestId"])
        .unwrap();
    assert_eq!(retained["status"], "pending");
    let run = opened["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["runId"] == started.run.run_id)
        .unwrap();
    assert_ne!(run["status"], "interrupted");
    assert_eq!(run["threadWriterActive"], true);
    other.shutdown_checked().unwrap();
    fixture.manager.resolve_permission_request(serde_json::from_value(json!({"permissionRequestId":permission["permissionRequestId"],"decision":"rejected","decidedBy":"isolated-fixture-human","applicationWriteArguments":null})).unwrap()).unwrap();
    assert_eq!(
        completed_run(&fixture.manager, &started.run.run_id)["status"],
        "completed"
    );
    assert_eq!(service.calls.load(Ordering::SeqCst), 0);
    fixture.assert_released();
    fixture.manager.shutdown_checked().unwrap();
}

#[test]
fn the_first_host_exits_while_the_second_host_finishes_its_real_official_turn() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    let peer = AppServerSupervisor::new();
    peer.start(
        &crate::supervisor::app_server_test_binary(),
        "test",
        &fixture.profile,
        &fixture.profile.join("run"),
    )
    .unwrap();
    assert_eq!(
        peer.status().connection_mode,
        Some(crate::supervisor::AppServerConnectionMode::Attached)
    );
    let process = fixture.server.status().pid.unwrap();
    let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(process)
        .unwrap()
        .unwrap();
    let manager = AiRuntimeManager::new();
    manager.initialize(&fixture.profile, &peer).unwrap();
    let snapshot = serde_json::to_value(manager.live_snapshot()).unwrap();
    fixture.hold.store(true, Ordering::Release);
    let started=manager.start_conversation_with_application_context(serde_json::from_value(json!({"title":"其他宿主继续执行","conversationType":"task","mode":"edit","agentId":snapshot["agents"][0]["agentId"],"context":{"workspaceRoot":""},"content":"首宿主退出后继续当前本机验证","clientMessageId":"official-shared-service-peer","modelSelection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},"modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":"request","planning":false})).unwrap(),None,None,None).unwrap();
    wait_for_requests(&fixture, 3);
    fixture.manager.shutdown_checked().unwrap();
    fixture.server.shutdown().unwrap();
    assert_eq!(
        fixture.server.status().server_lifecycle_mode.as_deref(),
        Some("shared_continued")
    );
    assert!(peer.request_result("ping", &json!({})).is_ok());
    fixture.hold.store(false, Ordering::Release);
    assert_eq!(
        completed_run(&manager, &started.run.run_id)["output"],
        "本机仿真正式结果"
    );
    fixture.assert_released_via(&peer);
    manager.shutdown_checked().unwrap();
    let ledger = peer.request_result("aiLedger/get", &json!({})).unwrap();
    let recorded = ledger["ledger"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["id"] == started.run.run_id)
        .unwrap();
    assert_eq!(recorded["value"]["status"], "completed");
    assert_eq!(recorded["value"]["threadWriterActive"], false);
    peer.shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while contexthub_platform_adapter::instance::process_start_fingerprint(process)
        .unwrap()
        .as_deref()
        == Some(fingerprint.as_str())
    {
        assert!(
            Instant::now() < deadline,
            "last client left but server is still running"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn official_guardian_uses_the_explicit_review_model_and_a_denied_command_never_runs() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    fixture.server.request_result("modelDescriptor/save", &json!({"connectionId":fixture.connection.connection_id,"modelId":"isolated-review-fixture","displayName":"本机独立审核模型","expectedRevision":null,"contextWindowTokens":null,"inputModalities":["text"],"defaultReasoningEffort":null,"supportedReasoningEfforts":[]})).unwrap();
    fixture.validate_model(&fixture.connection, "isolated-review-fixture");
    fixture.server.request_result("modelPurpose/save", &json!({"binding":{"bindingId":"fixture-real-review","purpose":"review","scopeId":null,"policy":"explicit","selection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-review-fixture","reasoningEffort":null},"enabled":true,"revision":0,"consent":{"serviceName":"本机独立审核服务","historyScope":"本轮审批历史","mayIncurCost":false,"confirmedAt":1}},"expectedRevision":null})).unwrap();
    let snapshot = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    let started=fixture.manager.start_conversation_with_application_context(serde_json::from_value(json!({"title":"官方独立审核拒绝","conversationType":"task","mode":"edit","agentId":snapshot["agents"][0]["agentId"],"context":{"workspaceRoot":""},"content":"GUARDIAN_REVIEW_FIXTURE_V1","clientMessageId":"official-guardian-review","modelSelection":{"connectionId":fixture.connection.connection_id,"modelId":"isolated-official-fixture","reasoningEffort":null},"modelOverride":"isolated-official-fixture","executionTarget":"model","permissionMode":"auto-review","planning":false})).unwrap(),None,None,None).unwrap();
    let run = completed_run(&fixture.manager, &started.run.run_id);
    assert_eq!(run["output"], "fixture completed");
    fixture.assert_released();
    let requests = (0..fixture.count.load(Ordering::Acquire))
        .map(|index| {
            serde_json::from_slice::<Value>(
                &fs::read(fixture.owner.join(format!("fixture-request-{index}.json"))).unwrap(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let reviews = requests
        .iter()
        .enumerate()
        .filter(|(index, request)| {
            let metadata: Value = serde_json::from_slice(
                &fs::read(fixture.owner.join(format!("fixture-metadata-{index}.json"))).unwrap(),
            )
            .unwrap();
            request["model"] == "isolated-review-fixture" && metadata["subagent"] == "guardian"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reviews.len(),
        1,
        "no actual independent guardian request: {requests:?}"
    );
    let home = scoped_home(
        &fixture.manager.gateway_data_dir().unwrap(),
        run["modelExecution"]["runtimeHomeScope"].as_str().unwrap(),
        ModelRuntimeKind::Workbench,
    );
    assert!(!home.join("process-home/review-denied-canary").exists());
    fixture.manager.shutdown_checked().unwrap();
}

#[test]
fn locked_official_v1_memory_reads_current_root_and_does_not_inject_retained_legacy_roots() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    bind_fixture_memory(&fixture);
    let service = fixture.manager.memory_settings().unwrap().service.unwrap();
    fixture
        .manager
        .save_personal_memory_settings(crate::memory::MemorySettingsSaveRequest {
            enabled: true,
            from_tool_chats: false,
            confirmation: Some(crate::memory::MemoryEnableConfirmation {
                consent_digest: service.consent_digest,
                from_tool_chats: false,
                confirmed_at: 1,
            }),
        })
        .unwrap();
    let snapshot = fixture
        .manager
        .personal_model_resolve(contexthub_protocol::ModelExecutionPrepareParams {
            launch_protocol: 1,
            owner: None,
            purpose: contexthub_protocol::ModelPurpose::Memory,
            binding_id: None,
            selection: None,
            conversation_id: None,
        })
        .unwrap();
    let home = scoped_home(
        &fixture.manager.gateway_data_dir().unwrap(),
        &snapshot.runtime_home_scope,
        ModelRuntimeKind::Workbench,
    );
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let current = format!("CURRENT_MEMORY_ROOT_{nonce}");
    let old = format!("RETAINED_LEGACY_ROOT_{nonce}");
    seed_memory_read_marker(&home.join("memories"), &current);
    for root in ["memory", "memory-publication", "memory-publication/current"] {
        seed_memory_read_marker(&home.join(root), &old);
    }
    let started = fixture.start();
    let run = completed_run(&fixture.manager, &started.run.run_id);
    assert_eq!(run["output"], "本机仿真正式结果");
    fixture
        .manager
        .save_personal_memory_settings(crate::memory::MemorySettingsSaveRequest {
            enabled: false,
            from_tool_chats: false,
            confirmation: None,
        })
        .unwrap();
    fixture.assert_released();
    let requests = (0..fixture.count.load(Ordering::Acquire))
        .map(|index| {
            fs::read_to_string(fixture.owner.join(format!("fixture-request-{index}.json"))).unwrap()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        requests.contains(&current),
        "current v1 memory root was not observed in the real request"
    );
    assert!(
        !requests.contains(&old),
        "retained legacy memory was injected"
    );
    assert!(home.join("memory-publication/memory_summary.md").is_file());
}

fn seed_memory_read_marker(root: &Path, marker: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("MEMORY.md"),
        format!("# 本机测试约定\n{marker}\n"),
    )
    .unwrap();
    fs::write(
        root.join("memory_summary.md"),
        format!("# 本机测试摘要\n{marker}\n"),
    )
    .unwrap();
}
