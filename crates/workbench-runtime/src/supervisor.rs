use contexthub_local_transport::{Endpoint, EndpointIdentity, LocalStream};
use contexthub_platform_adapter::executable_search::{
    BundledExecutable, bundled_executable_path, validate_executable_file,
};
use contexthub_platform_adapter::instance::{LifecycleMode, read_descriptor, validate_descriptor};
use contexthub_platform_adapter::process as platform_process;
use contexthub_protocol::{JsonlDecoder, MAX_MESSAGE_BYTES, PROTOCOL_VERSION};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const HOME_MIGRATION_START_TIMEOUT: Duration = Duration::from_mins(10);
const APPLICATION_CATALOG_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const APPLICATION_CATALOG_LIST_TIMEOUT: Duration = Duration::from_secs(20);
const UNIFIED_SEARCH_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_REPLAY_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const AI_HISTORY_REQUEST_TIMEOUT: Duration = Duration::from_mins(1);
const WORKSPACE_SCAN_TIMEOUT: Duration = Duration::from_mins(10);
const CLOUD_WORKSPACE_TRANSFER_TIMEOUT: Duration = Duration::from_hours(2);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);
const STDERR_MAX_LINES: usize = 80;
const STDERR_MAX_CHARS_PER_LINE: usize = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppServerPhase {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppServerConnectionMode {
    Owned,
    Attached,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppServerStatus {
    pub phase: AppServerPhase,
    pub pid: Option<u32>,
    pub server_instance_id: Option<String>,
    pub protocol_version: Option<String>,
    pub last_exit_code: Option<i32>,
    pub error_reason: Option<String>,
    pub stderr_tail: Vec<String>,
    pub unsolicited_message_count: u32,
    pub local_ipc_available: bool,
    pub connection_mode: Option<AppServerConnectionMode>,
    pub server_lifecycle_mode: Option<String>,
}

impl Default for AppServerStatus {
    fn default() -> Self {
        Self {
            phase: AppServerPhase::Stopped,
            pid: None,
            server_instance_id: None,
            protocol_version: None,
            last_exit_code: None,
            error_reason: None,
            stderr_tail: Vec::new(),
            unsolicited_message_count: 0,
            local_ipc_available: false,
            connection_mode: None,
            server_lifecycle_mode: None,
        }
    }
}

type PendingResult = Result<Value, String>;

struct Inner {
    #[cfg(test)]
    observed_request_methods: Mutex<Vec<String>>,
    #[cfg(test)]
    lost_successful_responses: Mutex<HashMap<String, usize>>,
    status: Mutex<AppServerStatus>,
    exit_changed: Condvar,
    writer: Mutex<Option<ConnectionWriter>>,
    pending: Mutex<HashMap<String, SyncSender<PendingResult>>>,
    next_request_id: AtomicU32,
    active_search: Mutex<Option<(u64, LocalStream)>>,
    next_search_id: AtomicU64,
    latest_search_id: AtomicU64,
    binary_path: Mutex<Option<PathBuf>>,
    environment: Mutex<Option<String>>,
    data_dir: Mutex<Option<PathBuf>>,
    runtime_dir: Mutex<Option<PathBuf>>,
    connection_generation: AtomicU64,
    lifecycle_events: Mutex<()>,
    service_ownership: Mutex<Option<platform_process::RetainableServiceProcess>>,
}

enum ConnectionWriter {
    Owned(ChildStdin),
    Attached(LocalStream),
}

struct ExistingConnection {
    stream: LocalStream,
    pid: u32,
    lifecycle_mode: String,
}

enum ShutdownDisposition {
    Exit,
    Continued,
}

impl ConnectionWriter {
    fn disconnect(&mut self) {
        if let Self::Attached(stream) = self {
            let _ = stream.disconnect();
        }
    }
}

impl Write for ConnectionWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Owned(writer) => writer.write(buffer),
            Self::Attached(writer) => writer.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Owned(writer) => writer.flush(),
            Self::Attached(writer) => writer.flush(),
        }
    }
}

#[derive(Clone)]
pub struct AppServerSupervisor {
    inner: Arc<Inner>,
}

impl AppServerSupervisor {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                #[cfg(test)]
                observed_request_methods: Mutex::new(Vec::new()),
                #[cfg(test)]
                lost_successful_responses: Mutex::new(HashMap::new()),
                status: Mutex::new(AppServerStatus::default()),
                exit_changed: Condvar::new(),
                writer: Mutex::new(None),
                pending: Mutex::new(HashMap::new()),
                next_request_id: AtomicU32::new(1),
                active_search: Mutex::new(None),
                next_search_id: AtomicU64::new(1),
                latest_search_id: AtomicU64::new(0),
                binary_path: Mutex::new(None),
                environment: Mutex::new(None),
                data_dir: Mutex::new(None),
                runtime_dir: Mutex::new(None),
                connection_generation: AtomicU64::new(0),
                lifecycle_events: Mutex::new(()),
                service_ownership: Mutex::new(None),
            }),
        }
    }

    #[must_use]
    pub fn status(&self) -> AppServerStatus {
        lock(&self.inner.status).clone()
    }

    #[cfg(test)]
    pub(crate) fn observed_request_methods(&self) -> Vec<String> {
        lock(&self.inner.observed_request_methods).clone()
    }

    #[cfg(test)]
    pub(crate) fn lose_next_successful_responses(&self, method: &str, count: usize) {
        lock(&self.inner.lost_successful_responses).insert(method.to_owned(), count);
    }

    /// 启动并完成 initialize/initialized 握手。
    ///
    /// # Errors
    ///
    /// 当前状态不允许启动、二进制无法执行、协议输出非法、握手超时或版本不匹配时失败。
    #[allow(clippy::too_many_lines)]
    pub fn start(
        &self,
        binary_path: &PathBuf,
        environment: &str,
        data_dir: &PathBuf,
        runtime_dir: &PathBuf,
    ) -> Result<(), String> {
        {
            let _events = lock(&self.inner.lifecycle_events);
            let mut status = lock(&self.inner.status);
            let can_start = status.phase == AppServerPhase::Stopped
                || (status.phase == AppServerPhase::Failed && status.pid.is_none());
            if !can_start {
                return Err(format!(
                    "cannot start App Server while phase is {:?}",
                    status.phase
                ));
            }
            *status = AppServerStatus {
                phase: AppServerPhase::Starting,
                ..AppServerStatus::default()
            };
            self.inner
                .connection_generation
                .fetch_add(1, Ordering::AcqRel);
        }
        *lock(&self.inner.binary_path) = Some(binary_path.clone());
        *lock(&self.inner.environment) = Some(environment.to_owned());
        *lock(&self.inner.data_dir) = Some(data_dir.clone());
        *lock(&self.inner.runtime_dir) = Some(runtime_dir.clone());

        let attached = Self::open_existing_connection(data_dir, environment)
            .map_err(|error| self.fail_start("instance_discovery_failed", &error))?;
        let use_owned = if let Some(existing) = attached {
            let reader = existing
                .stream
                .try_clone()
                .map_err(|error| self.fail_start("ipc_clone_failed", &error.to_string()))?;
            *lock(&self.inner.writer) = Some(ConnectionWriter::Attached(existing.stream));
            {
                let mut status = lock(&self.inner.status);
                status.pid = Some(existing.pid);
                status.connection_mode = Some(AppServerConnectionMode::Attached);
                status.server_lifecycle_mode = Some(existing.lifecycle_mode);
            }
            self.spawn_stdout_reader(reader);
            false
        } else {
            true
        };

        if use_owned {
            validate_executable_file(binary_path).map_err(|error| {
                self.fail_start(
                    "process_spawn_failed",
                    &format!("invalid App Server: {error}"),
                )
            })?;
            let mut command = Command::new(binary_path);
            command
                .args([
                    "serve",
                    "--transport",
                    "stdio",
                    "--enable-local-ipc",
                    "--environment",
                    environment,
                    "--data-dir",
                ])
                .arg(data_dir)
                .args(["--runtime-dir"])
                .arg(runtime_dir)
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            platform_process::retain_required_system_environment(&mut command).map_err(
                |error| self.fail_start("process_environment_failed", &error.to_string()),
            )?;
            platform_process::prepare_retainable_service_command(&mut command);
            let mut child = command
                .spawn()
                .map_err(|error| self.fail_start("process_spawn_failed", &error.to_string()))?;
            let ownership =
                platform_process::RetainableServiceProcess::adopt(&mut child).map_err(|error| {
                    self.fail_start("process_supervision_failed", &error.to_string())
                })?;
            *lock(&self.inner.service_ownership) = Some(ownership);
            let pid = child.id();
            let stdin = child.stdin.take().ok_or_else(|| {
                self.fail_start("stdin_unavailable", "child stdin is unavailable")
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                self.fail_start("stdout_unavailable", "child stdout is unavailable")
            })?;
            let stderr = child.stderr.take().ok_or_else(|| {
                self.fail_start("stderr_unavailable", "child stderr is unavailable")
            })?;
            *lock(&self.inner.writer) = Some(ConnectionWriter::Owned(stdin));
            {
                let mut status = lock(&self.inner.status);
                status.pid = Some(pid);
                status.connection_mode = Some(AppServerConnectionMode::Owned);
                status.server_lifecycle_mode = Some("owned".to_owned());
            }

            self.spawn_stdout_reader(stdout);
            self.spawn_stderr_reader(stderr);
            self.spawn_exit_monitor(child);
        }

        let (instance_id, negotiated) = self.negotiate_initialize()?;
        self.notify("initialized", &json!({}))
            .map_err(|error| self.fail_start("initialized_notification_failed", &error))?;
        let mut status = lock(&self.inner.status);
        if status.phase != AppServerPhase::Starting {
            return Err(status
                .error_reason
                .clone()
                .unwrap_or_else(|| "App Server exited during initialize".to_owned()));
        }
        status.phase = AppServerPhase::Running;
        status.server_instance_id = Some(instance_id);
        status.protocol_version = Some(negotiated);
        status.local_ipc_available = true;
        Ok(())
    }

    fn open_existing_connection(
        data_dir: &Path,
        environment: &str,
    ) -> Result<Option<ExistingConnection>, String> {
        let record_path = data_dir.join("run/instance.json");
        let Some(descriptor) = read_descriptor(&record_path).map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let runtime_dir = data_dir.join("run");
        let endpoint = Endpoint::resolve(&runtime_dir).map_err(|error| error.to_string())?;
        match validate_descriptor(&descriptor, data_dir, environment, &endpoint) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        }
        let identity = EndpointIdentity::parse(&descriptor.ipc_endpoint)
            .map_err(|error| format!("invalid App Server IPC endpoint: {error}"))?;
        let deadline = Instant::now() + startup_timeout(data_dir);
        loop {
            match LocalStream::connect_identity(&identity) {
                Ok(stream) => {
                    let lifecycle_mode = match descriptor.lifecycle_mode {
                        LifecycleMode::Owned => "owned",
                        LifecycleMode::Transient => "transient",
                    };
                    return Ok(Some(ExistingConnection {
                        stream,
                        pid: descriptor.pid,
                        lifecycle_mode: lifecycle_mode.to_owned(),
                    }));
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(format!("existing IPC endpoint is unavailable: {error}")),
            }
        }
    }

    fn negotiate_initialize(&self) -> Result<(String, String), String> {
        let params = json!({
            "client": {
                "name": "contexthub-desktop",
                "version": env!("CARGO_PKG_VERSION"),
                "platform": std::env::consts::OS,
                "arch": std::env::consts::ARCH
            },
            "protocol": { "minVersion": PROTOCOL_VERSION, "maxVersion": PROTOCOL_VERSION },
            "capabilities": {
                "interactive": true,
                "notifications": true,
                "osNotifications": false,
                "eventReplay": true,
                "resourceTickets": true,
                "richDiff": true,
                "deepLinks": false
            },
            "resume": {}
        });
        let timeout = lock(&self.inner.data_dir)
            .as_deref()
            .map_or(REQUEST_TIMEOUT, startup_timeout);
        let initialize = self
            .request_with_timeout("initialize", &params, timeout)
            .map_err(|error| self.fail_start("initialize_failed", &error))?;
        let result = response_result(&initialize)
            .map_err(|error| self.fail_start("initialize_failed", &error))?;
        let negotiated = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                self.fail_start("initialize_result_invalid", "protocolVersion is missing")
            })?;
        if negotiated != PROTOCOL_VERSION {
            return Err(self.fail_start("protocol_version_mismatch", negotiated));
        }
        let instance_id = result
            .pointer("/server/instanceId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                self.fail_start("initialize_result_invalid", "server.instanceId is missing")
            })?
            .to_owned();
        let local_ipc_available = result
            .pointer("/capabilities/localIpc")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                self.fail_start(
                    "initialize_result_invalid",
                    "capabilities.localIpc is missing",
                )
            })?;
        if !local_ipc_available {
            return Err(self.fail_start("local_ipc_unavailable", "server did not enable local IPC"));
        }
        let requested = lock(&self.inner.data_dir).clone();
        if let Some(data) = requested.as_deref().filter(|data| {
            data.parent().is_some_and(|p| {
                p.file_name().is_some_and(|n| n == "profiles")
                    && p.parent().is_some_and(|p| {
                        p.file_name().is_some_and(|n| n == "business")
                            && p.parent().is_some_and(|p| {
                                p.file_name()
                                    .is_some_and(|n| n == "cn.cilai.personal.workbench")
                            })
                    })
            })
        }) {
            let compatible = result
                .pointer("/capabilities/personalDataVersion")
                .and_then(Value::as_u64)
                == Some(u64::from(contexthub_protocol::PERSONAL_DATA_VERSION))
                && result
                    .pointer("/capabilities/modelConnectionProtocol")
                    .and_then(Value::as_u64)
                    == Some(u64::from(contexthub_protocol::MODEL_CONNECTION_PROTOCOL))
                && result
                    .pointer("/capabilities/personalProfileId")
                    .and_then(Value::as_str)
                    == data.file_name().and_then(|v| v.to_str());
            if !compatible {
                return Err(self.fail_start("personal_profile_upgrade_blocked_existing_server","the running App Server does not own the required personal data/model protocol; close the old personal instance before retrying"));
            }
        }
        Ok((instance_id, negotiated.to_owned()))
    }

    /// 显式重启当前固定路径的 App Server。
    ///
    /// # Errors
    ///
    /// 路径尚未确定、安全退出失败或重新启动失败时返回错误。
    pub fn restart(&self) -> Result<(), String> {
        let current = self.status();
        if current.phase == AppServerPhase::Running
            && current.connection_mode == Some(AppServerConnectionMode::Attached)
        {
            return Err(
                "cannot restart an attached App Server without lifecycle ownership".to_owned(),
            );
        }
        let phase = current.phase;
        if matches!(phase, AppServerPhase::Running) {
            self.shutdown_exclusive()?;
        } else if matches!(phase, AppServerPhase::Starting | AppServerPhase::Stopping) {
            return Err("App Server lifecycle transition is still in progress".to_owned());
        }
        let binary = lock(&self.inner.binary_path)
            .clone()
            .ok_or("App Server binary path is unavailable")?;
        let runtime_dir = lock(&self.inner.runtime_dir)
            .clone()
            .ok_or("App Server runtime directory is unavailable")?;
        let environment = lock(&self.inner.environment)
            .clone()
            .ok_or("App Server environment is unavailable")?;
        let data_dir = lock(&self.inner.data_dir)
            .clone()
            .ok_or("App Server data directory is unavailable")?;
        self.start(&binary, &environment, &data_dir, &runtime_dir)
    }

    /// Owned 连接两阶段安全退出 App Server；Attached 连接只断开桌面客户端。
    ///
    /// # Errors
    ///
    /// Owned 关闭被阻止、响应非法、通信失败或进程未在期限内结束时返回错误。
    pub fn shutdown(&self) -> Result<(), String> {
        self.shutdown_inner(true)
    }

    /// 资料恢复必须确认本服务退出；存在其他客户端时不转为共享续管。
    /// # Errors
    /// 当前连接没有生命周期归属、仍有其他客户端或停止结果未确认时失败。
    pub fn shutdown_exclusive(&self) -> Result<(), String> {
        self.shutdown_inner(false)
    }

    fn shutdown_inner(&self, allow_shared: bool) -> Result<(), String> {
        let phase = self.status().phase;
        if phase == AppServerPhase::Stopped {
            return Ok(());
        }
        if phase == AppServerPhase::Failed {
            return self.wait_for_failed_process_exit(EXIT_TIMEOUT);
        }
        if phase != AppServerPhase::Running {
            return Err(format!(
                "cannot shutdown App Server while phase is {phase:?}"
            ));
        }
        self.cancel_unified_search();
        if self.status().connection_mode == Some(AppServerConnectionMode::Attached) {
            if !allow_shared {
                return Err("app_server_lifecycle_ownership_required".into());
            }
            lock(&self.inner.status).phase = AppServerPhase::Stopping;
            disconnect_writer(&self.inner);
            fail_pending(&self.inner, "desktop detached from App Server");
            let mut status = lock(&self.inner.status);
            status.phase = AppServerPhase::Stopped;
            status.pid = None;
            status.connection_mode = None;
            self.inner.exit_changed.notify_all();
            return Ok(());
        }
        lock(&self.inner.status).phase = AppServerPhase::Stopping;
        let outcome = self.request_shutdown();
        if outcome.is_err() {
            let mut status = lock(&self.inner.status);
            if status.phase == AppServerPhase::Stopping {
                status.phase = AppServerPhase::Running;
            }
        }
        match outcome? {
            ShutdownDisposition::Exit => {
                disconnect_writer(&self.inner);
                self.wait_for_exit(EXIT_TIMEOUT)
            }
            ShutdownDisposition::Continued if allow_shared => self.detach_continued_service(),
            ShutdownDisposition::Continued => {
                lock(&self.inner.status).phase = AppServerPhase::Running;
                Err("app_server_other_clients_active".into())
            }
        }
    }

    fn request_shutdown(&self) -> Result<ShutdownDisposition, String> {
        let prepared = self.request("shutdown/prepare", &json!({}))?;
        let preparation = response_result(&prepared)?;
        if self.continuation_confirmed(preparation) {
            return Ok(ShutdownDisposition::Continued);
        }
        if preparation.get("ready").and_then(Value::as_bool) != Some(true) {
            return Err("App Server reported blocking shutdown reasons".to_owned());
        }
        let executed = self.request("shutdown/execute", &json!({}))?;
        let execution = response_result(&executed)?;
        if self.continuation_confirmed(execution) {
            return Ok(ShutdownDisposition::Continued);
        }
        if execution.get("accepted").and_then(Value::as_bool) != Some(true) {
            return Err("App Server did not accept shutdown execution".to_owned());
        }
        Ok(ShutdownDisposition::Exit)
    }

    fn continuation_confirmed(&self, result: &Value) -> bool {
        result["continuing"] == true
            && result["reasonCode"] == "app_server_shared_clients_active"
            && result["ipcClientCount"]
                .as_u64()
                .is_some_and(|count| count > 0)
            && self
                .status()
                .server_instance_id
                .as_deref()
                .is_some_and(|id| !id.is_empty() && result["serverInstanceId"].as_str() == Some(id))
    }

    fn detach_continued_service(&self) -> Result<(), String> {
        let _events = lock(&self.inner.lifecycle_events);
        let mut ownership = lock(&self.inner.service_ownership);
        let Some(service) = ownership.as_ref() else {
            lock(&self.inner.status).phase = AppServerPhase::Running;
            return Err("app_server_service_ownership_unconfirmed".into());
        };
        if service.continue_after_owner_exit().is_err() {
            lock(&self.inner.status).phase = AppServerPhase::Running;
            return Err("app_server_service_detach_unconfirmed".into());
        }
        self.inner
            .connection_generation
            .fetch_add(1, Ordering::AcqRel);
        disconnect_writer(&self.inner);
        fail_pending(&self.inner, "desktop detached from continued App Server");
        ownership.take();
        let mut status = lock(&self.inner.status);
        status.phase = AppServerPhase::Stopped;
        status.pid = None;
        status.connection_mode = None;
        status.server_lifecycle_mode = Some("shared_continued".into());
        self.inner.exit_changed.notify_all();
        Ok(())
    }

    fn request(&self, method: &str, params: &Value) -> Result<Value, String> {
        self.request_with_timeout(method, params, request_timeout(method))
    }

    fn request_with_timeout(
        &self,
        method: &str,
        params: &Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        #[cfg(test)]
        lock(&self.inner.observed_request_methods).push(method.to_owned());
        let sequence = self.inner.next_request_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("desktop_{sequence}");
        let (sender, receiver) = mpsc::sync_channel(1);
        lock(&self.inner.pending).insert(id.clone(), sender);
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(error) = self.write_message(&message) {
            lock(&self.inner.pending).remove(&id);
            return Err(error);
        }
        match receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                lock(&self.inner.pending).remove(&id);
                Err(format!("App Server request timed out: {method}"))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(format!("App Server response channel closed: {method}"))
            }
        }
    }

    /// # Errors
    /// 应用服务未就绪、请求超时或服务端返回错误时返回错误。
    pub fn request_result(&self, method: &str, params: &Value) -> Result<Value, String> {
        if self.status().phase != AppServerPhase::Running {
            return Err("App Server is not ready".to_owned());
        }
        if method == "unifiedSearch/query" {
            let search_id = self.begin_unified_search();
            return self.request_unified_search(params, search_id);
        }
        let response = self.request(method, params)?;
        #[cfg(test)]
        if response_result(&response).is_ok()
            && let Some(remaining) = lock(&self.inner.lost_successful_responses).get_mut(method)
            && *remaining > 0
        {
            *remaining -= 1;
            return Err(format!("App Server request timed out: {method}"));
        }
        response_result(&response).cloned()
    }

    #[must_use]
    pub fn begin_unified_search(&self) -> u64 {
        let search_id = self.inner.next_search_id.fetch_add(1, Ordering::SeqCst);
        self.inner
            .latest_search_id
            .store(search_id, Ordering::SeqCst);
        cancel_active_search(&self.inner);
        search_id
    }

    pub fn cancel_unified_search(&self) {
        let cancellation_id = self.inner.next_search_id.fetch_add(1, Ordering::SeqCst);
        self.inner
            .latest_search_id
            .store(cancellation_id, Ordering::SeqCst);
        cancel_active_search(&self.inner);
    }

    /// # Errors
    /// 检索请求被后续请求取代、超时或服务端失败时返回错误。
    pub fn request_unified_search(&self, params: &Value, search_id: u64) -> Result<Value, String> {
        if self.status().phase != AppServerPhase::Running {
            return Err("App Server is not ready".to_owned());
        }
        if self.inner.latest_search_id.load(Ordering::SeqCst) != search_id {
            return Err("unified_search_superseded".to_owned());
        }
        self.request_result_on_dedicated_connection("unifiedSearch/query", params, search_id)
    }

    fn request_result_on_dedicated_connection(
        &self,
        method: &str,
        params: &Value,
        search_id: u64,
    ) -> Result<Value, String> {
        let runtime_dir = lock(&self.inner.runtime_dir)
            .clone()
            .ok_or("App Server runtime directory is unavailable")?;
        let endpoint = Endpoint::resolve(&runtime_dir).map_err(|error| error.to_string())?;
        let stream = LocalStream::connect(&endpoint)
            .map_err(|error| format!("dedicated App Server connection failed: {error}"))?;
        let cancellation = stream
            .try_clone()
            .map_err(|error| format!("dedicated App Server connection clone failed: {error}"))?;
        let mut active = lock(&self.inner.active_search);
        if self.inner.latest_search_id.load(Ordering::SeqCst) != search_id {
            return Err("unified_search_superseded".to_owned());
        }
        if let Some((_, previous)) = active.replace((search_id, cancellation)) {
            let _ = previous.disconnect();
        }
        drop(active);
        let method = method.to_owned();
        let params = params.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        if let Err(error) = std::thread::Builder::new()
            .name("cilai-unified-search".to_owned())
            .spawn(move || {
                let _ = sender.send(dedicated_request(stream, &method, &params));
            })
        {
            cancel_search(&self.inner, search_id);
            return Err(format!("dedicated App Server worker failed: {error}"));
        }
        let outcome = match receiver.recv_timeout(UNIFIED_SEARCH_REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                cancel_search(&self.inner, search_id);
                Err("App Server request timed out: unifiedSearch/query".to_owned())
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("App Server response channel closed: unifiedSearch/query".to_owned())
            }
        };
        clear_search(&self.inner, search_id);
        if self.inner.latest_search_id.load(Ordering::SeqCst) == search_id {
            outcome
        } else {
            Err("unified_search_superseded".to_owned())
        }
    }

    fn notify(&self, method: &str, params: &Value) -> Result<(), String> {
        self.write_message(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn write_message(&self, value: &Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err("stdin_message_too_large".to_owned());
        }
        let mut connection = lock(&self.inner.writer);
        let writer = connection
            .as_mut()
            .ok_or("App Server connection is closed")?;
        writer
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        writer.write_all(b"\n").map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())
    }

    fn spawn_stdout_reader(&self, mut stdout: impl Read + Send + 'static) {
        let inner = self.inner.clone();
        let generation = inner.connection_generation.load(Ordering::Acquire);
        std::thread::spawn(move || {
            let mut decoder = JsonlDecoder::default();
            let mut chunk = [0_u8; 8192];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) => {
                        let _events = lock(&inner.lifecycle_events);
                        if inner.connection_generation.load(Ordering::Acquire) == generation {
                            record_connection_eof(&inner);
                        }
                        break;
                    }
                    Ok(count) => {
                        let _events = lock(&inner.lifecycle_events);
                        if inner.connection_generation.load(Ordering::Acquire) != generation {
                            continue;
                        }
                        for frame in decoder.push(&chunk[..count]) {
                            match frame {
                                contexthub_protocol::Frame::TooLarge => {
                                    fail_connection(&inner, "stdout_message_too_large");
                                    return;
                                }
                                contexthub_protocol::Frame::Message(bytes) => {
                                    if let Err(reason) = route_server_message(&inner, &bytes) {
                                        fail_connection(&inner, &reason);
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let _events = lock(&inner.lifecycle_events);
                        if inner.connection_generation.load(Ordering::Acquire) == generation {
                            fail_connection(&inner, &format!("stdout_read_failed: {error}"));
                        }
                        return;
                    }
                }
            }
        });
    }

    fn spawn_stderr_reader(&self, mut stderr: impl Read + Send + 'static) {
        let inner = self.inner.clone();
        let generation = inner.connection_generation.load(Ordering::Acquire);
        std::thread::spawn(move || {
            let mut decoder = StderrLineDecoder::default();
            let mut chunk = [0_u8; 1024];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) => {
                        let _events = lock(&inner.lifecycle_events);
                        if inner.connection_generation.load(Ordering::Acquire) == generation
                            && let Some(line) = decoder.finish()
                        {
                            push_stderr_line(&inner, line);
                        }
                        return;
                    }
                    Ok(count) => {
                        let _events = lock(&inner.lifecycle_events);
                        if inner.connection_generation.load(Ordering::Acquire) != generation {
                            continue;
                        }
                        for line in decoder.push(&chunk[..count]) {
                            push_stderr_line(&inner, line);
                        }
                    }
                    Err(_) => return,
                }
            }
        });
    }

    fn spawn_exit_monitor(&self, mut child: std::process::Child) {
        let inner = self.inner.clone();
        let generation = inner.connection_generation.load(Ordering::Acquire);
        std::thread::spawn(move || {
            let outcome = child
                .wait()
                .map(|exit| (exit.success(), exit.code()))
                .map_err(|error| error.to_string());
            let _events = lock(&inner.lifecycle_events);
            if inner.connection_generation.load(Ordering::Acquire) != generation {
                return;
            }
            lock(&inner.service_ownership).take();
            disconnect_writer(&inner);
            record_process_exit(&inner, outcome);
            fail_pending(&inner, "App Server process exited");
            inner.exit_changed.notify_all();
        });
    }

    fn wait_for_exit(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut status = lock(&self.inner.status);
        while status.phase == AppServerPhase::Stopping {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("App Server did not exit before timeout".to_owned());
            }
            let (next, result) = self
                .inner
                .exit_changed
                .wait_timeout(status, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            status = next;
            if result.timed_out() && status.phase == AppServerPhase::Stopping {
                return Err("App Server did not exit before timeout".to_owned());
            }
        }
        if status.phase == AppServerPhase::Stopped {
            Ok(())
        } else {
            Err(status
                .error_reason
                .clone()
                .unwrap_or_else(|| "App Server exit failed".to_owned()))
        }
    }

    fn wait_for_failed_process_exit(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut status = lock(&self.inner.status);
        while status.pid.is_some() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("failed App Server process did not exit before timeout".to_owned());
            }
            let (next, result) = self
                .inner
                .exit_changed
                .wait_timeout(status, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            status = next;
            if result.timed_out() && status.pid.is_some() {
                return Err("failed App Server process did not exit before timeout".to_owned());
            }
        }
        Ok(())
    }

    /// Keep an initialization/recovery failure visible before a server exists,
    /// including directory errors and cancelled administrator authorization.
    /// # Errors
    /// Refuses to mark a running or transitioning service as stopped/failed.
    pub fn record_quiescent_failure(&self, reason: &str, detail: &str) -> Result<(), String> {
        let status = self.status();
        if !matches!(
            status.phase,
            AppServerPhase::Stopped | AppServerPhase::Failed
        ) || status.pid.is_some()
        {
            return Err("service is not quiescent".into());
        }
        self.fail_start(reason, detail);
        Ok(())
    }

    fn fail_start(&self, reason: &str, detail: &str) -> String {
        disconnect_writer(&self.inner);
        let message = format!("{reason}: {detail}");
        push_stderr_line(&self.inner, format!("App Server failure: {message}"));
        let mut status = lock(&self.inner.status);
        if status.connection_mode == Some(AppServerConnectionMode::Attached) {
            status.pid = None;
        }
        status.phase = AppServerPhase::Failed;
        status.error_reason = Some(reason.to_owned());
        message
    }
}

fn cancel_active_search(inner: &Inner) {
    if let Some((_, stream)) = lock(&inner.active_search).take() {
        let _ = stream.disconnect();
    }
}

fn cancel_search(inner: &Inner, search_id: u64) {
    let mut active = lock(&inner.active_search);
    if active.as_ref().is_some_and(|(id, _)| *id == search_id)
        && let Some((_, stream)) = active.take()
    {
        let _ = stream.disconnect();
    }
}

fn clear_search(inner: &Inner, search_id: u64) {
    let mut active = lock(&inner.active_search);
    if active.as_ref().is_some_and(|(id, _)| *id == search_id) {
        active.take();
    }
}

fn dedicated_request(
    mut stream: LocalStream,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    let initialize_id = "desktop_search_initialize";
    write_jsonl(
        &mut stream,
        &json!({
            "jsonrpc": "2.0",
            "id": initialize_id,
            "method": "initialize",
            "params": {
                "client": {
                    "name": "contexthub-desktop-search",
                    "version": env!("CARGO_PKG_VERSION"),
                    "platform": std::env::consts::OS,
                    "arch": std::env::consts::ARCH
                },
                "protocol": { "minVersion": PROTOCOL_VERSION, "maxVersion": PROTOCOL_VERSION },
                "capabilities": {
                    "interactive": false,
                    "notifications": false,
                    "osNotifications": false,
                    "eventReplay": false,
                    "resourceTickets": false,
                    "richDiff": false,
                    "deepLinks": true
                },
                "resume": {}
            }
        }),
    )?;
    let mut decoder = JsonlDecoder::default();
    let initialized = read_jsonrpc_response(&mut stream, &mut decoder, initialize_id)?;
    let initialized = response_result(&initialized)?;
    if initialized.get("protocolVersion").and_then(Value::as_str) != Some(PROTOCOL_VERSION) {
        return Err("dedicated App Server protocol mismatch".to_owned());
    }
    write_jsonl(
        &mut stream,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )?;
    let request_id = "desktop_search_request";
    write_jsonl(
        &mut stream,
        &json!({ "jsonrpc": "2.0", "id": request_id, "method": method, "params": params }),
    )?;
    let response = read_jsonrpc_response(&mut stream, &mut decoder, request_id)?;
    response_result(&response).cloned()
}

fn write_jsonl(stream: &mut LocalStream, value: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .and_then(|()| stream.flush())
        .map_err(|error| format!("dedicated App Server write failed: {error}"))
}

fn read_jsonrpc_response(
    stream: &mut LocalStream,
    decoder: &mut JsonlDecoder,
    expected_id: &str,
) -> Result<Value, String> {
    let mut chunk = [0_u8; 8192];
    loop {
        let count = stream
            .read(&mut chunk)
            .map_err(|error| format!("dedicated App Server read failed: {error}"))?;
        if count == 0 {
            return Err("dedicated App Server connection closed".to_owned());
        }
        for frame in decoder.push(&chunk[..count]) {
            let contexthub_protocol::Frame::Message(bytes) = frame else {
                return Err("dedicated App Server response too large".to_owned());
            };
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| "dedicated App Server returned invalid JSON".to_owned())?;
            if value.get("id").and_then(Value::as_str) == Some(expected_id) {
                return Ok(value);
            }
        }
    }
}

fn startup_timeout(data_dir: &Path) -> Duration {
    let ai = data_dir.join("ai-runtime");
    let pending = ["codex-harness", "application-harness"]
        .iter()
        .any(|home| ai.join(home).exists())
        || ["cilai-harness", "cilai-application-harness"]
            .iter()
            .any(|home| {
                let job = ai.join("cilai-home-migrations-v1").join(home);
                (job.exists() && !job.join("complete.json").is_file())
                    || official_relocation_pending(&ai.join(home).join("official-runtime"), &job)
            });
    if pending {
        HOME_MIGRATION_START_TIMEOUT
    } else {
        REQUEST_TIMEOUT
    }
}

fn official_relocation_pending(home: &Path, job: &Path) -> bool {
    let receipt = home.join("cilai-public-history-migration.json");
    if !job.join("complete.json").is_file() || !receipt.is_file() {
        return false;
    }
    let repair = job.join("official-runtime-relocation-v1");
    if repair.exists() && !repair.join("complete.json").is_file() {
        return true;
    }
    let bound = std::fs::metadata(&receipt)
        .ok()
        .filter(|metadata| metadata.len() <= 64 * 1024)
        .and_then(|_| std::fs::read(&receipt).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .zip(home.canonicalize().ok());
    bound.is_none_or(|(receipt, home)| {
        receipt["homeDigest"] != crate::persistent_threads::digest(&serde_json::json!(home))
    })
}

fn request_timeout(method: &str) -> Duration {
    match method {
        // 历史读取、完整数据库备份及迁移切换包含磁盘 I/O，不能沿用普通查询的 5 秒上限。
        "aiHistory/execute" => AI_HISTORY_REQUEST_TIMEOUT,
        "personalData/backupCapture"
        | "personalData/restorePrepare"
        | "personalData/restoreApplyPrepare"
        | "personalData/reconcile" => Duration::from_mins(10),
        // 保存结果不明时用读取来收口，读取排在可能很慢的扫描之后，等待上限与扫描一致
        "workspace/scan" | "workspace/files" | "aiState/get" => WORKSPACE_SCAN_TIMEOUT,
        "cloudWorkspace/start"
        | "cloudWorkspace/status"
        | "cloudWorkspace/createTeamSpace"
        | "cloudWorkspace/attach"
        | "cloudWorkspace/preview"
        | "cloudWorkspace/material"
        | "cloudWorkspace/fileAction"
        | "cloudWorkspace/deleteItem" => CLOUD_WORKSPACE_TRANSFER_TIMEOUT,
        "applicationCatalog/list" => APPLICATION_CATALOG_LIST_TIMEOUT,
        "applicationCatalog/launch" | "aiBrowserPolicy/get" => APPLICATION_CATALOG_REQUEST_TIMEOUT,
        "unifiedSearch/query" => UNIFIED_SEARCH_REQUEST_TIMEOUT,
        "event/replay" => EVENT_REPLAY_REQUEST_TIMEOUT,
        _ => REQUEST_TIMEOUT,
    }
}

impl Default for AppServerSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

/// # Errors
/// 随包应用服务文件不存在或其路径无法确定时返回错误。
pub fn bundled_app_server_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(bundled_executable_path(BundledExecutable::AppServer)?)
}

fn response_result(response: &Value) -> Result<&Value, String> {
    if let Some(error) = response.get("error") {
        return Err(format!("App Server returned protocol error: {error}"));
    }
    response
        .get("result")
        .ok_or("App Server response has no result".to_owned())
}

fn route_server_message(inner: &Inner, bytes: &[u8]) -> Result<(), String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| "stdout_invalid_json".to_owned())?;
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err("stdout_invalid_jsonrpc_version".to_owned());
    }
    if let Some(id) = value.get("id").and_then(response_id_key)
        && let Some(sender) = lock(&inner.pending).remove(&id)
    {
        let _ = sender.send(Ok(value));
        return Ok(());
    }
    lock(&inner.status).unsolicited_message_count += 1;
    Ok(())
}

fn response_id_key(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .map(|value| value.to_string()),
        _ => None,
    }
}

fn fail_connection(inner: &Inner, reason: &str) {
    let mut status = lock(&inner.status);
    let attached = status.connection_mode == Some(AppServerConnectionMode::Attached);
    if !matches!(
        status.phase,
        AppServerPhase::Stopping | AppServerPhase::Stopped
    ) {
        status.phase = AppServerPhase::Failed;
        status.error_reason = Some(reason.to_owned());
    }
    drop(status);
    disconnect_writer(inner);
    if attached {
        lock(&inner.status).pid = None;
    }
    fail_pending(inner, reason);
    inner.exit_changed.notify_all();
}

fn disconnect_writer(inner: &Inner) {
    if let Some(mut writer) = lock(&inner.writer).take() {
        writer.disconnect();
    }
}

fn record_connection_eof(inner: &Inner) {
    let mut status = lock(&inner.status);
    if matches!(
        status.phase,
        AppServerPhase::Starting | AppServerPhase::Running
    ) {
        status.phase = AppServerPhase::Failed;
        status.error_reason = Some("app_server_connection_closed".to_owned());
        if status.connection_mode == Some(AppServerConnectionMode::Attached) {
            status.pid = None;
        }
    }
    drop(status);
    fail_pending(inner, "App Server connection closed");
    inner.exit_changed.notify_all();
}

fn fail_pending(inner: &Inner, reason: &str) {
    let pending = std::mem::take(&mut *lock(&inner.pending));
    for (_, sender) in pending {
        let _ = sender.send(Err(reason.to_owned()));
    }
}

fn record_process_exit(inner: &Inner, outcome: Result<(bool, Option<i32>), String>) {
    let mut status = lock(&inner.status);
    status.pid = None;
    status.connection_mode = None;
    match outcome {
        Ok((success, code)) => {
            status.last_exit_code = code;
            if status.phase == AppServerPhase::Stopping && success {
                status.phase = AppServerPhase::Stopped;
            } else if status.phase == AppServerPhase::Failed {
                // 保留先前更精确的协议、读写或握手失败原因。
            } else {
                status.phase = AppServerPhase::Failed;
                status.error_reason = Some(if success {
                    "unexpected_exit".to_owned()
                } else {
                    format!("unexpected_exit_code_{}", code.unwrap_or(-1))
                });
            }
        }
        Err(error) => {
            status.phase = AppServerPhase::Failed;
            status.error_reason = Some(format!("process_wait_failed: {error}"));
        }
    }
}

#[derive(Default)]
struct StderrLineDecoder {
    buffer: Vec<u8>,
    truncated: bool,
}

impl StderrLineDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                lines.push(self.take_line());
            } else if self.buffer.len() < STDERR_MAX_CHARS_PER_LINE {
                self.buffer.push(byte);
            } else {
                self.truncated = true;
            }
        }
        lines
    }

    fn finish(&mut self) -> Option<String> {
        if self.buffer.is_empty() && !self.truncated {
            None
        } else {
            Some(self.take_line())
        }
    }

    fn take_line(&mut self) -> String {
        if self.buffer.last() == Some(&b'\r') {
            self.buffer.pop();
        }
        let mut line = String::from_utf8_lossy(&std::mem::take(&mut self.buffer)).into_owned();
        if std::mem::take(&mut self.truncated) {
            line.push('…');
        }
        line
    }
}

fn push_stderr_line(inner: &Inner, line: String) {
    let mut status = lock(&inner.status);
    let mut tail = VecDeque::from(std::mem::take(&mut status.stderr_tail));
    if tail.len() == STDERR_MAX_LINES {
        tail.pop_front();
    }
    tail.push_back(line);
    status.stderr_tail = tail.into();
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
pub(crate) fn app_server_test_guard() -> MutexGuard<'static, ()> {
    static APP_SERVER_TEST_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    lock(APP_SERVER_TEST_LOCK.get_or_init(|| Mutex::new(())))
}

#[cfg(test)]
fn app_server_test_binary_from_executable(
    executable: &Path,
    target_triple: &str,
) -> Option<PathBuf> {
    sidecar_test_binary_from_executable(executable, target_triple, "cilai-app-server")
}

#[cfg(test)]
fn sidecar_test_binary_from_executable(
    executable: &Path,
    target_triple: &str,
    name: &str,
) -> Option<PathBuf> {
    let profile_dir = executable.parent()?.parent()?;
    let profile = profile_dir.file_name()?;
    let mut target_dir = profile_dir.parent()?;
    if target_dir.file_name()?.to_str() == Some(target_triple) {
        target_dir = target_dir.parent()?;
    }
    let extension = if target_triple.contains("-windows-") {
        ".exe"
    } else {
        ""
    };
    Some(
        target_dir
            .join("sidecar")
            .join(profile)
            .join(if name == "cilai-harness" {
                format!("codex-runtime/bin/codex-app-server{extension}")
            } else {
                format!("{name}-{target_triple}{extension}")
            }),
    )
}

#[cfg(test)]
fn app_server_test_target_triple() -> &'static str {
    option_env!("TAURI_ENV_TARGET_TRIPLE").unwrap_or(
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => "aarch64-apple-darwin",
            ("macos", "x86_64") => "x86_64-apple-darwin",
            ("windows", "x86_64") => "x86_64-pc-windows-msvc",
            _ => panic!("unsupported_test_target"),
        },
    )
}

#[cfg(test)]
pub(crate) fn app_server_test_binary() -> PathBuf {
    let executable = std::env::current_exe().expect("测试二进制路径可读");
    app_server_test_binary_from_executable(&executable, app_server_test_target_triple())
        .expect("产物目录布局为 <产物根>/<配置>/deps/<可执行文件>")
}

#[cfg(test)]
pub(crate) fn codex_harness_test_binary() -> PathBuf {
    let executable = std::env::current_exe().expect("测试二进制路径可读");
    sidecar_test_binary_from_executable(
        &executable,
        app_server_test_target_triple(),
        "cilai-harness",
    )
    .expect("产物目录布局为 <产物根>/<配置>/deps/<可执行文件>")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use contexthub_platform_adapter::instance::{
        INSTANCE_SCHEMA_VERSION, InstanceDescriptor, hash_canonical_path, process_start_fingerprint,
    };
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::io::{BufRead, BufReader};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::os::unix::net::UnixListener;
    #[cfg(unix)]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn recovery_failure_preserves_retry_state_without_hiding_a_live_service() {
        let supervisor = AppServerSupervisor::new();
        supervisor
            .record_quiescent_failure(
                "startup_permission_repair_failed",
                "administrator authorization cancelled",
            )
            .unwrap();
        let failed = supervisor.status();
        assert_eq!(failed.phase, AppServerPhase::Failed);
        assert_eq!(
            failed.error_reason.as_deref(),
            Some("startup_permission_repair_failed")
        );
        assert!(failed.pid.is_none());
        {
            let mut status = lock(&supervisor.inner.status);
            status.phase = AppServerPhase::Running;
            status.pid = Some(12345);
            status.error_reason = None;
        }
        assert!(
            supervisor
                .record_quiescent_failure("startup_permission_repair_failed", "cancelled")
                .is_err()
        );
        let running = supervisor.status();
        assert_eq!(running.phase, AppServerPhase::Running);
        assert_eq!(running.pid, Some(12345));
        assert!(running.error_reason.is_none());
    }

    #[test]
    fn application_catalog_requests_allow_upstream_timeout_to_return() {
        assert_eq!(
            request_timeout("aiHistory/execute"),
            AI_HISTORY_REQUEST_TIMEOUT
        );
        assert_eq!(request_timeout("aiLedger/apply"), REQUEST_TIMEOUT);
        assert_eq!(
            request_timeout("applicationCatalog/list"),
            APPLICATION_CATALOG_LIST_TIMEOUT
        );
        assert_eq!(
            request_timeout("applicationCatalog/launch"),
            APPLICATION_CATALOG_REQUEST_TIMEOUT
        );
        assert_eq!(
            request_timeout("aiBrowserPolicy/get"),
            APPLICATION_CATALOG_REQUEST_TIMEOUT
        );
        assert_eq!(
            request_timeout("event/replay"),
            EVENT_REPLAY_REQUEST_TIMEOUT
        );
        assert_eq!(
            request_timeout("unifiedSearch/query"),
            UNIFIED_SEARCH_REQUEST_TIMEOUT
        );
        assert_eq!(
            request_timeout("cloudWorkspace/start"),
            CLOUD_WORKSPACE_TRANSFER_TIMEOUT
        );
        assert_eq!(
            request_timeout("cloudWorkspace/status"),
            CLOUD_WORKSPACE_TRANSFER_TIMEOUT
        );
        assert_eq!(
            request_timeout("cloudWorkspace/fileAction"),
            CLOUD_WORKSPACE_TRANSFER_TIMEOUT
        );
        assert_eq!(request_timeout("ping"), REQUEST_TIMEOUT);
    }

    #[test]
    fn only_pending_home_migration_extends_startup_wait() {
        let root =
            std::env::temp_dir().join(format!("cilai-startup-migration-{}", uuid::Uuid::new_v4()));
        assert_eq!(startup_timeout(&root), REQUEST_TIMEOUT);
        let old = root.join("ai-runtime/codex-harness");
        std::fs::create_dir_all(&old).unwrap();
        assert_eq!(startup_timeout(&root), HOME_MIGRATION_START_TIMEOUT);
        std::fs::remove_dir(&old).unwrap();
        let job = root.join("ai-runtime/cilai-home-migrations-v1/cilai-harness");
        std::fs::create_dir_all(&job).unwrap();
        assert_eq!(startup_timeout(&root), HOME_MIGRATION_START_TIMEOUT);
        std::fs::write(job.join("complete.json"), "completed").unwrap();
        assert_eq!(startup_timeout(&root), REQUEST_TIMEOUT);
        let official = root.join("ai-runtime/cilai-harness/official-runtime");
        std::fs::create_dir_all(&official).unwrap();
        let receipt = official.join("cilai-public-history-migration.json");
        std::fs::write(&receipt, r#"{"homeDigest":"previous-home"}"#).unwrap();
        assert_eq!(startup_timeout(&root), HOME_MIGRATION_START_TIMEOUT);
        std::fs::write(&receipt, serde_json::json!({"homeDigest":crate::persistent_threads::digest(&serde_json::json!(official.canonicalize().unwrap()))}).to_string()).unwrap();
        assert_eq!(startup_timeout(&root), REQUEST_TIMEOUT);
        assert_eq!(request_timeout("ping"), REQUEST_TIMEOUT);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn app_server_test_binary_follows_active_build_profile() {
        let debug_executable = PathBuf::from("build-root")
            .join("target")
            .join("debug")
            .join("deps")
            .join("runtime-tests");
        let release_executable = PathBuf::from("build-root")
            .join("target")
            .join("release")
            .join("deps")
            .join("runtime-tests");

        assert_eq!(
            app_server_test_binary_from_executable(&debug_executable, "aarch64-apple-darwin"),
            Some(
                PathBuf::from("build-root")
                    .join("target")
                    .join("sidecar")
                    .join("debug")
                    .join("cilai-app-server-aarch64-apple-darwin")
            )
        );
        assert_eq!(
            app_server_test_binary_from_executable(&release_executable, "aarch64-apple-darwin"),
            Some(
                PathBuf::from("build-root")
                    .join("target")
                    .join("sidecar")
                    .join("release")
                    .join("cilai-app-server-aarch64-apple-darwin")
            )
        );
    }

    #[test]
    fn harness_tests_use_the_active_profile_and_target_sidecar() {
        for (triple, suffix) in [
            ("aarch64-apple-darwin", ""),
            ("x86_64-pc-windows-msvc", ".exe"),
        ] {
            for profile in ["debug", "release"] {
                for explicit_target in [false, true] {
                    let root = PathBuf::from("build-root").join("target");
                    let mut binary = root.clone();
                    if explicit_target {
                        binary.push(triple);
                    }
                    binary.extend([profile, "deps", "runtime-tests"]);
                    assert_eq!(
                        sidecar_test_binary_from_executable(&binary, triple, "cilai-harness"),
                        Some(
                            root.join("sidecar")
                                .join(profile)
                                .join(format!("codex-runtime/bin/codex-app-server{suffix}"))
                        )
                    );
                }
            }
        }
    }

    #[test]
    fn windows_test_sidecar_uses_exe_suffix_and_shared_stage() {
        for profile in ["debug", "release"] {
            for explicit_target in [false, true] {
                let root = PathBuf::from("build-root").join("target");
                let mut binary = root.clone();
                if explicit_target {
                    binary.push("x86_64-pc-windows-msvc");
                }
                binary.extend([profile, "deps", "runtime-tests.exe"]);
                assert_eq!(
                    app_server_test_binary_from_executable(&binary, "x86_64-pc-windows-msvc"),
                    Some(
                        root.join("sidecar")
                            .join(profile)
                            .join("cilai-app-server-x86_64-pc-windows-msvc.exe")
                    )
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn desktop_can_create_and_scan_workspace_with_bundled_server() {
        let _server_guard = app_server_test_guard();
        let root = PathBuf::from("/tmp").join(format!(
            "cilai-folder-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let folder = root.join("folder");
        let data_dir = root.join("test");
        let runtime_dir = data_dir.join("run");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("material.md"), "folder connection evidence").unwrap();
        let binary = app_server_test_binary();
        let supervisor = AppServerSupervisor::new();
        supervisor
            .start(&binary, "test", &data_dir, &runtime_dir)
            .unwrap();
        let created = supervisor.request_result(
            "workspace/create",
            &json!({
                "localRoot": folder,
                "displayName": "文件夹连接验证",
                "clientOperationId": "desktop-folder-contract"
            }),
        );
        let verified = created.and_then(|created| {
            let workspace_id = &created["workspace"]["workspaceId"];
            supervisor.request_result("workspace/scan", &json!({ "workspaceId": workspace_id }))?;
            let files = supervisor.request_result(
                "workspace/files",
                &json!({
                    "workspaceId": workspace_id, "cursor": null, "limit": 100
                }),
            )?;
            Ok(files["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|file| file["relativePath"] == "material.md"))
        });
        supervisor.shutdown().unwrap();
        assert_eq!(
            fs::read_to_string(folder.join("material.md")).unwrap(),
            "folder connection evidence"
        );
        fs::remove_dir_all(root).unwrap();
        assert_eq!(verified, Ok(true));
    }

    #[test]
    fn server_response_routes_only_to_matching_request() {
        let supervisor = AppServerSupervisor::new();
        let (sender, receiver) = mpsc::sync_channel(1);
        lock(&supervisor.inner.pending).insert("desktop_7".to_owned(), sender);
        route_server_message(
            &supervisor.inner,
            br#"{"jsonrpc":"2.0","id":"desktop_7","result":{"ok":true}}"#,
        )
        .unwrap();
        assert_eq!(receiver.recv().unwrap().unwrap()["result"]["ok"], true);
        assert_eq!(supervisor.status().unsolicited_message_count, 0);
    }

    #[test]
    fn malformed_stdout_marks_connection_failed() {
        let supervisor = AppServerSupervisor::new();
        lock(&supervisor.inner.status).phase = AppServerPhase::Running;
        assert_eq!(
            route_server_message(&supervisor.inner, b"not-json"),
            Err("stdout_invalid_json".to_owned())
        );
        fail_connection(&supervisor.inner, "stdout_invalid_json");
        assert_eq!(supervisor.status().phase, AppServerPhase::Failed);
    }

    #[test]
    fn bundled_path_never_uses_working_directory() {
        let path = bundled_app_server_path().unwrap();
        assert_eq!(path.parent(), std::env::current_exe().unwrap().parent());
    }

    #[test]
    fn stderr_decoder_is_incremental_bounded_and_supports_crlf() {
        let mut decoder = StderrLineDecoder::default();
        assert!(decoder.push(b"first").is_empty());
        assert_eq!(
            decoder.push(b" line\r\nsecond\n"),
            vec!["first line", "second"]
        );
        let oversized = vec![b'x'; STDERR_MAX_CHARS_PER_LINE + 20];
        assert!(decoder.push(&oversized).is_empty());
        let line = decoder.push(b"\n").pop().unwrap();
        assert_eq!(line.chars().count(), STDERR_MAX_CHARS_PER_LINE + 1);
        assert!(line.ends_with('…'));
    }

    #[test]
    fn discovery_failure_preserves_the_underlying_reason_without_starting_a_process() {
        let root =
            std::env::temp_dir().join(format!("cilai-instance-failure-{}", uuid::Uuid::new_v4()));
        let runtime = root.join("run");
        std::fs::create_dir_all(&runtime).unwrap();
        let record_path = runtime.join("instance.json");
        let mut record =
            contexthub_platform_adapter::security::create_new_restricted_file(&record_path)
                .unwrap();
        record.write_all(b"{invalid instance record").unwrap();
        drop(record);
        let supervisor = AppServerSupervisor::new();
        let error = supervisor
            .start(&root.join("not-a-server.exe"), "test", &root, &runtime)
            .unwrap_err();
        let status = supervisor.status();
        assert_eq!(status.phase, AppServerPhase::Failed);
        assert_eq!(status.pid, None);
        assert_eq!(
            status.error_reason.as_deref(),
            Some("instance_discovery_failed")
        );
        assert!(error.contains("key must be a string"));
        assert_eq!(
            status.stderr_tail,
            vec![format!("App Server failure: {error}")]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unexpected_process_exit_is_observable_and_clears_pid() {
        let supervisor = AppServerSupervisor::new();
        {
            let mut status = lock(&supervisor.inner.status);
            status.phase = AppServerPhase::Running;
            status.pid = Some(42);
        }
        record_process_exit(&supervisor.inner, Ok((false, Some(20))));
        let status = supervisor.status();
        assert_eq!(status.phase, AppServerPhase::Failed);
        assert_eq!(status.pid, None);
        assert_eq!(status.last_exit_code, Some(20));
        assert_eq!(
            status.error_reason.as_deref(),
            Some("unexpected_exit_code_20")
        );
    }

    #[cfg(unix)]
    #[test]
    fn existing_transient_is_attached_and_desktop_exit_only_disconnects() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            PathBuf::from("/tmp").join(format!("ch-desktop-attach-{}-{nanos}", std::process::id()));
        let data_dir = root.join("test");
        let runtime_dir = data_dir.join("run");
        fs::create_dir_all(&runtime_dir).unwrap();
        fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
        #[cfg(target_os = "macos")]
        let socket_path = runtime_dir.join("c.sock");
        #[cfg(all(unix, not(target_os = "macos")))]
        let socket_path =
            PathBuf::from(Endpoint::resolve(&runtime_dir).unwrap().identity().as_ref());
        let listener = UnixListener::bind(&socket_path).unwrap();
        let record = InstanceDescriptor {
            schema_version: INSTANCE_SCHEMA_VERSION,
            pid: std::process::id(),
            process_start_fingerprint: process_start_fingerprint(std::process::id())
                .unwrap()
                .unwrap(),
            server_instance_id: "srv_attached_test".to_owned(),
            lifecycle_mode: LifecycleMode::Transient,
            environment: "test".to_owned(),
            data_dir_hash: hash_canonical_path(&data_dir).unwrap(),
            protocol_version: PROTOCOL_VERSION.to_owned(),
            ipc_endpoint: socket_path.to_string_lossy().into_owned(),
            started_at_epoch_ms: 0,
        };
        let record_path = runtime_dir.join("instance.json");
        fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();
        fs::set_permissions(&record_path, fs::Permissions::from_mode(0o600)).unwrap();

        let fake_server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut initialize_line = String::new();
            reader.read_line(&mut initialize_line).unwrap();
            let initialize: Value = serde_json::from_str(&initialize_line).unwrap();
            assert_eq!(initialize["method"], "initialize");
            assert_eq!(initialize["params"]["client"]["name"], "contexthub-desktop");
            writeln!(
                stream,
                "{}",
                json!({
                    "jsonrpc": "2.0",
                    "id": initialize["id"],
                    "result": {
                        "protocolVersion": PROTOCOL_VERSION,
                        "server": { "instanceId": "srv_attached_test" },
                        "capabilities": { "localIpc": true }
                    }
                })
            )
            .unwrap();
            stream.flush().unwrap();
            let mut notification_line = String::new();
            reader.read_line(&mut notification_line).unwrap();
            let initialized: Value = serde_json::from_str(&notification_line).unwrap();
            assert_eq!(initialized["method"], "initialized");
            let mut after_detach = String::new();
            assert_eq!(reader.read_line(&mut after_detach).unwrap(), 0);
            assert!(after_detach.is_empty());
        });

        let supervisor = AppServerSupervisor::new();
        supervisor
            .start(
                &PathBuf::from("/path/that/must/not/be/spawned"),
                "test",
                &data_dir,
                &runtime_dir,
            )
            .unwrap();
        let status = supervisor.status();
        assert_eq!(status.phase, AppServerPhase::Running);
        assert_eq!(
            status.connection_mode,
            Some(AppServerConnectionMode::Attached)
        );
        assert_eq!(status.server_lifecycle_mode.as_deref(), Some("transient"));
        assert_eq!(
            status.server_instance_id.as_deref(),
            Some("srv_attached_test")
        );
        assert!(supervisor.restart().is_err());
        supervisor.shutdown().unwrap();
        assert_eq!(supervisor.status().phase, AppServerPhase::Stopped);
        fake_server.join().unwrap();

        fs::remove_file(record_path).unwrap();
        fs::remove_file(socket_path).unwrap();
        fs::remove_dir(runtime_dir).unwrap();
        fs::remove_dir(data_dir).unwrap();
        fs::remove_dir(root).unwrap();
    }
}

#[cfg(test)]
mod shared_tests;
