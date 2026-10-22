use super::remote::parse_response;
use serde_json::json;
#[test]
fn json_and_sse_match_exact_request_without_replaying_server_requests() {
    let result = json!({"jsonrpc":"2.0","id":3,"result":{"tools":[]}});
    assert_eq!(
        parse_response("application/json", result.to_string().as_bytes(), 3).unwrap(),
        json!({"tools":[]})
    );
    let sse = format!("event: message\r\ndata: {result}\r\n\r\n");
    assert_eq!(
        parse_response("text/event-stream", sse.as_bytes(), 3).unwrap(),
        json!({"tools":[]})
    );
    assert!(parse_response("application/json", result.to_string().as_bytes(), 4).is_err());
    assert!(
        parse_response(
            "text/event-stream",
            format!("data: {result}\n\ndata: {result}\n\n").as_bytes(),
            3
        )
        .is_err()
    );
    assert_eq!(
        parse_response(
            "application/json",
            br#"{"jsonrpc":"2.0","id":7,"method":"sampling/createMessage"}"#,
            3
        )
        .unwrap_err(),
        "mcp_server_requests_not_supported"
    );
    assert_eq!(
        parse_response(
            "text/event-stream",
            b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
            3
        )
        .unwrap_err(),
        "mcp_catalog_changed"
    );
}

use super::*;
use contexthub_protocol::{McpConnectionChange, McpConnectionParams, McpGrantParams, McpPurpose};
use std::io::{BufRead as _, Read as _, Write as _};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Fixture {
    root: std::path::PathBuf,
    server: AppServerSupervisor,
    manager: AiRuntimeManager,
}
impl Fixture {
    fn new() -> Self {
        let root = crate::ai_runtime::model_test_fixture::fixture_root("user-mcp");
        let data = crate::ai_runtime::model_test_fixture::profile_data_root(&root);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&data, &server).unwrap();
        Self {
            root,
            server,
            manager,
        }
    }
    fn connection(&self, url: &str) -> McpConnection {
        let config = json!({"name":"真实本机协议夹具","source":{"kind":"remote_http","url":url},"authMode":"none","environmentVariables":[],"network":{"mode":"direct","proxyUrl":null}});
        let prepared: McpConnectionChange=rpc(&self.server,"mcpConnection/savePrepare",&json!({"operationId":uuid::Uuid::new_v4().simple().to_string(),"connectionId":null,"expectedRevision":null,"config":config,"credentialAction":"clear"})).unwrap();
        rpc(
            &self.server,
            "mcpConnection/saveCommit",
            &json!({"operationId":prepared.operation_id,"credentialRef":null}),
        )
        .unwrap()
    }
    fn validate(&self, conn: &McpConnection) -> McpValidationRecord {
        self.manager
            .personal_mcp_validate(McpValidateParams {
                connection_id: conn.connection_id.clone(),
                expected_revision: conn.revision,
                source_consent: true,
            })
            .unwrap()
    }
    fn grant(&self, conn: &McpConnection, record: &McpValidationRecord) -> McpConnection {
        assert_eq!(
            record.state,
            McpValidationState::Passed,
            "{:?}",
            record.reason_code
        );
        rpc(
            &self.server,
            "mcpConnection/grant",
            &McpGrantParams {
                connection_id: conn.connection_id.clone(),
                expected_revision: conn.revision,
                validation_id: record.validation_id.clone(),
                approved_tool_names: vec!["echo".into()],
                purposes: vec![McpPurpose::Foreground],
                source_consent: true,
            },
        )
        .unwrap()
    }
    fn snapshot(&self, conn: &McpConnection) -> McpToolSnapshot {
        self.manager
            .personal_mcp_snapshot(&McpSnapshotParams {
                purpose: McpPurpose::Foreground,
                connection_ids: vec![conn.connection_id.clone()],
            })
            .unwrap()
    }
    fn leases(&self) -> Value {
        self.server
            .request_result("mcpExecution/list", &json!({}))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.manager.shutdown();
        self.server.shutdown().unwrap();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
pub(crate) struct Server {
    pub(crate) url: String,
    stop: Arc<AtomicBool>,
    pub(crate) calls: Arc<AtomicUsize>,
    changed: Arc<AtomicBool>,
    fail_call: Arc<AtomicBool>,
    reflection: Arc<Mutex<Option<String>>>,
    pub(crate) requests: Arc<Mutex<Vec<Value>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
fn read_request(
    stream: std::net::TcpStream,
    expected_bearer: Option<&str>,
) -> Option<(std::io::BufReader<std::net::TcpStream>, Value)> {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    let mut reader = std::io::BufReader::new(stream);
    let mut first = String::new();
    match reader.read_line(&mut first) {
        Ok(0) => return None,
        Ok(_) => (),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            return None;
        }
        Err(error) => panic!("{error}"),
    }
    let mut length = 0;
    let mut session = false;
    let mut protocol = false;
    let mut accept = false;
    let mut authenticated = expected_bearer.is_none();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("authorization:") {
            authenticated = expected_bearer.is_some_and(|secret| {
                line.split_once(':').unwrap().1.trim() == format!("Bearer {secret}")
            });
        }
        if let Some((_, v)) = lower.split_once("content-length:") {
            length = v.trim().parse().unwrap();
        }
        session |= lower.starts_with("mcp-session-id: session-fixture");
        protocol |= lower.starts_with("mcp-protocol-version: 2025-06-18");
        accept |= lower.starts_with("accept: application/json, text/event-stream");
    }
    assert!(length < 1024 * 1024);
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).unwrap();
    let request: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(accept);
    assert!(authenticated);
    let method = request["method"].as_str().unwrap();
    if method != "initialize" {
        assert!(session);
        assert!(protocol);
    }

    Some((reader, request))
}
impl Server {
    pub(crate) fn new(sse: bool) -> Self {
        Self::with_bearer(sse, None)
    }
    fn with_bearer(sse: bool, expected_bearer: Option<String>) -> Self {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let changed = Arc::new(AtomicBool::new(false));
        let fail_call = Arc::new(AtomicBool::new(false));
        let reflection = Arc::new(Mutex::new(None::<String>));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (
            stop_flag,
            call_count,
            changed_catalog,
            fail_response,
            reflection_secret,
            received_requests,
        ) = (
            stop.clone(),
            calls.clone(),
            changed.clone(),
            fail_call.clone(),
            reflection.clone(),
            requests.clone(),
        );
        let thread = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Acquire) {
                let (stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                let Some((mut reader, request)) = read_request(stream, expected_bearer.as_deref())
                else {
                    continue;
                };
                received_requests.lock().unwrap().push(request.clone());
                let method = request["method"].as_str().unwrap();
                let result = match method {
                    "initialize" => {
                        json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"isolated-local-mcp","version":"1"}})
                    }
                    "notifications/initialized" => {
                        write!(reader.get_mut(),"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                        continue;
                    }
                    "tools/list" => {
                        json!({"tools":[{"name":"echo","description":reflection_secret.lock().unwrap().clone().unwrap_or_else(||if changed_catalog.load(Ordering::Acquire){"changed".into()}else{"echo".into()}),"inputSchema":{"type":"object","properties":{"message":{"type":"string"}}},"annotations":{"readOnlyHint":true}}]})
                    }
                    "tools/call" => {
                        call_count.fetch_add(1, Ordering::SeqCst);
                        if fail_response.load(Ordering::Acquire) {
                            continue;
                        }
                        json!({"content":[{"type":"text","text":request["params"]["arguments"]["message"]}],"isError":false})
                    }
                    _ => panic!("{method}"),
                };
                let value = json!({"jsonrpc":"2.0","id":request["id"],"result":result});
                let (mime, body) = if sse && method == "tools/list" {
                    (
                        "text/event-stream",
                        format!("event: message\r\ndata: {value}\r\n\r\n"),
                    )
                } else {
                    ("application/json", value.to_string())
                };
                write!(reader.get_mut(),"HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nMcp-Session-Id: session-fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        Self {
            url,
            stop,
            calls,
            changed,
            fail_call,
            reflection,
            requests,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Err(error) = self.thread.take().unwrap().join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(error);
        }
    }
}
fn approve() -> ToolApproval {
    Arc::new(|_, _, args, execute| execute(args))
}

#[test]
fn streamable_http_handshake_catalog_grant_broker_and_revocation_use_real_local_transport() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    let service = Server::new(true);
    let conn = f.connection(&service.url);
    let denied = f
        .manager
        .personal_mcp_validate(McpValidateParams {
            connection_id: conn.connection_id.clone(),
            expected_revision: conn.revision,
            source_consent: false,
        })
        .unwrap_err();
    assert!(denied.contains("mcp_source_consent_required"));
    assert!(service.requests.lock().unwrap().is_empty());
    let record = f.validate(&conn);
    assert_eq!(record.state, McpValidationState::Passed);
    assert_eq!(record.tools.len(), 1);
    assert_eq!(record.server_name.as_deref(), Some("isolated-local-mcp"));
    let granted = f.grant(&conn, &record);
    let snapshot = f.snapshot(&granted);
    let approvals = Arc::new(AtomicUsize::new(0));
    let count = approvals.clone();
    let broker = broker::start(
        f.manager.clone(),
        snapshot.clone(),
        Arc::new(move |_, _, args, execute| {
            count.fetch_add(1, Ordering::SeqCst);
            execute(args)
        }),
        Arc::new(|| Ok(())),
    )
    .unwrap();
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    assert_eq!(
        client
            .post(&broker.url)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
            .send()
            .unwrap()
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let list: Value = client
        .post(&broker.url)
        .bearer_auth(&broker.bearer)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let name = list["result"]["tools"][0]["name"].as_str().unwrap();
    assert!(name.starts_with("mcp_"));
    assert!(!list.to_string().contains(&service.url));
    let call:Value=client.post(&broker.url).bearer_auth(&broker.bearer).json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":{"message":"本机随机回执"}}})).send().unwrap().json().unwrap();
    assert_eq!(call["result"]["content"][0]["text"], "本机随机回执");
    assert_eq!(approvals.load(Ordering::SeqCst), 1);
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    let _: McpConnection = rpc(
        &f.server,
        "mcpConnection/disable",
        &McpMutationParams {
            connection_id: granted.connection_id.clone(),
            expected_revision: granted.revision,
        },
    )
    .unwrap();
    let revoked: Value = client
        .post(&broker.url)
        .bearer_auth(&broker.bearer)
        .json(&json!({"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert!(revoked.get("error").is_some());
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    assert!(
        f.leases()
            .as_array()
            .unwrap()
            .iter()
            .all(|lease| lease["state"] == "finished")
    );
    drop(broker);
}

#[test]
fn changed_catalog_revokes_permission_without_calling_a_tool() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    let service = Server::new(false);
    let conn = f.connection(&service.url);
    let record = f.validate(&conn);
    let granted = f.grant(&conn, &record);
    let snapshot = f.snapshot(&granted);
    service.changed.store(true, Ordering::Release);
    assert_eq!(
        call_tool(
            &f.manager,
            &snapshot,
            &granted.connection_id,
            "echo",
            &json!({}),
            &approve()
        )
        .unwrap_err(),
        "mcp_catalog_changed"
    );
    assert_eq!(service.calls.load(Ordering::SeqCst), 0);
    let current: McpConnection = rpc(
        &f.server,
        "mcpConnection/get",
        &McpConnectionParams {
            connection_id: granted.connection_id,
        },
    )
    .unwrap();
    assert!(!current.enabled);
}

#[test]
fn lost_remote_tool_response_is_unconfirmed_and_never_retried() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    let service = Server::new(false);
    let conn = f.connection(&service.url);
    let record = f.validate(&conn);
    let granted = f.grant(&conn, &record);
    let snapshot = f.snapshot(&granted);
    service.fail_call.store(true, Ordering::Release);
    assert_eq!(
        call_tool(
            &f.manager,
            &snapshot,
            &granted.connection_id,
            "echo",
            &json!({}),
            &approve()
        )
        .unwrap_err(),
        "mcp_tool_outcome_unconfirmed"
    );
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    assert!(
        f.leases()
            .as_array()
            .unwrap()
            .iter()
            .any(|lease| lease["outcome"] == "outcome_unconfirmed")
    );
}

#[test]
fn validation_rejects_secret_reflection_before_persisting_server_metadata() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    let synthetic = "test-only-unusable-mcp-secret-731";
    let service = Server::with_bearer(true, Some(synthetic.into()));
    let conn = f.connection(&service.url);
    *service.reflection.lock().unwrap() = Some(synthetic.into());
    let mut execution = Execution::prepare(
        &f.manager,
        &McpExecutionPrepareParams {
            connection_id: conn.connection_id.clone(),
            expected_revision: conn.revision,
            intent: McpExecutionIntent::Validation,
            purpose: None,
            tool_name: None,
            catalog_digest: None,
            source_consent: true,
            owner: crate::model_connections::execution_owner().unwrap(),
            launch_protocol: 1,
        },
    )
    .unwrap();
    let mut transport_conn = conn.clone();
    transport_conn.config.auth_mode = McpAuthMode::Bearer;
    let result = crate::async_support::block_on(async {
        let mut session = transport::Session::Remote(Box::new(
            remote::Session::open(
                &transport_conn,
                Some(Secret(synthetic.into())),
                &super::credential_vault_root(&f.manager.gateway_data_dir().unwrap()).unwrap(),
            )
            .unwrap(),
        ));
        session.discover(&mut execution).await
    });
    assert_eq!(result.err().unwrap(), "mcp_response_contains_credentials");
    execution.finish("failed").unwrap();
    assert!(!f.leases().to_string().contains(synthetic));
}

#[test]
fn bearer_secret_map_is_exact_and_ordinary_json_is_never_the_auth_header() {
    let mut bytes = br#"{"token":"test-only-token-831"}"#.to_vec();
    let secret = decode_bearer(&mut bytes).unwrap();
    assert_eq!(secret.expose(), "test-only-token-831");
    assert!(bytes.iter().all(|b| *b == 0));
    for input in [
        r#"{"API_KEY":"wrong-name"}"#,
        r#"{"token":"valid","another":"forbidden"}"#,
        r#""a-plain-string""#,
        r#"{"token":31}"#,
    ] {
        let mut bytes = input.as_bytes().to_vec();
        assert!(decode_bearer(&mut bytes).is_err());
        assert!(bytes.iter().all(|b| *b == 0));
    }
}

#[test]
fn unsupported_child_permission_fails_explicitly_and_releases_only_the_unlaunched_lease() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    let prepared:McpConnectionChange=rpc(&f.server,"mcpConnection/savePrepare",&json!({"operationId":"local-blocked","connectionId":null,"expectedRevision":null,"config":{"name":"待平台隔离支持","source":{"kind":"local_command","program":std::env::current_exe().unwrap().canonicalize().unwrap().to_string_lossy(),"args":[],"cwd":f.root.canonicalize().unwrap().to_string_lossy(),"permissions":{"readRoots":[],"writeRoots":[],"network":false,"childProcesses":true}},"authMode":"none","environmentVariables":[],"network":{"mode":"direct","proxyUrl":null}},"credentialAction":"clear"})).unwrap();
    let conn: McpConnection = rpc(
        &f.server,
        "mcpConnection/saveCommit",
        &json!({"operationId":prepared.operation_id,"credentialRef":null}),
    )
    .unwrap();
    let record = f.validate(&conn);
    assert_eq!(record.state, McpValidationState::Failed);
    assert!(matches!(
        record.reason_code.as_deref(),
        Some("mcp_child_processes_not_supported" | "mcp_local_isolation_not_supported")
    ));
    let leases = f.leases();
    assert_eq!(leases.as_array().unwrap().len(), 1);
    assert_eq!(leases[0]["state"], "finished");
    assert_eq!(leases[0]["processes"], json!([]));
}

fn assert_processes_exited(leases: &Value) {
    assert_eq!(leases.as_array().unwrap().len(), 2);
    for lease in leases.as_array().unwrap() {
        assert_eq!(lease["state"], "finished");
        assert_eq!(lease["processes"].as_array().unwrap().len(), 1);
        let process = &lease["processes"][0];
        let now = contexthub_platform_adapter::instance::process_start_fingerprint(
            u32::try_from(process["pid"].as_u64().unwrap()).unwrap(),
        )
        .unwrap();
        assert_ne!(now.as_deref(), process["startFingerprint"].as_str());
    }
}
#[test]
#[ignore = "Run explicitly on macOS with the native MCP sandbox and official sidecar fixture"]
fn native_local_stdio_is_registered_before_permit_and_source_change_never_starts_the_new_script() {
    let _guard = crate::supervisor::app_server_test_guard();
    let f = Fixture::new();
    let source = f.root.canonicalize().unwrap().join("mcp-source");
    std::fs::create_dir_all(&source).unwrap();
    let entry = source.join("server.mjs");
    let code = r"import fs from 'node:fs';import readline from 'node:readline';if(process.env.MCP_TEST_TOKEN!=='synthetic-native-env-942')throw Error('mcp_fixture_auth_failed');fs.writeFileSync(process.env.HOME+'/launch.txt',String(process.pid));const lines=readline.createInterface({input:process.stdin});for await(const line of lines){const q=JSON.parse(line);if(!('id'in q))continue;let result;if(q.method==='initialize')result={protocolVersion:'2025-06-18',capabilities:{tools:{}},serverInfo:{name:'native-local-mcp',version:'1'}};else if(q.method==='tools/list')result={tools:[{name:'echo',description:'本机标准管道工具',inputSchema:{type:'object',properties:{message:{type:'string'}},required:['message']},annotations:{readOnlyHint:false}}]};else if(q.method==='tools/call'){fs.writeFileSync(process.env.HOME+'/call.txt',q.params.arguments.message);result={content:[{type:'text',text:q.params.arguments.message}],isError:false}}else throw Error('unexpected');console.log(JSON.stringify({jsonrpc:'2.0',id:q.id,result}));}";
    std::fs::write(&entry, code).unwrap();
    let program = contexthub_platform_adapter::executable_search::find_executables(&["node"])
        .into_iter()
        .next()
        .unwrap()
        .canonicalize()
        .unwrap();
    let prepared:McpConnectionChange=rpc(&f.server,"mcpConnection/savePrepare",&json!({"operationId":"native-stdio","connectionId":null,"expectedRevision":null,"config":{"name":"本机标准管道","source":{"kind":"local_command","program":program.to_string_lossy(),"args":[entry.to_string_lossy()],"cwd":source.to_string_lossy(),"permissions":{"readRoots":[source.to_string_lossy()],"writeRoots":[],"network":false,"childProcesses":false}},"authMode":"environment","environmentVariables":["MCP_TEST_TOKEN"],"network":{"mode":"direct","proxyUrl":null}},"credentialAction":"replace"})).unwrap();
    store_vault_map(
        &f,
        &prepared,
        &json!({"MCP_TEST_TOKEN":"synthetic-native-env-942"}),
    );
    let conn: McpConnection = rpc(
        &f.server,
        "mcpConnection/saveCommit",
        &json!({"operationId":prepared.operation_id,"credentialRef":prepared.connection.credential_ref}),
    )
    .unwrap();
    let record = f.validate(&conn);
    assert_eq!(
        record.state,
        McpValidationState::Passed,
        "{:?}",
        record.reason_code
    );
    let granted = f.grant(&conn, &record);
    let snapshot = f.snapshot(&granted);
    let value = call_tool(
        &f.manager,
        &snapshot,
        &granted.connection_id,
        "echo",
        &json!({"message":"native-stdio-731"}),
        &approve(),
    )
    .unwrap();
    assert_eq!(value["content"][0]["text"], "native-stdio-731");
    let leases = f.leases();
    assert_processes_exited(&leases);
    let data = f.manager.gateway_data_dir().unwrap();
    let call_home = data
        .join("mcp-runtime")
        .join(&granted.connection_id)
        .join(leases[1]["leaseId"].as_str().unwrap());
    let homes = std::fs::read_dir(data.join("mcp-runtime").join(&granted.connection_id))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(
        homes
            .iter()
            .filter(|home| home.join("launch.txt").is_file())
            .count(),
        2
    );
    assert!(homes.iter().any(|home| {
        std::fs::read_to_string(home.join("call.txt")).is_ok_and(|text| text == "native-stdio-731")
    }));
    let _ = call_home;
    std::fs::write(&entry, format!("{code}\n// changed entry source")).unwrap();
    assert_eq!(
        call_tool(
            &f.manager,
            &snapshot,
            &granted.connection_id,
            "echo",
            &json!({"message":"must-not-start"}),
            &approve()
        )
        .unwrap_err(),
        "mcp_source_changed"
    );
    let homes = std::fs::read_dir(data.join("mcp-runtime").join(&granted.connection_id))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(
        homes
            .iter()
            .filter(|home| home.join("launch.txt").is_file())
            .count(),
        2
    );
    let current: McpConnection = rpc(
        &f.server,
        "mcpConnection/get",
        &McpConnectionParams {
            connection_id: granted.connection_id,
        },
    )
    .unwrap();
    assert!(!current.enabled);
}

#[test]
fn malformed_tool_responses_cannot_be_recorded_as_success() {
    for value in [
        json!({}),
        json!({"content":null}),
        json!({"content":[{"type":"text","text":null}]}),
        json!({"content":[],"isError":"false"}),
    ] {
        assert_eq!(
            validate_tool_result(&value).unwrap_err(),
            "mcp_tool_response_invalid"
        );
    }
    assert!(
        validate_tool_result(&json!({"content":[{"type":"text","text":"已确认"}],"isError":false}))
            .is_ok()
    );
}

fn store_vault_map(fixture: &Fixture, prepared: &McpConnectionChange, secrets: &Value) {
    let root = credential_vault_root(&fixture.manager.gateway_data_dir().unwrap()).unwrap();
    let vault =
        contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(root).unwrap();
    let reference = prepared.connection.credential_ref.as_ref().unwrap();
    let mut bytes = serde_json::to_vec(secrets).unwrap();
    vault
        .write(&reference.service, &reference.account, &bytes)
        .unwrap();
    let mut readback = vault
        .read(&reference.service, &reference.account)
        .unwrap()
        .unwrap();
    assert_eq!(readback, bytes);
    readback.fill(0);
    bytes.fill(0);
}
fn vault_bearer_connection(fixture: &Fixture, url: &str, secret: &str) -> McpConnection {
    let prepared: McpConnectionChange = rpc(&fixture.server,"mcpConnection/savePrepare",&json!({"operationId":uuid::Uuid::new_v4().simple().to_string(),"connectionId":null,"expectedRevision":null,"config":{"name":"本地加密凭据传输夹具","source":{"kind":"remote_http","url":url},"authMode":"bearer","environmentVariables":[],"network":{"mode":"direct","proxyUrl":null}},"credentialAction":"replace"})).unwrap();
    store_vault_map(fixture, &prepared, &json!({"token":secret}));
    rpc(
        &fixture.server,
        "mcpConnection/saveCommit",
        &json!({"operationId":prepared.operation_id,"credentialRef":prepared.connection.credential_ref}),
    )
    .unwrap()
}
#[test]
fn encrypted_vault_bearer_is_used_by_real_local_transport_and_missing_secret_sends_nothing() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    let service = Server::with_bearer(true, Some("synthetic-local-mcp-credential-941".into()));
    let connection =
        vault_bearer_connection(&fixture, &service.url, "synthetic-local-mcp-credential-941");
    let record = fixture.validate(&connection);
    assert_eq!(record.state, McpValidationState::Passed);
    let granted = fixture.grant(&connection, &record);
    let snapshot = fixture.snapshot(&granted);
    let result = call_tool(
        &fixture.manager,
        &snapshot,
        &granted.connection_id,
        "echo",
        &json!({"message":"合成工具回执"}),
        &approve(),
    )
    .unwrap();
    assert_eq!(result["content"][0]["text"], "合成工具回执");
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    let disabled: McpConnection = rpc(
        &fixture.server,
        "mcpConnection/disable",
        &McpMutationParams {
            connection_id: granted.connection_id.clone(),
            expected_revision: granted.revision,
        },
    )
    .unwrap();
    assert!(
        call_tool(
            &fixture.manager,
            &snapshot,
            &granted.connection_id,
            "echo",
            &json!({"message":"不能重发"}),
            &approve()
        )
        .is_err()
    );
    assert_eq!(service.calls.load(Ordering::SeqCst), 1);
    let root = credential_vault_root(&fixture.manager.gateway_data_dir().unwrap()).unwrap();
    let vault =
        contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(root).unwrap();
    let reference = disabled.credential_ref.as_ref().unwrap();
    vault
        .delete(&reference.service, &reference.account)
        .unwrap();
    let before = service.requests.lock().unwrap().len();
    let failed = fixture.validate(&disabled);
    assert_eq!(failed.state, McpValidationState::Failed);
    assert_eq!(
        failed.reason_code.as_deref(),
        Some("mcp_credentials_required")
    );
    assert_eq!(service.requests.lock().unwrap().len(), before);
    assert!(
        fixture
            .leases()
            .as_array()
            .unwrap()
            .iter()
            .all(|lease| lease["state"] == "finished")
    );
}
