//! 官方运行时只接触回环授权，不获得用户 MCP 的凭据或原服务地址。
use super::{ToolApproval, call_tool};
use crate::ai_runtime::AiRuntimeManager;
use contexthub_protocol::McpToolSnapshot;
use http_body_util::{BodyExt as _, Full};
use hyper::{
    Request, Response, StatusCode,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

pub type ScopeCheck = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;
/// 一轮主对话或自动化独占持有；页面 AI 不接收此对象。
pub struct McpBroker {
    pub url: String,
    pub bearer: String,
    retired: Arc<AtomicBool>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for McpBroker {
    fn drop(&mut self) {
        self.retired.store(true, Ordering::Release);
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Clone)]
struct State {
    manager: AiRuntimeManager,
    snapshot: McpToolSnapshot,
    approval: ToolApproval,
    scope: ScopeCheck,
    token: String,
    authority: String,
    retired: Arc<AtomicBool>,
}
impl State {
    fn validate(&self) -> Result<(), String> {
        if self.retired.load(Ordering::Acquire) {
            return Err("mcp_broker_retired".into());
        }
        (self.scope)()?;
        let current =
            self.manager
                .personal_mcp_snapshot(&contexthub_protocol::McpSnapshotParams {
                    purpose: self.snapshot.purpose.clone(),
                    connection_ids: self
                        .snapshot
                        .connections
                        .iter()
                        .map(|c| c.connection_id.clone())
                        .collect(),
                })?;
        if current != self.snapshot {
            return Err("mcp_tool_snapshot_changed".into());
        }
        Ok(())
    }
    fn dispatch(&self, request: &Value) -> Result<Option<Value>, String> {
        self.validate()?;
        let method = request["method"].as_str().ok_or("mcp_request_invalid")?;
        match method {
            "initialize" => Ok(Some(
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"cilai-personal-mcp-broker","version":"1"}}),
            )),
            "notifications/initialized" => Ok(None),
            "tools/list" => {
                let tools=self.snapshot.connections.iter().flat_map(|c|c.validation.iter().flat_map(|v|v.tools.iter()).filter(|tool|c.approved_tool_names.contains(&tool.name)).map(|tool|json!({"name":wire_name(&c.connection_id,&tool.name),"description":tool.description,"inputSchema":tool.input_schema}))).collect::<Vec<_>>();
                Ok(Some(json!({"tools":tools})))
            }
            "tools/call" => {
                let name = request["params"]["name"]
                    .as_str()
                    .ok_or("mcp_tool_not_authorized")?;
                for conn in &self.snapshot.connections {
                    for tool in conn
                        .validation
                        .iter()
                        .flat_map(|v| v.tools.iter())
                        .filter(|tool| conn.approved_tool_names.contains(&tool.name))
                    {
                        if wire_name(&conn.connection_id, &tool.name) == name {
                            let result = call_tool(
                                &self.manager,
                                &self.snapshot,
                                &conn.connection_id,
                                &tool.name,
                                &request["params"]["arguments"],
                                &self.approval,
                            );
                            let value = match result {
                                Ok(value) => value,
                                Err(error) => {
                                    self.retired.store(true, Ordering::Release);
                                    return Err(error);
                                }
                            };
                            if value["isError"] == true {
                                self.retired.store(true, Ordering::Release);
                                return Ok(Some(value));
                            }
                            self.validate()?;
                            return Ok(Some(value));
                        }
                    }
                }
                Err("mcp_tool_not_authorized".into())
            }
            _ => Err("mcp_method_not_supported".into()),
        }
    }
}
fn wire_name(connection: &str, tool: &str) -> String {
    use sha2::{Digest as _, Sha256};
    format!(
        "mcp_{connection}_{}",
        &format!("{:x}", Sha256::digest(tool.as_bytes()))[..20]
    )
}
/// # Errors
/// 无当前用途授权、作用域或回环监听失败时拒绝建立工具服务。
pub fn start(
    manager: AiRuntimeManager,
    snapshot: McpToolSnapshot,
    approval: ToolApproval,
    scope: ScopeCheck,
) -> Result<McpBroker, String> {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| "mcp_broker_bind_failed")?;
    listener
        .set_nonblocking(true)
        .map_err(|_| "mcp_broker_bind_failed")?;
    let authority = listener
        .local_addr()
        .map_err(|_| "mcp_broker_bind_failed")?
        .to_string();
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let retired = Arc::new(AtomicBool::new(false));
    let state = State {
        manager,
        snapshot,
        approval,
        scope,
        token: token.clone(),
        authority: authority.clone(),
        retired: retired.clone(),
    };
    state.validate()?;
    let (stop, shutdown) = tokio::sync::oneshot::channel();
    let (ready, waiting) = mpsc::sync_channel(1);
    let thread=std::thread::Builder::new().name("personal-mcp-broker".into()).spawn(move|| {
        let runtime=tokio::runtime::Builder::new_current_thread().enable_all().build();
        let Ok(runtime)=runtime else {let _=ready.send(false);return;};
        runtime.block_on(async move {
            let Ok(listener)=tokio::net::TcpListener::from_std(listener) else {let _=ready.send(false);return;};
            let _=ready.send(true);
            let mut shutdown=shutdown;
            let mut tasks=tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _=&mut shutdown=>break,
                    accepted=listener.accept()=>{
                        let Ok((stream,peer))=accepted else {break;};
                        if !peer.ip().is_loopback(){continue;}
                        let state=state.clone();
                        tasks.spawn(async move {
                            let service=service_fn(move|request|handle(state.clone(),request));
                            let _=hyper::server::conn::http1::Builder::new().timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(std::time::Duration::from_secs(5)).keep_alive(false).serve_connection(TokioIo::new(stream),service).await;
                        });
                    },
                    _=tasks.join_next(),if !tasks.is_empty()=>{},
                }
            }
            // Drain every in-flight request so writer leases outlive the local transport.
            while tasks.join_next().await.is_some() {}
        });
    }).map_err(|_|"mcp_broker_start_failed")?;
    if !waiting
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or(false)
    {
        let _ = stop.send(());
        let _ = thread.join();
        return Err("mcp_broker_start_failed".into());
    }
    Ok(McpBroker {
        url: format!("http://{authority}/mcp"),
        bearer: token,
        retired,
        stop: Some(stop),
        thread: Some(thread),
    })
}
fn response(status: StatusCode, value: &Value) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(
        serde_json::to_string(&value).unwrap_or_default(),
    )));
    *response.status_mut() = status;
    response.headers_mut().insert(
        "Content-Type",
        hyper::header::HeaderValue::from_static("application/json"),
    );
    response
}
async fn handle(
    state: State,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let authorized = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| value.strip_prefix("Bearer ") == Some(&state.token));
    let host =
        request.headers().get("host").and_then(|v| v.to_str().ok()) == Some(&state.authority);
    if !authorized || !host || request.headers().contains_key("origin") {
        return Ok(response(
            StatusCode::FORBIDDEN,
            &json!({"error":"mcp_broker_access_denied"}),
        ));
    }
    if request.method() != hyper::Method::POST
        || request.uri().path() != "/mcp"
        || request.uri().query().is_some()
    {
        return Ok(response(
            StatusCode::METHOD_NOT_ALLOWED,
            &json!({"error":"mcp_broker_method_not_allowed"}),
        ));
    }
    let mut body = request.into_body();
    let mut bytes = Vec::new();
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Ok(data) = frame.into_data() {
                    if bytes.len() + data.len() > 1024 * 1024 {
                        return Ok(response(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            &json!({"error":"mcp_request_too_large"}),
                        ));
                    }
                    bytes.extend_from_slice(&data);
                }
            }
            Ok(None) => break,
            _ => {
                return Ok(response(
                    StatusCode::BAD_REQUEST,
                    &json!({"error":"mcp_request_invalid"}),
                ));
            }
        }
    }
    let Ok(request) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(response(
            StatusCode::BAD_REQUEST,
            &json!({"error":"mcp_request_invalid"}),
        ));
    };
    if request["jsonrpc"] != "2.0" || request["method"].as_str().is_none() {
        return Ok(response(
            StatusCode::BAD_REQUEST,
            &json!({"error":"mcp_request_invalid"}),
        ));
    }
    let id = request.get("id").cloned();
    if id.is_none() && request["method"] != "notifications/initialized" {
        return Ok(response(
            StatusCode::BAD_REQUEST,
            &json!({"error":"mcp_request_id_required"}),
        ));
    }
    let result = tokio::task::spawn_blocking(move || state.dispatch(&request)).await;
    Ok(match result {
        Ok(Ok(Some(result))) => response(
            StatusCode::OK,
            &json!({"jsonrpc":"2.0","id":id,"result":result}),
        ),
        Ok(Ok(None)) => {
            let mut response = Response::new(Full::new(Bytes::new()));
            *response.status_mut() = StatusCode::ACCEPTED;
            response
        }
        Ok(Err(reason)) => response(
            StatusCode::OK,
            &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":reason}}),
        ),
        Err(_) => response(
            StatusCode::OK,
            &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"mcp_local_execution_unconfirmed"}}),
        ),
    })
}
