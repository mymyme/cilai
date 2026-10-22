use contexthub_platform_adapter::OperatingSystem;
use serde::Serialize;
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) const REMOTE_CONTROL_DISABLED_ENV: &str =
    "CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED";
const WINDOWS_SANDBOX_CONFIG: &str = r#"windows.sandbox="unelevated""#;
const WINDOWS_SANDBOX_READINESS_METHOD: &str = "windowsSandbox/readiness";
const WINDOWS_SANDBOX_SETUP_METHOD: &str = "windowsSandbox/setupStart";
const WINDOWS_SANDBOX_SETUP_COMPLETED_METHOD: &str = "windowsSandbox/setupCompleted";
const WINDOWS_SANDBOX_HELPERS: [&str; 2] = [
    "codex-windows-sandbox-setup.exe",
    "codex-command-runner.exe",
];
static WINDOWS_SANDBOX_SETUP_ACTIVE: AtomicBool = AtomicBool::new(false);

pub(crate) fn execution_probe_params(thread: &Value, command: &[String]) -> Result<Value, String> {
    let policy = thread
        .pointer("/result/sandbox")
        .filter(|policy| matches!(policy["type"].as_str(), Some("readOnly" | "workspaceWrite")))
        .ok_or("windows_sandbox_preflight_failed:invalid_sandbox_policy")?;
    let cwd = thread
        .pointer("/result/cwd")
        .and_then(Value::as_str)
        .filter(|cwd| !cwd.is_empty())
        .ok_or("windows_sandbox_preflight_failed:missing_cwd")?;
    // Use the server's effective policy, including all materialized project roots.
    // Never probe with full access or turn a failed probe into an unsandboxed retry.
    // Windows command/exec rejects custom output caps before execution. Leave
    // the server's bounded default intact; this fixed no-op emits no output.
    Ok(json!({"command":command,"cwd":cwd,"sandboxPolicy":policy,
        "timeoutMs":10000,"tty":false,"streamStdin":false,"streamStdoutStderr":false}))
}

pub(crate) fn execution_probe_error_message(error: &str) -> String {
    let rpc = error
        .strip_prefix("windows_sandbox_preflight_failed:codex_harness_rpc_failed:")
        .and_then(|value| serde_json::from_str::<Value>(value).ok());
    let (code, message) = if rpc
        .as_ref()
        .and_then(|rpc| rpc["code"].as_i64())
        .is_some_and(|code| matches!(code, -32600 | -32602))
    {
        (
            "parameters_invalid",
            "沙箱执行预检查参数不兼容，本轮尚未开始模型操作。请更新工作台；如仍失败，请导出错误日志反馈。",
        )
    } else if error.contains("os error 1326") {
        (
            "credentials_invalid",
            "Windows 沙箱运行身份校验失败，本轮尚未开始模型操作。请导出错误日志反馈，不要重新设置机器共用沙箱账户。",
        )
    } else if error.contains("os error 1385") {
        (
            "logon_denied",
            "Windows 登录策略阻止了沙箱账户运行，本轮尚未开始模型操作。请将错误日志交给管理员检查沙箱账户的登录策略。",
        )
    } else if error.contains("os error 5)") || error.contains("SetNamedSecurityInfoW failed: 5") {
        (
            "access_denied",
            "Windows 拒绝沙箱所需的目录访问或权限更新，本轮尚未开始模型操作。请在模型设置检查项目目录权限；如仍失败，请导出错误日志反馈。",
        )
    } else if error.contains("setup refresh had errors") {
        (
            "refresh_failed",
            "当前项目的沙箱权限刷新失败，本轮尚未开始模型操作。请在模型设置检查项目目录权限，并导出错误日志反馈。",
        )
    } else if error.ends_with("exit_code=124") || error.contains("timed out") {
        (
            "timeout",
            "沙箱执行预检查超时，本轮尚未开始模型操作。请重试；如仍失败，请导出错误日志反馈。",
        )
    } else if error.contains("windows_sandbox_preflight_failed:exit_code=") {
        (
            "exit_failed",
            "沙箱探测命令异常退出，本轮尚未开始模型操作。请导出错误日志反馈。",
        )
    } else if [
        "invalid_sandbox_policy",
        "missing_cwd",
        "invalid_response",
        "codex_harness_protocol_invalid",
    ]
    .iter()
    .any(|value| error.contains(value))
    {
        (
            "protocol_invalid",
            "沙箱执行预检查未取得有效的运行配置或响应，本轮尚未开始模型操作。请导出错误日志反馈。",
        )
    } else if error.contains("helper_unknown_error") || error.contains("failed to spawn") {
        (
            "helper_failed",
            "沙箱执行组件未能完成探测命令，本轮尚未开始模型操作。请导出错误日志反馈。",
        )
    } else {
        (
            "failed",
            "当前项目的沙箱执行检查失败，本轮尚未开始模型操作。请导出错误日志反馈。",
        )
    };
    // Only fixed categories and descriptions leave the raw RPC error. Do not
    // put paths, protocol payloads, stdout/stderr, or credentials in the run.
    format!("{message}\n错误类型：sandbox_preflight_{code}")
}

pub(crate) fn verify_execution_probe(response: &Value) -> Result<(), String> {
    match response.pointer("/result/exitCode").and_then(Value::as_i64) {
        Some(0) => Ok(()),
        Some(code) => Err(format!("windows_sandbox_preflight_failed:exit_code={code}")),
        None => Err("windows_sandbox_preflight_failed:invalid_response".into()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowsSandboxReadiness {
    Ready,
    NotConfigured,
    UpdateRequired,
}

impl WindowsSandboxReadiness {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotConfigured => "notConfigured",
            Self::UpdateRequired => "updateRequired",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsSandboxStatus {
    pub supported: bool,
    pub status: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

impl WindowsSandboxStatus {
    fn not_applicable() -> Self {
        Self {
            supported: false,
            status: "notApplicable".to_owned(),
            error_code: None,
            error_message: None,
        }
    }

    fn ready_state(readiness: WindowsSandboxReadiness) -> Self {
        Self {
            supported: true,
            status: readiness.as_str().to_owned(),
            error_code: None,
            error_message: None,
        }
    }

    fn credentials_invalid() -> Self {
        Self {
            supported: true,
            status: WindowsSandboxReadiness::UpdateRequired.as_str().to_owned(),
            error_code: Some("windows_sandbox_credentials_invalid".to_owned()),
            error_message: Some(
                "Windows 沙箱运行配置不兼容，请更新工作台并重新检查；不要重新设置机器共用沙箱账户。"
                    .to_owned(),
            ),
        }
    }

    pub(crate) fn failed(error: &str) -> Self {
        // Failures can occur before the Harness and its stderr log are opened.
        // Preserve a safe category in the RPC result instead of hiding every
        // preparation failure behind the setup error. Never expose raw paths,
        // credentials, or protocol payloads in this public status.
        let (error_code, error_message) = if error.contains("windows_sandbox_helper_missing:") {
            (
                "windows_sandbox_helper_missing",
                "Windows 沙箱运行组件不完整，请重新安装词来工作台。",
            )
        } else if error.contains("private file is not owned by the current user") {
            (
                "windows_sandbox_private_owner_mismatch",
                "沙箱运行所需的工作台数据不属于当前 Windows 用户。请检查并修复启动权限后重试。",
            )
        } else if error.contains("windows_sandbox_home_failed:") {
            if error.contains("runtime directory is not owned") {
                (
                    "windows_sandbox_runtime_owner_mismatch",
                    "沙箱运行目录中存在不属于当前用户的目录，无法准备运行环境。请导出错误日志反馈。",
                )
            } else if error.contains("os error 5)") || error.contains("Permission denied") {
                (
                    "windows_sandbox_runtime_access_denied",
                    "Windows 拒绝访问沙箱运行目录，无法准备运行环境。请导出错误日志反馈。",
                )
            } else {
                (
                    "windows_sandbox_runtime_prepare_failed",
                    "沙箱运行目录准备失败，初始化程序尚未启动。请导出错误日志反馈。",
                )
            }
        } else if error.contains("windows_sandbox_log_failed:") {
            (
                "windows_sandbox_log_failed",
                "无法写入沙箱运行日志，初始化程序尚未启动。请检查磁盘空间和工作台目录权限。",
            )
        } else if error.contains("windows_sandbox_spawn_failed:") {
            (
                "windows_sandbox_spawn_failed",
                "Windows 无法启动沙箱运行组件。请导出错误日志反馈。",
            )
        } else if error.contains("windows_sandbox_probe_failed:") {
            (
                "windows_sandbox_probe_failed",
                "Windows 沙箱账户试运行失败，本地模型暂不可用。请导出错误日志反馈。",
            )
        } else if error.contains("windows_sandbox_workspace_root_") {
            (
                "windows_sandbox_workspace_unavailable",
                "当前项目目录不可用，请检查目录是否仍然存在并可访问。",
            )
        } else if error.contains("windows_sandbox_rpc_failed:")
            || error.contains("windows_sandbox_readiness_invalid:")
            || error.contains("windows_sandbox_read_failed:")
            || error.contains("windows_sandbox_control_closed")
            || error.contains("windows_sandbox_protocol_invalid:")
        {
            (
                "windows_sandbox_control_failed",
                "沙箱状态检查未完成，运行组件未返回有效状态。请导出错误日志反馈。",
            )
        } else {
            (
                "windows_sandbox_setup_failed",
                "无法确认 Windows 沙箱状态，请重试；如仍失败，请导出错误日志反馈。",
            )
        };
        Self {
            supported: true,
            status: "error".to_owned(),
            error_code: Some(error_code.to_owned()),
            error_message: Some(error_message.to_owned()),
        }
    }
}

/// Windows 10/11 均使用官方当前用户受限令牌沙箱，避免重设机器共用账户。
pub(crate) fn configure_harness_command(command: &mut Command, os: OperatingSystem) {
    if os == OperatingSystem::Windows {
        command.args(["-c", WINDOWS_SANDBOX_CONFIG]);
    }
}

/// Use a session override on Windows so starting a conversation does not need to
/// persist `config.toml` in the Harness home. Other platforms retain their
/// existing skill configuration flow.
pub(crate) fn configure_harness_skill_creator(
    command: &mut Command,
    os: OperatingSystem,
    harness_home: &Path,
) {
    if os == OperatingSystem::Windows {
        let path = harness_home.join("skills/.system/skill-creator/SKILL.md");
        let quoted_path = Value::String(path.to_string_lossy().into_owned());
        command.args([
            "-c",
            &format!("skills.config=[{{path={quoted_path},enabled=false}}]"),
        ]);
    }
}

pub(crate) fn validate_harness_helper_layout(harness: &Path) -> Result<(), String> {
    let directory = harness
        .parent()
        .ok_or_else(|| "codex_harness_windows_helper_directory_unavailable".to_owned())?;
    let missing = WINDOWS_SANDBOX_HELPERS
        .iter()
        .filter(|name| {
            !directory.join(name).is_file()
                && !directory.join("codex-resources").join(name).is_file()
                && !directory
                    .parent()
                    .is_some_and(|root| root.join("codex-resources").join(name).is_file())
        })
        .copied()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "windows_sandbox_helper_missing:{}",
            missing.join(",")
        ))
    }
}

pub(crate) fn parse_readiness_response(
    response: &Value,
) -> Result<WindowsSandboxReadiness, String> {
    match response.pointer("/result/status").and_then(Value::as_str) {
        Some("ready") => Ok(WindowsSandboxReadiness::Ready),
        Some("notConfigured") => Ok(WindowsSandboxReadiness::NotConfigured),
        Some("updateRequired") => Ok(WindowsSandboxReadiness::UpdateRequired),
        _ => Err(format!(
            "windows_sandbox_readiness_invalid:{}",
            compact_json(response)
        )),
    }
}

pub(crate) fn require_ready_for_run(readiness: WindowsSandboxReadiness) -> Result<(), String> {
    match readiness {
        WindowsSandboxReadiness::Ready => Ok(()),
        WindowsSandboxReadiness::NotConfigured => Err("windows_sandbox_not_configured".to_owned()),
        WindowsSandboxReadiness::UpdateRequired => {
            Err("windows_sandbox_update_required".to_owned())
        }
    }
}

pub(crate) fn status(
    executable: &Path,
    harness_home: &Path,
    workspace_root: Option<&str>,
    os: OperatingSystem,
) -> WindowsSandboxStatus {
    if os != OperatingSystem::Windows {
        return WindowsSandboxStatus::not_applicable();
    }
    query_verified_status(executable, harness_home, workspace_root, os)
        .unwrap_or_else(|error| WindowsSandboxStatus::failed(&error))
}

/// 就绪响应之外仍执行实际空命令，保证状态反映当前机器可执行的沙箱配置。
fn query_verified_status(
    executable: &Path,
    harness_home: &Path,
    workspace_root: Option<&str>,
    os: OperatingSystem,
) -> Result<WindowsSandboxStatus, String> {
    let mut session =
        ControlSession::spawn(executable, harness_home, workspace_root, os, "readiness")?;
    session.initialize()?;
    let readiness = session.readiness()?;
    if readiness != WindowsSandboxReadiness::Ready {
        return Ok(WindowsSandboxStatus::ready_state(readiness));
    }
    match session.execution_probe(&probe_directory(harness_home)?) {
        Ok(()) => Ok(WindowsSandboxStatus::ready_state(readiness)),
        Err(error) if probe_requires_setup(&error) => {
            Ok(WindowsSandboxStatus::credentials_invalid())
        }
        Err(error) => Err(error),
    }
}

/// 试运行使用专用的空目录，不触碰任何项目目录的权限。
fn probe_directory(harness_home: &Path) -> Result<PathBuf, String> {
    let directory = harness_home
        .parent()
        .ok_or_else(|| "windows_sandbox_probe_failed:probe_directory_unavailable".to_owned())?
        .join("sandbox-probe");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("windows_sandbox_probe_failed:probe_directory:{error}"))?;
    Ok(directory)
}

/// 沙箱账户登录失败（1326），或登录失败后无法清除旧凭据文件，都说明保存的凭据已失效，
/// 只能通过重新初始化恢复；其余试运行失败按原因如实上报。
fn probe_requires_setup(error: &str) -> bool {
    error.contains("os error 1326")
        || error.contains("sandbox_users.json")
        || error.contains("sandbox setup is missing or out of date")
}

pub(crate) fn setup(
    executable: &Path,
    harness_home: &Path,
    workspace_root: Option<&str>,
    os: OperatingSystem,
) -> Result<WindowsSandboxStatus, String> {
    if os != OperatingSystem::Windows {
        return Ok(WindowsSandboxStatus::not_applicable());
    }
    let _guard = SetupGuard::acquire()?;
    setup_inner(executable, harness_home, workspace_root, os)
        .map(WindowsSandboxStatus::ready_state)
        .map_err(|error| {
            if error.starts_with("windows_sandbox_helper_missing:") {
                error
            } else {
                format!("windows_sandbox_setup_failed:{error}")
            }
        })
}

fn setup_inner(
    executable: &Path,
    harness_home: &Path,
    workspace_root: Option<&str>,
    os: OperatingSystem,
) -> Result<WindowsSandboxReadiness, String> {
    let mut session = ControlSession::spawn(executable, harness_home, workspace_root, os, "setup")?;
    session.initialize()?;
    let readiness = session.readiness()?;
    if readiness == WindowsSandboxReadiness::Ready {
        // 当前用户受限令牌无需创建账户；仍验证真实执行，失败不得切换为提升模式。
        match session.execution_probe(&probe_directory(harness_home)?) {
            Ok(()) => return Ok(readiness),
            Err(error) => return Err(error),
        }
    }
    session.send(&json!({
        "method": WINDOWS_SANDBOX_SETUP_METHOD,
        "id": 3,
        "params": {
            "mode": "unelevated",
            "cwd": normalized_workspace_root(workspace_root)?
                .map(|path| path.to_string_lossy().into_owned())
        }
    }))?;
    wait_for_setup_completion(&mut session.stdout, 3)?;
    drop(session);

    let verified = query_verified_status(executable, harness_home, workspace_root, os)?;
    if verified.status != WindowsSandboxReadiness::Ready.as_str() {
        return Err(format!(
            "windows_sandbox_setup_verify_failed:{}",
            verified.error_code.as_deref().unwrap_or(&verified.status)
        ));
    }
    Ok(WindowsSandboxReadiness::Ready)
}

struct SetupGuard;

impl SetupGuard {
    fn acquire() -> Result<Self, String> {
        WINDOWS_SANDBOX_SETUP_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "windows_sandbox_setup_in_progress".to_owned())?;
        Ok(Self)
    }
}

impl Drop for SetupGuard {
    fn drop(&mut self) {
        WINDOWS_SANDBOX_SETUP_ACTIVE.store(false, Ordering::Release);
    }
}

struct ControlSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl ControlSession {
    fn spawn(
        executable: &Path,
        harness_home: &Path,
        workspace_root: Option<&str>,
        os: OperatingSystem,
        operation: &str,
    ) -> Result<Self, String> {
        validate_harness_helper_layout(executable)?;
        contexthub_platform_adapter::security::ensure_owner_writable_runtime_tree(harness_home)
            .map_err(|error| format!("windows_sandbox_home_failed:{error}"))?;
        let cwd = normalized_workspace_root(workspace_root)?
            .unwrap_or_else(|| harness_home.to_path_buf());
        let stderr_path = harness_home.join(format!("windows-sandbox-{operation}.stderr.log"));
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_path)
            .map_err(|error| format!("windows_sandbox_log_failed:{error}"))?;
        let mut command = Command::new(executable);
        crate::model_connections::isolate_runtime_environment(&mut command, harness_home)?;
        command
            .args(["-c", "cli_auth_credentials_store=\"file\""])
            .args(["-c", "features.secret_auth_storage=false"])
            .args(["-c", "features.system_proxy_fallback=false"])
            .current_dir(cwd)
            .env("CODEX_HOME", harness_home)
            .env("CODEX_SQLITE_HOME", harness_home)
            .env(REMOTE_CONTROL_DISABLED_ENV, "1")
            .args(["--listen", "stdio://"])
            .args(["-c", "check_for_update_on_startup=false"])
            .args(["-c", "project_doc_max_bytes=0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(stderr));
        configure_harness_command(&mut command, os);
        contexthub_platform_adapter::process::prepare_command_for_supervision(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("windows_sandbox_spawn_failed:{error}"))?;
        contexthub_platform_adapter::process::supervise_spawned_child(&mut child)
            .map_err(|error| format!("windows_sandbox_supervision_failed:{error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "windows_sandbox_stdin_unavailable".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "windows_sandbox_stdout_unavailable".to_owned())?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn initialize(&mut self) -> Result<(), String> {
        self.send(&json!({
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": {
                    "name": "contexthub_desktop_windows_sandbox",
                    "title": "词来工作台",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": { "experimentalApi": true }
            }
        }))?;
        read_control_response(&mut self.stdout, 1)?;
        self.send(&json!({ "method": "initialized", "params": {} }))
    }

    fn readiness(&mut self) -> Result<WindowsSandboxReadiness, String> {
        self.send(&json!({
            "method": WINDOWS_SANDBOX_READINESS_METHOD,
            "id": 2
        }))?;
        parse_readiness_response(&read_control_response(&mut self.stdout, 2)?)
    }

    /// 以只读策略用沙箱账户执行固定的空命令，与每轮运行前的执行预检查同一条路径。
    fn execution_probe(&mut self, cwd: &Path) -> Result<(), String> {
        let command = contexthub_platform_adapter::process::sandbox_probe_command()
            .map_err(|error| format!("windows_sandbox_probe_failed:{error}"))?;
        self.send(&json!({
            "method": "command/exec",
            "id": 4,
            "params": {
                "command": command,
                "cwd": cwd,
                "sandboxPolicy": {"type": "readOnly", "networkAccess": false},
                "timeoutMs": 10000,
                "tty": false,
                "streamStdin": false,
                "streamStdoutStderr": false
            }
        }))?;
        let response = read_control_response(&mut self.stdout, 4)
            .map_err(|error| format!("windows_sandbox_probe_failed:{error}"))?;
        verify_execution_probe(&response)
            .map_err(|error| format!("windows_sandbox_probe_failed:{error}"))
    }

    fn send(&mut self, message: &Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.stdin, message)
            .map_err(|error| format!("windows_sandbox_request_invalid:{error}"))?;
        self.stdin
            .write_all(b"\n")
            .and_then(|()| self.stdin.flush())
            .map_err(|error| format!("windows_sandbox_write_failed:{error}"))
    }
}

impl Drop for ControlSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn normalized_workspace_root(workspace_root: Option<&str>) -> Result<Option<PathBuf>, String> {
    let Some(root) = workspace_root
        .map(str::trim)
        .filter(|root| !root.is_empty())
    else {
        return Ok(None);
    };
    let path = PathBuf::from(root);
    if !path.is_absolute() {
        return Err("windows_sandbox_workspace_root_not_absolute".to_owned());
    }
    if !path.is_dir() {
        return Err(format!(
            "windows_sandbox_workspace_root_unavailable:{}",
            path.display()
        ));
    }
    Ok(Some(path))
}

fn read_control_response(reader: &mut impl BufRead, id: i64) -> Result<Value, String> {
    loop {
        let message = read_control_message(reader)?;
        if message.get("id").and_then(Value::as_i64) == Some(id) {
            if let Some(error) = message.get("error") {
                return Err(format!(
                    "windows_sandbox_rpc_failed:{}",
                    compact_json(error)
                ));
            }
            return Ok(message);
        }
        fail_on_control_server_request(&message)?;
    }
}

fn wait_for_setup_completion(reader: &mut impl BufRead, id: i64) -> Result<(), String> {
    let mut acknowledged = false;
    let mut completion: Option<Result<(), String>> = None;
    loop {
        let message = read_control_message(reader)?;
        if message.get("id").and_then(Value::as_i64) == Some(id) {
            if let Some(error) = message.get("error") {
                return Err(format!(
                    "windows_sandbox_setup_rpc_failed:{}",
                    compact_json(error)
                ));
            }
            if message.pointer("/result/started").and_then(Value::as_bool) != Some(true) {
                return Err(format!(
                    "windows_sandbox_setup_ack_invalid:{}",
                    compact_json(&message)
                ));
            }
            acknowledged = true;
        } else if message.get("method").and_then(Value::as_str)
            == Some(WINDOWS_SANDBOX_SETUP_COMPLETED_METHOD)
        {
            completion = Some(parse_setup_completion(&message));
        } else {
            fail_on_control_server_request(&message)?;
        }
        if acknowledged && let Some(completion) = completion.take() {
            return completion;
        }
    }
}

fn parse_setup_completion(message: &Value) -> Result<(), String> {
    let params = message
        .get("params")
        .filter(|params| params.is_object())
        .ok_or_else(|| {
            format!(
                "windows_sandbox_setup_completion_invalid:{}",
                compact_json(message)
            )
        })?;
    if params.get("mode").and_then(Value::as_str) != Some("unelevated") {
        return Err(format!(
            "windows_sandbox_setup_mode_mismatch:{}",
            compact_json(params)
        ));
    }
    match params.get("success").and_then(Value::as_bool) {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "windows_sandbox_setup_completion_failed:{}",
            params
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Windows 沙箱初始化未完成")
        )),
        None => Err(format!(
            "windows_sandbox_setup_completion_invalid:{}",
            compact_json(params)
        )),
    }
}

fn read_control_message(reader: &mut impl BufRead) -> Result<Value, String> {
    let mut line = String::new();
    let size = reader
        .read_line(&mut line)
        .map_err(|error| format!("windows_sandbox_read_failed:{error}"))?;
    if size == 0 {
        return Err("windows_sandbox_control_closed".to_owned());
    }
    serde_json::from_str(&line).map_err(|error| format!("windows_sandbox_protocol_invalid:{error}"))
}

fn fail_on_control_server_request(message: &Value) -> Result<(), String> {
    if message.get("id").is_some() && message.get("method").is_some() {
        return Err(format!(
            "windows_sandbox_unexpected_server_request:{}",
            compact_json(message)
        ));
    }
    Ok(())
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

#[cfg(test)]
mod tests {
    use super::probe_requires_setup;
    use super::{
        WindowsSandboxReadiness, WindowsSandboxStatus, configure_harness_command,
        configure_harness_skill_creator, parse_readiness_response, require_ready_for_run,
        validate_harness_helper_layout, wait_for_setup_completion,
    };
    use super::{execution_probe_error_message, execution_probe_params, verify_execution_probe};
    use contexthub_platform_adapter::OperatingSystem;
    use serde_json::Value;
    use serde_json::json;
    use std::io::Cursor;
    use std::process::Command;

    #[test]
    fn probe_treats_only_stale_credentials_as_repairable() {
        for repairable in [
            r#"windows_sandbox_probe_failed:windows_sandbox_rpc_failed:{"message":"exec error: windows sandbox: delete C:\\h\\.sandbox-secrets\\sandbox_users.json"}"#,
            "windows_sandbox_probe_failed:LogonUserW failed (os error 1326)",
            "windows_sandbox_probe_failed:Windows sandbox setup is missing or out of date; rerun the sandbox setup with elevation",
        ] {
            assert!(probe_requires_setup(repairable), "{repairable}");
        }
        for reported in [
            "windows_sandbox_probe_failed:exit_code=1",
            "windows_sandbox_probe_failed:LogonUserW failed (os error 1385)",
            "windows_sandbox_probe_failed:windows_sandbox_control_closed",
        ] {
            assert!(!probe_requires_setup(reported), "{reported}");
        }
        let status = WindowsSandboxStatus::credentials_invalid();
        assert_eq!(status.status, "updateRequired");
        assert_eq!(
            status.error_code.as_deref(),
            Some("windows_sandbox_credentials_invalid")
        );
        assert_eq!(
            WindowsSandboxStatus::failed("windows_sandbox_probe_failed:exit_code=1")
                .error_code
                .as_deref(),
            Some("windows_sandbox_probe_failed")
        );
    }

    #[test]
    fn readiness_accepts_all_three_protocol_states() {
        for (value, expected) in [
            ("ready", WindowsSandboxReadiness::Ready),
            ("notConfigured", WindowsSandboxReadiness::NotConfigured),
            ("updateRequired", WindowsSandboxReadiness::UpdateRequired),
        ] {
            assert_eq!(
                parse_readiness_response(&json!({"result": {"status": value}})).unwrap(),
                expected
            );
        }
        assert!(parse_readiness_response(&json!({"result": {"status": "unknown"}})).is_err());
    }

    #[test]
    fn run_preflight_only_accepts_ready() {
        assert!(require_ready_for_run(WindowsSandboxReadiness::Ready).is_ok());
        assert_eq!(
            require_ready_for_run(WindowsSandboxReadiness::NotConfigured),
            Err("windows_sandbox_not_configured".to_owned())
        );
        assert_eq!(
            require_ready_for_run(WindowsSandboxReadiness::UpdateRequired),
            Err("windows_sandbox_update_required".to_owned())
        );
    }

    #[test]
    fn setup_requires_positive_ack_and_matching_successful_completion() {
        let messages = [
            json!({"id": 3, "result": {"started": true}}),
            json!({"method": "windowsSandbox/setupCompleted", "params": {
                "mode": "unelevated", "success": true, "error": null
            }}),
        ]
        .into_iter()
        .map(|message| message.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        assert!(wait_for_setup_completion(&mut Cursor::new(messages), 3).is_ok());

        let bad_ack = json!({"id": 3, "result": {"started": false}}).to_string() + "\n";
        assert!(
            wait_for_setup_completion(&mut Cursor::new(bad_ack), 3)
                .unwrap_err()
                .starts_with("windows_sandbox_setup_ack_invalid:")
        );
    }

    #[test]
    fn setup_preserves_failure_diagnostics_and_rejects_mode_mismatch() {
        let failed = [
            json!({"id": 3, "result": {"started": true}}),
            json!({"method": "windowsSandbox/setupCompleted", "params": {
                "mode": "unelevated", "success": false, "error": "setup failed"
            }}),
        ]
        .into_iter()
        .map(|message| message.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        assert!(
            wait_for_setup_completion(&mut Cursor::new(failed), 3)
                .unwrap_err()
                .contains("setup failed")
        );

        let mismatch = [
            json!({"method": "windowsSandbox/setupCompleted", "params": {
                "mode": "elevated", "success": true, "error": null
            }}),
            json!({"id": 3, "result": {"started": true}}),
        ]
        .into_iter()
        .map(|message| message.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
        assert!(
            wait_for_setup_completion(&mut Cursor::new(mismatch), 3)
                .unwrap_err()
                .starts_with("windows_sandbox_setup_mode_mismatch:")
        );
    }

    #[test]
    fn windows_commands_force_unelevated_sandbox_without_shared_account_setup() {
        let mut windows = Command::new("codex-app-server.exe");
        configure_harness_command(&mut windows, OperatingSystem::Windows);
        let args = windows
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(args, ["-c", r#"windows.sandbox="unelevated""#]);
        assert_eq!(windows.get_envs().count(), 0);

        let mut macos = Command::new("codex-app-server");
        configure_harness_command(&mut macos, OperatingSystem::MacOs);
        assert_eq!(macos.get_args().count(), 0);
        assert_eq!(macos.get_envs().count(), 0);
    }

    #[test]
    fn windows_creator_setting_is_session_only_and_macos_args_are_unchanged() {
        let home = std::path::Path::new(r"C:\Users\example\AppData\Local\词来工作台");
        let mut windows = Command::new("codex-app-server.exe");
        configure_harness_skill_creator(&mut windows, OperatingSystem::Windows, home);
        let args = windows
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-c");
        assert!(args[1].starts_with("skills.config=[{path=\"C:\\\\Users\\\\example"));
        assert!(args[1].ends_with("skills/.system/skill-creator/SKILL.md\",enabled=false}]"));

        let mut macos = Command::new("codex-app-server");
        configure_harness_skill_creator(&mut macos, OperatingSystem::MacOs, home);
        assert_eq!(macos.get_args().count(), 0);
    }

    #[test]
    fn helper_layout_accepts_siblings_or_codex_resources_and_requires_both() {
        let root = std::env::temp_dir().join(format!(
            "cilai-windows-sandbox-layout-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let harness = root.join("cilai-harness.exe");
        std::fs::write(&harness, []).unwrap();
        std::fs::write(root.join("codex-windows-sandbox-setup.exe"), []).unwrap();
        let resources = root.join("codex-resources");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::write(resources.join("codex-command-runner.exe"), []).unwrap();
        assert!(validate_harness_helper_layout(&harness).is_ok());

        std::fs::remove_file(resources.join("codex-command-runner.exe")).unwrap();
        assert!(
            validate_harness_helper_layout(&harness)
                .unwrap_err()
                .contains("codex-command-runner.exe")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn execution_probe_preserves_effective_policy_and_requires_real_success() {
        for policy in [
            json!({"type":"readOnly","networkAccess":false}),
            json!({"type":"workspaceWrite","writableRoots":["C:\\one","D:\\two"],"networkAccess":false,"excludeTmpdirEnvVar":true}),
        ] {
            let thread = json!({"result":{"cwd":"C:\\one","sandbox":policy}});
            let params = execution_probe_params(
                &thread,
                &[
                    "system-cmd".into(),
                    "/d".into(),
                    "/c".into(),
                    "exit 0".into(),
                ],
            )
            .unwrap();
            assert_eq!(params["sandboxPolicy"], policy);
            assert_eq!(params["cwd"], "C:\\one");
            assert_eq!(params["tty"], false);
            assert!(
                params.get("outputBytesCap").is_none(),
                "Windows sandbox rejects custom output caps before execution"
            );
            assert!(params.get("disableOutputCap").is_none());
            assert_eq!(params["streamStdin"], false);
            assert_eq!(params["streamStdoutStderr"], false);
        }
        for policy in [
            json!({"type":"dangerFullAccess"}),
            json!({"type":"externalSandbox"}),
            Value::Null,
        ] {
            assert!(
                execution_probe_params(&json!({"result":{"cwd":"C:\\one","sandbox":policy}}), &[])
                    .is_err()
            );
        }
        assert!(verify_execution_probe(&json!({"result":{"exitCode":0}})).is_ok());
        for response in [
            json!({"error":{"message":"setup refresh had errors"}}),
            json!({"result":{"exitCode":5}}),
            json!({"result":{}}),
        ] {
            assert!(verify_execution_probe(&response).is_err());
        }
    }

    #[test]
    fn execution_probe_errors_distinguish_parameters_from_permissions_without_raw_payloads() {
        for rpc_code in [-32600, -32602] {
            let error = format!(
                "windows_sandbox_preflight_failed:codex_harness_rpc_failed:{}",
                json!({"code":rpc_code,"message":"custom outputBytesCap is not supported with windows sandbox PRIVATE_TOKEN C:\\private\\project","data":{"stdout":"PRIVATE_REPLY"}})
            );
            let public = execution_probe_error_message(&error);
            assert!(public.contains("sandbox_preflight_parameters_invalid"));
            assert!(public.contains("参数不兼容"));
            assert!(!public.contains("权限"));
            assert!(!public.contains("PRIVATE_"));
            assert!(!public.contains("C:\\private"));
            assert!(!public.contains("outputBytesCap"));
        }
        for (detail, code) in [
            (
                "Windows logon failed (os error 1326)",
                "credentials_invalid",
            ),
            ("Windows logon failed (os error 1385)", "logon_denied"),
            ("Access is denied. (os error 5)", "access_denied"),
            ("SetNamedSecurityInfoW failed: 5", "access_denied"),
            (
                "helper_unknown_error: setup refresh had errors",
                "refresh_failed",
            ),
            ("exit_code=124", "timeout"),
            ("request timed out", "timeout"),
            ("exit_code=5", "exit_failed"),
            ("invalid_sandbox_policy", "protocol_invalid"),
            ("missing_cwd", "protocol_invalid"),
            ("invalid_response", "protocol_invalid"),
            ("codex_harness_protocol_invalid", "protocol_invalid"),
            (
                "helper_unknown_error: unexpected helper exit",
                "helper_failed",
            ),
            ("failed to spawn runner", "helper_failed"),
            ("unrecognized PRIVATE_TOKEN C:\\private\\project", "failed"),
            ("codex_harness_rpc_failed:not JSON PRIVATE_TOKEN", "failed"),
        ] {
            let error = format!("windows_sandbox_preflight_failed:{detail}");
            let public = execution_probe_error_message(&error);
            assert!(
                public.ends_with(&format!("sandbox_preflight_{code}")),
                "{public}"
            );
            assert!(public.contains("本轮尚未开始模型操作"));
            assert!(!public.contains("PRIVATE_"));
            assert!(!public.contains("C:\\private"));
        }
    }

    #[test]
    fn status_contract_uses_camel_case_fields() {
        let value = serde_json::to_value(WindowsSandboxStatus::ready_state(
            WindowsSandboxReadiness::UpdateRequired,
        ))
        .unwrap();
        assert_eq!(value["supported"], true);
        assert_eq!(value["status"], "updateRequired");
        assert!(value.get("errorCode").is_some());
        assert!(value.get("errorMessage").is_some());
    }

    #[test]
    fn failed_status_preserves_pre_harness_categories_without_private_details() {
        for (error, expected) in [
            (
                "windows_sandbox_helper_missing:codex-command-runner.exe",
                "helper_missing",
            ),
            (
                "private file is not owned by the current user",
                "private_owner_mismatch",
            ),
            (
                "windows_sandbox_home_failed:runtime directory is not owned by the current user or an accessible administrator child",
                "runtime_owner_mismatch",
            ),
            (
                "windows_sandbox_home_failed:拒绝访问。 (os error 5)",
                "runtime_access_denied",
            ),
            (
                "windows_sandbox_home_failed:runtime directory must not be a symbolic link",
                "runtime_prepare_failed",
            ),
            ("windows_sandbox_log_failed:disk full", "log_failed"),
            ("windows_sandbox_spawn_failed:blocked", "spawn_failed"),
            (
                "windows_sandbox_workspace_root_unavailable:",
                "workspace_unavailable",
            ),
            (
                "windows_sandbox_rpc_failed:{\"message\":\"PRIVATE_PAYLOAD\"}",
                "control_failed",
            ),
            ("windows_sandbox_control_closed", "control_failed"),
            (
                "windows_sandbox_protocol_invalid:unexpected response",
                "control_failed",
            ),
            ("unknown failure", "setup_failed"),
        ] {
            let status = WindowsSandboxStatus::failed(&format!(
                "{error} PRIVATE_PATH C:\\Users\\private\\secrets"
            ));
            assert_eq!(status.status, "error");
            assert_eq!(
                status.error_code,
                Some(format!("windows_sandbox_{expected}"))
            );
            let public = serde_json::to_string(&status).unwrap();
            assert!(!public.contains("PRIVATE_"));
            assert!(!public.contains("secrets"));
        }
        // An unrelated Windows error must not be mislabeled as access denied.
        let status = WindowsSandboxStatus::failed("windows_sandbox_home_failed:(os error 50)");
        assert_eq!(
            status.error_code.as_deref(),
            Some("windows_sandbox_runtime_prepare_failed")
        );
    }
}
