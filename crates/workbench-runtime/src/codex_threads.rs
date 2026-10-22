//! 常驻 Codex 进程：读取与管理对话线程。
//!
//! 新轮次内容以 Codex 线程为准，迁入历史的显示结构由工作台保存。模型执行进程跨轮复用，
//! 每轮仍执行沙箱探测；这里的进程只读取线程的轮次与条目，不连接模型，也关闭记忆整理、插件与更新检查等后台工作，避免与
//! 执行进程争抢同一份数据目录里的后台任务。进程按需启动，请求失败时回收，下一次请求重新启动。

use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
mod archive_state;
use archive_state::native_archive_state_from_path;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const TURN_PAGE_LIMIT: u32 = 50;
type LaunchRegistration = Arc<dyn Fn(u32) -> Result<(), String> + Send + Sync>;

fn delete_context_before_native(
    clear_context: impl FnOnce() -> Result<(), String>,
    delete_native: impl FnOnce() -> Result<Value, String>,
) -> Result<Value, String> {
    clear_context()?;
    delete_native()
}

pub(crate) struct CodexThreadService {
    executable: PathBuf,
    home: PathBuf,
    connection: Mutex<Option<Connection>>,
    history_server: Option<crate::supervisor::AppServerSupervisor>,
    maintenance_writer: Mutex<Option<NativeWriterLease>>,
    maintenance_only: bool,
    import_lock: Mutex<()>,
    archive_states: Mutex<std::collections::BTreeMap<String, bool>>,
    provider_aliases: Vec<String>,
    session_source: &'static str,
    profile_dir: Option<PathBuf>,
    connection_epoch: std::sync::atomic::AtomicU64,
    network: contexthub_protocol::ModelNetworkConfig,
    credential_vault_root: Option<PathBuf>,
    network_route: std::sync::OnceLock<Result<crate::model_connections::ResolvedNetwork, String>>,
    network_guard: Option<crate::model_connections::native_proxy::Admission>,
    network_bridge: Mutex<Option<Arc<crate::model_connections::native_proxy::NativeProxy>>>,
    runtime_managed_chatgpt: bool,
    home_isolation: std::sync::OnceLock<
        Result<
            Option<contexthub_platform_adapter::macos_runtime_isolation::ChatgptIsolationSnapshot>,
            String,
        >,
    >,
    launch_registration: Option<LaunchRegistration>,
    launch_scope: Option<Arc<crate::model_connections::ExecutionScope>>,
    launch_attempted: std::sync::atomic::AtomicBool,
}

struct Connection {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    next_id: u64,
    login_notifications: Vec<Value>,
    writer: Option<NativeWriterLease>,
}

struct NativeWriterLease {
    server: crate::supervisor::AppServerSupervisor,
    lease: contexthub_protocol::PersonalWriterLease,
    launched: bool,
    released: bool,
    release_attempted: bool,
}
impl NativeWriterLease {
    fn prepare(
        server: &crate::supervisor::AppServerSupervisor,
        home: &std::path::Path,
        kind: &str,
    ) -> Result<Self, String> {
        let result=server.request_result("profileWriter/prepare",&json!({
            "clientOperationId":format!("native-history-{}",uuid::Uuid::new_v4().simple()),
            "kind":kind,"resourceId":crate::persistent_threads::digest(&json!(home)),
            "resourceRevision":crate::model_connections::OFFICIAL_RUNTIME_VERSION,
            "owner":crate::model_connections::execution_owner()?,"launchProtocol":1,"processMode":"local_process"
        }))?;
        Ok(Self {
            server: server.clone(),
            lease: serde_json::from_value(result)
                .map_err(|_| "personal_writer_response_invalid")?,
            launched: false,
            released: false,
            release_attempted: false,
        })
    }
    fn register(&self, pid: u32) -> Result<(), String> {
        let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
            .map_err(|_| "personal_writer_process_unconfirmed")?
            .ok_or("personal_writer_process_unconfirmed")?;
        self.server.request_result("profileWriter/registerProcess",&json!({"leaseId":self.lease.lease_id,"process":{"pid":pid,"startFingerprint":fingerprint}}))?;
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        if self.released {
            return Ok(());
        }
        if self.release_attempted {
            return Err("personal_writer_release_unconfirmed".into());
        }
        self.release_attempted = true;
        self.server.request_result(
            "profileWriter/release",
            &json!({"leaseId":self.lease.lease_id}),
        )?;
        self.released = true;
        Ok(())
    }
}
impl Drop for NativeWriterLease {
    fn drop(&mut self) {
        if !self.launched {
            if self
                .server
                .request_result(
                    "profileWriter/abortUnlaunched",
                    &json!({"leaseId":self.lease.lease_id,"owner":self.lease.owner}),
                )
                .is_err()
            {
                eprintln!("native_history_writer_abort_unconfirmed");
            }
        } else if !self.released {
            eprintln!("native_history_writer_release_unconfirmed");
        }
    }
}

pub(crate) struct ImportedFork {
    pub synthetic: bool,
    pub display: Vec<Value>,
    pub context: Vec<Value>,
}

struct CatalogLoad<'a> {
    service: &'a CodexThreadService,
    loaded: bool,
}
impl Drop for CatalogLoad<'_> {
    fn drop(&mut self) {
        if self.loaded {
            self.service.shutdown();
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let stopped = match self.child.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) | Err(_) => self
                .child
                .kill()
                .and_then(|()| self.child.wait().map(|_| ())),
        };
        if stopped.is_ok()
            && let Some(writer) = self.writer.as_mut()
            && !writer.release_attempted
            && writer.finish().is_err()
        {
            eprintln!("native_history_writer_release_unconfirmed");
        }
    }
}

impl CodexThreadService {
    /// 上游索引会省略尚无真实用户轮次的迁入线程；按已核验的业务绑定补读。
    pub(crate) fn catalog_for_threads<'a>(
        &self,
        threads: impl IntoIterator<Item = &'a str>,
    ) -> Result<std::collections::BTreeMap<String, Value>, String> {
        let mut result = self.catalog_with_mode(false)?;
        let mut metadata = CatalogLoad {
            service: self,
            loaded: false,
        };
        for thread in threads {
            if let Some(current) = result.get(thread) {
                let archived = native_archive_state_from_path(&self.home, current)?;
                if current["archived"].as_bool() != Some(archived) {
                    return Err("codex_thread_archive_state_unconfirmed".into());
                }
                continue;
            }
            let (current, loaded) = self.catalog_thread(thread)?;
            metadata.loaded |= loaded;
            result.insert(thread.to_owned(), current);
        }
        Ok(result)
    }

    pub(crate) fn catalog(&self) -> Result<std::collections::BTreeMap<String, Value>, String> {
        self.catalog_with_mode(true)
    }

    fn catalog_with_mode(
        &self,
        use_state_db_only: bool,
    ) -> Result<std::collections::BTreeMap<String, Value>, String> {
        let mut result = std::collections::BTreeMap::new();
        let mut metadata = CatalogLoad {
            service: self,
            loaded: false,
        };
        for archived in [false, true] {
            let mut cursor = Value::Null;
            let mut seen = HashSet::new();
            loop {
                let page = self.request("thread/list", &json!({"archived":archived,"sourceKinds":["vscode","appServer"],"modelProviders":[],"useStateDbOnly":use_state_db_only,"limit":100,"cursor":cursor}))?;
                for thread in page["data"]
                    .as_array()
                    .ok_or("codex_thread_catalog_invalid")?
                {
                    let id = thread["id"]
                        .as_str()
                        .ok_or("codex_thread_catalog_invalid")?;
                    let mut value = thread.clone();
                    value["archived"] = json!(archived);
                    if result.insert(id.into(), value).is_some() {
                        return Err("codex_thread_catalog_duplicate".into());
                    }
                }
                cursor = page
                    .get("nextCursor")
                    .cloned()
                    .ok_or("codex_thread_catalog_invalid")?;
                if cursor.is_null() {
                    break;
                }
                if !cursor.is_string() || !seen.insert(cursor.to_string()) {
                    return Err("codex_thread_catalog_cursor_invalid".into());
                }
            }
        }
        // 旧工作台线程使用自定义来源，上游不提供对应 list 过滤器；按业务绑定读取。
        if (self.home.ends_with("cilai-harness")
            || self
                .home
                .parent()
                .is_some_and(|parent| parent.ends_with("cilai-harness")))
            && let Some(server) = &self.history_server
        {
            let source = server.request_result(
                "aiHistory/execute",
                &json!({"action":"source","payload":null}),
            )?;
            let source = &source["value"];
            let conversations = if source["active"] == true {
                &source["ledger"]["conversations"]
            } else {
                &source["state"]["conversations"]
            };
            for record in conversations
                .as_array()
                .ok_or("ai_history_catalog_invalid")?
            {
                let conversation = record.get("value").unwrap_or(record);
                let id = conversation["conversationId"]
                    .as_str()
                    .ok_or("ai_history_catalog_invalid")?;
                let binding =
                    server.request_result("aiThread/get", &json!({"conversationId":id}))?;
                if let Some(thread) = binding["binding"]["nativeThreadId"].as_str() {
                    if result.contains_key(thread) {
                        continue;
                    }
                    let (current, loaded) = self.catalog_thread(thread)?;
                    metadata.loaded |= loaded;
                    result.insert(thread.to_owned(), current);
                }
            }
        }
        Ok(result)
    }

    fn catalog_thread(&self, thread: &str) -> Result<(Value, bool), String> {
        let mut current = self.request(
            "thread/read",
            &json!({"threadId":thread,"includeTurns":false}),
        )?["thread"]
            .clone();
        let archived = native_archive_state_from_path(&self.home, &current)?;
        self.archive_states
            .lock()
            .map_err(|_| "codex_thread_catalog_lock_failed")?
            .insert(thread.to_owned(), archived);
        current["archived"] = json!(archived);
        Ok((current, true))
    }
    pub(crate) fn connection_epoch(&self) -> u64 {
        self.connection_epoch
            .load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn login_completion(&self, login: &str) -> Option<bool> {
        let mut connection = self.connection.lock().ok()?;
        let messages = &mut connection.as_mut()?.login_notifications;
        let index = messages
            .iter()
            .position(|message| message["params"]["loginId"] == login)?;
        messages.remove(index)["params"]["success"].as_bool()
    }

    pub(crate) fn home(&self) -> &std::path::Path {
        &self.home
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.shutdown_checked();
    }

    pub(crate) fn shutdown_checked(&self) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut connection) = connection {
            if connection
                .child
                .try_wait()
                .map_err(|_| "model_auth_process_unconfirmed")?
                .is_none()
            {
                connection
                    .child
                    .kill()
                    .map_err(|_| "model_auth_process_unconfirmed")?;
            }
            connection
                .child
                .wait()
                .map_err(|_| "model_auth_process_unconfirmed")?;
            self.stop_network_bridge()?;
            if let Some(writer) = connection.writer.as_mut() {
                writer.finish()?;
            }
        }
        self.stop_network_bridge()?;
        Ok(())
    }

    fn stop_network_bridge(&self) -> Result<(), String> {
        let mut bridge = self
            .network_bridge
            .lock()
            .map_err(|_| "model_native_proxy_state_unavailable")?;
        if let Some(current) = bridge.as_ref() {
            current.shutdown_checked()?;
        }
        *bridge = None;
        Ok(())
    }

    pub(crate) fn has_running_connection(&self) -> Result<bool, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "model_auth_state_unavailable")?;
        match connection.as_mut() {
            Some(connection) => Ok(connection
                .child
                .try_wait()
                .map_err(|_| "model_auth_process_unconfirmed")?
                .is_none()),
            None => Ok(false),
        }
    }

    pub(crate) fn with_launch_registration(mut self, registration: LaunchRegistration) -> Self {
        self.launch_registration = Some(registration);
        self
    }

    pub(crate) fn with_execution_scope(
        mut self,
        scope: Arc<crate::model_connections::ExecutionScope>,
    ) -> Self {
        let registration = Arc::clone(&scope);
        self.launch_registration = Some(Arc::new(move |pid| registration.register_process(pid)));
        self.launch_scope = Some(scope);
        self
    }

    pub(crate) fn launch_attempted(&self) -> bool {
        self.launch_attempted
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn new(executable: PathBuf, home: PathBuf) -> Self {
        Self {
            executable,
            home,
            connection: Mutex::new(None),
            history_server: None,
            maintenance_writer: Mutex::new(None),
            maintenance_only: false,
            import_lock: Mutex::new(()),
            archive_states: Mutex::new(std::collections::BTreeMap::new()),
            provider_aliases: Vec::new(),
            session_source: "app-server",
            profile_dir: None,
            connection_epoch: std::sync::atomic::AtomicU64::new(0),
            network: contexthub_protocol::ModelNetworkConfig::default(),
            credential_vault_root: None,
            network_route: std::sync::OnceLock::new(),
            network_guard: None,
            network_bridge: Mutex::new(None),
            runtime_managed_chatgpt: false,
            home_isolation: std::sync::OnceLock::new(),
            launch_registration: None,
            launch_scope: None,
            launch_attempted: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub(crate) fn with_runtime_managed_chatgpt(mut self) -> Self {
        self.runtime_managed_chatgpt = true;
        self
    }

    pub(crate) fn with_network(mut self, network: contexthub_protocol::ModelNetworkConfig) -> Self {
        self.network = network;
        self
    }

    pub(crate) fn with_credential_vault_root(mut self, root: PathBuf) -> Self {
        self.credential_vault_root = Some(root);
        self
    }

    pub(crate) fn with_network_guard(
        mut self,
        guard: crate::model_connections::native_proxy::Admission,
    ) -> Self {
        self.network_guard = Some(guard);
        self
    }

    pub(crate) fn with_profile_dir(mut self, data_dir: PathBuf) -> Self {
        self.profile_dir = Some(data_dir);
        self
    }

    pub(crate) fn for_runtime_scope(&self, scope: &Value) -> Result<Arc<Self>, String> {
        let scope: contexthub_protocol::ModelThreadRuntimeScope =
            serde_json::from_value(scope.clone()).map_err(|_| "model_thread_scope_invalid")?;
        if scope.runtime_home_scope
            != contexthub_protocol::model_runtime_home_scope(
                &scope.profile_id,
                &scope.connection_id,
                scope.auth_generation,
            )
        {
            return Err("model_thread_scope_invalid".into());
        }
        let kind = match scope.runtime_kind.as_str() {
            "workbench" => crate::model_connections::ModelRuntimeKind::Workbench,
            "application" => crate::model_connections::ModelRuntimeKind::Application,
            _ => return Err("model_thread_scope_invalid".into()),
        };
        let profile = self
            .profile_dir
            .as_ref()
            .ok_or("model_thread_profile_unavailable")?;
        let mut service = Self::new(
            self.executable.clone(),
            crate::model_connections::scoped_home(profile, &scope.runtime_home_scope, kind),
        )
        .with_profile_dir(profile.clone());
        if let Some(server) = &self.history_server {
            service = service.with_history_server(server.clone());
        }
        Ok(Arc::new(service))
    }

    pub(crate) fn with_provider_aliases(mut self, aliases: Vec<String>) -> Self {
        self.provider_aliases = aliases;
        self
    }

    pub(crate) fn with_session_source(mut self, source: &'static str) -> Self {
        self.session_source = source;
        self
    }

    pub(crate) fn start_workbench_thread(
        &self,
        workspace: &str,
        preferences: crate::memory::MemoryPreferences,
    ) -> Result<Value, String> {
        self.request("thread/start", &json!({
            "cwd": workspace, "approvalPolicy": "never", "sandbox": "read-only", "ephemeral": false,
            "config": {
                "memories.generate_memories": preferences.enabled,
                "memories.use_memories": false,
                "memories.disable_on_external_context": !preferences.from_tool_chats
            }
        }))
    }

    pub(crate) fn with_history_server(
        mut self,
        server: crate::supervisor::AppServerSupervisor,
    ) -> Self {
        self.history_server = Some(server);
        self
    }

    pub(crate) fn with_memory_reset_writer(
        mut self,
        server: crate::supervisor::AppServerSupervisor,
        lease: contexthub_protocol::PersonalWriterLease,
    ) -> Result<Self, String> {
        if lease.kind != "native_memory_reset"
            || lease.state != contexthub_protocol::PersonalWriterState::Prepared
            || lease.process_mode != contexthub_protocol::PersonalWriterMode::LocalProcess
            || lease.launch_protocol != 1
            || !lease.processes.is_empty()
        {
            return Err("personal_memory_reset_writer_invalid".into());
        }
        self.maintenance_only = true;
        self.maintenance_writer = Mutex::new(Some(NativeWriterLease {
            server,
            lease,
            launched: false,
            released: false,
            release_attempted: false,
        }));
        Ok(self)
    }

    fn context_store(
        &self,
        action: &str,
        thread_id: &str,
        turns: Option<&Value>,
    ) -> Result<Value, String> {
        let server = self
            .history_server
            .as_ref()
            .ok_or("ai_context_store_unavailable")?;
        let home = self
            .home
            .canonicalize()
            .map_err(|_| "ai_context_home_unavailable")?;
        let payload = json!({"scope":crate::persistent_threads::digest(&json!(home)),"threadId":thread_id,"turns":turns});
        server
            .request_result(
                "aiHistory/execute",
                &json!({"action":action,"payload":payload}),
            )?
            .get("value")
            .cloned()
            .ok_or_else(|| "ai_context_store_response_invalid".into())
    }

    /// 导入回执和显示结构属于工作台，不扩展上游协议。未知结果禁止重复注入。
    pub(crate) fn import_history(&self, params: &Value) -> Result<Value, String> {
        let _guard = self
            .import_lock
            .lock()
            .map_err(|_| "ai_context_import_lock_failed")?;
        let thread = params["threadId"]
            .as_str()
            .ok_or("ai_context_import_thread_invalid")?;
        let turns = params["turns"]
            .as_array()
            .ok_or("ai_context_import_turns_invalid")?;
        let (_, items) = crate::context_import::project(turns)?;
        let name = params["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .ok_or("ai_context_import_name_invalid")?;
        if turns.is_empty() {
            self.request("thread/name/set", &json!({"threadId":thread,"name":name}))?;
            self.shutdown();
            return Ok(json!({"threadId":thread,"turnCount":0,"messageCount":0}));
        }
        let receipt = self.context_store("contextPrepare", thread, Some(&params["turns"]))?;
        if receipt["status"] != "committed" && receipt["replayed"] == true {
            return Err("ai_context_import_delivery_uncertain".into());
        }
        // 公开命名接口保证首次真实消息前线程可恢复；沿用业务名称。
        self.request("thread/name/set", &json!({"threadId":thread,"name":name}))?;
        if receipt["status"] != "committed" {
            self.request(
                "thread/inject_items",
                &json!({"threadId":thread,"items":items}),
            )?;
            self.request("thread/name/set", &json!({"threadId":thread,"name":name}))?;
            self.context_store("contextCommit", thread, Some(&params["turns"]))?;
        }
        // 注入已确认后回收写入进程，后续业务读取与前台执行不会争用原生写入锁。
        self.shutdown();
        Ok(
            json!({"threadId":thread,"turnCount":turns.len(),"messageCount":turns.iter().filter_map(|turn|turn["messages"].as_array()).map(Vec::len).sum::<usize>()}),
        )
    }

    /// 分叉时保留迁入历史的显示结构；编辑迁入轮次时同时返回应注入的上下文前缀。
    pub(crate) fn imported_context_before(
        &self,
        thread: &str,
        before: &str,
    ) -> Result<Option<ImportedFork>, String> {
        let state = self.context_store("contextGet", thread, None)?;
        if state.is_null() {
            return Ok(None);
        }
        if state["status"] != "committed" {
            return Err("ai_context_import_delivery_uncertain".into());
        }
        let turns = state["turns"]
            .as_array()
            .ok_or("ai_context_import_invalid")?;
        if let Some(position) = turns.iter().position(|turn| turn["id"] == before) {
            let prefix = turns[..position].to_vec();
            let (_, context) = crate::context_import::project(&prefix)?;
            Ok(Some(ImportedFork {
                synthetic: true,
                display: prefix,
                context,
            }))
        } else {
            // 原生分叉已保留模型历史；工作台只复制自己拥有的显示结构。
            Ok(Some(ImportedFork {
                synthetic: false,
                display: turns.clone(),
                context: Vec::new(),
            }))
        }
    }

    /// 读取线程的全部轮次（含每轮条目），按时间先后排列。
    pub(crate) fn turns(&self, thread_id: &str) -> Result<Vec<Value>, String> {
        let mut turns = Vec::new();
        let mut cursor = Value::Null;
        let mut cursors = HashSet::new();
        loop {
            let page = self.request(
                "thread/turns/list",
                &json!({"threadId": thread_id, "cursor": cursor, "limit": TURN_PAGE_LIMIT, "sortDirection": "asc", "itemsView": "full"}),
            )?;
            let (items, next) = turn_page(&page)?;
            turns.extend(items.iter().cloned());
            cursor = next.clone();
            if cursor.is_null() {
                if self.history_server.is_some() {
                    let stored = self.context_store("contextGet", thread_id, None)?;
                    if !stored.is_null() {
                        if stored["status"] != "committed" {
                            return Err("ai_context_import_delivery_uncertain".into());
                        }
                        let (mut imported, _) = crate::context_import::project(
                            stored["turns"]
                                .as_array()
                                .ok_or("ai_context_import_invalid")?,
                        )?;
                        imported.extend(turns);
                        turns = imported;
                    }
                }
                return Ok(turns);
            }
            if !cursors.insert(cursor.to_string()) {
                return Err("codex_thread_service_cursor_repeated".to_owned());
            }
        }
    }

    pub(crate) fn request(&self, method: &str, params: &Value) -> Result<Value, String> {
        if self.maintenance_only && method != "memory/reset" {
            return Err("personal_memory_reset_method_not_authorized".into());
        }
        let _import_guard = if method == "thread/delete" {
            Some(
                self.import_lock
                    .lock()
                    .map_err(|_| "ai_context_import_lock_failed")?,
            )
        } else {
            None
        };
        if method == "thread/delete" {
            let thread = params["threadId"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or("ai_context_import_thread_invalid")?;
            return delete_context_before_native(
                || {
                    if self.history_server.is_some() {
                        self.context_store("contextDelete", thread, None)?;
                    }
                    Ok(())
                },
                || self.request_native(method, params),
            );
        }
        self.request_native(method, params)
    }

    fn request_native(&self, method: &str, params: &Value) -> Result<Value, String> {
        let mut guard = self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(self.start()?);
        }
        let connection = guard.as_mut().ok_or("codex_thread_service_unavailable")?;
        let result = connection.call(method, params);
        if result.is_ok()
            && let Some(thread) = params["threadId"].as_str()
        {
            if matches!(method, "thread/archive" | "thread/unarchive") {
                self.archive_states
                    .lock()
                    .map_err(|_| "codex_thread_catalog_lock_failed")?
                    .insert(thread.to_owned(), method == "thread/archive");
            } else if method == "thread/delete" {
                self.archive_states
                    .lock()
                    .map_err(|_| "codex_thread_catalog_lock_failed")?
                    .remove(thread);
            }
        }
        if result
            .as_ref()
            .is_err_and(|error| error.starts_with("codex_thread_service_"))
        {
            // 进程失去响应或已退出：回收，下一次请求重新启动
            *guard = None;
        }
        result
    }

    fn runtime_home_isolation(
        &self,
    ) -> Result<
        Option<&contexthub_platform_adapter::macos_runtime_isolation::ChatgptIsolationSnapshot>,
        String,
    > {
        self.home_isolation.get_or_init(|| {
            let snapshot = if let Some(scope) = &self.launch_scope {
                scope.home_isolation().cloned()
            } else if self.runtime_managed_chatgpt {
                contexthub_platform_adapter::macos_runtime_isolation::ChatgptIsolationSnapshot::capture()
                    .map_err(|_| "model_runtime_home_isolation_unavailable".to_owned())?
            } else {
                None
            };
            snapshot.map(|snapshot| snapshot.with_file_access(&[], std::slice::from_ref(&self.home))
                .map_err(|_| "model_runtime_file_permissions_invalid".to_owned())).transpose()
        }).as_ref().map(Option::as_ref).map_err(Clone::clone)
    }

    fn prepare_local_auth_home(&self) -> Result<(), String> {
        contexthub_platform_adapter::security::ensure_restricted_directory(&self.home)
            .map_err(|_| "codex_thread_service_home_failed")?;
        contexthub_platform_adapter::security::ensure_owner_writable_runtime_tree(&self.home)
            .map_err(|_| "codex_thread_service_home_failed")?;
        if self
            .home
            .join("auth.json")
            .try_exists()
            .map_err(|_| "model_credential_access_failed")?
        {
            contexthub_platform_adapter::security::verify_current_user_private_file(
                &self.home.join("auth.json"),
            )
            .map_err(|_| "model_credential_access_failed")?;
        }
        Ok(())
    }

    fn command(&self) -> Result<Command, String> {
        self.prepare_local_auth_home()?;
        let isolation = self.runtime_home_isolation()?;
        if isolation.is_some() && self.launch_registration.is_none() {
            return Err("model_runtime_home_isolation_gate_required".into());
        }
        let mut command = if self.launch_registration.is_some()
            || self.history_server.is_some()
            || self.maintenance_only
        {
            let mut command = Command::new(crate::model_connections::launch_gate_executable()?);
            command.arg(contexthub_platform_adapter::runtime_launch_gate::COMMAND);
            if isolation.is_some() {
                command.arg("--protected-home-v1");
            }
            command.arg("--").arg(&self.executable);
            command
        } else {
            Command::new(&self.executable)
        };
        crate::codex_upgrade::configure_aliases(&mut command, &self.home)?;
        for alias in &self.provider_aliases {
            crate::codex_upgrade::add_alias(&mut command, alias)?;
        }
        crate::model_connections::isolate_runtime_environment(&mut command, &self.home)?;
        if let Some(isolation) = isolation {
            isolation
                .apply_environment(&mut command)
                .map_err(|_| "model_runtime_home_isolation_invalid")?;
        }
        let route = if let Some(scope) = &self.launch_scope {
            scope.network()?
        } else {
            self.network_route
                .get_or_init(|| match &self.credential_vault_root {
                    Some(root) => crate::model_connections::resolve_network(&self.network, root),
                    None => {
                        crate::model_connections::resolve_network_without_credentials(&self.network)
                    }
                })
                .as_ref()
                .map_err(Clone::clone)?
        };
        if let Some(scope) = &self.launch_scope {
            scope.apply_native_network(&mut command)?;
        } else if route.requires_native_bridge() {
            let mut bridge = self
                .network_bridge
                .lock()
                .map_err(|_| "model_native_proxy_state_unavailable")?;
            if bridge.is_none() {
                let guard = self
                    .network_guard
                    .clone()
                    .ok_or("model_native_proxy_admission_required")?;
                *bridge = Some(Arc::new(
                    crate::model_connections::native_proxy::NativeProxy::start(
                        route.clone(),
                        crate::model_connections::native_proxy::allowed_targets(
                            &contexthub_protocol::ModelConnectionKind::Chatgpt,
                            None,
                        )?,
                        guard,
                    )?,
                ));
            }
            bridge
                .as_ref()
                .ok_or("model_native_proxy_start_failed")?
                .apply(route, &mut command);
        } else {
            route.apply_native(&mut command);
        }
        route.apply_native_certificate(&mut command, &self.home)?;
        command
            .current_dir(&self.home)
            .env("CODEX_HOME", &self.home)
            .env("CODEX_SQLITE_HOME", &self.home)
            .env(crate::windows_sandbox::REMOTE_CONTROL_DISABLED_ENV, "1")
            .args(["--listen", "stdio://"])
            .args(["--session-source", self.session_source])
            .args(["-c", "check_for_update_on_startup=false"])
            .args(["-c", "cli_auth_credentials_store=\"file\""])
            .args(["-c", "features.secret_auth_storage=false"])
            .args(["-c", "features.system_proxy_fallback=false"])
            .args(["-c", "features.memories=false"])
            .args(["-c", "memories.generate_memories=false"])
            .args(["-c", "memories.use_memories=false"])
            .args(["-c", "features.plugins=false"])
            .args(["-c", "skills.bundled.enabled=false"])
            .args(["-c", "project_doc_max_bytes=0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        Ok(command)
    }

    fn start(&self) -> Result<Connection, String> {
        let mut writer = if self.maintenance_only {
            Some(
                self.maintenance_writer
                    .lock()
                    .map_err(|_| "personal_memory_reset_writer_unavailable")?
                    .take()
                    .ok_or("personal_memory_reset_writer_consumed")?,
            )
        } else if self.launch_scope.is_none() {
            self.history_server
                .as_ref()
                .map(|server| {
                    NativeWriterLease::prepare(
                        server,
                        &self.home,
                        if self.launch_registration.is_some() {
                            "authentication"
                        } else {
                            "native_history"
                        },
                    )
                })
                .transpose()?
        } else {
            None
        };
        let mut command = self.command()?;
        contexthub_platform_adapter::process::prepare_command_for_supervision(&mut command);
        self.launch_attempted
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(scope) = &self.launch_scope {
            scope.mark_launch_attempted();
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.launch_attempted
                    .store(false, std::sync::atomic::Ordering::Release);
                if let Some(scope) = &self.launch_scope {
                    scope.mark_spawn_failed();
                }
                return Err(format!("codex_thread_service_spawn_failed:{error}"));
            }
        };
        if let Some(writer) = writer.as_mut() {
            writer.launched = true;
        }
        if let Err(error) =
            contexthub_platform_adapter::process::supervise_spawned_child(&mut child)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("codex_thread_service_supervision_failed:{error}"));
        }
        let stdin = child
            .stdin
            .take()
            .ok_or("codex_thread_service_stdio_unavailable")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("codex_thread_service_stdio_unavailable")?;
        let (sender, lines) = mpsc::channel();
        let mut connection = Connection {
            child,
            stdin,
            lines,
            next_id: 1,
            login_notifications: Vec::new(),
            writer,
        };
        if let Some(register) = &self.launch_registration {
            register(connection.child.id())?;
        }
        if let Some(writer) = connection.writer.as_ref() {
            writer.register(connection.child.id())?;
        }
        if self.launch_registration.is_some() || connection.writer.is_some() {
            connection
                .stdin
                .write_all(contexthub_platform_adapter::runtime_launch_gate::PERMIT)
                .and_then(|()| connection.stdin.flush())
                .map_err(|_| "personal_writer_launch_unconfirmed")?;
        }
        std::thread::Builder::new()
            .name("codex-thread-service".to_owned())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("codex_thread_service_spawn_failed:{error}"))?;
        self.verify_isolated_connection(&mut connection)?;
        self.connection_epoch
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Ok(connection)
    }
    fn verify_isolated_connection(&self, connection: &mut Connection) -> Result<(), String> {
        connection.call(
            "initialize",
            &json!({
                "clientInfo": {"name": "contexthub_desktop", "title": "词来工作台", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": true}
            }),
        )?;
        connection.notify("initialized")?;
        let config = connection
            .call(
                "config/read",
                &json!({"includeLayers":true,"cwd":self.home}),
            )
            .map_err(|_| "model_runtime_config_unconfirmed")?;
        crate::model_connections::isolation::validate_configuration(&config)?;
        crate::model_connections::isolation::validate_tool_environment(&config, &self.home)?;
        crate::model_connections::isolation::validate_mcp(
            &config,
            &std::collections::BTreeMap::new(),
        )?;
        let skills = connection
            .call(
                "skills/list",
                &json!({"cwds":[self.home],"forceReload":true}),
            )
            .map_err(|_| "model_runtime_skills_unconfirmed")?;
        crate::model_connections::isolation::validate_skills(&skills, &[])?;
        Ok(())
    }
}

impl Connection {
    fn notify(&mut self, method: &str) -> Result<(), String> {
        writeln!(self.stdin, "{}", json!({"method": method, "params": {}}))
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("codex_thread_service_write_failed:{error}"))
    }

    fn call(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"id": id, "method": method, "params": params})
        )
        .and_then(|()| self.stdin.flush())
        .map_err(|error| format!("codex_thread_service_write_failed:{error}"))?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(remaining) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!("codex_thread_service_timeout:{method}"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("codex_thread_service_exited".to_owned());
                }
            };
            let message = serde_json::from_str::<Value>(&line)
                .map_err(|_| "codex_thread_service_response_invalid".to_owned())?;
            if message["method"] == "account/login/completed" {
                self.login_notifications.push(message.clone());
            }
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!(
                    "codex_thread_request_failed:{method}:{}",
                    error["message"].as_str().unwrap_or("unknown")
                ));
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| "codex_thread_service_response_invalid".to_owned());
        }
    }
}

fn turn_page(page: &Value) -> Result<(&[Value], &Value), String> {
    let data = page["data"]
        .as_array()
        .ok_or("codex_thread_service_page_invalid")?;
    let cursor = page
        .get("nextCursor")
        .filter(|value| value.is_null() || value.as_str().is_some_and(|text| !text.is_empty()))
        .ok_or("codex_thread_service_page_invalid")?;
    if data.iter().any(|turn| {
        turn["id"].as_str().is_none() || turn["itemsView"] != "full" || !turn["items"].is_array()
    }) {
        return Err("codex_thread_service_items_incomplete".to_owned());
    }
    Ok((data, cursor))
}

pub(crate) fn user_text(item: &Value) -> String {
    item["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["type"] == "text")
        .filter_map(|part| part["text"].as_str().map(str::to_owned))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 一轮中模型最后给出的回复正文。
pub(crate) fn turn_final_reply(turn: &Value) -> Option<String> {
    turn["items"]
        .as_array()?
        .iter()
        .rev()
        .find(|item| {
            item["type"] == "agentMessage"
                && (item["phase"] == "final_answer" || item["phase"].is_null())
        })
        .and_then(|item| item["text"].as_str())
        .map(str::to_owned)
}

/// 以投递回执映射重试与补充输入，不把消息编号直接当作客户端投递编号。
pub(crate) fn compare_run(
    turns: &[Value],
    journal: &Value,
    run_id: &str,
    thread_id: &str,
    expected_users: &[(String, String)],
    expected_reply: Option<&str>,
    nonce: Option<&str>,
) -> Result<(), String> {
    let operations = journal["operations"]
        .as_array()
        .ok_or("thread_journal_invalid")?;
    let mut run_turn = None;
    let mut client_ids = HashSet::new();
    for (message_id, text) in expected_users {
        let accepted = operations
            .iter()
            .filter(|op| {
                op["state"] == "acknowledged"
                    && op["request"]["runId"] == run_id
                    && matches!(op["request"]["kind"].as_str(), Some("send" | "steer"))
                    && op["request"]["input"]["messageId"] == *message_id
            })
            .collect::<Vec<_>>();
        let [operation] = accepted.as_slice() else {
            return Err("message_receipt_missing_or_duplicate".to_owned());
        };
        let receipt = &operation["receipt"];
        if receipt["nativeThreadId"] != thread_id {
            return Err("thread_mapping_mismatch".to_owned());
        }
        let client_id = receipt["clientMessageId"]
            .as_str()
            .ok_or("client_message_id_missing")?;
        let turn_id = receipt["nativeTurnId"].as_str().ok_or("turn_id_missing")?;
        if run_turn.is_some_and(|id| id != turn_id) {
            return Err("run_turn_mismatch".to_owned());
        }
        run_turn = Some(turn_id);
        client_ids.insert(client_id);
        let matches = turns
            .iter()
            .filter(|turn| turn["id"] == turn_id)
            .collect::<Vec<_>>();
        let [turn] = matches.as_slice() else {
            return Err("turn_missing_or_duplicate".to_owned());
        };
        if turn["status"] != "completed" {
            return Err("turn_not_completed".to_owned());
        }
        let users = turn["items"]
            .as_array()
            .ok_or("turn_items_invalid")?
            .iter()
            .filter(|item| item["type"] == "userMessage" && item["clientId"] == client_id)
            .collect::<Vec<_>>();
        let [user] = users.as_slice() else {
            return Err("user_message_missing_or_duplicate".to_owned());
        };
        if user_text(user) != *text {
            return Err("user_text_mismatch".to_owned());
        }
    }
    let turn_id = run_turn.ok_or("turn_missing")?;
    let turn = turns
        .iter()
        .find(|turn| turn["id"] == turn_id)
        .ok_or("turn_missing")?;
    let user_count = turn["items"]
        .as_array()
        .ok_or("turn_items_invalid")?
        .iter()
        .filter(|item| item["type"] == "userMessage")
        .count();
    if user_count != client_ids.len() {
        return Err("user_message_count_mismatch".to_owned());
    }
    let raw = turn_final_reply(turn).ok_or("reply_missing")?;
    let candidates = if let Some(nonce) = nonce {
        // 既有记录按保存时的规则生成，显示规则后来增加的过期信封隐藏不得使其失配。
        vec![
            crate::work_classification::visible_reply(&raw, nonce, false)?
                .ok_or("reply_missing")?,
            crate::work_classification::saved_visible_reply(&raw, nonce)?.ok_or("reply_missing")?,
        ]
    } else {
        vec![raw]
    };
    if !candidates
        .iter()
        .any(|actual| Some(actual.trim()) == expected_reply.map(str::trim))
    {
        return Err("reply_mismatch".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CodexThreadService, compare_run, delete_context_before_native, turn_page};
    use serde_json::json;

    #[test]
    fn verifies_retry_and_steer_receipts_and_decodes_reply_exactly() {
        let mut turns = vec![json!({"id":"t","status":"completed","items":[
            {"type":"userMessage","clientId":"retry","content":[{"type":"text","text":"原文"}]},
            {"type":"userMessage","clientId":"steer","content":[{"type":"text","text":"补充"}]},
            {"type":"agentMessage","phase":"commentary","text":"处理中"},
            {"type":"agentMessage","phase":"final_answer","text":"{\"schemaVersion\":1,\"nonce\":\"n\",\"reply\":\"第一行\\n第二行\"}"}
        ]})];
        let journal = json!({"operations":[
            {"state":"acknowledged","request":{"runId":"r","kind":"send","input":{"messageId":"m"}},"receipt":{"nativeThreadId":"thread","nativeTurnId":"t","clientMessageId":"retry"}},
            {"state":"acknowledged","request":{"runId":"r","kind":"steer","input":{"messageId":"s"}},"receipt":{"nativeThreadId":"thread","nativeTurnId":"t","clientMessageId":"steer"}}
        ]});
        let users = vec![
            ("m".to_owned(), "原文".to_owned()),
            ("s".to_owned(), "补充".to_owned()),
        ];
        assert!(
            compare_run(
                &turns,
                &journal,
                "r",
                "thread",
                &users,
                Some("第一行\n第二行"),
                Some("n")
            )
            .is_ok()
        );
        assert_eq!(
            compare_run(
                &turns,
                &journal,
                "r",
                "thread",
                &users,
                Some("第二行"),
                Some("n")
            )
            .unwrap_err(),
            "reply_mismatch"
        );
        turns[0]["items"][0]["content"][0]["text"] = json!("原文\n附加说明");
        assert_eq!(
            compare_run(
                &turns,
                &journal,
                "r",
                "thread",
                &users,
                Some("第一行\n第二行"),
                Some("n")
            )
            .unwrap_err(),
            "user_text_mismatch"
        );
    }

    #[test]
    fn refuses_missing_or_summary_pages() {
        assert!(turn_page(&json!({"data":[],"nextCursor":null})).is_ok());
        assert!(turn_page(&json!({"data":[]})).is_err());
        assert!(
            turn_page(
                &json!({"data":[{"id":"t","items":[],"itemsView":"summary"}],"nextCursor":null})
            )
            .is_err()
        );
    }

    #[test]
    fn verifies_ordinary_json_using_the_live_reply_format() {
        let journal = json!({"operations":[{"state":"acknowledged","request":{"runId":"r","kind":"send","input":{"messageId":"m"}},"receipt":{"nativeThreadId":"thread","nativeTurnId":"t","clientMessageId":"client-m"}}]});
        for raw in [r#"{"reply":"用户数据"}"#, r#"{"status":"ok"}"#] {
            let turns = vec![json!({"id":"t","status":"completed","items":[
                {"type":"userMessage","clientId":"client-m","content":[{"type":"text","text":"问题"}]},
                {"type":"agentMessage","phase":"final_answer","text":raw}
            ]})];
            assert!(
                compare_run(
                    &turns,
                    &journal,
                    "r",
                    "thread",
                    &[("m".into(), "问题".into())],
                    Some(raw),
                    Some("n")
                )
                .is_ok()
            );
            assert_eq!(
                compare_run(
                    &turns,
                    &journal,
                    "r",
                    "thread",
                    &[("m".into(), "问题".into())],
                    Some("用户数据"),
                    Some("n")
                )
                .unwrap_err(),
                "reply_mismatch"
            );
        }
    }

    #[test]
    fn deletion_clears_context_before_native_dispatch_and_keeps_failures_unacknowledged() {
        use std::cell::Cell;
        let has_context = Cell::new(true);
        let native_calls = Cell::new(0);
        let failed_persistence = delete_context_before_native(
            || Err("persistence_unavailable".into()),
            || {
                native_calls.set(native_calls.get() + 1);
                Ok(json!({}))
            },
        );
        assert_eq!(failed_persistence.unwrap_err(), "persistence_unavailable");
        assert!(has_context.get());
        assert_eq!(native_calls.get(), 0);
        let failed_native = delete_context_before_native(
            || {
                has_context.set(false);
                Ok(())
            },
            || {
                native_calls.set(native_calls.get() + 1);
                Err("native_delete_unconfirmed".into())
            },
        );
        assert_eq!(failed_native.unwrap_err(), "native_delete_unconfirmed");
        assert!(!has_context.get());
        assert_eq!(native_calls.get(), 1);
        let retried = delete_context_before_native(
            || {
                has_context.set(false);
                Ok(())
            },
            || {
                native_calls.set(native_calls.get() + 1);
                Ok(json!({"deleted":true}))
            },
        )
        .unwrap();
        assert_eq!(retried["deleted"], true);
        assert!(!has_context.get());
        assert_eq!(native_calls.get(), 2);
    }

    #[test]
    fn real_reader_reuses_its_process_and_recovers_only_on_the_next_request() {
        let root = std::env::temp_dir().join(format!("cilai-reader-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let executable = crate::supervisor::codex_harness_test_binary();
        let service = CodexThreadService::new(executable, root.clone());
        assert!(service.request("thread/loaded/list", &json!({})).is_ok());
        let pid = service
            .connection
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .child
            .id();
        assert!(service.request("thread/loaded/list", &json!({})).is_ok());
        assert_eq!(
            pid,
            service
                .connection
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .child
                .id()
        );
        {
            let mut guard = service.connection.lock().unwrap();
            let child = &mut guard.as_mut().unwrap().child;
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert!(service.request("thread/loaded/list", &json!({})).is_err());
        assert!(service.connection.lock().unwrap().is_none());
        assert!(service.request("thread/loaded/list", &json!({})).is_ok());
        drop(service);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workbench_metadata_uses_an_official_memory_source_without_starting_a_turn() {
        let root =
            std::env::temp_dir().join(format!("cilai-reader-memory-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let service =
            CodexThreadService::new(crate::supervisor::codex_harness_test_binary(), root.clone())
                .with_session_source("vscode");
        for preferences in [
            crate::memory::MemoryPreferences::default(),
            crate::memory::MemoryPreferences::disabled(),
        ] {
            let created = service
                .start_workbench_thread(root.to_str().unwrap(), preferences)
                .unwrap();
            assert_eq!(created["thread"]["source"], "vscode");
            assert!(created["thread"]["turns"].as_array().unwrap().is_empty());
        }
        let settings = service.request("config/read", &json!({})).unwrap();
        assert_eq!(settings["config"]["memories"]["generate_memories"], false);
        assert_eq!(settings["config"]["memories"]["use_memories"], false);
        drop(service);
        std::fs::remove_dir_all(root).unwrap();
    }
}
