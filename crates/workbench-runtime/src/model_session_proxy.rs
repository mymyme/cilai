use crate::application_tool_adapter::AdapterSlot;
use crate::model_connections::ExecutionBundle;
#[cfg(test)]
use crate::model_connections::connection_client;
use futures_util::future::BoxFuture;
use futures_util::stream;
use http_body_util::{BodyExt as _, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use reqwest::{Client, Url};
use serde_json::{Value, json};
use std::convert::Infallible;
use std::error::Error;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

type ProxyError = Box<dyn Error + Send + Sync>;
type ProxyBody = UnsyncBoxBody<Bytes, ProxyError>;
type TokenProvider = Arc<dyn Fn() -> BoxFuture<'static, Result<String, String>> + Send + Sync>;
mod lifecycle;
pub(crate) mod service_error;
mod usage;
use lifecycle::RequestScopeMonitor;

#[derive(Clone)]
struct ProxyState {
    #[cfg(test)]
    routing_only_fixture: bool,
    start_admission: Option<crate::application_tool_adapter::ScopeValidator>,
    scope_id: String,
    active: bool,
    memory: Option<NativeMemoryGrant>,
    page_tool_names: Option<Vec<String>>,
    retired: Arc<AtomicBool>,
    token_provider: TokenProvider,
    execution: Option<ExecutionBundle>,
    upstream_base: Option<Url>,
    client: Client,
    gateway_origin: Url,
    local_token: String,
    application_tools: AdapterSlot,
}

#[derive(Clone)]
struct NativeMemoryGrant {
    model: String,
    scope_id: String,
}

type ProxyBinding = Arc<Mutex<Arc<ProxyState>>>;

pub(crate) struct ModelSessionProxy {
    pub base_url: String,
    pub local_token: String,
    pub application_mcp_url: String,
    pub(crate) execution: Option<ExecutionBundle>,
    binding: ProxyBinding,
    shutdown: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl ModelSessionProxy {
    pub(crate) fn set_start_admission(
        &self,
        admission: Option<crate::application_tool_adapter::ScopeValidator>,
    ) -> Result<(), String> {
        let mut binding = self
            .binding
            .lock()
            .map_err(|_| "model_session_proxy_binding_unavailable")?;
        if binding.retired.load(Ordering::Acquire) {
            return Err("model_session_proxy_retired".into());
        }
        let mut next = (**binding).clone();
        next.start_admission = admission;
        *binding = Arc::new(next);
        Ok(())
    }
    pub(crate) fn constrain_personal_page_tools(
        &self,
        has_tools: bool,
        has_actions: bool,
    ) -> Result<(), String> {
        let mut binding = self
            .binding
            .lock()
            .map_err(|_| "model_session_proxy_binding_unavailable")?;
        if binding.retired.load(Ordering::Acquire) || binding.page_tool_names.is_some() {
            return Err("personal_page_proxy_scope_invalid".into());
        }
        let mut next = (**binding).clone();
        let mut names = Vec::new();
        if has_tools {
            names.push("cilai_personal_tools".to_owned());
        }
        if has_actions {
            names.push("cilai_application_action".to_owned());
        }
        next.page_tool_names = Some(names);
        *binding = Arc::new(next);
        Ok(())
    }

    /// 仅由空闲运行时的独占持有者调用；在途请求继续持有原轮次，不改写其权限和计时归属。
    pub(crate) fn bind_run(&self, scope_id: &str) -> Result<AdapterSlot, String> {
        let mut binding = self
            .binding
            .lock()
            .map_err(|_| "model_session_proxy_binding_unavailable")?;
        if binding.retired.load(Ordering::Acquire) {
            return Err("model_session_proxy_retired".into());
        }
        let mut next = (**binding).clone();
        scope_id.clone_into(&mut next.scope_id);
        next.start_admission = None;
        next.active = true;
        next.application_tools = AdapterSlot::default();
        let tools = next.application_tools.clone();
        *binding = Arc::new(next);
        Ok(tools)
    }

    pub(crate) fn configure_native_memory(&self, model: Option<&str>) -> Result<(), String> {
        let mut binding = self
            .binding
            .lock()
            .map_err(|_| "model_session_proxy_binding_unavailable")?;
        if binding.retired.load(Ordering::Acquire) {
            return Err("model_session_proxy_retired".into());
        }
        let mut next = (**binding).clone();
        next.memory = model.map(|model| {
            next.memory
                .as_ref()
                .filter(|grant| grant.model == model)
                .cloned()
                .unwrap_or_else(|| NativeMemoryGrant {
                    model: model.to_owned(),
                    scope_id: format!("native-memory-{}", uuid::Uuid::new_v4()),
                })
        });
        *binding = Arc::new(next);
        Ok(())
    }

    /// 仅在旧运行进程已确认退出或尚未启动进程时建立新授权代次。
    pub(crate) fn begin_runtime(&self, model: Option<&str>) -> Result<(), String> {
        let mut binding = self
            .binding
            .lock()
            .map_err(|_| "model_session_proxy_binding_unavailable")?;
        binding.retired.store(true, Ordering::Release);
        let mut next = (**binding).clone();
        next.retired = Arc::new(AtomicBool::new(false));
        next.memory = model.map(|model| NativeMemoryGrant {
            model: model.to_owned(),
            scope_id: format!("native-memory-{}", uuid::Uuid::new_v4()),
        });
        *binding = Arc::new(next);
        Ok(())
    }

    pub(crate) fn suspend(&self) -> Result<(), String> {
        let mut binding = self
            .binding
            .lock()
            .map_err(|_| "model_session_proxy_binding_unavailable")?;
        let mut idle = (**binding).clone();
        idle.active = false;
        *binding = Arc::new(idle);
        Ok(())
    }

    pub(crate) fn retire(&self) {
        self.binding
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retired
            .store(true, Ordering::Release);
    }
}

impl Drop for ModelSessionProxy {
    fn drop(&mut self) {
        self.retire();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn start_model_session_proxy(
    execution: ExecutionBundle,
    gateway_base_url: &str,
    scope_id: &str,
) -> Result<ModelSessionProxy, String> {
    let gateway_url = Url::parse(gateway_base_url)
        .map_err(|error| format!("model_session_proxy_gateway_invalid:{error}"))?;
    if !matches!(gateway_url.scheme(), "http" | "https")
        || gateway_url.host_str().is_none()
        || !gateway_url.username().is_empty()
        || gateway_url.password().is_some()
        || gateway_url.query().is_some()
        || gateway_url.fragment().is_some()
    {
        return Err("model_session_proxy_gateway_invalid".to_owned());
    }
    let gateway_origin = origin_url(&gateway_url)?;
    let execution_for_thread = execution.clone();
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("model_session_proxy_bind_failed:{error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("model_session_proxy_bind_failed:{error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("model_session_proxy_bind_failed:{error}"))?
        .port();
    let local_token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let token_for_thread = local_token.clone();
    let application_tools = AdapterSlot::default();
    let tools_for_thread = application_tools.clone();
    let scope_id = scope_id.to_owned();
    let thread = std::thread::Builder::new()
        .name("model-session-proxy".to_owned())
        .spawn(move || {
            run_proxy_thread(
                listener,
                execution_for_thread,
                scope_id,
                gateway_origin,
                gateway_url,
                token_for_thread,
                tools_for_thread,
                ready_tx,
                shutdown_rx,
            );
        })
        .map_err(|error| format!("model_session_proxy_spawn_failed:{error}"))?;
    match ready_rx.recv_timeout(Duration::from_secs(25)) {
        Ok(Ok(binding)) => Ok(ModelSessionProxy {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            application_mcp_url: format!("http://127.0.0.1:{port}/application/mcp"),
            execution: Some(execution),
            binding,
            local_token,
            shutdown: Some(shutdown_tx),
            thread: Some(thread),
        }),
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(error) => {
            let _ = shutdown_tx.send(());
            let _ = thread.join();
            Err(format!("model_session_proxy_start_failed:{error}"))
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "代理线程接收各独立服务范围与生命周期通道"
)]
fn run_proxy_thread(
    listener: TcpListener,
    execution: ExecutionBundle,
    scope_id: String,
    gateway_origin: Url,
    upstream_base: Url,
    local_token: String,
    application_tools: AdapterSlot,
    ready_tx: mpsc::SyncSender<Result<ProxyBinding, String>>,
    shutdown_rx: mpsc::Receiver<()>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready_tx.send(Err(format!("model_session_proxy_runtime_failed:{error}")));
            return;
        }
    };
    runtime.block_on(async move {
        let result = async {
            execution.foreground.validate()?;
            let primary = Arc::clone(&execution.foreground);
            let token_provider: TokenProvider = Arc::new(move || {
                let primary = Arc::clone(&primary);
                Box::pin(async move {
                    primary.validate_for_request()?;
                    primary.credential()
                })
            });
            let client = crate::model_connections::connection_client_with_route(
                execution.foreground.network()?,
            )?;
            let listener = tokio::net::TcpListener::from_std(listener)
                .map_err(|error| format!("model_session_proxy_listener_failed:{error}"))?;
            Ok::<_, String>((
                listener,
                Arc::new(ProxyState {
                    #[cfg(test)]
                    routing_only_fixture: false,
                    start_admission: None,
                    scope_id,
                    active: true,
                    memory: None,
                    page_tool_names: None,
                    retired: Arc::new(AtomicBool::new(false)),
                    token_provider,
                    execution: Some(execution),
                    upstream_base: Some(upstream_base),
                    client,
                    gateway_origin,
                    local_token,
                    application_tools,
                }),
            ))
        }
        .await;
        let (listener, state) = match result {
            Ok(value) => value,
            Err(error) => {
                let _ = ready_tx.send(Err(error));
                return;
            }
        };
        let binding = Arc::new(Mutex::new(state));
        if ready_tx.send(Ok(Arc::clone(&binding))).is_err() {
            return;
        }
        serve(listener, binding, shutdown_rx).await;
    });
}

async fn serve(
    listener: tokio::net::TcpListener,
    state: ProxyBinding,
    shutdown_rx: mpsc::Receiver<()>,
) {
    loop {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }
        match tokio::time::timeout(Duration::from_millis(250), listener.accept()).await {
            Ok(Ok((stream, _))) => {
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let service = service_fn(move |request| {
                        let binding = Arc::clone(
                            &state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner),
                        );
                        forward_request(request, binding)
                    });
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
}

fn origin_url(url: &Url) -> Result<Url, String> {
    Url::parse(&url.origin().ascii_serialization())
        .map_err(|error| format!("model_session_proxy_origin_invalid:{error}"))
}

fn empty_body() -> ProxyBody {
    Full::new(Bytes::new())
        .map_err(|never: Infallible| -> ProxyError { match never {} })
        .boxed_unsync()
}

fn error_response(status: StatusCode, code: &'static str) -> Response<ProxyBody> {
    let body = serde_json::json!({ "error": { "code": code } }).to_string();
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(
            Full::new(Bytes::from(body))
                .map_err(|never: Infallible| -> ProxyError { match never {} })
                .boxed_unsync(),
        )
        .unwrap_or_else(|_| Response::new(empty_body()))
}

fn json_response(status: StatusCode, value: &Value) -> Response<ProxyBody> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(
            Full::new(Bytes::from(value.to_string()))
                .map_err(|never: Infallible| -> ProxyError { match never {} })
                .boxed_unsync(),
        )
        .unwrap_or_else(|_| Response::new(empty_body()))
}

fn mcp_result(id: Value, result: Value) -> Response<ProxyBody> {
    let mut body = json!({"jsonrpc":"2.0"});
    body["id"] = id;
    body["result"] = result;
    json_response(StatusCode::OK, &body)
}

fn mcp_error(id: Value, code: i64, message: &str) -> Response<ProxyBody> {
    let mut body = json!({"jsonrpc":"2.0","error":{"code":code,"message":message}});
    body["id"] = id;
    json_response(StatusCode::OK, &body)
}

fn mcp_tool_error(id: Value, message: &str) -> Response<ProxyBody> {
    mcp_result(
        id,
        json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true
        }),
    )
}

async fn handle_application_mcp(
    request: Request<Incoming>,
    state: &ProxyState,
) -> Response<ProxyBody> {
    if request.method() != hyper::Method::POST {
        return error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "APPLICATION_MCP_METHOD_INVALID",
        );
    }
    let body = match http_body_util::Limited::new(request.into_body(), 1024 * 1024)
        .collect()
        .await
    {
        Ok(body) => body.to_bytes(),
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "APPLICATION_MCP_BODY_INVALID"),
    };
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return error_response(StatusCode::BAD_REQUEST, "APPLICATION_MCP_BODY_INVALID");
    };
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    match message.get("method").and_then(Value::as_str) {
        Some("initialize") => mcp_result(
            id,
            json!({"protocolVersion": message.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or("2025-06-18"), "capabilities":{"tools":{}}, "serverInfo":{"name":"cilai-application","version":"1"}}),
        ),
        Some("notifications/initialized") => Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(empty_body())
            .unwrap_or_else(|_| Response::new(empty_body())),
        Some("ping") => mcp_result(id, json!({})),
        Some(method @ ("tools/list" | "tools/call")) => {
            // 异步等待不会占住代理执行器。
            let Ok(adapter) = state.application_tools.wait_ready().await else {
                return mcp_error(id, -32_603, "application_tool_preparation_failed");
            };
            let list = method == "tools/list";
            let result = tokio::task::spawn_blocking(move || {
                if list {
                    return adapter.list();
                }
                let name = message
                    .pointer("/params/name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "application_tool_name_invalid".to_owned())?;
                adapter.call(
                    name,
                    message
                        .pointer("/params/arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({})),
                )
            })
            .await;
            match result {
                Ok(Ok(value)) => mcp_result(id, value),
                Ok(Err(error)) => mcp_tool_error(id, &error),
                Err(_) => mcp_tool_error(id, "application_tool_worker_failed"),
            }
        }
        _ => mcp_error(id, -32_601, "application_mcp_method_not_found"),
    }
}

async fn forward_request(
    request: Request<Incoming>,
    state: Arc<ProxyState>,
) -> Result<Response<ProxyBody>, Infallible> {
    Ok(forward_request_inner(request, &state).await)
}

fn request_is_authorized(request: &Request<Incoming>, state: &ProxyState) -> bool {
    request
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        == Some(format!("Bearer {}", state.local_token).as_str())
}

// 仅输出固定用途标签，不输出可能含调用方数据的原始请求头。
fn request_purpose(headers: &hyper::HeaderMap) -> Result<&'static str, &'static str> {
    let metadata_memory = if let Some(header) = headers.get("x-codex-turn-metadata") {
        let metadata: Value = serde_json::from_slice(header.as_bytes())
            .map_err(|_| "MODEL_SESSION_PROXY_METADATA_INVALID")?;
        let metadata = metadata
            .as_object()
            .ok_or("MODEL_SESSION_PROXY_METADATA_INVALID")?;
        match metadata.get("request_kind") {
            Some(kind) => {
                kind.as_str()
                    .ok_or("MODEL_SESSION_PROXY_METADATA_INVALID")?
                    == "memory"
            }
            None => false,
        }
    } else {
        false
    };
    let subagent = headers.get("x-openai-subagent");
    Ok(
        if metadata_memory
            || headers
                .get("x-openai-memgen-request")
                .is_some_and(|value| value == "true")
            || subagent.is_some_and(|value| value == "memory_consolidation")
        {
            "memory"
        } else if subagent.is_some_and(|value| value == "review" || value == "guardian") {
            "review"
        } else if subagent.is_some() {
            "auxiliary"
        } else {
            "foreground"
        },
    )
}

#[expect(
    clippy::too_many_lines,
    reason = "代理按鉴权、路由、透传和流式计时顺序处理同一请求"
)]
async fn forward_request_inner(
    request: Request<Incoming>,
    state: &ProxyState,
) -> Response<ProxyBody> {
    if !request_is_authorized(&request, state) {
        return error_response(StatusCode::UNAUTHORIZED, "MODEL_SESSION_PROXY_UNAUTHORIZED");
    }
    if state.retired.load(Ordering::Acquire) {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "MODEL_SESSION_PROXY_RETIRED",
        );
    }
    let purpose = match request_purpose(request.headers()) {
        Ok(purpose) => purpose,
        Err(code) => return error_response(StatusCode::BAD_REQUEST, code),
    };
    let purpose = if purpose == "foreground" {
        match state
            .execution
            .as_ref()
            .map(|execution| &execution.foreground.lease.snapshot.purpose)
        {
            Some(contexthub_protocol::ModelPurpose::PageAi) => "page_ai",
            Some(contexthub_protocol::ModelPurpose::Automation) => "automation",
            _ => purpose,
        }
    } else {
        purpose
    };
    let memory_request = purpose == "memory";
    let path = request.uri().path();
    if (!state.active || memory_request)
        && !(memory_request
            && path == "/v1/responses"
            && request.method() == hyper::Method::POST
            && state.memory.is_some())
    {
        return error_response(StatusCode::SERVICE_UNAVAILABLE, "MODEL_SESSION_PROXY_IDLE");
    }
    let Ok(selected) = state
        .execution
        .as_ref()
        .map(|execution| execution.validate_request(purpose))
        .transpose()
    else {
        return error_response(StatusCode::UNAUTHORIZED, "MODEL_CONNECTION_CHANGED");
    };
    if path == "/application/mcp" {
        return handle_application_mcp(request, state).await;
    }
    let (origin, upstream_path) = if gateway_path_allowed(path) {
        (&state.gateway_origin, path)
    } else {
        return error_response(StatusCode::NOT_FOUND, "MODEL_SESSION_PROXY_PATH_INVALID");
    };
    let route = match upstream_path {
        "/v1/responses" => "responses",
        _ => "other",
    };
    let mut timing = crate::request_timing::RequestTiming::new(
        if memory_request {
            &state
                .memory
                .as_ref()
                .expect("authorized native memory")
                .scope_id
        } else {
            &state.scope_id
        },
        route,
        purpose,
    );
    timing.mark("received");
    let token = match match selected {
        Some(scope) => scope.credential(),
        None => (state.token_provider)().await,
    } {
        Ok(token) => token,
        Err(error) if error == "model_auth_required" => {
            return error_response(StatusCode::UNAUTHORIZED, "MODEL_AUTH_REQUIRED");
        }
        Err(error) if error == "model_execution_snapshot_changed" => {
            return error_response(StatusCode::UNAUTHORIZED, "MODEL_CONNECTION_CHANGED");
        }
        Err(_) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "MODEL_CONNECTION_CHECK_FAILED",
            );
        }
    };
    timing.mark("session_check");
    let Ok(selected_route) = selected
        .map(crate::model_connections::ExecutionScope::network)
        .transpose()
    else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "MODEL_NETWORK_CONFIG_INVALID",
        );
    };
    let client = match selected_route {
        Some(route) => match crate::model_connections::connection_client_with_route(route) {
            Ok(client) => client,
            Err(_) => {
                return error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "MODEL_NETWORK_CONFIG_INVALID",
                );
            }
        },
        None => state.client.clone(),
    };
    let selected_base = match selected {
        Some(scope) => match Url::parse(
            scope
                .lease
                .snapshot
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1"),
        ) {
            Ok(base) => Some(base),
            Err(_) => return error_response(StatusCode::BAD_REQUEST, "MODEL_ENDPOINT_INVALID"),
        },
        None => state.upstream_base.clone(),
    };
    let mut url = origin.clone();
    if let Some(base) = &selected_base {
        url = base.clone();
        url.set_path(&format!(
            "{}{}",
            base.path().trim_end_matches('/'),
            upstream_path.strip_prefix("/v1").unwrap_or(upstream_path)
        ));
    } else {
        url.set_path(upstream_path);
    }
    url.set_query(request.uri().query());
    let Ok(method) = reqwest::Method::from_bytes(request.method().as_str().as_bytes()) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "MODEL_SESSION_PROXY_METHOD_INVALID",
        );
    };
    let model_request = upstream_path == "/v1/responses";
    let assign_response_idempotency_key = model_request
        && request.method() == hyper::Method::POST
        && !request.headers().contains_key("idempotency-key");
    let mut upstream = client.request(method, url);
    if !token.is_empty() {
        upstream = upstream.bearer_auth(&token);
    }
    for (name, value) in request.headers() {
        if !hop_header(name.as_str())
            && !selected_route.is_some_and(|route| route.declares_header(name.as_str()))
        {
            upstream = upstream.header(name.as_str(), value.as_bytes());
        }
    }
    if let Some(route) = selected_route {
        upstream = match route.decorate(upstream, &token) {
            Ok(request) => request,
            Err(_) => {
                return error_response(
                    StatusCode::UNAUTHORIZED,
                    "MODEL_NETWORK_AUTHENTICATION_FAILED",
                );
            }
        };
    }
    if assign_response_idempotency_key {
        upstream = upstream.header("idempotency-key", uuid::Uuid::new_v4().to_string());
    }
    let mut body = match request.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => {
            return error_response(StatusCode::BAD_REQUEST, "MODEL_SESSION_PROXY_BODY_INVALID");
        }
    };
    if model_request
        && let Some(names) = &state.page_tool_names
        && let Err(code) = validate_page_tool_request(&body, names)
    {
        return error_response(StatusCode::FORBIDDEN, code);
    }
    if model_request && let Some(execution) = &state.execution {
        let Ok(scope) = execution.validate_request(purpose) else {
            return error_response(StatusCode::UNAUTHORIZED, "MODEL_CONNECTION_CHANGED");
        };
        body = match constrained_request(
            &body,
            &scope.lease.snapshot.model_id,
            selected_base.as_ref(),
        ) {
            Ok(body) => body,
            Err(code) => return error_response(StatusCode::BAD_REQUEST, code),
        };
    }
    if memory_request {
        let grant = state.memory.as_ref().expect("authorized native memory");
        let model = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| value["model"].as_str().map(str::to_owned));
        if model.as_deref() != Some(grant.model.as_str()) {
            return error_response(
                StatusCode::FORBIDDEN,
                "MODEL_SESSION_PROXY_MEMORY_SCOPE_INVALID",
            );
        }
    }
    if state.retired.load(Ordering::Acquire) {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "MODEL_SESSION_PROXY_RETIRED",
        );
    }
    timing.mark("request_ready");
    if model_request
        && let Some(admission) = &state.start_admission
        && admission().is_err()
    {
        return error_response(StatusCode::FORBIDDEN, "AUTOMATION_NOT_STARTED");
    }
    let Ok(mut request_lifecycle) = RequestScopeMonitor::start(Arc::new(state.clone()), purpose)
    else {
        return error_response(StatusCode::UNAUTHORIZED, "MODEL_CONNECTION_CHANGED");
    };
    let Ok(mut usage) = begin_usage(state, purpose, model_request) else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "MODEL_USAGE_RECORDING_UNAVAILABLE",
        );
    };
    let response = match tokio::select! {
        response=upstream.body(body).send()=>response,
        ()=request_lifecycle.revoked()=> {
            timing.mark("request_revoked");
            return error_response(StatusCode::UNAUTHORIZED,"MODEL_CONNECTION_CHANGED");
        }
    } {
        Ok(response) => response,
        Err(error) => {
            timing.mark("upstream_failed");
            let reason = crate::model_connections::network_failure_reason(
                &error,
                "MODEL_SESSION_PROXY_UPSTREAM_FAILED",
            );
            return error_response(
                StatusCode::BAD_GATEWAY,
                if reason == "model_proxy_authentication_rejected" {
                    "MODEL_PROXY_AUTHENTICATION_REJECTED"
                } else {
                    reason
                },
            );
        }
    };
    timing.mark("upstream_headers");
    if !response.status().is_success() {
        if let Some(usage) = usage.as_mut() {
            let _ = usage.finish_rejected();
        }
        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let diagnostic = service_error::read(response).await;
        return json_response(status, &diagnostic.response());
    }
    let Some(content_type) = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| forwarded_response_header("content-type", value, true, ""))
    else {
        return error_response(
            StatusCode::BAD_GATEWAY,
            "MODEL_SESSION_PROXY_RESPONSE_TYPE_INVALID",
        );
    };
    let normalize_sse = model_request && content_type == "text/event-stream";
    let mut builder = Response::builder().status(response.status().as_u16());
    for (name, value) in response.headers() {
        if let Some(value) = forwarded_response_header(
            name.as_str(),
            value,
            selected_route.is_some_and(crate::model_connections::ResolvedNetwork::has_credentials),
            &token,
        ) {
            builder = builder.header(name.as_str(), value);
        }
    }
    let stream = stream::unfold(
        Some((
            response,
            timing,
            normalize_sse.then(Vec::new),
            request_lifecycle,
            usage,
        )),
        |state| async move {
            let (mut response, mut timing, mut pending, mut lifecycle, mut usage) = state?;
            let chunk = tokio::select! {
                chunk=response.chunk()=>chunk,
                ()=lifecycle.revoked()=> {
                    timing.mark("stream_revoked");
                    return Some((Err(std::io::Error::other("MODEL_CONNECTION_CHANGED")),None));
                }
            };
            match chunk {
                Ok(Some(chunk)) => {
                    if let Some(usage) = usage.as_mut() {
                        usage.observe(&chunk);
                    }
                    timing.chunk(&chunk);
                    let chunk = if let Some(pending) = pending.as_mut() {
                        match normalize_sse_chunk(pending, &chunk, false) {
                            Ok(bytes) => bytes,
                            Err(error) => return Some((Err(error), None)),
                        }
                    } else {
                        chunk
                    };
                    Some((
                        Ok::<_, std::io::Error>(Frame::data(chunk)),
                        Some((response, timing, pending, lifecycle, usage)),
                    ))
                }
                Ok(None) => {
                    if let Some(usage) = usage.as_mut() {
                        let _ = usage.finish_stream();
                    }
                    timing.mark("stream_complete");
                    pending.filter(|bytes| !bytes.is_empty()).map(|mut bytes| {
                        (
                            normalize_sse_chunk(&mut bytes, &[], true).map(Frame::data),
                            None,
                        )
                    })
                }
                Err(error) => {
                    timing.mark("stream_failed");
                    Some((Err(std::io::Error::other(error)), None))
                }
            }
        },
    );
    let body = StreamBody::new(Box::pin(stream))
        .map_err(|error| -> ProxyError { Box::new(error) })
        .boxed_unsync();
    builder.body(body).unwrap_or_else(|_| {
        error_response(
            StatusCode::BAD_GATEWAY,
            "MODEL_SESSION_PROXY_RESPONSE_INVALID",
        )
    })
}

fn forwarded_response_header(
    name: &str,
    value: &reqwest::header::HeaderValue,
    protected_network: bool,
    token: &str,
) -> Option<reqwest::header::HeaderValue> {
    if name == "content-type" {
        let mime = value.to_str().ok()?.split(';').next()?.trim();
        return if mime.eq_ignore_ascii_case("text/event-stream") {
            Some(reqwest::header::HeaderValue::from_static(
                "text/event-stream",
            ))
        } else if mime.eq_ignore_ascii_case("application/json") {
            Some(reqwest::header::HeaderValue::from_static(
                "application/json",
            ))
        } else {
            None
        };
    }
    if protected_network
        || !matches!(
            name,
            "retry-after" | "request-id" | "x-request-id" | "openai-processing-ms"
        )
        || !token.is_empty()
            && value
                .as_bytes()
                .windows(token.len())
                .any(|part| part == token.as_bytes())
    {
        return None;
    }
    Some(value.clone())
}

fn begin_usage(
    state: &ProxyState,
    purpose: &'static str,
    model_request: bool,
) -> Result<Option<usage::RequestUsage>, String> {
    if !model_request {
        return Ok(None);
    }
    #[cfg(test)]
    if state.routing_only_fixture {
        return Ok(None);
    }
    usage::RequestUsage::begin(state, purpose)
}

/// 只归一化 Responses 错误帧；逐行缓冲支持网络分片和 UTF-8，普通输出字节保持原样。
fn normalize_sse_chunk(
    pending: &mut Vec<u8>,
    chunk: &[u8],
    finish: bool,
) -> std::io::Result<Bytes> {
    pending.extend_from_slice(chunk);
    if pending.len() > 2 * 1024 * 1024 {
        return Err(std::io::Error::other(
            "model_session_proxy_sse_frame_too_large",
        ));
    }
    let end = if finish {
        pending.len()
    } else {
        pending
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1)
    };
    let complete = pending.drain(..end).collect::<Vec<_>>();
    let mut output = Vec::with_capacity(complete.len());
    for line in complete.split_inclusive(|byte| *byte == b'\n') {
        let Some(data) = line.strip_prefix(b"data:") else {
            output.extend(line);
            continue;
        };
        let parsed = serde_json::from_slice::<Value>(data);
        if data.trim_ascii() == b"[DONE]" {
            output.extend(line);
            continue;
        }
        let event = match parsed {
            Ok(event) if event["type"] == "response.failed" => {
                json!({"type":"response.failed","response":{"error":sanitized_service_error(&event["response"]["error"])}})
            }
            Ok(event) if event["type"] == "error" || event.get("error").is_some() => {
                let error = sanitized_service_error(event.get("error").unwrap_or(&event));
                json!({"type":"error","code":error["code"],"message":error["message"],"error":error})
            }
            Ok(_) => {
                output.extend(line);
                continue;
            }
            Err(_) => {
                let error = sanitized_service_error(&Value::Null);
                json!({"type":"error","code":error["code"],"message":error["message"],"error":error})
            }
        };
        output.extend_from_slice(b"data: ");
        output.extend_from_slice(event.to_string().as_bytes());
        if line.ends_with(b"\r\n") {
            output.extend_from_slice(b"\r\n");
        } else if line.ends_with(b"\n") {
            output.push(b'\n');
        }
    }
    Ok(Bytes::from(output))
}

fn sanitized_service_error(error: &Value) -> Value {
    let code = error["code"]
        .as_str()
        .filter(|code| {
            matches!(
                *code,
                "invalid_api_key"
                    | "authentication_error"
                    | "rate_limit_exceeded"
                    | "insufficient_quota"
                    | "model_not_found"
                    | "invalid_request_error"
                    | "context_length_exceeded"
            )
        })
        .unwrap_or("model_upstream_error");
    json!({"type":"model_service_error","code":code,"message":format!("Model service failed ({code}).")})
}

fn validate_page_tool_request(body: &[u8], allowed: &[String]) -> Result<(), &'static str> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_| "MODEL_SESSION_PROXY_BODY_INVALID")?;
    let tools = value["tools"]
        .as_array()
        .ok_or("PERSONAL_PAGE_NATIVE_TOOLS_NOT_ISOLATED")?;
    let mut seen = std::collections::BTreeSet::new();
    if tools.len() != allowed.len()
        || tools.iter().any(|tool| {
            tool["type"] != "function"
                || tool["name"].as_str().is_none_or(|name| {
                    !allowed.iter().any(|allowed| allowed == name) || !seen.insert(name)
                })
                || tool.get("namespace").is_some()
        })
    {
        return Err("PERSONAL_PAGE_NATIVE_TOOLS_NOT_ISOLATED");
    }
    Ok(())
}

fn constrained_request(
    body: &[u8],
    model: &str,
    endpoint: Option<&Url>,
) -> Result<Bytes, &'static str> {
    let mut value: Value =
        serde_json::from_slice(body).map_err(|_| "MODEL_SESSION_PROXY_BODY_INVALID")?;
    let object = value
        .as_object_mut()
        .ok_or("MODEL_SESSION_PROXY_BODY_INVALID")?;
    if object.get("model").and_then(Value::as_str) != Some(model)
        || object.contains_key("models")
        || object.contains_key("route")
    {
        return Err("MODEL_SESSION_PROXY_MODEL_SCOPE_INVALID");
    }
    if endpoint.is_some_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("openrouter.ai")
            && url.port().is_none()
            && url.path().trim_end_matches('/') == "/api/v1"
    }) {
        let provider = object
            .entry("provider")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or("MODEL_SESSION_PROXY_PROVIDER_INVALID")?;
        if provider
            .get("allow_fallbacks")
            .is_some_and(|value| value != false)
        {
            return Err("MODEL_SESSION_PROXY_FALLBACK_FORBIDDEN");
        }
        provider.insert("allow_fallbacks".into(), json!(false));
    }
    normalize_empty_tools(object);
    serde_json::to_vec(&value)
        .map(Bytes::from)
        .map_err(|_| "MODEL_SESSION_PROXY_BODY_INVALID")
}

fn normalize_empty_tools(object: &mut serde_json::Map<String, Value>) {
    if object
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
        && object
            .get("tool_choice")
            .is_none_or(|choice| choice == "auto")
        && object
            .get("parallel_tool_calls")
            .is_none_or(Value::is_boolean)
    {
        for field in ["tools", "tool_choice", "parallel_tool_calls"] {
            object.remove(field);
        }
    }
}

#[cfg(test)]
mod empty_tool_controls_tests;

fn gateway_path_allowed(path: &str) -> bool {
    matches!(path, "/v1/models" | "/v1/responses") || path.starts_with("/v1/responses/")
}

fn hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization"
            | "host"
            | "connection"
            | "content-length"
            | "transfer-encoding"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "upgrade"
    )
}

#[cfg(test)]
pub(crate) fn local_tools_test_proxy(
    local_token: &str,
    application_tools: AdapterSlot,
) -> ModelSessionProxy {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = Arc::new(ProxyState {
        routing_only_fixture: true,
        start_admission: None,
        scope_id: "local-tools-protocol-fixture".into(),
        active: true,
        memory: None,
        page_tool_names: None,
        retired: Arc::new(AtomicBool::new(false)),
        execution: None,
        upstream_base: None,
        token_provider: Arc::new(|| Box::pin(async { Err("model_calls_not_authorized".into()) })),
        client: Client::new(),
        gateway_origin: Url::parse("http://127.0.0.1:1").unwrap(),
        local_token: local_token.to_owned(),
        application_tools,
    });
    let binding = Arc::new(Mutex::new(state));
    let server_binding = Arc::clone(&binding);
    let (shutdown, stopped) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                serve(
                    tokio::net::TcpListener::from_std(listener).unwrap(),
                    server_binding,
                    stopped,
                )
                .await;
            });
    });
    ModelSessionProxy {
        execution: None,
        base_url: format!("http://127.0.0.1:{port}/v1"),
        application_mcp_url: format!("http://127.0.0.1:{port}/application/mcp"),
        local_token: local_token.to_owned(),
        binding,
        shutdown: Some(shutdown),
        thread: Some(thread),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_authentication_values_do_not_corrupt_media_types_and_arbitrary_success_headers_are_not_forwarded()
     {
        use reqwest::header::HeaderValue;
        for short in ["u", "p", "a", "json", "application", "text"] {
            for (input, expected) in [
                ("Text/Event-Stream; charset=utf-8", "text/event-stream"),
                ("application/json; charset=utf-8", "application/json"),
            ] {
                assert_eq!(
                    super::forwarded_response_header(
                        "content-type",
                        &HeaderValue::from_str(input).unwrap(),
                        true,
                        short
                    )
                    .unwrap(),
                    expected
                );
            }
            assert!(
                super::forwarded_response_header(
                    "x-request-id",
                    &HeaderValue::from_str(short).unwrap(),
                    true,
                    ""
                )
                .is_none()
            );
            assert!(
                super::forwarded_response_header(
                    "retry-after",
                    &HeaderValue::from_str(short).unwrap(),
                    true,
                    ""
                )
                .is_none()
            );
        }
        for invalid in [
            "text/plain",
            "text/event-stream-secret",
            "application/json-secret",
            "application/octet-stream",
        ] {
            assert!(
                super::forwarded_response_header(
                    "content-type",
                    &HeaderValue::from_str(invalid).unwrap(),
                    true,
                    ""
                )
                .is_none()
            );
        }
    }
    #[test]
    fn personal_page_tool_gate_rejects_unknown_native_tools_before_forwarding() {
        let allowed = vec!["cilai_personal_tools".to_owned()];
        for tools in [
            serde_json::json!([]),
            serde_json::json!([{"type":"function","name":"exec_command"}]),
            serde_json::json!([{"type":"namespace","name":"cilai_personal_tools"}]),
            serde_json::json!([{"type":"function","name":"cilai_personal_tools","namespace":"functions"}]),
            serde_json::json!([{"type":"function","name":"cilai_personal_tools"},{"type":"function","name":"cilai_personal_tools"}]),
        ] {
            assert_eq!(
                super::validate_page_tool_request(
                    &serde_json::to_vec(&serde_json::json!({"tools":tools})).unwrap(),
                    &allowed
                ),
                Err("PERSONAL_PAGE_NATIVE_TOOLS_NOT_ISOLATED")
            );
        }
        assert!(
            super::validate_page_tool_request(
                br#"{"tools":[{"type":"function","name":"cilai_personal_tools"}]}"#,
                &allowed
            )
            .is_ok()
        );
    }

    use super::*;
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn timing_separates_memory_and_never_logs_custom_subagent_values() {
        let mut headers = hyper::HeaderMap::new();
        assert_eq!(request_purpose(&headers), Ok("foreground"));
        headers.insert("x-openai-subagent", "private-custom-value".parse().unwrap());
        assert_eq!(request_purpose(&headers), Ok("auxiliary"));
        headers.insert("x-openai-memgen-request", "true".parse().unwrap());
        assert_eq!(request_purpose(&headers), Ok("memory"));
        headers.remove("x-openai-memgen-request");
        headers.insert("x-openai-subagent", "memory_consolidation".parse().unwrap());
        assert_eq!(request_purpose(&headers), Ok("memory"));
    }

    #[test]
    fn native_memory_metadata_is_explicit_and_malformed_metadata_is_rejected() {
        let mut headers = hyper::HeaderMap::new();
        for (metadata, expected) in [
            (r#"{"request_kind":"memory"}"#, Ok("memory")),
            (r#"{"request_kind":"turn"}"#, Ok("foreground")),
            (r#"{"request_kind":"compaction"}"#, Ok("foreground")),
            (r#"{"request_kind":"Memory"}"#, Ok("foreground")),
            (
                r#"{"request_kind":true}"#,
                Err("MODEL_SESSION_PROXY_METADATA_INVALID"),
            ),
            ("[]", Err("MODEL_SESSION_PROXY_METADATA_INVALID")),
            ("{", Err("MODEL_SESSION_PROXY_METADATA_INVALID")),
        ] {
            headers.insert("x-codex-turn-metadata", metadata.parse().unwrap());
            assert_eq!(request_purpose(&headers), expected);
        }
        headers.insert("x-openai-memgen-request", "true".parse().unwrap());
        assert_eq!(
            request_purpose(&headers),
            Err("MODEL_SESSION_PROXY_METADATA_INVALID")
        );
    }

    fn start_native_memory_upstream(request_count: usize) -> (u16, JoinHandle<()>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            for _ in 0..request_count {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut headers = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    headers.push_str(&line);
                }
                assert!(headers.starts_with("POST /v1/responses "));
                assert!(
                    headers
                        .to_lowercase()
                        .contains("authorization: bearer current-session")
                );
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                )
                .unwrap();
            }
        });
        (port, worker)
    }

    struct NativeMemoryProxyFixture {
        proxy: ModelSessionProxy,
        shutdown: mpsc::Sender<()>,
        server: tokio::task::JoinHandle<()>,
        upstream: JoinHandle<()>,
    }

    async fn native_memory_proxy_fixture(upstream_requests: usize) -> NativeMemoryProxyFixture {
        let (upstream_port, upstream) = start_native_memory_upstream(upstream_requests);
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let binding = Arc::new(Mutex::new(Arc::new(ProxyState {
            routing_only_fixture: true,
            start_admission: None,
            scope_id: "first-turn".into(),
            active: true,
            memory: None,
            page_tool_names: None,
            retired: Arc::new(AtomicBool::new(false)),
            execution: None,
            upstream_base: None,
            token_provider: Arc::new(|| Box::pin(async { Ok("current-session".into()) })),
            client: Client::new(),
            gateway_origin: Url::parse(&format!("http://127.0.0.1:{upstream_port}")).unwrap(),
            local_token: "local-secret".into(),
            application_tools: AdapterSlot::default(),
        })));
        let proxy = ModelSessionProxy {
            execution: None,
            base_url: origin.clone(),
            local_token: "local-secret".into(),
            application_mcp_url: format!("{origin}/application/mcp"),
            binding: Arc::clone(&binding),
            shutdown: None,
            thread: None,
        };
        proxy.configure_native_memory(Some("review-model")).unwrap();
        proxy.suspend().unwrap();
        let (shutdown, stopped) = mpsc::channel();
        let server = tokio::spawn(serve(listener, binding, stopped));
        NativeMemoryProxyFixture {
            proxy,
            shutdown,
            server,
            upstream,
        }
    }

    fn native_memory_scope(proxy: &ModelSessionProxy) -> String {
        proxy
            .binding
            .lock()
            .unwrap()
            .memory
            .as_ref()
            .unwrap()
            .scope_id
            .clone()
    }

    #[tokio::test]
    async fn expired_automation_admission_never_reaches_upstream_and_is_cleared_for_a_new_run() {
        let fixture = native_memory_proxy_fixture(1).await;
        fixture.proxy.bind_run("queued-automation").unwrap();
        fixture
            .proxy
            .set_start_admission(Some(Arc::new(|| {
                Err("automation_missed_during_sleep".into())
            })))
            .unwrap();
        let request = || {
            reqwest::Client::new()
                .post(format!("{}/v1/responses", fixture.proxy.base_url))
                .bearer_auth(&fixture.proxy.local_token)
                .json(&json!({"model":"explicit-model", "input":"isolated admission test"}))
        };
        assert_proxy_status(request(), StatusCode::FORBIDDEN).await;
        fixture.proxy.bind_run("new-explicit-run").unwrap();
        assert_proxy_status(request(), StatusCode::OK).await;
        fixture.shutdown.send(()).unwrap();
        fixture.server.await.unwrap();
        fixture.upstream.join().unwrap();
    }

    async fn assert_proxy_status(request: reqwest::RequestBuilder, expected: StatusCode) {
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), expected);
        let _ = response.bytes().await.unwrap();
    }

    fn native_model_request(
        client: &Client,
        origin: &str,
        path: &str,
        model: &str,
    ) -> reqwest::RequestBuilder {
        client
            .post(format!("{origin}{path}"))
            .bearer_auth("local-secret")
            .json(&json!({"model":model}))
    }

    #[test]
    fn native_memory_keeps_its_scope_and_only_its_model_route_while_foreground_is_idle() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let NativeMemoryProxyFixture {
                proxy,
                shutdown,
                server,
                upstream,
            } = native_memory_proxy_fixture(3).await;
            let origin = &proxy.base_url;
            let memory_scope = native_memory_scope(&proxy);
            let client = Client::new();
            assert_proxy_status(
                client
                    .post(format!("{origin}/v1/responses"))
                    .header("x-openai-memgen-request", "true")
                    .json(&json!({"model":"review-model"})),
                StatusCode::UNAUTHORIZED,
            )
            .await;
            for (path, memory_header, model, expected) in [
                (
                    "/v1/responses",
                    "false",
                    "review-model",
                    StatusCode::SERVICE_UNAVAILABLE,
                ),
                (
                    "/application/mcp",
                    "true",
                    "review-model",
                    StatusCode::SERVICE_UNAVAILABLE,
                ),
                (
                    "/v1/models",
                    "true",
                    "review-model",
                    StatusCode::SERVICE_UNAVAILABLE,
                ),
                (
                    "/v1/responses",
                    "true",
                    "foreground-model",
                    StatusCode::FORBIDDEN,
                ),
                ("/v1/responses", "true", "review-model", StatusCode::OK),
            ] {
                assert_proxy_status(
                    native_model_request(&client, origin, path, model)
                        .header("x-openai-memgen-request", memory_header),
                    expected,
                )
                .await;
            }
            for (model, expected) in [
                ("foreground-model", StatusCode::FORBIDDEN),
                ("review-model", StatusCode::OK),
            ] {
                assert_proxy_status(
                    native_model_request(&client, origin, "/v1/responses", model)
                        .header("x-codex-turn-metadata", r#"{"request_kind":"memory"}"#)
                        .json(&json!({"model":model,"text":{"format":{"type":"json_schema","name":"stage1_output","strict":true,"schema":{"type":"object"}}}})),
                    expected,
                )
                .await;
            }
            assert_proxy_status(
                native_model_request(&client, origin, "/v1/responses", "review-model")
                    .header("x-codex-turn-metadata", "{"),
                StatusCode::BAD_REQUEST,
            )
            .await;
            proxy.bind_run("second-turn").unwrap();
            assert_eq!(proxy.binding.lock().unwrap().scope_id, "second-turn");
            assert_eq!(native_memory_scope(&proxy), memory_scope);
            proxy.suspend().unwrap();
            assert_proxy_status(
                native_model_request(&client, origin, "/v1/responses", "review-model")
                    .header("x-openai-subagent", "memory_consolidation"),
                StatusCode::OK,
            )
            .await;
            proxy.configure_native_memory(None).unwrap();
            assert_proxy_status(
                native_model_request(&client, origin, "/v1/responses", "review-model")
                    .header("x-openai-memgen-request", "true"),
                StatusCode::SERVICE_UNAVAILABLE,
            )
            .await;
            shutdown.send(()).unwrap();
            server.await.unwrap();
            upstream.join().unwrap();
        });
    }

    #[test]
    fn native_memory_rechecks_original_identity_after_foreground_rebinding() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let NativeMemoryProxyFixture {
                proxy,
                shutdown,
                server,
                upstream,
            } = native_memory_proxy_fixture(1).await;
            let scope = native_memory_scope(&proxy);
            let client = Client::new();
            let request = || {
                client
                    .post(format!("{}/v1/responses", proxy.base_url))
                    .bearer_auth("local-secret")
                    .header("x-openai-memgen-request", "true")
                    .json(&json!({"model":"review-model"}))
            };
            assert_proxy_status(request(), StatusCode::OK).await;
            proxy.bind_run("later-foreground-turn").unwrap();
            {
                let mut binding = proxy.binding.lock().unwrap();
                let mut next = (**binding).clone();
                next.token_provider =
                    Arc::new(|| Box::pin(async { Err("model_execution_snapshot_changed".into()) }));
                *binding = Arc::new(next);
            }
            proxy.suspend().unwrap();
            let response = request().send().await.unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                response.json::<Value>().await.unwrap()["error"]["code"],
                "MODEL_CONNECTION_CHANGED"
            );
            assert_eq!(native_memory_scope(&proxy), scope);
            shutdown.send(()).unwrap();
            server.await.unwrap();
            upstream.join().unwrap();
        });
    }

    #[test]
    fn retirement_revokes_captured_requests_across_explicit_runtime_restart() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let NativeMemoryProxyFixture {
                proxy,
                shutdown,
                server,
                upstream,
            } = native_memory_proxy_fixture(1).await;
            let entered = Arc::new(tokio::sync::Notify::new());
            let release = Arc::new(tokio::sync::Notify::new());
            let captured = {
                let mut binding = proxy.binding.lock().unwrap();
                let mut next = (**binding).clone();
                next.token_provider = Arc::new({
                    let entered = Arc::clone(&entered);
                    let release = Arc::clone(&release);
                    move || {
                        let entered = Arc::clone(&entered);
                        let release = Arc::clone(&release);
                        Box::pin(async move {
                            entered.notify_one();
                            release.notified().await;
                            Ok("current-session".into())
                        })
                    }
                });
                *binding = Arc::new(next);
                Arc::clone(&binding)
            };
            let old_scope = native_memory_scope(&proxy);
            let client = Client::new();
            let pending_request =
                native_model_request(&client, &proxy.base_url, "/v1/responses", "review-model")
                    .header("x-openai-subagent", "memory_consolidation");
            let pending = tokio::spawn(async move { pending_request.send().await.unwrap() });
            tokio::time::timeout(Duration::from_secs(2), entered.notified())
                .await
                .unwrap();
            proxy.retire();
            assert!(captured.retired.load(Ordering::Acquire));
            assert_eq!(
                proxy.bind_run("after-retirement").err().unwrap(),
                "model_session_proxy_retired"
            );
            assert_eq!(
                proxy
                    .configure_native_memory(Some("other-model"))
                    .unwrap_err(),
                "model_session_proxy_retired"
            );
            for memory_header in ["true", "false"] {
                assert_proxy_status(
                    native_model_request(&client, &proxy.base_url, "/v1/responses", "review-model")
                        .header("x-openai-memgen-request", memory_header),
                    StatusCode::SERVICE_UNAVAILABLE,
                )
                .await;
            }
            proxy.begin_runtime(Some("review-model")).unwrap();
            assert!(captured.retired.load(Ordering::Acquire));
            let current = Arc::clone(&proxy.binding.lock().unwrap());
            assert!(!current.retired.load(Ordering::Acquire));
            assert_ne!(native_memory_scope(&proxy), old_scope);
            release.notify_one();
            let response = pending.await.unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(
                response.json::<Value>().await.unwrap()["error"]["code"],
                "MODEL_SESSION_PROXY_RETIRED"
            );
            {
                let mut binding = proxy.binding.lock().unwrap();
                let mut next = (**binding).clone();
                next.token_provider = Arc::new(|| Box::pin(async { Ok("current-session".into()) }));
                *binding = Arc::new(next);
            }
            proxy.bind_run("new-runtime-turn").unwrap();
            assert_proxy_status(
                native_model_request(&client, &proxy.base_url, "/v1/responses", "review-model")
                    .header("x-openai-subagent", "memory_consolidation"),
                StatusCode::OK,
            )
            .await;
            shutdown.send(()).unwrap();
            server.await.unwrap();
            upstream.join().unwrap();
        });
    }

    fn start_upstream() -> (u16, JoinHandle<()>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let thread = std::thread::spawn(move || {
            for (path, token, idempotency_key) in [
                ("/v1/responses", "first", Some("generated")),
                ("/v1/responses", "second", Some("generated")),
                ("/v1/responses", "third", Some("generated")),
                ("/v1/responses", "fourth", Some("caller-key")),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut lines = Vec::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    lines.push(line);
                }
                assert!(lines[0].contains(path));
                assert!(lines.iter().any(|line| {
                    line.eq_ignore_ascii_case(&format!("authorization: Bearer {token}\r\n"))
                }));
                match idempotency_key {
                    Some("generated") => assert!(lines.iter().any(|line| {
                        line.to_ascii_lowercase().starts_with("idempotency-key: ")
                            && !line.eq_ignore_ascii_case("idempotency-key: caller-key\r\n")
                    })),
                    Some(value) => assert!(lines.iter().any(|line| {
                        line.eq_ignore_ascii_case(&format!("idempotency-key: {value}\r\n"))
                    })),
                    None => assert!(
                        !lines
                            .iter()
                            .any(|line| line.to_ascii_lowercase().starts_with("idempotency-key: "))
                    ),
                }
                let body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n";
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body.as_bytes()[..5]).unwrap();
                stream.flush().unwrap();
                stream.write_all(&body.as_bytes()[5..]).unwrap();
            }
        });
        (port, thread)
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "在真实 HTTP 连接上连续验证等待、轮次换绑、空闲停用及重新启用"
    )]
    fn pending_tools_wait_without_blocking_other_proxy_requests() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let url = format!("http://{}/application/mcp", listener.local_addr().unwrap());
            let slot = AdapterSlot::default();
            let state = Arc::new(ProxyState {
                routing_only_fixture: true,
                start_admission: None,
                scope_id: "pending-tools".into(),
                active: true,
                memory: None,
                page_tool_names: None,
                retired: Arc::new(AtomicBool::new(false)),
                execution: None,
                upstream_base: None,
                token_provider: Arc::new(|| Box::pin(async { Ok("unused".into()) })),
                client: Client::new(),
                gateway_origin: Url::parse("http://127.0.0.1:9").unwrap(),
                local_token: "local-only".into(),
                application_tools: slot.clone(),
            });
            let (shutdown, stopped) = mpsc::channel();
            let binding = Arc::new(Mutex::new(Arc::clone(&state)));
            let server = tokio::spawn(serve(listener, Arc::clone(&binding), stopped));
            let client = Client::new();
            let tools = client
                .post(&url)
                .bearer_auth("local-only")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
            let mut pending =
                tokio::spawn(
                    async move { tools.send().await.unwrap().json::<Value>().await.unwrap() },
                );
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut pending)
                    .await
                    .is_err()
            );
            let ping = tokio::time::timeout(
                Duration::from_secs(2),
                client
                    .post(&url)
                    .bearer_auth("local-only")
                    .json(&json!({"jsonrpc":"2.0","id":2,"method":"ping"}))
                    .send(),
            )
            .await
            .unwrap()
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
            assert_eq!(ping["result"], json!({}));
            let reusable = ModelSessionProxy {
                execution: None,
                base_url: String::new(),
                local_token: "local-only".into(),
                application_mcp_url: url.clone(),
                binding,
                shutdown: None,
                thread: None,
            };
            let next_slot = reusable.bind_run("next-run").unwrap();
            next_slot.complete(Err("next_snapshot_failed".into()));
            assert_eq!(state.scope_id, "pending-tools");
            assert_eq!(reusable.binding.lock().unwrap().scope_id, "next-run");
            // 同一 HTTP 连接的新请求使用新轮次；旧的等待请求仍由旧快照唤醒。
            let next = client
                .post(&url)
                .bearer_auth("local-only")
                .json(&json!({"jsonrpc":"2.0","id":4,"method":"tools/list"}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap();
            assert!(next.get("error").is_some());
            slot.complete(Ok(Arc::new(
                crate::application_tool_adapter::ApplicationToolAdapter::new_local(
                    None,
                    Arc::new(|| Ok(())),
                ),
            )));
            let ready = tokio::time::timeout(Duration::from_secs(2), pending)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(ready["result"]["tools"], json!([]));
            next_slot.complete(Err("snapshot_failed".into()));
            let failed = client
                .post(&url)
                .bearer_auth("local-only")
                .json(&json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap();
            assert_eq!(
                failed["error"]["message"],
                "application_tool_preparation_failed"
            );
            reusable.suspend().unwrap();
            let idle = client
                .post(&url)
                .bearer_auth("local-only")
                .json(&json!({"jsonrpc":"2.0","id":5,"method":"ping"}))
                .send()
                .await
                .unwrap();
            assert_eq!(idle.status(), StatusCode::SERVICE_UNAVAILABLE);
            reusable.bind_run("third-run").unwrap();
            let active = client
                .post(&url)
                .bearer_auth("local-only")
                .json(&json!({"jsonrpc":"2.0","id":6,"method":"ping"}))
                .send()
                .await
                .unwrap();
            assert_eq!(active.status(), StatusCode::OK);
            shutdown.send(()).unwrap();
            server.await.unwrap();
        });
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "同一场景内验证模型请求撤权与禁止企业路由"
    )]
    fn model_requests_refresh_session_tokens_and_enterprise_routes_never_reach_upstream() {
        let (upstream_port, upstream) = start_upstream();

        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let token_provider: TokenProvider = Arc::new({
            let calls = Arc::clone(&calls);
            move || {
                let token = match calls.fetch_add(1, Ordering::SeqCst) {
                    0 => Ok("first".to_owned()),
                    1 => Ok("second".to_owned()),
                    2 => Ok("third".to_owned()),
                    3 => Ok("fourth".to_owned()),
                    4 => Err("model_execution_snapshot_changed".to_owned()),
                    _ => Err("model_auth_required".to_owned()),
                };
                Box::pin(async move { token })
            }
        });
        let state = Arc::new(ProxyState {
            routing_only_fixture: true,
            start_admission: None,
            scope_id: "test-run".to_owned(),
            active: true,
            memory: None,
            page_tool_names: None,
            retired: Arc::new(AtomicBool::new(false)),
            token_provider,
            execution: None,
            upstream_base: None,
            client: Client::new(),
            gateway_origin: Url::parse(&format!("http://127.0.0.1:{upstream_port}")).unwrap(),
            local_token: "run-only".to_owned(),
            application_tools: AdapterSlot::default(),
        });
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let proxy = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                serve(
                    tokio::net::TcpListener::from_std(listener).unwrap(),
                    Arc::new(Mutex::new(state)),
                    shutdown_rx,
                )
                .await;
            });
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let origin = format!("http://127.0.0.1:{port}");
        let unauthorized = client
            .post(format!("{origin}/v1/responses"))
            .send()
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let invalid_path = client
            .get(format!("{origin}/v1/admin"))
            .bearer_auth("run-only")
            .send()
            .unwrap();
        assert_eq!(invalid_path.status(), StatusCode::NOT_FOUND);
        for path in [
            "/mcp",
            "/collaboration/mcp",
            "/image/mcp",
            "/v1/tool-snapshots",
            "/v1/applications",
            "/v1/search",
        ] {
            let response = client
                .post(format!("{origin}{path}"))
                .bearer_auth("run-only")
                .body("{}")
                .send()
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        for _ in 0..3 {
            let path = "/v1/responses";
            let response = client
                .post(format!("{origin}{path}"))
                .bearer_auth("run-only")
                .body("{}")
                .send()
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.text().unwrap(),
                "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n"
            );
        }
        let caller_key = client
            .post(format!("{origin}/v1/responses"))
            .bearer_auth("run-only")
            .header("idempotency-key", "caller-key")
            .body("{}")
            .send()
            .unwrap();
        assert_eq!(caller_key.status(), StatusCode::OK);
        assert_eq!(
            caller_key.text().unwrap(),
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n"
        );
        let expired = client
            .post(format!("{origin}/v1/responses"))
            .bearer_auth("run-only")
            .send()
            .unwrap();
        assert_eq!(expired.status(), StatusCode::UNAUTHORIZED);
        assert!(expired.text().unwrap().contains("MODEL_CONNECTION_CHANGED"));
        let login_required = client
            .post(format!("{origin}/v1/responses"))
            .bearer_auth("run-only")
            .send()
            .unwrap();
        assert_eq!(login_required.status(), StatusCode::UNAUTHORIZED);
        assert!(
            login_required
                .text()
                .unwrap()
                .contains("MODEL_AUTH_REQUIRED")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 6);
        shutdown_tx.send(()).unwrap();
        proxy.join().unwrap();
        upstream.join().unwrap();
    }
}

#[cfg(test)]
mod personal_policy_tests {
    use super::*;

    #[test]
    fn exact_model_and_responses_are_pinned_without_fallback_fields() {
        let endpoint = Url::parse("https://openrouter.ai/api/v1").unwrap();
        let request = br#"{"model":"nvidia/example:free","input":[],"stream":true}"#;
        let result: Value = serde_json::from_slice(
            &constrained_request(request, "nvidia/example:free", Some(&endpoint)).unwrap(),
        )
        .unwrap();
        assert_eq!(result["model"], "nvidia/example:free");
        assert_eq!(result["provider"]["allow_fallbacks"], false);
        assert!(!result.as_object().unwrap().contains_key("models"));
        assert!(constrained_request(request, "nvidia/example", Some(&endpoint)).is_err());
        for body in [
            br#"{"model":"m","models":["fallback"]}"#.as_slice(),
            br#"{"model":"m","route":"fallback"}"#,
            br#"{"model":"m","provider":{"allow_fallbacks":true}}"#,
        ] {
            assert!(constrained_request(body, "m", Some(&endpoint)).is_err());
        }
        let other = Url::parse("https://example.invalid/v1").unwrap();
        let result: Value = serde_json::from_slice(
            &constrained_request(request, "nvidia/example:free", Some(&other)).unwrap(),
        )
        .unwrap();
        assert!(result.get("provider").is_none());
    }

    #[test]
    fn failure_frames_strip_provider_messages_and_leave_successful_deltas_intact() {
        let text = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"正文\"}\n\n";
        let error = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"unknown-private-value\",\"message\":\"Authorization: Bearer fixture-secret\",\"headers\":{\"authorization\":\"fixture-secret\"}}}}\n\n";
        let mut pending = Vec::new();
        let mut result = Vec::new();
        for byte in text.bytes().chain(error.bytes()) {
            result.extend(normalize_sse_chunk(&mut pending, &[byte], false).unwrap());
        }
        assert!(result.starts_with(text.as_bytes()));
        let result = String::from_utf8(result).unwrap();
        assert!(!result.contains("fixture-secret"));
        assert!(!result.contains("unknown-private-value"));
        assert!(result.contains("model_upstream_error"));
    }

    #[test]
    fn malformed_and_flat_error_frames_never_echo_provider_payload() {
        for raw in [
            "data: {\"type\":\"error\",\"code\":\"private-fixture\",\"message\":\"private-fixture\"}\n\n",
            "data: {\"type\":\"response.failed\",\"message\":\"private-fixture\",\"response\":{\"output\":\"private-fixture\",\"error\":{\"message\":\"private-fixture\"}}}\n\n",
            "data: Authorization: private-fixture\n\n",
        ] {
            let normalized = normalize_sse_chunk(&mut Vec::new(), raw.as_bytes(), true).unwrap();
            let normalized = String::from_utf8(normalized.to_vec()).unwrap();
            assert!(!normalized.contains("private-fixture"));
            assert!(normalized.contains("model_upstream_error"));
        }
    }

    #[test]
    fn explicit_review_uses_its_own_model_purpose() {
        let mut headers = hyper::HeaderMap::new();
        headers.insert("x-openai-subagent", "review".parse().unwrap());
        assert_eq!(request_purpose(&headers), Ok("review"));
    }
}

#[cfg(test)]
mod personal_error_transport_tests {
    use super::*;
    use std::io::{BufRead as _, BufReader, Write as _};

    fn upstream() -> (u16, JoinHandle<()>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let thread = std::thread::spawn(move || {
            for (status, content_type, body) in [
                (
                    "500 Internal Server Error",
                    "application/json",
                    r#"{"error":{"message":"Bearer isolated-error-fixture"}}"#,
                ),
                (
                    "401 Unauthorized",
                    "text/plain",
                    "Authorization: isolated-error-fixture",
                ),
                (
                    "200 OK",
                    "text/event-stream",
                    "data: {\"type\":\"error\",\"error\":{\"message\":\"isolated-error-fixture\"}}\n\n",
                ),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                write!(stream,"HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nAuthorization: isolated-error-fixture\r\nX-Request-Id: isolated-error-fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
        });
        (port, thread)
    }

    #[test]
    fn provider_error_bodies_and_secret_headers_do_not_reach_the_runtime() {
        let (upstream_port, upstream) = upstream();
        crate::async_support::block_on(async {
            let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            let state = Arc::new(ProxyState {
                routing_only_fixture: true,
                start_admission: None,
                scope_id: "isolated-error-fixture".into(),
                active: true,
                memory: None,
                page_tool_names: None,
                retired: Arc::new(AtomicBool::new(false)),
                execution: None,
                upstream_base: None,
                token_provider: Arc::new(|| {
                    Box::pin(async { Ok("isolated-error-fixture".into()) })
                }),
                client: connection_client(
                    &contexthub_protocol::ModelNetworkConfig::default(),
                    &std::env::temp_dir().join("credentials"),
                )
                .unwrap(),
                gateway_origin: Url::parse(&format!("http://127.0.0.1:{upstream_port}")).unwrap(),
                local_token: "local-test".into(),
                application_tools: AdapterSlot::default(),
            });
            let (shutdown, stopped) = mpsc::channel();
            let server = tokio::spawn(serve(
                tokio::net::TcpListener::from_std(listener).unwrap(),
                Arc::new(Mutex::new(state)),
                stopped,
            ));
            let client = Client::builder()
                .no_proxy()
                .retry(reqwest::retry::never())
                .build()
                .unwrap();
            for status in [500, 401, 200] {
                let response = client
                    .post(format!("http://127.0.0.1:{port}/v1/responses"))
                    .bearer_auth("local-test")
                    .body("{}")
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status().as_u16(), status);
                assert!(response.headers().get("authorization").is_none());
                assert!(response.headers().get("x-request-id").is_none());
                assert!(
                    !response
                        .text()
                        .await
                        .unwrap()
                        .contains("isolated-error-fixture")
                );
            }
            shutdown.send(()).unwrap();
            server.await.unwrap();
        });
        upstream.join().unwrap();
    }
}
