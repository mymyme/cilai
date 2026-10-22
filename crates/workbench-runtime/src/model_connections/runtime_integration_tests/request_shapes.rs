use super::*;
use contexthub_protocol::{
    ModelValidationLayer, ModelValidationOutcome, ModelValidationPrepareParams,
};

struct ShapeFixture {
    root: PathBuf,
    manager: AiRuntimeManager,
    server: AppServerSupervisor,
    connection: ModelConnection,
    shapes: Arc<Mutex<Vec<Value>>>,
    active_layer: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn known_enum(value: &Value, allowed: &[&str]) -> Value {
    value
        .as_str()
        .filter(|text| allowed.contains(text))
        .map_or(Value::Null, |text| json!(text))
}

fn request_shape(body: &Value, layer: usize) -> Value {
    let fields = body
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| {
            let empty = match value {
                Value::String(value) => value.is_empty(),
                Value::Array(value) => value.is_empty(),
                Value::Object(value) => value.is_empty(),
                _ => false,
            };
            (
                name.clone(),
                json!({"type":type_name(value),"isNull":value.is_null(),"isEmpty":empty}),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let tools = body.get("tools").and_then(Value::as_array);
    let tool_types = tools.map(|tools| {
        tools
            .iter()
            .map(|tool| {
                known_enum(
                    &tool["type"],
                    &[
                        "function",
                        "custom",
                        "tool_search",
                        "namespace",
                        "web_search",
                        "web_search_preview",
                    ],
                )
            })
            .collect::<Vec<_>>()
    });
    let flag = |name| body.get(name).and_then(Value::as_bool);
    json!({"layerIndex":layer,"topLevelFields":fields,
        "toolCount":tools.map(Vec::len),"toolTypes":tool_types,
        "knownEnums":{"toolChoice":known_enum(&body["tool_choice"], &["auto","none","required"]),
            "reasoningEffort":known_enum(&body["reasoning"]["effort"], &["none","minimal","low","medium","high","xhigh"]),
            "reasoningSummary":known_enum(&body["reasoning"]["summary"], &["auto","concise","detailed"]),
            "textVerbosity":known_enum(&body["text"]["verbosity"], &["low","medium","high"]),
            "textFormat":known_enum(&body["text"]["format"]["type"], &["text","json_schema","json_object"])},
        "flags":{"stream":flag("stream"),"store":flag("store"),"parallelToolCalls":flag("parallel_tool_calls")},
        "includeCount":body["include"].as_array().map(Vec::len),
        "inputItemCount":body["input"].as_array().map(Vec::len),
        "reasoningFieldNames":body["reasoning"].as_object().map(|value|value.keys().collect::<Vec<_>>()),
        "textFieldNames":body["text"].as_object().map(|value|value.keys().collect::<Vec<_>>())})
}

fn create_connection(
    server: &AppServerSupervisor,
    address: std::net::SocketAddr,
) -> ModelConnection {
    let prepared = server.request_result("modelConnection/savePrepare", &json!({
        "operationId":"synthetic-request-shapes","connectionId":null,"expectedConfigRevision":null,
        "config":{"name":"本机请求结构探针","kind":"custom_responses","baseUrl":format!("http://{address}/v1"),
            "localRuntime":null,"authMode":"none","enabled":true,"network":{"mode":"direct"}},"credentialAction":"clear"
    })).unwrap();
    let connection: ModelConnection = serde_json::from_value(
        server
            .request_result(
                "modelConnection/saveCommit",
                &json!({"operationId":prepared["operationId"],"credentialRef":null}),
            )
            .unwrap(),
    )
    .unwrap();
    server.request_result("modelDescriptor/save", &json!({
        "connectionId":connection.connection_id,"modelId":"isolated-official-fixture","displayName":"结构探针模型",
        "expectedRevision":null,"contextWindowTokens":512_000,"inputModalities":["text"],
        "defaultReasoningEffort":null,"supportedReasoningEfforts":[]
    })).unwrap();
    connection
}

impl ShapeFixture {
    fn new() -> Self {
        let root = fixture_root("safe-request-shapes");
        let profile = profile_data_root(&root);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let active_layer = Arc::new(AtomicUsize::new(0));
        let shapes = Arc::new(Mutex::new(Vec::new()));
        let worker = start_worker(
            listener,
            Arc::clone(&stop),
            Arc::clone(&active_layer),
            Arc::clone(&shapes),
        );
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &profile,
                &profile.join("run"),
            )
            .unwrap();
        let connection = create_connection(&server, address);
        let manager = AiRuntimeManager::new();
        manager.initialize(&profile, &server).unwrap();
        Self {
            root,
            manager,
            server,
            connection,
            shapes,
            active_layer,
            stop,
            worker: Some(worker),
        }
    }
}

fn start_worker(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    layer: Arc<AtomicUsize>,
    shapes: Arc<Mutex<Vec<Value>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    let Some((body, subagent)) = request_body(&mut socket) else {
                        continue;
                    };
                    assert!(subagent.is_none());
                    assert_eq!(body["model"], "isolated-official-fixture");
                    let active_layer = layer.load(Ordering::Acquire);
                    assert_normalized_tool_shape(&body, active_layer);
                    shapes
                        .lock()
                        .unwrap()
                        .push(request_shape(&body, active_layer));
                    respond(&mut socket, &body, None);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("synthetic_request_shape_listener_failed:{:?}", error.kind()),
            }
        }
    })
}

fn assert_normalized_tool_shape(body: &Value, layer: usize) {
    if layer < 3 {
        for field in ["tools", "tool_choice", "parallel_tool_calls"] {
            assert!(body.get(field).is_none());
        }
    } else {
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], true);
    }
}

impl Drop for ShapeFixture {
    fn drop(&mut self) {
        let runtime_stopped = self.manager.shutdown_checked().is_ok();
        let server_stopped = self.server.shutdown().is_ok();
        self.stop.store(true, Ordering::Release);
        let joined = self.worker.take().unwrap().join().is_ok();
        if !std::thread::panicking() {
            assert!(runtime_stopped && server_stopped && joined);
        }
        if runtime_stopped && server_stopped && joined {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

#[test]
#[ignore = "显式本机结构诊断，需指定受管证据路径；不调用真实模型服务"]
fn official_four_validation_probes_capture_only_nonsecret_upstream_request_shapes() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = ShapeFixture::new();
    for (index, layer) in [
        ModelValidationLayer::Protocol,
        ModelValidationLayer::Inference,
        ModelValidationLayer::Streaming,
        ModelValidationLayer::Tools,
    ]
    .into_iter()
    .enumerate()
    {
        fixture.active_layer.store(index, Ordering::Release);
        let record = fixture
            .manager
            .personal_model_validate(ModelValidationPrepareParams {
                runtime_kind: ModelRuntimeKind::Workbench,
                connection_id: fixture.connection.connection_id.clone(),
                model_id: Some("isolated-official-fixture".into()),
                layer,
                expected_config_revision: fixture.connection.config_revision,
                expected_auth_generation: fixture.connection.auth_generation,
            })
            .unwrap();
        assert_eq!(record.outcome, Some(ModelValidationOutcome::Passed));
    }
    let shapes = fixture.shapes.lock().unwrap().clone();
    for index in 0..4 {
        assert!(shapes.iter().any(|shape| shape["layerIndex"] == index));
    }
    let target = std::env::var_os("CILAI_PERSONAL_REQUEST_SHAPE_EVIDENCE")
        .map(PathBuf::from)
        .unwrap();
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .unwrap();
    assert_eq!(
        target.parent(),
        Some(project.join("07测试/实施证据").as_path())
    );
    fs::write(target,serde_json::to_vec_pretty(&json!({"runtime":"0.163.0","layers":["protocol","inference","streaming","tools"],"requests":shapes,
        "scope":"loopback synthetic endpoints only; no raw prompt, body, headers, credentials or real model requests recorded"})).unwrap()).unwrap();
}
