//! 新宿主壳的业务服务进程。
//!
//! 渲染底座统一后，业务逻辑不再编译进宿主壳，而是作为独立进程由壳启动并经标准输入输出
//! 通信。这样做有两个理由：业务层的十二个 crate 本就不依赖任何界面框架，让它们留在进程
//! 外可以避免在迁移期同时改变接入方式；壳崩溃或替换时业务进程的实现不受牵动。
//!
//! 协议为按行分隔的 JSON：请求 `{"id":number,"command":string,"args":object}`，
//! 响应 `{"id":number,"result":value}` 或 `{"id":number,"error":string}`。
//! 选择行分隔而非长度前缀，是为了让人能直接用管道观察与复现。
//!
//! 同一对管道还承载反向调用：业务侧需要壳去操作页面时写出
//! `{"hostRequest":number,"command":string,"args":object}`，壳以
//! `{"hostResponse":number,"result":value}` 或 `{"hostResponse":number,"error":string}` 回执。
//! 两个方向靠字段区分。

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// 业务逻辑在共用的业务库里，两个壳引同一份。此前这里以源码路径穿透引用现有壳的内部
// 模块，并为其中取不到的部分携带副本；提取为库之后两者都不再需要。
use contexthub_workbench_runtime::{
    ai_runtime, app_bridge, desktop_rpc, personal_runtime, supervisor, workspace,
};

use ai_runtime::AiRuntimeManager;
use supervisor::{AppServerPhase, AppServerStatus, AppServerSupervisor};

mod headless_data;
mod macos_disk_access;
mod mcp_connections;
mod model_connections;
mod personal_applications;
mod personal_data;
mod personal_memory;
mod personal_namespace_storage;
mod personal_notifications;
mod personal_page_ai;
mod personal_profile;
mod personal_storage;
mod powershell_policy;
mod startup_permissions;
mod test_diagnostics;
mod workspace_permissions;

/// 壳发来的行：或是一条命令，或是一次反向调用的回执。字段不重叠，按内容区分。
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Incoming {
    HostResponse {
        #[serde(rename = "hostResponse")]
        host_response: u64,
        result: Option<serde_json::Value>,
        error: Option<String>,
    },
    Command(Request),
}

#[derive(serde::Deserialize)]
struct Request {
    id: u64,
    command: String,
    #[serde(default)]
    args: serde_json::Value,
}

struct QueuedRequest {
    request: Request,
    unified_search_id: Option<u64>,
    received_at: std::time::Instant,
}

#[derive(serde::Serialize)]
struct Response {
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// 应用服务的运行环境标识，与现有壳一致。
const DEFAULT_ENVIRONMENT: &str = if cfg!(debug_assertions) {
    "dev"
} else {
    "prod"
};

fn resolve_data_environment(value: Option<&str>) -> Result<&'static str, String> {
    match value {
        None => Ok(DEFAULT_ENVIRONMENT),
        Some("dev") => Ok("dev"),
        Some("prod") => Ok("prod"),
        Some(_) => Err("数据环境配置无效，只允许 dev 或 prod".into()),
    }
}

fn data_environment() -> Result<&'static str, String> {
    match std::env::var("CILAI_DATA_ENVIRONMENT") {
        Ok(value) => resolve_data_environment(Some(&value)),
        Err(std::env::VarError::NotPresent) => resolve_data_environment(None),
        Err(error) => Err(format!("无法读取数据环境配置：{error}")),
    }
}

fn data_dir() -> Result<PathBuf, String> {
    if let Some(directory) = HEADLESS_DATA_DIRECTORY.get() {
        return Ok(directory.clone());
    }
    if let Some(directory) = PERSONAL_DATA_DIRECTORY.get() {
        return Ok(directory.clone());
    }
    let base = contexthub_platform_adapter::system::application_data_directory()
        .map_err(|error| format!("无法定位个人版数据目录：{error}"))?;
    Ok(base.join(personal_profile::APPLICATION_ID).join("business"))
}

static PERSONAL_DATA_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

fn initialize_personal_data_directory(arguments: &[String]) -> Result<(), String> {
    if arguments.is_empty() || arguments.first().is_some_and(|v| v == "--headless-check") {
        return Ok(());
    }
    if arguments.len() != 2 || arguments[0] != "--data-dir" {
        return Err("个人版数据参数无效".into());
    }
    let directory = PathBuf::from(&arguments[1]);
    personal_profile::validate_data_directory(&directory)?;
    PERSONAL_DATA_DIRECTORY
        .set(directory)
        .map_err(|_| "个人版数据目录已初始化".into())
}

static HEADLESS_DATA_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

fn initialize_headless_data_directory(arguments: &[String]) -> Result<(), String> {
    let Some(directory) = headless_data::create(arguments, &std::env::temp_dir())? else {
        return Ok(());
    };
    contexthub_platform_adapter::security::ensure_restricted_directory(&directory)
        .map_err(|error| format!("无法保护自检数据目录：{error}"))?;
    let business = headless_data::business_directory(&directory, data_environment()?)?;
    contexthub_platform_adapter::security::ensure_restricted_directory(&business)
        .map_err(|error| format!("无法保护自检环境目录：{error}"))?;
    HEADLESS_DATA_DIRECTORY
        .set(business)
        .map_err(|_| "宿主自检目录已初始化".to_owned())
}

#[derive(Clone)]
struct ActiveProfile {
    id: String,
    data_dir: PathBuf,
}

fn active_profile() -> &'static Mutex<Option<ActiveProfile>> {
    static PROFILE: OnceLock<Mutex<Option<ActiveProfile>>> = OnceLock::new();
    PROFILE.get_or_init(|| Mutex::new(None))
}

fn current_profile() -> Result<ActiveProfile, String> {
    active_profile()
        .lock()
        .map_err(|_| "本地资料上下文不可用".to_owned())?
        .clone()
        .ok_or_else(|| "本地资料上下文尚未初始化".to_owned())
}

fn set_active_profile(profile: ActiveProfile) -> Result<(), String> {
    startup_permissions::remember_profile(&profile.id);
    *active_profile()
        .lock()
        .map_err(|_| "本地资料上下文不可用".to_owned())? = Some(profile);
    Ok(())
}

fn local_profile() -> Result<ActiveProfile, String> {
    let profile = personal_profile::load_or_create(&data_dir()?)?;
    startup_permissions::remember_profile(&profile.profile_id);
    Ok(ActiveProfile {
        id: profile.profile_id.clone(),
        data_dir: profile.directory,
    })
}

fn local_profile_status() -> Result<serde_json::Value, String> {
    if let Some(error) = current_app_server_failure() {
        return Err(error);
    }
    let active = current_profile()?;
    let profile = personal_profile::load_or_create(&data_dir()?)?;
    if active.id != profile.profile_id || active.data_dir != profile.directory {
        return Err("local_profile_changed".into());
    }
    Ok(serde_json::json!({
        "profileId": profile.profile_id,
        "displayName": profile.display_name,
        "schemaVersion": profile.schema_version,
        "ready": true,
    }))
}

fn supervisor() -> &'static AppServerSupervisor {
    static INSTANCE: OnceLock<AppServerSupervisor> = OnceLock::new();
    INSTANCE.get_or_init(AppServerSupervisor::new)
}

/// 异步方法所需的运行时。现有宿主由框架提供，新壳自建一个。
fn runtime() -> &'static tokio::runtime::Runtime {
    static INSTANCE: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("无法创建异步运行时")
    })
}

/// 宿主能力回调：受控资料操作与界面通知。
///
/// 反向请求发生在命令处理线程内，因此标准输入必须由独立线程读取——否则等回执时没有人
/// 在读管道，必然自锁。写出共用一把锁，避免两端的行相互穿插。
struct HostBridge {
    next_id: std::sync::atomic::AtomicU64,
    pending: std::sync::Mutex<
        std::collections::HashMap<u64, std::sync::mpsc::Sender<Result<serde_json::Value, String>>>,
    >,
}

/// 壳未在此时限内回执即判定失败：让调用方拿到明确错误，而不是无限期挂着。
const HOST_CALLBACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

impl HostBridge {
    fn request(
        &self,
        command: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (sender, receiver) = std::sync::mpsc::channel();
        self.pending
            .lock()
            .map_err(|_| "宿主回调表不可用".to_owned())?
            .insert(id, sender);
        let line = serde_json::json!({ "hostRequest": id, "command": command, "args": args });
        if let Err(error) = write_line(&line) {
            drop(self.pending.lock().map(|mut table| table.remove(&id)));
            return Err(error);
        }
        if let Ok(result) = receiver.recv_timeout(HOST_CALLBACK_TIMEOUT) {
            result
        } else {
            drop(self.pending.lock().map(|mut table| table.remove(&id)));
            Err(format!("宿主未在限期内回执：{command}"))
        }
    }

    fn resolve(&self, id: u64, outcome: Result<serde_json::Value, String>) {
        if let Ok(mut table) = self.pending.lock()
            && let Some(sender) = table.remove(&id)
        {
            drop(sender.send(outcome));
        }
    }
}

fn host_bridge() -> &'static HostBridge {
    static BRIDGE: OnceLock<HostBridge> = OnceLock::new();
    BRIDGE.get_or_init(|| HostBridge {
        next_id: std::sync::atomic::AtomicU64::new(1),
        pending: std::sync::Mutex::new(std::collections::HashMap::new()),
    })
}

/// 两个方向共用标准输出，写出时加锁，避免行与行交错。
fn write_line(value: &serde_json::Value) -> Result<(), String> {
    static OUT: OnceLock<std::sync::Mutex<std::io::Stdout>> = OnceLock::new();
    let out = OUT.get_or_init(|| std::sync::Mutex::new(std::io::stdout()));
    let mut stdout = out.lock().map_err(|_| "标准输出不可用".to_owned())?;
    writeln!(stdout, "{value}")
        .and_then(|()| stdout.flush())
        .map_err(|error| format!("写出失败：{error}"))
}

fn ai_manager() -> &'static AiRuntimeManager {
    static INSTANCE: OnceLock<AiRuntimeManager> = OnceLock::new();
    INSTANCE.get_or_init(AiRuntimeManager::new)
}

/// 前端按小驼峰传参，宿主按蛇形命名接收，取值时按两种写法各查一次。
fn arg<T: serde::de::DeserializeOwned>(args: &serde_json::Value, name: &str) -> Result<T, String> {
    let camel = {
        let mut out = String::with_capacity(name.len());
        let mut upper = false;
        for ch in name.chars() {
            if ch == '_' {
                upper = true;
            } else if upper {
                out.push(ch.to_ascii_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        }
        out
    };
    let value = args
        .get(name)
        .or_else(|| args.get(camel.as_str()))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    serde_json::from_value(value).map_err(|error| format!("参数 {name} 无法解析：{error}"))
}

/// PDF 命令的公共前置：路径校验与文件校验一次完成，避免各命令各写一遍而彼此放宽。
fn validated_pdf(args: &serde_json::Value) -> Result<PathBuf, String> {
    let root = workspace::validated_directory(&arg::<String>(args, "local_root")?)?;
    let item = workspace::resolved_item(&root, &arg::<String>(args, "relative_path")?)?;
    workspace::pdf::validate_file(&item)?;
    Ok(item)
}

/// 二进制回传：行协议只承载文本，字节以 base64 装入带标记的对象，由界面侧还原为
/// 字节数组。相比逐字节的数字数组，体积约为其三分之一。
fn binary(bytes: &[u8]) -> serde_json::Value {
    use base64::Engine as _;
    serde_json::json!({ "__binary": base64::engine::general_purpose::STANDARD.encode(bytes) })
}

fn reject_retired_application_context(
    selection: Option<&app_bridge::ApplicationContextSelection>,
) -> Result<(), String> {
    if selection.is_some() {
        return Err("personal_feature_removed".to_owned());
    }
    Ok(())
}

fn encode<T: serde::Serialize>(value: T) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|error| format!("结果无法编码：{error}"))
}

/// 启动或附着应用服务。附着优先：服务已在运行时不重复拉起。
fn start_app_server(data: &PathBuf) -> Result<(), String> {
    let binary = supervisor::bundled_app_server_path()
        .map_err(|error| format!("无法定位应用服务可执行文件：{error}"))?;
    let runtime = data.join("run");
    supervisor()
        .start(&binary, data_environment()?, data, &runtime)
        .map_err(|error| current_app_server_failure().unwrap_or(error))
}

fn app_server_failure_message(status: &AppServerStatus) -> Option<String> {
    if status.phase != AppServerPhase::Failed {
        return None;
    }
    let detail = status
        .stderr_tail
        .iter()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().trim_start_matches("App Server failure: "))
        .or(status.error_reason.as_deref())
        .unwrap_or("本地服务没有返回启动失败原因");
    let exit = status
        .last_exit_code
        .map(|code| format!("（退出码 {code}）"))
        .unwrap_or_default();
    Some(format!("本地服务启动失败{exit}：{detail}"))
}

fn current_app_server_failure() -> Option<String> {
    app_server_failure_message(&supervisor().status())
}

fn runtime_initialization_error() -> &'static Mutex<Option<String>> {
    static ERROR: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    ERROR.get_or_init(|| Mutex::new(None))
}

fn initialize_ai_runtime(data: &std::path::Path) -> Result<(), String> {
    let result = ai_manager().initialize(data, supervisor());
    *runtime_initialization_error()
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = result.as_ref().err().cloned();
    result
}

fn app_runtime_status() -> Result<serde_json::Value, String> {
    let status = supervisor().status();
    let data_state = personal_data::state();
    let initialization_error = runtime_initialization_error()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let profile = current_profile().ok();
    let automation = automation_runtime_summary(
        status.phase == AppServerPhase::Running
            && initialization_error.is_none()
            && data_state == "idle",
        profile.as_ref(),
        |root| ai_manager().automation_enabled_count(root),
    );
    let mut value = encode(status)?;
    value["personalDataState"] = encode(data_state)?;
    value["runtimeInitializationError"] = encode(initialization_error)?;
    value["profileId"] = encode(profile.map(|profile| profile.id))?;
    value["automationSummary"] = encode(automation)?;
    Ok(value)
}

fn automation_runtime_summary(
    ready: bool,
    profile: Option<&ActiveProfile>,
    enabled_count: impl FnOnce(&std::path::Path) -> Result<usize, String>,
) -> serde_json::Value {
    let count = profile
        .filter(|_| ready)
        .and_then(|profile| enabled_count(&profile.data_dir).ok());
    serde_json::json!({"profileId":profile.map(|profile| &profile.id),"enabledCount":count,"known":count.is_some()})
}

struct AiSendTiming {
    command: &'static str,
    started: Instant,
    previous: Instant,
}

impl AiSendTiming {
    fn new(command: &'static str, client_sent_at_ms: Option<i64>) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok());
        if let (Some(now), Some(sent)) = (now, client_sent_at_ms)
            && (0..=300_000).contains(&now.saturating_sub(sent))
        {
            eprintln!(
                "ai_send_timing command={command} stage=client_to_host elapsed_ms={}",
                now - sent
            );
        }
        let started = Instant::now();
        Self {
            command,
            started,
            previous: started,
        }
    }

    fn measure<T>(
        &mut self,
        stage: &'static str,
        action: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let result = action();
        let now = Instant::now();
        eprintln!(
            "ai_send_timing command={} stage={} status={} stage_ms={} total_ms={}",
            self.command,
            stage,
            if result.is_ok() { "ok" } else { "failed" },
            now.duration_since(self.previous).as_millis(),
            now.duration_since(self.started).as_millis(),
        );
        self.previous = now;
        result
    }
}

/// 经应用服务转发的命令：方法名与现有壳逐一对应，参数与返回结构不变。
fn app_server_method(command: &str) -> Option<&'static str> {
    Some(match command {
        "preview_open" => "preview/open",
        "text_resource_read" => "textResource/read",
        "text_diff_compute" => "textDiff/compute",
        "text_diff_read" => "textDiff/read",
        "file_confirm" => "file/confirm",
        "confirmation_history" => "file/confirmationHistory",
        _ => return None,
    })
}

fn personal_command_allowed(command: &str, args: &serde_json::Value) -> Result<(), String> {
    if command.starts_with("enterprise_")
        || command.starts_with("capture_")
        || matches!(
            command,
            "ai_gateway_credential_status"
                | "ai_approval_review_settings"
                | "ai_approval_review_model_save"
                | "personal_capability_submit"
                | "personal_capability_submissions"
                | "application_report_notifications"
                | "application_catalog_list"
                | "application_launch_web"
                | "application_launch_search_result"
                | "application_install_local"
                | "ai_browser_policy_get"
        )
    {
        return Err("personal_feature_removed".into());
    }
    if command == "app_server_request" {
        let method = args["method"].as_str().ok_or("缺少 method")?;
        if !desktop_rpc::desktop_method_allowed(method) {
            return Err("desktop_method_not_allowed".into());
        }
    }
    if matches!(
        command,
        "ai_message_submit"
            | "ai_conversation_submit"
            | "ai_conversation_start"
            | "ai_message_send"
    ) && args["request"]
        .get("applicationContext")
        .is_some_and(|context| !context.is_null())
    {
        return Err("personal_feature_removed".into());
    }
    Ok(())
}

fn handle(command: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    static DATA_ACCESS: OnceLock<std::sync::RwLock<()>> = OnceLock::new();
    personal_command_allowed(command, args)?;
    if !personal_data::command_allowed(command) {
        return Err("personal_data_restart_required".into());
    }
    let gate = DATA_ACCESS.get_or_init(|| std::sync::RwLock::new(()));
    let exclusive = matches!(
        command,
        "personal_data_backup_capture"
            | "personal_data_restore_apply"
            | "personal_storage_execute"
            | "personal_memory_reset_confirm"
            | "personal_capability_upgrade"
            | "personal_package_import_commit"
            | "personal_application_data_rollback"
            | "personal_namespace_storage_execute_internal"
            | "personal_namespace_storage_finish"
    );
    let observation = personal_page_ai::is_command(command)
        || matches!(
            command,
            "app_server_status"
                | "host_platform"
                | "test_runtime_diagnostics"
                | "personal_namespace_storage_action_begin_internal"
                | "personal_namespace_storage_action_ack_internal"
                | "personal_namespace_storage_segment_sync_internal"
                | "personal_namespace_storage_get"
                | "personal_namespace_storage_list"
                | "personal_namespace_storage_reconcile"
                | "ai_automation_trigger_history"
        );
    let _write = if exclusive {
        Some(gate.try_write().map_err(|_| "personal_data_host_busy")?)
    } else {
        None
    };
    let _read = if exclusive || observation {
        None
    } else {
        Some(
            gate.try_read()
                .map_err(|_| "personal_data_maintenance_active")?,
        )
    };
    let catalog_generation = command
        .starts_with("ai_")
        .then(|| ai_manager().model_catalog_generation());
    let result = handle_inner(command, args);
    if let Some(generation) = catalog_generation
        && let Err(reason) = &result
    {
        ai_manager().invalidate_model_catalog_for_error(generation, reason);
    }
    result
}

#[expect(
    clippy::too_many_lines,
    reason = "RPC 命令逐项分发，集中核对协议覆盖范围"
)]
fn handle_inner(command: &str, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    if personal_page_ai::is_command(command) {
        return personal_page_ai::handle(command, args);
    }
    if personal_notifications::is_command(command) {
        return personal_notifications::handle(command, args);
    }
    if let Some(method) = app_server_method(command) {
        let params = args
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        return supervisor().request_result(method, &params);
    }
    match command {
        "model_connection_catalog_refresh" => encode(ai_manager().personal_model_catalog_refresh(
            &arg::<String>(&arg::<serde_json::Value>(args, "request")?, "connection_id")?,
        )?),
        "model_connection_validate" => {
            encode(ai_manager().personal_model_validate(arg(args, "request")?)?)
        }
        "model_execution_preflight" => encode(ai_manager().personal_model_execution_preflight(
            arg(args, "request")?,
            arg(args, "runtime_kind")?,
        )?),
        "model_connection_auth_read" => {
            ai_manager().personal_model_auth_read(&arg(args, "request")?)
        }
        "model_connection_auth_start" => {
            ai_manager().personal_model_auth_start(&arg(args, "request")?)
        }
        "model_connection_auth_cancel" => {
            ai_manager().personal_model_auth_cancel(&arg(args, "request")?)
        }
        "model_connection_auth_logout" => {
            ai_manager().personal_model_auth_logout(&arg(args, "request")?)
        }
        "model_connection_auth_resume" => {
            ai_manager().personal_model_auth_resume(&arg(args, "request")?)
        }
        "model_connection_auth_finish_logout" => {
            ai_manager().personal_model_auth_finish_logout(&arg(args, "request")?)
        }
        "model_connection_auth_cancel_readonly" => {
            ai_manager().personal_model_auth_cancel_readonly(&arg(args, "request")?)
        }
        "model_connection_list"
        | "model_credential_cleanup_list"
        | "model_credential_cleanup_execute"
        | "model_connection_change_list"
        | "model_connection_auth_changes"
        | "model_connection_recover_leases"
        | "model_connection_get"
        | "model_connection_save"
        | "model_connection_resume"
        | "model_connection_abort"
        | "model_connection_credential_cleanup"
        | "model_connection_change_status"
        | "model_connection_preflight"
        | "model_connection_set_enabled"
        | "model_connection_delete"
        | "model_descriptor_list"
        | "model_descriptor_save"
        | "model_purpose_list"
        | "model_purpose_save"
        | "model_validation_list" => {
            let profile = current_profile()?;
            let result = model_connections::handle(
                command,
                args,
                &profile.id,
                &profile.data_dir,
                |method, params| supervisor().request_result(method, params),
            )?;
            Ok(model_connections::finish_metadata_change(
                command,
                result,
                || ai_manager().refresh_model_catalog().map(|_| ()),
            ))
        }
        "personal_data_backup_prepare"
        | "personal_data_backup_capture"
        | "personal_data_backup_export"
        | "personal_data_restore_prepare"
        | "personal_data_restore_workspace_plan"
        | "personal_data_restore_apply"
        | "personal_data_get"
        | "personal_data_list"
        | "personal_data_cancel"
        | "personal_data_reconcile"
        | "personal_data_resolve_interrupted"
        | "personal_maintenance_status"
        | "personal_maintenance_recover"
        | "personal_maintenance_cancel" => {
            let profile = current_profile()?;
            personal_data::handle(
                command,
                args,
                &profile.id,
                &profile.data_dir,
                &data_dir()?,
                ai_manager(),
                supervisor(),
            )
        }
        "mcp_connection_validate" => {
            let _operation = ai_manager().begin_capability_operation()?;
            encode(ai_manager().personal_mcp_validate(arg(args, "request")?)?)
        }
        "personal_memory_reset_prepare"
        | "personal_memory_reset_confirm"
        | "personal_memory_reset_get"
        | "personal_memory_reset_list"
        | "personal_memory_reset_cancel"
        | "personal_memory_reset_reconcile"
        | "personal_memory_reset_resolve_interrupted" => {
            let profile = current_profile()?;
            personal_memory::handle(command, args, &profile.data_dir, ai_manager(), supervisor())
        }
        "personal_storage_inventory"
        | "personal_storage_prepare"
        | "personal_storage_execute"
        | "personal_storage_get"
        | "personal_storage_list"
        | "personal_storage_reconcile"
        | "personal_storage_cancel"
        | "personal_storage_resolve_interrupted" => {
            let profile = current_profile()?;
            personal_storage::handle(command, args, &profile.data_dir, ai_manager(), supervisor())
        }
        "personal_namespace_storage_prepare_internal"
        | "personal_namespace_storage_segment_create_internal"
        | "personal_namespace_storage_segment_sync_internal"
        | "personal_namespace_storage_execute_internal"
        | "personal_namespace_storage_action_begin_internal"
        | "personal_namespace_storage_action_ack_internal"
        | "personal_namespace_storage_finish"
        | "personal_namespace_storage_get"
        | "personal_namespace_storage_list"
        | "personal_namespace_storage_reconcile"
        | "personal_namespace_storage_cancel"
        | "personal_namespace_storage_resolve_interrupted" => {
            let profile = current_profile()?;
            personal_namespace_storage::handle(
                command,
                args,
                &profile.data_dir,
                ai_manager(),
                supervisor(),
                &|command, args| host_bridge().request(command, args),
            )
        }
        "mcp_connection_list"
        | "mcp_connection_changes"
        | "mcp_connection_get"
        | "mcp_connection_save"
        | "mcp_connection_continue_save"
        | "mcp_connection_cancel_save"
        | "mcp_connection_cleanup"
        | "mcp_connection_grant"
        | "mcp_connection_disable"
        | "mcp_connection_delete" => {
            let _operation = ai_manager().begin_capability_operation()?;
            let profile = current_profile()?;
            mcp_connections::handle(
                command,
                args,
                &profile.id,
                &profile.data_dir,
                |method, params| supervisor().request_result(method, params),
            )
        }
        "macos_disk_access_open_settings" => macos_disk_access::open_settings(args),
        "macos_disk_access_reveal_application" => macos_disk_access::reveal_application(args),
        "host_platform" => Ok(serde_json::Value::String(
            contexthub_platform_adapter::current_operating_system()
                .as_str()
                .to_owned(),
        )),
        "app_server_status" => app_runtime_status(),
        "app_server_restart" => {
            personal_page_ai::revoke_all()?;
            supervisor().restart()?;
            initialize_ai_runtime(&current_profile()?.data_dir)?;
            app_runtime_status()
        }
        // AI 运行时：单调用转发，参数与返回结构与现有壳一致
        "ai_memory_settings" => encode(ai_manager().memory_settings()?),
        "ai_memory_settings_save" => {
            encode(ai_manager().save_personal_memory_settings(arg(args, "settings")?)?)
        }
        "ai_memory_reset" => Err("ai_memory_reset_preview_required".into()),
        "ai_windows_sandbox_status" => {
            let workspace_root: Option<String> = arg(args, "workspace_root")?;
            encode(ai_manager().windows_sandbox_status(workspace_root.as_deref()))
        }
        "powershell_policy_check" => powershell_policy::check(),
        "powershell_policy_apply" => powershell_policy::apply(&arg::<String>(args, "planId")?),
        "test_runtime_diagnostics" => Ok(test_diagnostics::collect()),
        "ai_usage_view" => ai_manager().usage_view(),
        "startup_permissions_check" => startup_permissions::check(),
        "startup_permissions_repair" => {
            startup_permissions::repair(&arg::<String>(args, "planId")?)
        }
        "ai_windows_sandbox_setup" => {
            let workspace_root: Option<String> = arg(args, "workspace_root")?;
            encode(ai_manager().windows_sandbox_setup(workspace_root.as_deref())?)
        }
        "workspace_permissions_check" => {
            workspace_permissions::check(&arg::<String>(args, "workspace_id")?)
        }
        "workspace_permissions_repair" => workspace_permissions::repair(
            &arg::<String>(args, "workspace_id")?,
            &arg::<String>(args, "plan_id")?,
        ),
        "ai_run_event_detail" => encode(ai_manager().run_event_detail(
            &arg::<String>(args, "run_id")?,
            &arg::<String>(args, "item_id")?,
        )?),
        "ai_conversation_create" => {
            encode(ai_manager().create_conversation(arg(args, "request")?)?)
        }
        "ai_conversation_set_archived" => {
            encode(ai_manager().set_conversations_archived(arg(args, "request")?)?)
        }
        "ai_conversation_update" => {
            encode(ai_manager().update_conversation(arg(args, "request")?)?)
        }
        "ai_conversation_delete" => {
            encode(ai_manager().delete_conversations(arg(args, "request")?)?)
        }

        // AI 运行时：按各自签名转发。接收引用的先绑定局部变量，异步方法经运行时执行。
        "ai_model_catalog_read" => {
            let profile = current_profile()?;
            let reason = arg(args, "reason")?;
            ai_manager().read_model_catalog(reason)?;
            let snapshot = ai_manager().model_catalog_for_profile(&profile.data_dir)?;
            if current_profile()?.id != profile.id {
                return Err("ai_profile_changed".to_owned());
            }
            Ok(serde_json::json!({ "profileId": profile.id, "catalog": snapshot }))
        }
        "ai_artifact_open" => {
            let run_id: String = arg(args, "run_id")?;
            let artifact_id: String = arg(args, "artifact_id")?;
            open_registered_artifact(
                &run_id,
                &artifact_id,
                &arg::<String>(args, "open_mode")?,
                |run, artifact| ai_manager().artifact_path(run, artifact),
                workspace::open_item,
            )?;
            Ok(serde_json::Value::Null)
        }
        "personal_capability_upgrade"
        | "personal_package_import_commit"
        | "personal_application_data_prepare"
        | "personal_application_data_get"
        | "personal_application_data_list"
        | "personal_application_data_cancel"
        | "personal_application_data_rollback"
        | "personal_application_data_resolve_interrupted" => {
            let profile = current_profile()?;
            personal_applications::handle(
                command,
                args,
                &profile.data_dir,
                ai_manager(),
                supervisor(),
            )
        }
        "personal_capability_editor" | "personal_capability_build" => {
            let _operation = ai_manager().begin_capability_operation()?;
            let method = if command == "personal_capability_editor" {
                "personalCapability/editor"
            } else {
                "personalCapability/build"
            };
            supervisor().request_result(method, &arg::<serde_json::Value>(args, "request")?)
        }
        "personal_package_import_prepare"
        | "personal_package_import_get"
        | "personal_package_import_list"
        | "personal_package_import_cancel" => {
            let _operation = ai_manager().begin_capability_operation()?;
            let method = match command {
                "personal_package_import_prepare" => "personalPackage/importPrepare",
                "personal_package_import_get" => "personalPackage/importGet",
                "personal_package_import_list" => "personalPackage/importList",
                _ => "personalPackage/importCancel",
            };
            supervisor().request_result(method, &arg::<serde_json::Value>(args, "request")?)
        }
        "personal_capability_preview" => {
            let _operation = ai_manager().begin_capability_operation()?;
            let request: serde_json::Value = arg(args, "request")?;
            let binding = supervisor().request_result("personalCapability/previewRuntime", &serde_json::json!({"creationProjectId":request["creationProjectId"],"packageDigest":request["packageDigest"]}))?;
            if binding["targetType"] == "web_application" {
                return Ok(
                    serde_json::json!({"html":binding["html"],"packageDigest":binding["packageDigest"]}),
                );
            }
            let mut result = personal_runtime::invoke_binding(
                supervisor(),
                &binding,
                request["tool"].as_str(),
                &request
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
            )?;
            if request["tool"].is_null() {
                result["html"] = binding["html"].clone();
            }
            Ok(result)
        }
        "personal_application_open" => {
            let _operation = ai_manager().begin_capability_operation()?;
            let id: String = arg(args, "creation_project_id")?;
            supervisor().request_result(
                "personalApplication/open",
                &serde_json::json!({"creationProjectId":id}),
            )
        }
        "personal_capability_status" | "personal_capability_manage" => {
            let _operation = ai_manager().begin_capability_operation()?;
            let (method, params) = if command == "personal_capability_manage" {
                (
                    "personalCapability/manage",
                    arg::<serde_json::Value>(args, "request")?,
                )
            } else {
                (
                    "personalCapability/status",
                    serde_json::json!({"creationProjectId":arg::<String>(args,"creation_project_id")?}),
                )
            };
            supervisor().request_result(method, &params)
        }
        "personal_capability_register_file"
        | "personal_capability_apply_file"
        | "personal_capability_file_write_preview" => {
            let _operation = ai_manager().begin_capability_operation()?;
            let request: serde_json::Value = arg(args, "request")?;
            request["creationProjectId"]
                .as_str()
                .ok_or("个人能力标识无效。")?;
            supervisor().request_result(
                if command == "personal_capability_register_file" {
                    "personalCapability/selectFile"
                } else if command == "personal_capability_file_write_preview" {
                    "personalCapability/fileWritePreview"
                } else {
                    "personalCapability/writeFile"
                },
                &arg::<serde_json::Value>(args, "request")?,
            )
        }
        "personal_capability_export" => {
            let _operation = ai_manager().begin_capability_operation()?;
            let package=supervisor().request_result("personalCapability/export", &serde_json::json!({"creationProjectId":arg::<String>(args,"creation_project_id")?}))?;
            let bytes: Vec<u8> = serde_json::from_value(package["bytes"].clone())
                .map_err(|error| error.to_string())?;
            let destination = std::path::PathBuf::from(arg::<String>(args, "destination_path")?);
            if !destination.is_absolute()
                || destination.is_dir()
                || std::fs::symlink_metadata(&destination)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err("个人包保存位置无效。".into());
            }
            std::fs::write(destination, bytes).map_err(|error| error.to_string())?;
            Ok(serde_json::json!({"packageDigest":package["packageDigest"]}))
        }
        "personal_capability_tools" | "personal_capability_call" => {
            let _operation = ai_manager().begin_capability_operation()?;
            let tool: Option<String> = if command == "personal_capability_call" {
                Some(arg(args, "tool")?)
            } else {
                None
            };
            let mut arguments: serde_json::Value = if tool.is_some() {
                arg(args, "arguments")?
            } else {
                serde_json::json!({})
            };
            let project_id: String = arg(args, "creation_project_id")?;
            let mut expected_digest = arg::<Option<String>>(args, "expected_digest")
                .ok()
                .flatten();
            if let Some(grant_id) = arg::<Option<String>>(args, "file_grant_id").ok().flatten() {
                let binding = supervisor().request_result(
                    "personalCapability/runtime",
                    &serde_json::json!({"creationProjectId":project_id}),
                )?;
                let digest = binding["packageDigest"]
                    .as_str()
                    .ok_or("个人能力版本不可用。")?;
                if expected_digest
                    .as_deref()
                    .is_some_and(|expected| expected != digest)
                {
                    return Err("个人能力版本已变化。".into());
                }
                let selected = supervisor().request_result("personalCapability/consumeFile", &serde_json::json!({"creationProjectId":project_id,"packageDigest":digest,"grantId":grant_id}))?;
                arguments
                    .as_object_mut()
                    .ok_or("工具参数格式无效。")?
                    .insert("selectedFile".into(), selected);
                expected_digest = Some(digest.to_owned());
            }
            personal_runtime::invoke(
                supervisor(),
                &project_id,
                tool.as_deref(),
                &arguments,
                expected_digest.as_deref(),
            )
        }
        "ai_artifact_feedback" => encode(ai_manager().artifact_feedback(
            &arg::<String>(args, "run_id")?,
            &arg::<String>(args, "artifact_id")?,
            &arg::<String>(args, "version_digest")?,
            &arg::<String>(args, "feedback_id")?,
            &arg::<String>(args, "content")?,
        )?),
        "ai_artifact_submit_candidate" => encode(ai_manager().submit_artifact_candidate(
            &arg::<String>(args, "run_id")?,
            &arg::<String>(args, "artifact_id")?,
            &arg::<String>(args, "version_digest")?,
            &arg::<String>(args, "deliverable_id")?,
            &arg::<String>(args, "summary")?,
            &arg::<String>(args, "operation_id")?,
        )?),
        "ai_artifact_read_image" => Ok(binary(&ai_manager().read_image_artifact(
            &arg::<String>(args, "run_id")?,
            &arg::<String>(args, "artifact_id")?,
        )?)),
        "ai_message_regenerate" => {
            let request: ai_runtime::MessageRegenerateRequest = arg(args, "request")?;
            let mut timing =
                AiSendTiming::new("ai_message_regenerate", arg(args, "client_sent_at_ms").ok());
            let application_selection =
                ai_manager().regeneration_application_selection(&request)?;
            reject_retired_application_context(application_selection.as_ref())?;
            encode(timing.measure("run_start", || {
                ai_manager().regenerate_message(request, None, None)
            })?)
        }
        "ai_message_steer" => {
            let request: ai_runtime::MessageSteerRequest = arg(args, "request")?;
            encode(ai_manager().steer_run_with_application_context(&request, |_| Ok((None, None)))?)
        }
        "ai_attachment_create" => encode(ai_manager().create_attachment(arg(args, "request")?)?),
        "ai_attachment_append" => {
            encode(ai_manager().append_attachment_chunk(arg(args, "request")?)?)
        }
        "ai_attachment_discard" => {
            let attachment_id: String = arg(args, "attachment_id")?;
            encode(ai_manager().discard_attachment(&attachment_id)?)
        }
        "ai_run_control" => {
            let request: ai_runtime::RunControlRequest = arg(args, "request")?;
            encode(ai_manager().control_run(&request)?)
        }
        "ai_permission_request_create" => {
            let request: ai_runtime::PermissionRequestCreateRequest = arg(args, "request")?;
            encode(ai_manager().create_permission_request(&request)?)
        }
        "ai_permission_request_resolve" => {
            encode(ai_manager().resolve_permission_request(arg(args, "request")?)?)
        }
        "ai_inbox_set_read" => {
            let request: ai_runtime::InboxReadRequest = arg(args, "request")?;
            encode(ai_manager().set_inbox_read(&request)?)
        }
        "ai_automation_rule_save" => {
            encode(ai_manager().save_automation_rule(arg(args, "request")?)?)
        }
        "ai_automation_rule_run" => {
            let request: ai_runtime::AutomationRuleRunRequest = arg(args, "request")?;
            encode(ai_manager().run_automation_rule(&request)?)
        }
        "ai_automation_rule_delete" => {
            let request: ai_runtime::AutomationRuleDeleteRequest = arg(args, "request")?;
            encode(ai_manager().delete_automation_rule(&request)?)
        }
        "ai_automation_trigger_history" => {
            encode(ai_manager().automation_trigger_history(&arg(args, "request")?)?)
        }
        "ai_automation_claim_reconcile" => {
            let request: contexthub_protocol::PersonalAutomationResolveInterruptedParams =
                arg(args, "request")?;
            encode(ai_manager().reconcile_automation_claim(&request.claim_id)?)
        }

        "local_profile_status" => local_profile_status(),

        // 对话与消息：先解析所选应用的上下文，再交由运行时，与现有壳同一条路径
        "ai_conversation_start" => {
            let request: ai_runtime::ConversationStartRequest = arg(args, "request")?;
            let mut timing =
                AiSendTiming::new("ai_conversation_start", arg(args, "client_sent_at_ms").ok());
            reject_retired_application_context(request.application_context.as_ref())?;
            encode(timing.measure("run_start", || {
                ai_manager().start_conversation_with_application_context(request, None, None, None)
            })?)
        }
        "ai_recent_conversations" => encode(ai_manager().recent_conversations()),
        "ai_conversation_session_get" => encode(ai_manager().conversation_session()?),
        "ai_application_handoff_cancel" => {
            encode(ai_manager().cancel_application_handoff(&arg(args, "request")?)?)
        }
        "ai_application_handoff_commit" => {
            encode(ai_manager().commit_application_handoff(arg(args, "request")?)?)
        }
        "ai_conversation_session_patch" => {
            encode(ai_manager().patch_conversation_session(arg(args, "request")?)?)
        }
        "ai_message_send" => {
            let request: ai_runtime::MessageSendRequest = arg(args, "request")?;
            let mut timing =
                AiSendTiming::new("ai_message_send", arg(args, "client_sent_at_ms").ok());
            reject_retired_application_context(request.application_context.as_ref())?;
            encode(timing.measure("run_start", || {
                ai_manager().send_message_with_application_context(request, None, None, None)
            })?)
        }

        // 受理回执须先于后台准备写出，只能经并行调度的 submit_message 处理
        "ai_message_submit" | "ai_conversation_submit" => {
            Err("ai_message_submit_requires_parallel_dispatch".to_owned())
        }
        "ai_message_submission_dismiss" => {
            ai_manager().dismiss_message_submission(&arg::<String>(args, "client_message_id")?)?;
            Ok(serde_json::Value::Null)
        }

        // PDF 预览：校验后交由辅助进程渲染，字节经二进制通道回传
        "workspace_pdf_page_count" => encode(workspace::pdf::page_count(&validated_pdf(args)?)?),
        "workspace_pdf_read_bytes" => {
            let root = workspace::validated_directory(&arg::<String>(args, "local_root")?)?;
            let relative_path: String = arg(args, "relative_path")?;
            let item = workspace::resolved_item(&root, &relative_path)?;
            let bytes = workspace::pdf::read_bytes(&item)?;
            let current = workspace::resolved_item(&root, &relative_path)
                .map_err(|_| "workspace_pdf_source_changed".to_owned())?;
            if current != item {
                return Err("workspace_pdf_source_changed".to_owned());
            }
            Ok(binary(&bytes))
        }
        "workspace_render_pdf_page" => {
            let item = validated_pdf(args)?;
            let page_number: usize = arg(args, "page_number")?;
            let max_width: u32 = arg(args, "max_width")?;
            Ok(binary(&workspace::pdf::render_page(
                &item,
                page_number,
                max_width,
            )?))
        }
        "ai_attachment_read" => Ok(binary(
            &ai_manager().read_attachment(&arg::<String>(args, "attachment_id")?)?,
        )),

        // 本机账户：取自平台适配层，不经应用服务
        "local_user_profile" => {
            let user = contexthub_platform_adapter::system::current_user()
                .map_err(|error| format!("local_user_profile_unavailable:{error}"))?;
            Ok(serde_json::json!({
                "displayName": user.display_name,
                "username": user.username,
                "accountMode": "本地账户",
            }))
        }

        "ai_runtime_snapshot" => {
            if arg::<Option<bool>>(args, "refresh_native_catalog")?.unwrap_or(true) {
                encode(ai_manager().snapshot()?)
            } else {
                encode(ai_manager().live_snapshot())
            }
        }
        "desktop_update_preflight" => {
            let mut blockers = ai_manager().tenant_switch_blockers();
            blockers.extend(ai_manager().application_restart_blockers()?);
            blockers.sort();
            blockers.dedup();
            Ok(serde_json::json!({ "ready": blockers.is_empty(), "blockers": blockers }))
        }
        "ai_conversation_search" => encode(ai_manager().search_conversations(
            &arg::<String>(args, "query")?,
            arg(args, "archived")?,
            arg::<Option<String>>(args, "cursor")?.as_deref(),
            arg(args, "limit")?,
        )?),
        "ai_runtime_activity_snapshot" => {
            let run_ids: Vec<String> = arg(args, "run_ids")?;
            encode(ai_manager().activity_snapshot(&run_ids)?)
        }

        // 工作区：路径校验后调用平台能力
        "workspace_reveal_directory" => {
            let root: String = arg(args, "local_root")?;
            let directory = workspace::validated_directory(&root)?;
            contexthub_platform_adapter::system::reveal_in_file_manager(&directory)
                .map_err(|error| format!("workspace_reveal_failed:{error}"))?;
            Ok(serde_json::Value::Null)
        }
        "workspace_open_item" => {
            let root = workspace::validated_directory(&arg::<String>(args, "local_root")?)?;
            let item = workspace::resolved_item(&root, &arg::<String>(args, "relative_path")?)?;
            workspace::open_item(&item, &arg::<String>(args, "open_mode")?)?;
            Ok(serde_json::Value::Null)
        }
        "workspace_default_application" => {
            let root = workspace::validated_directory(&arg::<String>(args, "local_root")?)?;
            let item = workspace::resolved_item(&root, &arg::<String>(args, "relative_path")?)?;
            encode(workspace::default_application(&item)?)
        }
        "workspace_item_export_copy" => {
            let root = workspace::validated_directory(&arg::<String>(args, "local_root")?)?;
            let item = workspace::resolved_item(&root, &arg::<String>(args, "relative_path")?)?;
            workspace::copy_item(&item, &arg::<String>(args, "destination_path")?)?;
            Ok(serde_json::Value::Null)
        }
        "workspace_item_clipboard_content" => {
            let root = workspace::validated_directory(&arg::<String>(args, "local_root")?)?;
            let item = workspace::resolved_item(&root, &arg::<String>(args, "relative_path")?)?;
            encode(workspace::clipboard_content(&item)?)
        }
        "workspace_delete_item" => {
            let workspace_id: String = arg(args, "workspace_id")?;
            let relative_path: String = arg(args, "relative_path")?;
            supervisor().request_result(
                "workspace/targetValidate",
                &serde_json::json!({
                    "workspaceId": workspace_id,
                    "relativePath": relative_path,
                    "requireWrite": true,
                }),
            )?;
            let root = workspace::validated_directory(&arg::<String>(args, "local_root")?)?;
            let item = workspace::resolved_item(&root, &relative_path)?;
            // 拒绝删除根目录本身，避免一次删除带走整个工作区
            if item == root {
                return Err("workspace_item_is_root".to_owned());
            }
            trash::delete(&item)
                .map_err(|error| format!("workspace_item_delete_failed:{error}"))?;
            Ok(serde_json::Value::Null)
        }

        "app_server_request" => {
            let method = args
                .get("method")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "缺少 method".to_owned())?;
            // 方法名来自界面，必须逐项核准。缺了这道核准，凡能到达本命令的代码都能
            // 调用应用服务的全部方法，这条通用通道本身就成了绕过命令边界的入口。
            // 错误串与现有壳一致，界面对两端的失败处置不必分叉。
            if !desktop_rpc::desktop_method_allowed(method) {
                return Err("desktop_method_not_allowed".to_owned());
            }
            let params = args
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            supervisor().request_result(method, &params)
        }
        // 未迁移的命令显式失败，不以空值掩盖缺口
        other => Err(format!("命令未迁移到宿主服务：{other}")),
    }
}

fn unified_search_params(request: &Request) -> Option<serde_json::Value> {
    (request.command == "app_server_request"
        && request
            .args
            .get("method")
            .and_then(serde_json::Value::as_str)
            == Some("unifiedSearch/query"))
    .then(|| {
        request
            .args
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    })
}

fn command_changes_local_context(command: &str) -> bool {
    matches!(
        command,
        "app_server_restart"
            | "startup_permissions_check"
            | "startup_permissions_repair"
            | "powershell_policy_apply"
    )
}

fn write_response(id: u64, result: Result<serde_json::Value, String>) -> Result<(), String> {
    let response = match result {
        Ok(result) => Response {
            id,
            result: Some(result),
            error: None,
        },
        Err(error) => Response {
            id,
            result: None,
            error: Some(error),
        },
    };
    let encoded = serde_json::to_value(&response).unwrap_or_else(
        |error| serde_json::json!({ "id": 0, "error": format!("响应无法编码：{error}") }),
    );
    write_line(&encoded)
}

fn initialize_host_services() {
    let initial = local_profile().and_then(|profile| {
        set_active_profile(profile.clone())?;
        start_app_server(&profile.data_dir)?;
        initialize_ai_runtime(&profile.data_dir)
    });
    if let Err(error) = initial {
        if supervisor().status().phase == AppServerPhase::Stopped
            && let Err(state_error) =
                supervisor().record_quiescent_failure("startup_data_initialization_failed", &error)
        {
            eprintln!("启动失败状态记录失败：{state_error}");
        }
        *runtime_initialization_error()
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(error.clone());
        eprintln!("本地资料启动失败：{error}");
    }
}

fn platform_helper_exit_code(arguments: &[String]) -> Option<i32> {
    if let Some(code) = contexthub_platform_adapter::powershell_policy::helper_exit_code(arguments)
    {
        return Some(code);
    }
    if let Some(code) =
        contexthub_platform_adapter::startup_permissions::helper_exit_code(arguments)
    {
        return Some(code);
    }
    if let Some(code) =
        contexthub_platform_adapter::workspace_permissions::helper_exit_code(arguments)
    {
        return Some(code);
    }
    contexthub_platform_adapter::managed_component::helper_exit_code(arguments)
}

fn writer_launch_gate_exit_code() -> Option<i32> {
    let raw_arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if raw_arguments
        .first()
        .is_some_and(|value| value == contexthub_platform_adapter::writer_launch_gate::COMMAND)
    {
        let code = if let Ok(code) =
            contexthub_platform_adapter::writer_launch_gate::run(&raw_arguments[1..])
        {
            code
        } else {
            eprintln!("personal_writer_launch_gate_failed");
            1
        };
        return Some(code);
    }
    None
}

fn main() {
    if let Some(code) = writer_launch_gate_exit_code() {
        std::process::exit(code);
    }
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if let Some(code) = platform_helper_exit_code(&arguments) {
        std::process::exit(code);
    }
    if arguments
        .first()
        .is_some_and(|value| value == "--migrate-codex-public-history")
    {
        let result = if arguments.len() == 4 {
            contexthub_workbench_runtime::codex_upgrade::migrate(
                std::path::Path::new(&arguments[1]),
                std::path::Path::new(&arguments[2]),
                std::path::Path::new(&arguments[3]),
            )
        } else {
            Err("codex_upgrade_arguments_invalid".into())
        };
        match result {
            Ok(receipt) => println!("{receipt}"),
            Err(error) => {
                eprintln!("旧对话迁移未完成：{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Err(error) = initialize_personal_data_directory(&arguments) {
        eprintln!("{error}");
        std::process::exit(1);
    }
    let headless_arguments = if arguments.first().is_some_and(|v| v == "--data-dir") {
        &[][..]
    } else {
        arguments.as_slice()
    };
    if let Err(error) = initialize_headless_data_directory(headless_arguments) {
        eprintln!("{error}");
        std::process::exit(1);
    }
    initialize_host_services();

    // 读取独立成线程：反向调用发生在命令处理过程中，若仍由本线程读取，等回执时
    // 没有人在读管道，必然自锁。回执直接交给等待者，命令投递到队列由本线程处理。
    let (commands, incoming) = std::sync::mpsc::channel::<QueuedRequest>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Incoming>(&line) {
                Ok(Incoming::HostResponse {
                    host_response,
                    result,
                    error,
                }) => host_bridge().resolve(
                    host_response,
                    error.map_or_else(|| Ok(result.unwrap_or(serde_json::Value::Null)), Err),
                ),
                Ok(Incoming::Command(request)) => {
                    let unified_search_id = if unified_search_params(&request).is_some() {
                        Some(supervisor().begin_unified_search())
                    } else {
                        if command_changes_local_context(&request.command) {
                            supervisor().cancel_unified_search();
                        }
                        None
                    };
                    if commands
                        .send(QueuedRequest {
                            request,
                            unified_search_id,
                            received_at: std::time::Instant::now(),
                        })
                        .is_err()
                    {
                        if unified_search_id.is_some() {
                            supervisor().cancel_unified_search();
                        }
                        break;
                    }
                }
                // 无法解析的行没有编号可回，只能记到标准错误：回一条编号为零的响应，
                // 壳那边也找不到对应的等待者。
                Err(error) => eprintln!("请求无法解析：{error}"),
            }
        }
    });

    let catalog_publisher = start_model_catalog_publisher();
    dispatch_commands(&incoming);
    catalog_publisher.abort();
    if let Err(reason) = finish_host_shutdown() {
        eprintln!("{reason}");
        std::process::exit(1);
    }
}

fn finish_host_shutdown() -> Result<(), String> {
    let page_shutdown = personal_page_ai::revoke_all();
    let runtime_shutdown = ai_manager().shutdown_checked();
    let service_shutdown = supervisor().shutdown();
    if runtime_shutdown.is_err() {
        eprintln!("personal_runtime_shutdown_unconfirmed");
    }
    if service_shutdown.is_err() {
        eprintln!("personal_app_server_shutdown_unconfirmed");
    }
    page_shutdown
        .and(runtime_shutdown)
        .and(service_shutdown)
        .map_err(|_| "personal_host_shutdown_unconfirmed".into())
}

fn start_model_catalog_publisher() -> tokio::task::JoinHandle<()> {
    let mut updates = ai_manager().subscribe_model_catalog();
    runtime().spawn(async move {
        loop {
            updates.borrow_and_update();
            let publication = tokio::task::spawn_blocking(|| {
                let profile = current_profile()?;
                let snapshot = ai_manager().model_catalog_for_profile(&profile.data_dir)?;
                if current_profile()?.id != profile.id {
                    return Ok(());
                }
                host_bridge()
                    .request(
                        "notify_renderer",
                        &serde_json::json!({
                            "event": "model-catalog-changed",
                            "payload": { "profileId": profile.id, "catalog": snapshot },
                        }),
                    )
                    .map(|_| ())
            })
            .await;
            match publication {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("模型目录通知失败：{error}"),
                Err(error) => eprintln!("模型目录通知任务失败：{error}"),
            }
            if updates.changed().await.is_err() {
                break;
            }
        }
    })
}

/// 逐条受理命令：发送类命令交给独立线程，其余按到达顺序执行。
fn dispatch_commands(incoming: &std::sync::mpsc::Receiver<QueuedRequest>) {
    let in_flight = Arc::new(InFlight::default());
    loop {
        let queued = match incoming.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(queued) => queued,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                dispatch_follow_up(&in_flight);
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let request = queued.request;
        if let Err(error) = personal_command_allowed(&request.command, &request.args) {
            if write_response(request.id, Err(error)).is_err() {
                break;
            }
            continue;
        }
        if queued.unified_search_id.is_none() && runs_in_parallel(&request.command, &request.args) {
            dispatch_in_parallel(request, queued.received_at, &in_flight);
            continue;
        }
        if command_changes_local_context(&request.command) {
            // 切换数据上下文前等并行中的命令做完，它们是在旧上下文里受理的
            in_flight.wait_idle();
        }
        let handling_started = std::time::Instant::now();
        let result = match queued.unified_search_id {
            Some(search_id) => supervisor().request_unified_search(
                &unified_search_params(&request).unwrap_or(serde_json::Value::Null),
                search_id,
            ),
            None => handle(&request.command, &request.args),
        };
        let has_partial = result.as_ref().ok().is_some_and(|value| {
            value["runs"].as_array().is_some_and(|runs| {
                runs.iter().any(|run| {
                    run["partialOutput"]
                        .as_str()
                        .is_some_and(|text| !text.is_empty())
                })
            })
        });
        let written = write_response(request.id, result);
        if matches!(
            request.command.as_str(),
            "ai_runtime_snapshot" | "ai_runtime_activity_snapshot"
        ) || handling_started.elapsed().as_millis() >= 100
            || handling_started
                .duration_since(queued.received_at)
                .as_millis()
                >= 100
        {
            eprintln!(
                "ai_delivery_timing request_id={} command={} queue_ms={} handling_ms={} total_ms={} has_partial={has_partial}",
                request.id,
                request.command,
                handling_started
                    .duration_since(queued.received_at)
                    .as_millis(),
                handling_started.elapsed().as_millis(),
                queued.received_at.elapsed().as_millis(),
            );
        }
        if written.is_err() {
            break;
        }
    }
    // 壳关闭输入即退出；并行中的发送各自持有事务，等它们写完回执再结束进程
    in_flight.wait_idle();
}

/// 需要多次网络往返的发送类命令，在独立线程执行。
///
/// 宿主逐条处理命令时，一次发送的准备和运行启动会让归档、
/// 刷新列表、打开文件等所有命令都排在它后面。这些命令本就会被后台自动化从其他线程
/// 并发调用，运行时管理器以内部锁保证一致。
fn runs_in_parallel(command: &str, args: &serde_json::Value) -> bool {
    personal_page_ai::is_command(command)
        || personal_notifications::is_command(command)
        || matches!(
            command,
            "personal_data_backup_capture"
                | "personal_data_restore_apply"
                | "personal_storage_execute"
                | "personal_namespace_storage_execute_internal"
                | "personal_namespace_storage_action_begin_internal"
                | "personal_namespace_storage_action_ack_internal"
                | "personal_namespace_storage_segment_sync_internal"
                | "personal_namespace_storage_finish"
                | "personal_memory_reset_confirm"
                | "personal_capability_upgrade"
                | "personal_package_import_commit"
                | "personal_application_data_rollback"
                | "mcp_connection_validate"
                | "ai_conversation_search"
                | "ai_conversation_start"
                | "ai_message_send"
                | "ai_message_submit"
                | "ai_conversation_submit"
                | "ai_message_regenerate"
                | "ai_automation_rule_run"
                | "ai_model_catalog_read"
                | "model_connection_auth_read"
                | "model_execution_preflight"
                | "model_connection_catalog_refresh"
                | "model_connection_validate"
                | "model_connection_auth_start"
                | "model_connection_auth_cancel"
                | "model_connection_auth_logout"
                | "model_connection_auth_resume"
                | "model_connection_auth_finish_logout"
                | "model_connection_auth_cancel_readonly"
        )
        || (command == "app_server_request"
            && args
                .get("method")
                .and_then(serde_json::Value::as_str)
                .is_some_and(waits_for_local_files))
}

/// 可能等待远端或本地文件系统的读取与文件操作使用同一并发闸门，避免阻塞后续宿主命令。
fn waits_for_local_files(method: &str) -> bool {
    method == "workspace/listDirectory"
}

/// 并行中的命令数。重启本地服务或调整执行环境前须等待其归零。
#[derive(Default)]
struct InFlight {
    count: Mutex<usize>,
    idle: std::sync::Condvar,
}

impl InFlight {
    fn begin(&self) {
        *self.count.lock().unwrap_or_else(PoisonError::into_inner) += 1;
    }

    fn end(&self) {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.idle.notify_all();
        }
    }

    fn wait_idle(&self) {
        let mut count = self.count.lock().unwrap_or_else(PoisonError::into_inner);
        while *count > 0 {
            count = self
                .idle
                .wait(count)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

struct InFlightOperation(Arc<InFlight>);

impl Drop for InFlightOperation {
    fn drop(&mut self) {
        self.0.end();
    }
}

fn dispatch_follow_up(in_flight: &Arc<InFlight>) {
    if *in_flight
        .count
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        > 0
    {
        return;
    }
    let claimed = match ai_manager().claim_follow_up() {
        Ok(Some(claimed)) => claimed,
        Ok(None) => return,
        Err(error) => {
            eprintln!("后续指令领取失败：{error}");
            return;
        }
    };
    let queue_id = claimed.queue_id.clone();
    in_flight.begin();
    let worker = Arc::clone(in_flight);
    let spawned = std::thread::Builder::new()
        .name("host-follow-up".to_owned())
        .spawn(move || {
            let _in_flight = InFlightOperation(worker);
            let operation = claimed.operation;
            let result = handle(
                "ai_message_send",
                &serde_json::json!({ "request": claimed.request }),
            );
            if let Err(error) = ai_manager()
                .finish_follow_up(&claimed.queue_id, result.as_ref().err().map(String::as_str))
            {
                eprintln!("后续指令结果保存失败：{error}");
            }
            drop(operation);
        });
    if let Err(error) = spawned {
        in_flight.end();
        if let Err(save_error) =
            ai_manager().finish_follow_up(&queue_id, Some(&format!("发送线程无法启动：{error}")))
        {
            eprintln!("后续指令结果保存失败：{save_error}");
        }
    }
}

/// 受理对话消息：本地核对并登记回执后立即答复界面，企业会话校验、应用发现与运行启动
/// 留在本线程继续完成。线程计入并行数，企业上下文切换与退出仍等它写完结果。
fn submit_message(id: u64, args: &serde_json::Value, new_conversation: bool) {
    let mut timing = AiSendTiming::new(
        if new_conversation {
            "ai_conversation_submit"
        } else {
            "ai_message_submit"
        },
        arg(args, "client_sent_at_ms").ok(),
    );
    let accepted = timing.measure("acceptance", || {
        if new_conversation {
            arg(args, "request").and_then(|request| {
                ai_manager().accept_conversation_submission(request).map(
                    |(conversation, request, acceptance)| (Some(conversation), request, acceptance),
                )
            })
        } else {
            arg::<ai_runtime::MessageSendRequest>(args, "request").and_then(|request| {
                ai_manager()
                    .accept_message_submission(&request)
                    .map(|acceptance| (None, request, acceptance))
            })
        }
    });
    let (conversation, request, acceptance) = match accepted {
        Ok(accepted) => accepted,
        Err(error) => {
            if let Err(write_error) = write_response(id, Err(error)) {
                eprintln!("发送受理回执写出失败：{write_error}");
            }
            return;
        }
    };
    let operation = match acceptance {
        ai_runtime::SubmissionAcceptance::Recorded(run) => {
            let reply = encode(
                serde_json::json!({ "status": "recorded", "run": run, "conversation": conversation }),
            );
            if let Err(write_error) = write_response(id, reply) {
                eprintln!("发送受理回执写出失败：{write_error}");
            }
            return;
        }
        ai_runtime::SubmissionAcceptance::Accepted(operation) => operation,
    };
    let client_message_id = request.client_message_id.clone().unwrap_or_default();
    if let Err(write_error) = write_response(
        id,
        Ok(serde_json::json!({ "status": "accepted", "conversation": conversation })),
    ) {
        eprintln!("发送受理回执写出失败：{write_error}");
    }
    let client_sent_at_ms = arg::<i64>(args, "client_sent_at_ms").ok();
    let result = handle(
        "ai_message_send",
        &serde_json::json!({ "request": request, "clientSentAtMs": client_sent_at_ms }),
    );
    if let Err(error) = &result {
        eprintln!("ai_send_submission failed client_message_id={client_message_id} reason={error}");
    }
    ai_manager().finish_message_submission(
        &client_message_id,
        result.as_ref().err().map(String::as_str),
    );
    drop(operation);
}

fn dispatch_in_parallel(
    request: Request,
    received_at: std::time::Instant,
    in_flight: &Arc<InFlight>,
) {
    let id = request.id;
    let spawned = spawn_in_flight(in_flight, format!("host-{}", request.command), move || {
        if matches!(
            request.command.as_str(),
            "ai_message_submit" | "ai_conversation_submit"
        ) {
            submit_message(
                request.id,
                &request.args,
                request.command == "ai_conversation_submit",
            );
        } else {
            let handling_started = std::time::Instant::now();
            let result = handle(&request.command, &request.args);
            if let Err(error) = write_response(request.id, result) {
                eprintln!("并行命令回执写出失败：{} {error}", request.command);
            }
            if handling_started.elapsed().as_millis() >= 100 {
                eprintln!(
                    "ai_delivery_timing request_id={} command={} parallel=true queue_ms={} handling_ms={} total_ms={}",
                    request.id,
                    request.command,
                    handling_started.duration_since(received_at).as_millis(),
                    handling_started.elapsed().as_millis(),
                    received_at.elapsed().as_millis()
                );
            }
        }
    });
    if let Err(error) = spawned
        && let Err(write_error) = write_response(id, Err(format!("命令线程无法启动：{error}")))
    {
        eprintln!("并行命令回执写出失败：{write_error}");
    }
}

fn spawn_in_flight(
    in_flight: &Arc<InFlight>,
    name: String,
    operation: impl FnOnce() + Send + 'static,
) -> Result<(), std::io::Error> {
    in_flight.begin();
    let worker_in_flight = Arc::clone(in_flight);
    let spawned = std::thread::Builder::new().name(name).spawn(move || {
        let _in_flight = InFlightOperation(worker_in_flight);
        operation();
    });
    match spawned {
        Ok(_) => Ok(()),
        Err(error) => {
            in_flight.end();
            Err(error)
        }
    }
}

fn open_registered_artifact(
    run_id: &str,
    artifact_id: &str,
    open_mode: &str,
    resolve: impl FnOnce(&str, &str) -> Result<std::path::PathBuf, String>,
    open: impl FnOnce(&std::path::Path, &str) -> Result<(), String>,
) -> Result<(), String> {
    if !matches!(open_mode, "default" | "finder") {
        return Err("ai_artifact_open_mode_invalid".to_owned());
    }
    let item = resolve(run_id, artifact_id)?;
    open(&item, open_mode)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        DEFAULT_ENVIRONMENT, InFlight, Request, app_runtime_status, app_server_failure_message,
        command_changes_local_context, handle, open_registered_artifact, personal_command_allowed,
        reject_retired_application_context, resolve_data_environment, runs_in_parallel,
        runtime_initialization_error, spawn_in_flight, unified_search_params,
    };
    use contexthub_workbench_runtime::supervisor::{AppServerPhase, AppServerStatus};

    #[test]
    fn personal_host_rejects_removed_features_before_service_access() {
        for command in [
            "enterprise_login",
            "enterprise_session_status",
            "enterprise_logout",
            "enterprise_capability_prepare",
            "personal_capability_submit",
            "personal_capability_submissions",
            "capture_request",
            "application_report_notifications",
            "application_catalog_list",
            "application_launch_web",
            "application_launch_search_result",
            "application_install_local",
            "ai_browser_policy_get",
        ] {
            assert_eq!(
                handle(command, &serde_json::Value::Null).unwrap_err(),
                "personal_feature_removed",
                "{command}"
            );
        }
        for method in [
            "cloudWorkspace/attach",
            "sync/execute",
            "employeeCredential/get",
        ] {
            assert_eq!(
                handle("app_server_request", &serde_json::json!({"method": method})).unwrap_err(),
                "desktop_method_not_allowed"
            );
        }
    }

    #[test]
    fn historical_application_selection_cannot_restore_enterprise_tools() {
        let selection = contexthub_workbench_runtime::app_bridge::ApplicationContextSelection {
            application_id: "historical-app".into(),
            context_ids: Vec::new(),
            tool_names: Vec::new(),
            invocation_source: Some("slash".into()),
        };
        let result = reject_retired_application_context(Some(&selection));
        assert!(matches!(result, Err(ref error) if error == "personal_feature_removed"));
        assert!(reject_retired_application_context(None).is_ok());
        for command in [
            "ai_message_submit",
            "ai_conversation_submit",
            "ai_conversation_start",
            "ai_message_send",
        ] {
            assert_eq!(personal_command_allowed(command, &serde_json::json!({"request":{"executionTarget":"agent","applicationContext":selection}})).unwrap_err(), "personal_feature_removed");
        }
    }

    #[test]
    fn model_commands_reach_the_runtime_connection_and_purpose_preflight() {
        for command in [
            "ai_message_submit",
            "ai_conversation_submit",
            "ai_automation_rule_run",
        ] {
            assert!(
                personal_command_allowed(
                    command,
                    &serde_json::json!({"request":{"executionTarget":"model","content":"保留输入"}})
                )
                .is_ok()
            );
        }
        assert!(personal_command_allowed("ai_runtime_snapshot", &serde_json::Value::Null).is_ok());
        assert!(
            personal_command_allowed("ai_conversation_create", &serde_json::Value::Null).is_ok()
        );
    }

    #[test]
    fn data_environment_preserves_defaults_and_validates_explicit_adoption() {
        assert_eq!(resolve_data_environment(None).unwrap(), DEFAULT_ENVIRONMENT);
        assert_eq!(resolve_data_environment(Some("dev")).unwrap(), "dev");
        assert_eq!(resolve_data_environment(Some("prod")).unwrap(), "prod");
        for invalid in ["", "test", "../dev", "DEV"] {
            assert!(resolve_data_environment(Some(invalid)).is_err());
        }
    }

    #[test]
    fn history_initialization_failure_remains_visible_without_enterprise_login() {
        let reason = "备份历史会话失败：App Server request timed out: aiHistory/execute";
        *runtime_initialization_error().lock().unwrap() = Some(reason.to_owned());
        let session = handle("enterprise_session_status", &serde_json::json!({}));
        let login = handle("enterprise_login", &serde_json::json!({}));
        let status = app_runtime_status().unwrap();
        *runtime_initialization_error().lock().unwrap() = None;
        assert_eq!(session.unwrap_err(), "personal_feature_removed");
        assert_eq!(login.unwrap_err(), "personal_feature_removed");
        assert_eq!(status["runtimeInitializationError"], reason);
        assert_eq!(status["automationSummary"]["known"], false);
        assert!(status["automationSummary"]["enabledCount"].is_null());
        assert!(app_runtime_status().unwrap()["runtimeInitializationError"].is_null());
    }

    #[test]
    fn automation_status_summary_requires_ready_and_profile_bound_known_count() {
        let profile = super::ActiveProfile {
            id: "1".repeat(32),
            data_dir: std::env::temp_dir().join("synthetic-automation-status"),
        };
        for count in [0, 1] {
            let summary = super::automation_runtime_summary(true, Some(&profile), |root| {
                assert_eq!(root, profile.data_dir);
                Ok(count)
            });
            assert_eq!(
                summary,
                serde_json::json!({"profileId":profile.id,"enabledCount":count,"known":true})
            );
        }
        let unready = super::automation_runtime_summary(false, Some(&profile), |_| {
            panic!("unready_runtime_must_not_publish_count")
        });
        let no_profile = super::automation_runtime_summary(true, None, |_| {
            panic!("missing_profile_must_not_publish_count")
        });
        let failed_count = super::automation_runtime_summary(true, Some(&profile), |_| {
            Err("ai_runtime_not_initialized".into())
        });
        for summary in [unready, no_profile, failed_count] {
            assert_eq!(summary["known"], false);
            assert!(summary["enabledCount"].is_null());
        }
    }

    #[test]
    fn artifact_open_uses_registered_path_and_propagates_platform_errors() {
        for mode in ["default", "finder"] {
            let result = open_registered_artifact(
                "run-1",
                "image-1",
                mode,
                |run, artifact| {
                    assert_eq!((run, artifact), ("run-1", "image-1"));
                    Ok(std::path::PathBuf::from("registered-image.png"))
                },
                |path, actual_mode| {
                    assert_eq!(path, std::path::Path::new("registered-image.png"));
                    assert_eq!(actual_mode, mode);
                    Err("platform_open_failed".to_owned())
                },
            );
            assert_eq!(
                result.as_ref().map_err(String::as_str),
                Err("platform_open_failed")
            );
        }
    }

    #[test]
    fn unavailable_artifacts_and_invalid_modes_never_open_a_file() {
        let unavailable = open_registered_artifact(
            "run-1",
            "missing",
            "default",
            |_, _| Err("ai_artifact_unavailable".to_owned()),
            |_, _| panic!("unavailable artifact must not reach the platform"),
        );
        assert_eq!(
            unavailable.as_ref().map_err(String::as_str),
            Err("ai_artifact_unavailable")
        );
        let invalid = open_registered_artifact(
            "run-1",
            "image-1",
            "arbitrary-program",
            |_, _| panic!("invalid mode must not resolve an artifact"),
            |_, _| panic!("invalid mode must not reach the platform"),
        );
        assert_eq!(
            invalid.as_ref().map_err(String::as_str),
            Err("ai_artifact_open_mode_invalid")
        );
    }

    #[test]
    fn app_server_failure_preserves_exit_code_and_stderr_root_cause() {
        let status = AppServerStatus {
            phase: AppServerPhase::Failed,
            last_exit_code: Some(13),
            error_reason: Some("unexpected_exit_code_13".to_owned()),
            stderr_tail: vec![
                "App Server failure: applied migration 29 does not match embedded name and checksum"
                    .to_owned(),
            ],
            ..AppServerStatus::default()
        };

        assert_eq!(
            app_server_failure_message(&status).as_deref(),
            Some(
                "本地服务启动失败（退出码 13）：applied migration 29 does not match embedded name and checksum"
            )
        );
    }

    #[test]
    fn unified_search_is_identified_before_the_serial_command_queue() {
        let request = Request {
            id: 7,
            command: "app_server_request".to_owned(),
            args: serde_json::json!({
                "method": "unifiedSearch/query",
                "params": { "query": "报告" }
            }),
        };
        assert_eq!(
            unified_search_params(&request),
            Some(serde_json::json!({ "query": "报告" }))
        );
    }

    #[test]
    fn local_context_changes_wait_for_active_work() {
        for command in [
            "app_server_restart",
            "startup_permissions_check",
            "startup_permissions_repair",
            "powershell_policy_apply",
        ] {
            assert!(command_changes_local_context(command));
        }
        assert!(!command_changes_local_context("enterprise_switch_tenant"));
        assert!(!command_changes_local_context("application_catalog_list"));
    }

    #[test]
    fn sends_and_local_directory_reads_leave_the_ordered_queue() {
        for command in [
            "ai_conversation_start",
            "ai_conversation_submit",
            "ai_message_send",
            "ai_message_submit",
            "ai_message_regenerate",
            "ai_automation_rule_run",
            "model_connection_auth_read",
        ] {
            assert!(
                runs_in_parallel(command, &serde_json::Value::Null),
                "{command}"
            );
            assert!(!command_changes_local_context(command), "{command}");
        }
        assert!(runs_in_parallel(
            "app_server_request",
            &serde_json::json!({"method":"workspace/listDirectory"})
        ));
        for method in [
            "applicationCatalog/list",
            "applicationCatalog/launch",
            "aiBrowserPolicy/get",
            "cloudWorkspace/preview",
            "cloudWorkspace/fileAction",
        ] {
            assert!(!runs_in_parallel(
                "app_server_request",
                &serde_json::json!({"method":method})
            ));
        }
        assert!(!runs_in_parallel(
            "app_server_request",
            &serde_json::json!({ "method": "workspace/list" })
        ));
        for method in [
            "workspace/createDirectory",
            "workspace/indexPath",
            "workspace/delete",
        ] {
            assert!(!runs_in_parallel(
                "app_server_request",
                &serde_json::json!({ "method": method })
            ));
        }
        for command in [
            "ai_conversation_set_archived",
            "ai_runtime_snapshot",
            "enterprise_switch_tenant",
            "enterprise_logout",
            "app_server_restart",
        ] {
            assert!(
                !runs_in_parallel(command, &serde_json::Value::Null),
                "{command}"
            );
        }
    }

    #[test]
    fn context_change_waits_for_sends_already_accepted() {
        let in_flight = std::sync::Arc::new(InFlight::default());
        in_flight.begin();
        let finished = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker = {
            let in_flight = std::sync::Arc::clone(&in_flight);
            let finished = std::sync::Arc::clone(&finished);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                finished.store(true, std::sync::atomic::Ordering::SeqCst);
                in_flight.end();
            })
        };
        in_flight.wait_idle();
        assert!(finished.load(std::sync::atomic::Ordering::SeqCst));
        worker.join().unwrap();
        // 没有并行命令时不等待
        in_flight.wait_idle();
    }

    #[test]
    fn parallel_work_keeps_the_context_barrier_until_completion() {
        let in_flight = Arc::new(InFlight::default());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        spawn_in_flight(&in_flight, "host-model-test".to_owned(), move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
        .unwrap();
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        let (idle_tx, idle_rx) = std::sync::mpsc::channel();
        let barrier = Arc::clone(&in_flight);
        let waiter = std::thread::spawn(move || {
            barrier.wait_idle();
            idle_tx.send(()).unwrap();
        });
        assert!(matches!(
            idle_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        release_tx.send(()).unwrap();
        idle_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        waiter.join().unwrap();
        in_flight.wait_idle();
    }

    #[test]
    fn parallel_unwind_releases_context_and_exit_waiters() {
        struct UnwindObserved(std::sync::mpsc::Sender<bool>);
        impl Drop for UnwindObserved {
            fn drop(&mut self) {
                let _ = self.0.send(std::thread::panicking());
            }
        }

        let in_flight = Arc::new(InFlight::default());
        let (ended_tx, ended_rx) = std::sync::mpsc::channel();
        spawn_in_flight(&in_flight, "host-unwind-test".to_owned(), move || {
            let _observed = UnwindObserved(ended_tx);
            panic!("execution interrupted while replying to a closed channel");
        })
        .unwrap();
        assert!(
            ended_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap()
        );
        let (count, timeout) = in_flight
            .idle
            .wait_timeout_while(
                in_flight.count.lock().unwrap(),
                std::time::Duration::from_secs(1),
                |count| *count > 0,
            )
            .unwrap();
        assert!(
            !timeout.timed_out(),
            "finished thread still blocks shutdown"
        );
        assert_eq!(*count, 0);
    }
}
