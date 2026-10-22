use super::{Fixture, respond};
use crate::ai_runtime::AiRuntimeManager;
use crate::codex_threads::CodexThreadService;
use crate::model_connections::{ExecutionBundle, ExecutionScope, ModelRuntimeKind};
use contexthub_protocol::{
    MODEL_CREDENTIAL_SERVICE, MODEL_NETWORK_CREDENTIAL_SERVICE, ModelConnection,
    ModelExecutionPrepareParams, ModelPurpose, ModelSecretRef, ModelSelection,
};
use serde_json::{Value, json};
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;

const MODEL: &str = "isolated-official-fixture";

struct Endpoint {
    url: String,
    password: String,
    header: String,
    accepted: Arc<AtomicUsize>,
    rejected: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Endpoint {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let password = uuid::Uuid::new_v4().simple().to_string();
        let header = uuid::Uuid::new_v4().simple().to_string();
        let authorization = format!("Bearer {password}");
        let expected_header = header.clone();
        let accepted = Arc::new(AtomicUsize::new(0));
        let rejected = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (worker_stop, worker_accepted, worker_rejected) = (
            Arc::clone(&stop),
            Arc::clone(&accepted),
            Arc::clone(&rejected),
        );
        let worker = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(10)))
                            .unwrap();
                        let Some((headers, body)) = request(&mut stream) else {
                            continue;
                        };
                        let authorized = headers.get("authorization") == Some(&authorization)
                            && headers.get("x-network-scope") == Some(&expected_header)
                            && !headers.contains_key("proxy-authorization");
                        if authorized {
                            worker_accepted.fetch_add(1, Ordering::AcqRel);
                            respond(
                                &mut stream,
                                &body,
                                headers.get("x-openai-subagent").map(String::as_str),
                            );
                        } else {
                            worker_rejected.fetch_add(1, Ordering::AcqRel);
                            let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            }
        });
        Self {
            url,
            password,
            header,
            accepted,
            rejected,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn request(stream: &mut TcpStream) -> Option<(std::collections::BTreeMap<String, String>, Value)> {
    let mut raw = Vec::new();
    while !raw.ends_with(b"\r\n\r\n") && raw.len() <= 32768 {
        let mut byte = [0_u8];
        stream.read_exact(&mut byte).ok()?;
        raw.push(byte[0]);
    }
    let text = String::from_utf8(raw).ok()?;
    let headers = text
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<std::collections::BTreeMap<_, _>>();
    let size = headers.get("content-length")?.parse::<usize>().ok()?;
    let mut body = vec![0_u8; size];
    stream.read_exact(&mut body).ok()?;
    Some((headers, serde_json::from_slice(&body).ok()?))
}

struct Credentials(
    Vec<ModelSecretRef>,
    contexthub_platform_adapter::credential_vault::LocalCredentialVault,
);
impl Credentials {
    fn write(&mut self, reference: &ModelSecretRef, mut bytes: Vec<u8>) {
        self.0.push(reference.clone());
        let result = self.1.write(&reference.service, &reference.account, &bytes);
        bytes.fill(0);
        result.unwrap();
    }

    fn clear(&self) {
        for reference in &self.0 {
            self.1
                .delete(&reference.service, &reference.account)
                .unwrap();
            assert!(
                self.1
                    .read(&reference.service, &reference.account)
                    .unwrap()
                    .is_none()
            );
        }
    }
}
impl Drop for Credentials {
    fn drop(&mut self) {
        for reference in &self.0 {
            let _ = self.1.delete(&reference.service, &reference.account);
        }
    }
}

fn connection(
    fixture: &Fixture,
    endpoint: &Endpoint,
    credentials: &mut Credentials,
) -> ModelConnection {
    let prepared = fixture.server.request_result("modelConnection/savePrepare", &json!({
        "operationId":uuid::Uuid::new_v4().simple().to_string(),"credentialAction":"replace","networkCredentialAction":"replace",
        "config":{"name":"本机独立用途认证","kind":"local","localRuntime":"ollama","baseUrl":endpoint.url,"authMode":"api_key","enabled":true,
            "network":{"mode":"direct","serviceAuth":"none","headerNames":["x-network-scope"]}}
    })).unwrap();
    let reference: ModelSecretRef = serde_json::from_value(
        prepared["connection"]["config"]["network"]["proxySecretRef"].clone(),
    )
    .unwrap();
    assert_eq!(reference.service, MODEL_NETWORK_CREDENTIAL_SERVICE);
    let model_reference: ModelSecretRef =
        serde_json::from_value(prepared["connection"]["secretRef"].clone()).unwrap();
    assert_eq!(model_reference.service, MODEL_CREDENTIAL_SERVICE);
    credentials.write(&model_reference, endpoint.password.as_bytes().to_vec());
    credentials.write(&reference, serde_json::to_vec(&json!({"proxyBasic":null,"serviceBasic":null,"headers":{"x-network-scope":endpoint.header}})).unwrap());
    let current: ModelConnection = serde_json::from_value(
        fixture
            .server
            .request_result(
                "modelConnection/saveCommit",
                &json!({
                    "operationId":prepared["operationId"],"credentialRef":model_reference,"networkCredentialRef":reference
                }),
            )
            .unwrap(),
    )
    .unwrap();
    fixture.server.request_result("modelDescriptor/save",&json!({"connectionId":current.connection_id,"modelId":MODEL,"displayName":"本机用途认证模型"})).unwrap();
    fixture.validate_model(&current, MODEL);
    current
}

fn binding(fixture: &Fixture, connection: &ModelConnection, purpose: &str) {
    fixture.server.request_result("modelPurpose/save", &json!({"binding":{
        "bindingId":format!("profile:{purpose}"),"purpose":purpose,"scopeId":null,"policy":"explicit",
        "selection":{"connectionId":connection.connection_id,"modelId":MODEL,"reasoningEffort":null},"enabled":true,"revision":0,
        "consent":{"serviceName":"本机用途认证服务","historyScope":"native_scope_history","mayIncurCost":false,"confirmedAt":1}
    },"expectedRevision":null})).unwrap();
}

fn enable_memory(manager: &AiRuntimeManager) {
    let service = manager.memory_settings().unwrap().service.unwrap();
    manager
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
}

fn scope(
    fixture: &Fixture,
    connection: &ModelConnection,
    purpose: ModelPurpose,
) -> Arc<ExecutionScope> {
    let foreground = purpose == ModelPurpose::Foreground;
    ExecutionScope::prepare(
        &fixture.manager,
        ModelExecutionPrepareParams {
            owner: None,
            launch_protocol: 1,
            binding_id: match purpose {
                ModelPurpose::Review => Some("profile:review".into()),
                ModelPurpose::Memory => Some("profile:memory".into()),
                _ => None,
            },
            purpose,
            selection: foreground.then(|| ModelSelection {
                connection_id: connection.connection_id.clone(),
                model_id: MODEL.into(),
                reasoning_effort: None,
            }),
            conversation_id: None,
        },
    )
    .unwrap()
}

fn start_owner(scope: Arc<ExecutionScope>) -> CodexThreadService {
    let service = CodexThreadService::new(
        crate::codex_harness::bundled_codex_harness_executable()
            .unwrap()
            .into(),
        scope.home(ModelRuntimeKind::Workbench),
    )
    .with_network(scope.lease.snapshot.network.clone())
    .with_execution_scope(scope);
    service
        .request("config/read", &json!({"includeLayers":false}))
        .unwrap();
    service
}

fn invoke(
    proxy: &crate::model_session_proxy::ModelSessionProxy,
    subagent: Option<&str>,
) -> reqwest::StatusCode {
    crate::async_support::block_on(async {
        let mut request = reqwest::Client::new().post(format!("{}/responses",proxy.base_url)).bearer_auth(&proxy.local_token)
            .header("x-network-scope","must-be-replaced-by-selected-scope")
            .json(&json!({"model":MODEL,"input":[{"type":"message","role":"user","content":"only local routing component"}],"tools":[]}));
        if let Some(subagent) = subagent {
            request = request.header("x-openai-subagent", subagent);
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let _ = response.bytes().await.unwrap();
        status
    })
}

#[test]
fn each_actual_purpose_uses_its_own_endpoint_local_authentication_and_revocation_without_fallback()
{
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    let endpoints = [Endpoint::new(), Endpoint::new(), Endpoint::new()];
    let mut credentials = Credentials(
        Vec::new(),
        contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(
            fixture.profile.join("credentials"),
        )
        .unwrap(),
    );
    let connections = endpoints
        .each_ref()
        .map(|endpoint| connection(&fixture, endpoint, &mut credentials));
    binding(&fixture, &connections[1], "review");
    binding(&fixture, &connections[2], "memory");
    enable_memory(&fixture.manager);
    let scopes = [
        scope(&fixture, &connections[0], ModelPurpose::Foreground),
        scope(&fixture, &connections[1], ModelPurpose::Review),
        scope(&fixture, &connections[2], ModelPurpose::Memory),
    ];
    let services = scopes
        .each_ref()
        .map(|scope| start_owner(Arc::clone(scope)));
    let bundle = ExecutionBundle {
        foreground: Arc::clone(&scopes[0]),
        review: Some(Arc::clone(&scopes[1])),
        memory: Some(Arc::clone(&scopes[2])),
    };
    let proxy = crate::model_session_proxy::start_model_session_proxy(
        bundle.clone(),
        &endpoints[0].url,
        "network-scope-component",
    )
    .unwrap();
    proxy.configure_native_memory(Some(MODEL)).unwrap();
    let before = endpoints
        .each_ref()
        .map(|endpoint| endpoint.accepted.load(Ordering::Acquire));
    for subagent in [None, Some("guardian"), Some("memory_consolidation")] {
        assert_eq!(invoke(&proxy, subagent), reqwest::StatusCode::OK);
    }
    assert_eq!(
        endpoints
            .each_ref()
            .map(|endpoint| endpoint.accepted.load(Ordering::Acquire)),
        before.map(|count| count + 1)
    );
    assert!(
        endpoints
            .iter()
            .all(|endpoint| endpoint.rejected.load(Ordering::Acquire) == 0)
    );
    let before = endpoints
        .each_ref()
        .map(|endpoint| endpoint.accepted.load(Ordering::Acquire));
    let review = connections[1]
        .config
        .network
        .proxy_secret_ref
        .as_ref()
        .unwrap();
    credentials
        .1
        .delete(&review.service, &review.account)
        .unwrap();
    assert_eq!(
        invoke(&proxy, Some("guardian")),
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        endpoints
            .each_ref()
            .map(|endpoint| endpoint.accepted.load(Ordering::Acquire)),
        before
    );
    drop(proxy);
    for service in &services {
        service.shutdown_checked().unwrap();
    }
    bundle.finish_release().unwrap();
    credentials.clear();
}
