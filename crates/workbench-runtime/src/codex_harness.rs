use crate::application_tool_adapter::{AdapterSlot, ApplicationToolAdapter, ScopeValidator};
use crate::persistent_threads::PersistentThread;
use crate::work_classification;
use contexthub_platform_adapter::executable_search::validate_executable_file;
#[cfg(not(test))]
use contexthub_platform_adapter::executable_search::{BundledExecutable, bundled_executable_path};
use contexthub_platform_adapter::{OperatingSystem, current_operating_system};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "native_thread_reuse.rs"]
mod native_thread_reuse;
use native_thread_reuse::NativeThread;

const STEER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const TOOL_OUTPUT_TOO_LARGE_RESPONSE: &str =
    "output exceeds maximal length 1000000, original too long.";
const CODEX_HARNESS_VERSION: &str = "0.163.0";
const CODEX_MODEL_CATALOG_SOURCE: &str =
    include_str!("../../../third_party/openai-codex/codex-rs/models-manager/models.json");
const CODEX_PROMPT_MODEL: &str = "gpt-5.6-sol";
pub(crate) const PRODUCT_IDENTITY_INSTRUCTIONS: &str = "你是词来。对外身份统一为产品名称“词来”，不使用任何底层模型、模型供应商、执行框架或本地智能体的名称作为自称。用户询问你的名字或身份时，简短回答“我是词来。”，不附加模型名称、版本或运行框架介绍。历史对话中的其他自称不代表你的当前身份。";
/// 替换上游默认的压缩提示词。压缩后接手的模型看不到工具调用和输出，上游提示词
/// 不要求记下已执行的操作，实测压缩后同一条命令又被执行了两次；有副作用的操作
/// 重复执行会造成重复写入，所以这里要求逐条交代已完成的操作并禁止重做。
const COMPACT_PROMPT: &str = "你正在为同一任务做上下文交接。之前的对话将被这份摘要替换，接手的模型只能看到这份摘要和用户的原话，看不到任何工具调用及其输出。请用中文写一份结构化摘要，包含：
1. 用户目标与约束：用户要完成什么，明确提出的要求、偏好和限制。
2. 已完成的操作：逐条列出已经执行过的命令、读取或写入过的文件、调用过的工具和业务写入、已登记的产物，写明操作对象和结果（成功、失败或返回的关键数据）。这些操作已经发生，接手后不得重复执行；需要结果时直接引用这里的记录。
3. 关键数据：后续回答要用到的具体数值、编号、路径和结论，原样保留。
4. 未完成事项：还差哪些步骤，下一步具体做什么。
5. 待确认问题：正在等待用户决定的事项。
只记录真实发生的事，不要把计划写成已完成的结果。";
const APPLICATION_TOOL_SEARCH_MAX_CALLS: usize = 3;
const APPLICATION_TOOL_SEARCH_MAX_RESULTS: usize = 8;
const APPLICATION_TOOL_SEARCH_MAX_APPLICATIONS: usize = 3;
const APPLICATION_MCP_SERVER_NAME: &str = "cilai_application";
const APPLICATION_WRITE_TOOL_NAMESPACE: &str = "cilai_application_writes";
const ADDITIONAL_CONTEXT_PART_BYTES: usize = 3_000;
const ADDITIONAL_CONTEXT_TOTAL_BYTES: usize = 4 * 1024 * 1024;

pub(crate) struct CodexHarnessRequest {
    pub turn_admission: Option<ScopeValidator>,
    pub validation_probe: bool,
    pub execution: Option<crate::model_connections::ExecutionBundle>,
    pub memory_model: Option<String>,
    pub conversation_kind: crate::conversation_kernel::ConversationKind,
    pub planning: bool,
    pub executable: String,
    pub harness_home: PathBuf,
    pub tool_snapshot_scope_id: String,
    /// 本次运行所属的对话。浏览器按它归属标签——浏览器是某一次对话里开出来的，切到别的
    /// 对话就不该看到它。应用内模型调用不属于任何对话，此处为空。
    pub conversation_id: Option<String>,
    pub workspace_root: PathBuf,
    pub workspace_roots: Vec<PathBuf>,
    pub model: String,
    pub model_provider: String,
    pub provider_label: String,
    pub base_url: String,
    pub api_key_env: String,
    pub api_key: String,
    pub context_window: Option<u64>,
    pub reasoning_effort: Option<String>,
    pub model_catalog: Vec<CodexHarnessModel>,
    pub developer_instructions: String,
    pub classification: Option<CodexHarnessClassificationRequest>,
    pub report_result: Option<crate::report_result::ReportResultContract>,
    pub application_result: Option<crate::app_bridge::ApplicationModelTask>,
    pub user_input: String,
    /// 工作台为本轮附加的材料与附件说明，经附加上下文交给模型，不写入用户消息。
    pub turn_context: String,
    pub conversation_text: String,
    pub attachments: Vec<CodexHarnessAttachment>,
    pub skills: Vec<CodexHarnessSkill>,
    pub personal_tools: Option<PersonalToolsCallback>,
    pub user_mcp: Option<Arc<crate::mcp_connections::broker::McpBroker>>,
    pub application_action: Option<ApplicationActionCallback>,
    pub application_write: Option<ApplicationWriteCallback>,
    pub application_mcp: Option<ApplicationMcpScope>,
    pub resume_thread_id: Option<String>,
    pub persistent_thread: Option<PersistentThread>,
    pub application_adapter: Option<(String, AdapterSlot, ScopeValidator)>,
    pub approval_mode: String,
    pub approval_review_model: Option<String>,
    pub memory_preferences: crate::memory::MemoryPreferences,
    pub sandbox_mode: String,
    pub event_log_path: PathBuf,
    pub stderr_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodexHarnessSkill {
    pub skill_id: String,
    pub name: String,
    pub path: PathBuf,
    pub explicit: bool,
}

fn authorized_skill_roots(skills: &[CodexHarnessSkill]) -> Result<Vec<String>, String> {
    let mut roots = skills
        .iter()
        .map(|skill| {
            if !skill.path.is_absolute()
                || skill.path.file_name().and_then(|name| name.to_str()) != Some("SKILL.md")
            {
                return Err("codex_harness_skill_root_invalid".to_owned());
            }
            skill
                .path
                .parent()
                .filter(|parent| parent.parent().is_some())
                .and_then(|parent| parent.to_str())
                .map(str::to_owned)
                .ok_or_else(|| "codex_harness_skill_root_invalid".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    roots.sort();
    roots.dedup();
    Ok(roots)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApplicationMcpScope {
    pub application_id: String,
    pub application_name: String,
    pub application_ids: Vec<String>,
    pub restrict_to_applications: bool,
    pub browser_origins: Vec<String>,
    pub report_review: bool,
    pub tool_names: Vec<String>,
    pub read_tools: Vec<ApplicationReadToolScope>,
    pub write_tools: Vec<ApplicationWriteToolScope>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApplicationReadToolScope {
    pub application_id: String,
    pub tool_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApplicationWriteToolScope {
    pub application_id: String,
    pub application_name: String,
    pub dynamic_name: String,
    pub tool_name: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub risk_level: String,
    pub timeout_ms: Option<u32>,
    pub confirmation_view: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ModelToolCapabilities {
    pub version: u32,
    pub profile: String,
    pub function_calling: bool,
    pub client_tool_search: bool,
    pub custom_tools: bool,
    pub web_search_mode: String,
}

impl ModelToolCapabilities {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || !matches!(
                self.profile.as_str(),
                "responses" | "responses-preview" | "responses-functions" | "functions"
            )
            || !matches!(
                self.web_search_mode.as_str(),
                "disabled" | "cached" | "live"
            )
        {
            return Err("模型工具能力配置无效，请同步模型目录".to_owned());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodexHarnessModel {
    pub model_id: String,
    pub display_name: String,
    pub context_window: Option<u64>,
    pub supported_reasoning_efforts: Vec<String>,
    pub default_reasoning_effort: Option<String>,
    pub tool_capabilities: Option<ModelToolCapabilities>,
    pub input_modalities: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodexHarnessAttachment {
    pub media_type: String,
    pub path: PathBuf,
}

type ClassificationValidator = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub(crate) struct CodexHarnessClassificationRequest {
    pub developer_instructions: String,
    pub output_schema: Value,
    pub validate: ClassificationValidator,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CodexHarnessUsage {
    pub input: u64,
    pub cached_input: u64,
    pub output: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodexHarnessEvent {
    pub event_type: String,
    pub item_id: Option<String>,
    pub phase: String,
    pub kind: String,
    pub status: String,
    pub summary: String,
    pub duration_ms: Option<u64>,
    pub detail_available: bool,
    pub streamed_output: Option<String>,
    pub steered_input: Option<(String, String)>,
    pub context_usage: Option<(u64, Option<u64>)>,
    pub read_paths: Vec<String>,
    pub created_at: i64,
}

impl CodexHarnessEvent {
    fn status(event_type: &str, phase: &str, status: &str, summary: impl Into<String>) -> Self {
        Self {
            event_type: event_type.to_owned(),
            item_id: None,
            phase: phase.to_owned(),
            kind: "status".to_owned(),
            status: status.to_owned(),
            summary: summary.into(),
            duration_ms: None,
            detail_available: false,
            streamed_output: None,
            steered_input: None,
            read_paths: Vec::new(),
            context_usage: None,
            created_at: now_millis(),
        }
    }
}

/// 员工的决定按各条审批通道各自的格式回给 harness。三种通道的报文形状互不相同。
fn approval_result(
    method: &str,
    decision: &str,
    requested_permissions: Option<&Value>,
) -> Result<Value, String> {
    let session = decision == "approved_session";
    Ok(match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => json!({
            "decision": if decision == "rejected" {
                "decline"
            } else if session {
                "acceptForSession"
            } else {
                "accept"
            }
        }),
        "item/permissions/requestApproval" => json!({
            "permissions": if decision == "rejected" {
                json!({})
            } else {
                requested_permissions.cloned().unwrap_or_else(|| json!({}))
            },
            "scope": if session { "session" } else { "turn" }
        }),
        /*
         * MCP 工具审批走 elicitation 的回复格式：动作在 action 上，"这次批过之后还算不算数"
         * 在 _meta.persist 上。两者分开——只回 accept 而不带 persist，harness 就只放行这一次。
         */
        MCP_TOOL_APPROVAL_METHOD => {
            let mut result = json!({
                "action": if decision == "rejected" { "decline" } else { "accept" },
                "content": {}
            });
            if session && let Some(object) = result.as_object_mut() {
                object.insert("_meta".to_owned(), json!({ "persist": "session" }));
            }
            result
        }
        _ => return Err("codex_harness_approval_method_unsupported".to_owned()),
    })
}

/// Harness 把 MCP 工具调用的审批走 elicitation 通道发过来。
const MCP_TOOL_APPROVAL_METHOD: &str = "mcpServer/elicitation/request";

#[derive(Clone, Debug)]
pub(crate) struct CodexHarnessApprovalRequest {
    pub request_id: Value,
    pub method: String,
    pub item_id: String,
    pub reason: String,
    pub command: Option<String>,
    pub resource: Option<String>,
    pub requested_permissions: Option<Value>,
    pub available_decisions: Vec<String>,
    pub requested_at: i64,
}

#[derive(Debug)]
pub(crate) struct CodexHarnessResult {
    pub thread_id: String,
    pub output: String,
    pub classification_output: Option<String>,
    pub classification_error: Option<String>,
    pub file_changes: Vec<CodexHarnessFileChange>,
    pub usage: CodexHarnessUsage,
    pub reply_usage: Option<CodexHarnessUsage>,
    pub classification_usage: Option<CodexHarnessUsage>,
    pub context_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub context_composition: Option<CodexHarnessContextComposition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_field_names)]
pub(crate) struct CodexHarnessContextComposition {
    pub conversation_tokens: u64,
    pub system_prompt_tokens: u64,
    pub tool_tokens: u64,
    pub skill_tokens: u64,
    pub mcp_tokens: Option<u64>,
    pub internal_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodexHarnessFileChange {
    pub path: String,
    pub kind: String,
}

#[derive(Debug)]
struct CodexHarnessTurnResult {
    output: String,
    classification_output: Option<String>,
    file_changes: Vec<CodexHarnessFileChange>,
    usage: CodexHarnessUsage,
    usage_reported: bool,
    context_tokens: Option<u64>,
    context_window: Option<u64>,
    discovered_tool_tokens: u64,
}

pub(crate) type ProgressCallback = Arc<dyn Fn(CodexHarnessEvent) + Send + Sync>;
pub(crate) type ApprovalCallback = Arc<dyn Fn(CodexHarnessApprovalRequest) + Send + Sync>;
pub(crate) type ArtifactCallback = Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;
pub(crate) type PersonalToolsCallback = Arc<dyn Fn(Value) -> Result<Value, String> + Send + Sync>;
pub(crate) type ApplicationActionCallback =
    Arc<dyn Fn(&str, Value) -> Result<Value, String> + Send + Sync>;
pub(crate) type ApplicationWriteCallback =
    Arc<dyn Fn(&ApplicationWriteToolScope, Value) -> Result<Value, String> + Send + Sync>;
type PendingResponses = Arc<Mutex<std::collections::HashMap<i64, PendingSteerResponse>>>;

struct PendingSteerResponse {
    sender: mpsc::Sender<Result<(), String>>,
    client_message_id: String,
    input: String,
}

const REGISTER_ARTIFACT_TOOL: &str = "cilai_register_artifact";
const APPLICATION_ACTION_TOOL: &str = "cilai_application_action";

fn personal_tools_definition() -> Value {
    json!({"name":"cilai_personal_tools","description":"读取个人能力目录或调用已安装的个人工具。先 action=list 获取能力，再携带 creationProjectId 获取工具及参数格式。调用必须携带当前 packageDigest；工作台将向你确认最终参数。仅使用获准的本地个人能力。",
        "inputSchema":{"type":"object","properties":{"action":{"type":"string","enum":["list","call"]},"creationProjectId":{"type":"string"},"packageDigest":{"type":"string"},"tool":{"type":"string"},"arguments":{"type":"object"}},"required":["action"],"additionalProperties":false}})
}

fn handle_personal_tools_request(
    message: &Value,
    thread_id: &str,
    turn_id: &str,
    callback: Option<&PersonalToolsCallback>,
    send: &mut impl FnMut(&Value) -> Result<(), String>,
) -> Result<bool, String> {
    if message["method"] != "item/tool/call" || message["params"]["tool"] != "cilai_personal_tools"
    {
        return Ok(false);
    }
    let id = message.get("id").ok_or("personal_tool_request_invalid")?;
    let params = &message["params"];
    let result = if params["threadId"] != thread_id || params["turnId"] != turn_id {
        Err("个人工具请求不属于当前运行。".into())
    } else if let Some(callback) = callback {
        callback(params["arguments"].clone())
    } else {
        Err("本轮未允许个人工具运行。".into())
    };
    let success = result.is_ok();
    let text = result.map_or_else(|error: String| error, |value| value.to_string());
    send(
        &json!({"id":id,"result":{"success":success,"contentItems":[{"type":"inputText","text":text}]}}),
    )?;
    Ok(true)
}

fn register_artifact_tool() -> Value {
    json!({
        "type": "function",
        "name": REGISTER_ARTIFACT_TOOL,
        "description": "登记本轮对话已经创建或更新且当前真实存在的产物路径。每个产物完成后调用一次；相对路径按当前工作目录解析，工作目录外的绝对路径也可以登记。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "minLength": 1 }
            },
            "required": ["path"],
            "additionalProperties": false
        }
    })
}

fn application_action_tool() -> Value {
    json!({
        "type": "function",
        "name": APPLICATION_ACTION_TOOL,
        "description": "仅在本轮明确提供了应用页面上下文且用户要求操作页面时，调用当前页面已登记的读取或页面草稿语义动作。不得传入选择器、坐标、脚本或正式业务写入请求。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "actionId": { "type": "string", "minLength": 1 },
                "arguments": { "type": "object" }
            },
            "required": ["actionId", "arguments"],
            "additionalProperties": false
        }
    })
}

fn application_write_tool(tool: &ApplicationWriteToolScope) -> Value {
    json!({
        "type": "function",
        "name": tool.dynamic_name,
        "deferLoading": true,
        "description": format!(
            "应用：{}（{}）。工具：{}。{}。这是{}操作，参数完整后将由词来展示精确内容并要求员工逐次确认；未确认不得提交。",
            tool.application_name,
            tool.application_id,
            tool.tool_name,
            tool.description,
            if tool.risk_level == "highRisk" { "高风险写入" } else { "业务写入" }
        ),
        "inputSchema": application_write_model_schema(tool)
    })
}

fn application_write_tool_namespace(tools: &[ApplicationWriteToolScope]) -> Value {
    json!({
        "type": "namespace",
        "name": APPLICATION_WRITE_TOOL_NAMESPACE,
        "description": "当前员工已授权的企业应用写入工具。只在用户明确要求业务写入且参数完整时调用。",
        "tools": tools.iter().map(application_write_tool).collect::<Vec<_>>()
    })
}

fn application_tool_search_metadata(enabled: bool) -> Value {
    json!({
        "cilai_application_tool_search": if enabled { "enabled" } else { "disabled" },
        "cilai_application_tool_search_max_calls": APPLICATION_TOOL_SEARCH_MAX_CALLS.to_string(),
        "cilai_application_tool_search_max_results": APPLICATION_TOOL_SEARCH_MAX_RESULTS.to_string(),
        "cilai_application_tool_search_max_applications": APPLICATION_TOOL_SEARCH_MAX_APPLICATIONS.to_string()
    })
}

pub(crate) fn application_write_model_schema(tool: &ApplicationWriteToolScope) -> Value {
    let mut schema = tool.input_schema.clone();
    let Some(host_fields) = tool
        .confirmation_view
        .as_ref()
        .and_then(|view| view.get("hostGeneratedFields"))
        .and_then(Value::as_array)
    else {
        return schema;
    };
    let Some(object) = schema.as_object_mut() else {
        return schema;
    };
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        for field in host_fields.iter().filter_map(Value::as_str) {
            properties.remove(field);
        }
    }
    if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|field| {
            field
                .as_str()
                .is_none_or(|field| !host_fields.iter().any(|item| item.as_str() == Some(field)))
        });
    }
    schema
}

#[derive(Clone)]
pub(crate) struct CodexHarnessControl {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    active_turn: Arc<Mutex<Option<ActiveTurn>>>,
    terminal_thread: Arc<Mutex<Option<String>>>,
    native_thread: Arc<Mutex<Option<NativeThread>>>,
    pending_responses: PendingResponses,
    accepted_steers: Arc<Mutex<std::collections::HashMap<String, String>>>,
    next_request_id: Arc<AtomicI64>,
    journal: Option<crate::persistent_threads::ThreadJournal>,
}

#[derive(Clone)]
struct ActiveTurn {
    thread_id: String,
    turn_id: String,
}

pub(crate) struct SpawnedCodexHarness {
    pub pid: u32,
    pub control: CodexHarnessControl,
}

#[derive(Default)]
pub(crate) struct HarnessProcessState {
    pub pid: AtomicU32,
    pub exited: AtomicBool,
}

type HarnessLifetime = (Box<dyn Send>, Option<Arc<HarnessProcessState>>);

struct HarnessChild {
    child: Child,
    keep_alive: Option<HarnessLifetime>,
    executions: Arc<Mutex<Vec<crate::model_connections::ExecutionBundle>>>,
    execution_monitor_stop: Arc<AtomicBool>,
    execution_monitor: Option<std::thread::JoinHandle<()>>,
}

impl HarnessChild {
    fn new(child: Child, keep_alive: HarnessLifetime) -> Self {
        if let Some(state) = keep_alive.1.as_ref() {
            state.pid.store(child.id(), Ordering::Release);
        }
        Self {
            child,
            keep_alive: Some(keep_alive),
            executions: Arc::new(Mutex::new(Vec::new())),
            execution_monitor_stop: Arc::new(AtomicBool::new(false)),
            execution_monitor: None,
        }
    }

    fn attach_execution(
        &mut self,
        execution: crate::model_connections::ExecutionBundle,
        progress: ProgressCallback,
    ) -> Result<(), String> {
        self.executions
            .lock()
            .map_err(|_| "model_execution_monitor_failed")?
            .push(execution);
        if self.execution_monitor.is_some() {
            return Ok(());
        }
        let pid = self.child.id();
        let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
            .map_err(|_| "model_execution_process_unavailable")?
            .ok_or("model_execution_process_unavailable")?;
        let executions = Arc::clone(&self.executions);
        let stop = Arc::clone(&self.execution_monitor_stop);
        self.execution_monitor = Some(
            std::thread::Builder::new()
                .name(format!("cilai-model-lease-{pid}"))
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        let valid = executions.lock().is_ok_and(|scopes| {
                            scopes
                                .iter()
                                .all(|scope| scope.validate_lifecycle().is_ok())
                        });
                        if !valid {
                            if contexthub_platform_adapter::instance::process_start_fingerprint(pid)
                                .ok()
                                .flatten()
                                .as_deref()
                                == Some(fingerprint.as_str())
                            {
                                let _ = contexthub_platform_adapter::process::terminate(pid);
                                progress(CodexHarnessEvent::status(
                                    "model.execution_revoked",
                                    "lifecycle",
                                    "failed",
                                    "模型执行授权已失效，正在停止运行",
                                ));
                            }
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                })
                .map_err(|_| "model_execution_monitor_failed")?,
        );
        Ok(())
    }

    fn release_executions_after_exit(&mut self) -> Result<(), String> {
        if self
            .try_wait()
            .map_err(|_| "model_execution_exit_unconfirmed")?
            .is_none()
        {
            return Err("model_execution_exit_unconfirmed".into());
        }
        self.execution_monitor_stop.store(true, Ordering::Release);
        if let Some(monitor) = self.execution_monitor.take() {
            monitor
                .join()
                .map_err(|_| "model_execution_monitor_unconfirmed")?;
        }
        let mut executions = self
            .executions
            .lock()
            .map_err(|_| "model_execution_release_unconfirmed")?;
        for execution in executions.iter() {
            execution
                .finish_release()
                .map_err(|_| "model_execution_release_unconfirmed")?;
        }
        executions.clear();
        Ok(())
    }

    fn confirmed_exit(&self) {
        self.execution_monitor_stop.store(true, Ordering::Release);
        if let Some((_, Some(state))) = self.keep_alive.as_ref() {
            state.exited.store(true, Ordering::Release);
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.confirmed_exit();
        }
        Ok(status)
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.child.wait()?;
        self.confirmed_exit();
        Ok(status)
    }
}

impl std::ops::Deref for HarnessChild {
    type Target = Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl std::ops::DerefMut for HarnessChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

impl Drop for HarnessChild {
    fn drop(&mut self) {
        let stopped = match self.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) | Err(_) => self.child.kill().and_then(|()| self.wait().map(|_| ())),
        };
        if let Err(error) = &stopped {
            eprintln!("harness_runtime_retire_failed:{error}");
        }
        self.execution_monitor_stop.store(true, Ordering::Release);
        if let Some(monitor) = self.execution_monitor.take() {
            let _ = monitor.join();
        }
        if stopped.is_ok()
            && let Ok(mut executions) = self.executions.lock()
        {
            for execution in executions.drain(..) {
                if execution.finish_release().is_err() {
                    eprintln!("personal_model_execution_release_unconfirmed");
                }
            }
        }
        self.keep_alive.take();
    }
}

#[cfg(test)]
#[test]
fn process_owner_reaps_the_official_child_before_releasing_its_lifetime() {
    struct Lifetime(Arc<HarnessProcessState>, Arc<AtomicBool>);
    impl Drop for Lifetime {
        fn drop(&mut self) {
            assert!(self.0.exited.load(Ordering::Acquire));
            self.1.store(true, Ordering::Release);
        }
    }
    let root = std::env::temp_dir().join(format!("cilai-process-owner-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let state = Arc::new(HarnessProcessState::default());
    let released = Arc::new(AtomicBool::new(false));
    let child = Command::new(bundled_codex_harness_executable().unwrap())
        .args(["--listen", "stdio://", "-c", "features.memories=false"])
        .env("CODEX_HOME", &root)
        .env("CODEX_SQLITE_HOME", &root)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let owner = HarnessChild::new(
        child,
        (
            Box::new(Lifetime(Arc::clone(&state), Arc::clone(&released))),
            Some(Arc::clone(&state)),
        ),
    );
    assert_eq!(state.pid.load(Ordering::Acquire), pid);
    assert!(!released.load(Ordering::Acquire));
    let transferred = Some(owner);
    drop(transferred);
    assert!(released.load(Ordering::Acquire));
    fs::remove_dir_all(root).unwrap();
}

pub(crate) struct CachedHarness {
    native_thread: Option<NativeThread>,
    child: Option<HarnessChild>,
    input: Option<ChildStdin>,
    reader: Option<BufReader<ChildStdout>>,
    fingerprint: String,
    pub proxy: Arc<crate::model_session_proxy::ModelSessionProxy>,
}

impl CachedHarness {
    pub(crate) fn retire(&mut self) -> Result<(), String> {
        if self.child.is_none() {
            return Ok(());
        }
        self.proxy.retire();
        self.input.take();
        if let Some(child) = self.child.as_mut() {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Ok(None) => {
                        child
                            .kill()
                            .map_err(|error| format!("harness_runtime_retire_failed:{error}"))?;
                        child
                            .wait()
                            .map_err(|error| format!("harness_runtime_retire_failed:{error}"))?;
                        break;
                    }
                    Err(error) => return Err(format!("harness_runtime_retire_failed:{error}")),
                }
            }
        }
        self.child.take();
        if let Some(native) = self.native_thread.take() {
            native.journal.release()?;
        }
        Ok(())
    }
}

impl Drop for CachedHarness {
    fn drop(&mut self) {
        if let Err(error) = self.retire() {
            eprintln!("harness_runtime_retire_failed:{error}");
        }
    }
}

pub(crate) struct HarnessReuse {
    pub lease: crate::harness_pool::Lease<CachedHarness>,
    pub cached: Option<CachedHarness>,
    pub proxy: Arc<crate::model_session_proxy::ModelSessionProxy>,
}

fn runtime_home_isolation(
    request: &CodexHarnessRequest,
) -> Result<
    Option<contexthub_platform_adapter::macos_runtime_isolation::ChatgptIsolationSnapshot>,
    String,
> {
    let Some(snapshot) = request
        .execution
        .as_ref()
        .and_then(|bundle| bundle.foreground.home_isolation())
    else {
        return Ok(None);
    };
    if request.sandbox_mode == "danger-full-access" {
        return Err("model_runtime_isolation_full_access_not_supported".into());
    }
    let mut read_only = request.workspace_roots.clone();
    read_only.push(request.workspace_root.clone());
    read_only.extend(
        request
            .attachments
            .iter()
            .map(|attachment| attachment.path.clone()),
    );
    read_only.extend(
        request
            .skills
            .iter()
            .filter_map(|skill| skill.path.parent().map(Path::to_path_buf)),
    );
    let mut writable = vec![request.harness_home.clone()];
    if request.sandbox_mode == "workspace-write" {
        writable.extend(request.workspace_roots.clone());
        writable.push(request.workspace_root.clone());
    }
    snapshot
        .with_file_access(&read_only, &writable)
        .map(Some)
        .map_err(|_| "model_runtime_file_permissions_invalid".into())
}

fn runtime_fingerprint(request: &CodexHarnessRequest) -> String {
    // 只散列不可热更新的进程配置。逐轮指令、输入和权限回执仍由本轮线程恢复协议重新传入。
    crate::persistent_threads::digest(&json!({
        "executable":request.executable,"home":request.harness_home,
        "workspace":request.workspace_root,"roots":request.workspace_roots,
        "model":request.model,"provider":request.model_provider,"providerLabel":request.provider_label,
        "baseUrl":request.base_url,"keyEnv":request.api_key_env,"key":request.api_key,
        "window":request.context_window,"models":format!("{:?}",request.model_catalog),
        "planning":request.planning,"scope":format!("{:?}",request.application_mcp),
        "approval":request.approval_mode,"reviewModel":request.approval_review_model,
        "memory":request.memory_preferences,"memoryModel":request.memory_model,"sandbox":request.sandbox_mode,
        "instructions":request.developer_instructions,"skills":format!("{:?}",request.skills),
        "browserUrl":std::env::var("CILAI_BROWSER_MCP_URL").ok(),
        "browserToken":std::env::var("CILAI_BROWSER_MCP_TOKEN").ok(),
        "userMcp":request.user_mcp.as_ref().map(|broker|json!({"url":broker.url,"bearer":broker.bearer})),
        "homeIsolation":request.execution.as_ref().and_then(|bundle| bundle.foreground.home_isolation()).map(contexthub_platform_adapter::macos_runtime_isolation::ChatgptIsolationSnapshot::digest),
        "networkRoute":request.execution.as_ref().and_then(|bundle| bundle.foreground.network().ok()).map(crate::model_connections::ResolvedNetwork::snapshot_digest),
    }))
}

pub(crate) fn bundled_codex_harness_executable() -> Result<String, String> {
    let os = current_operating_system();
    // 单元测试与 App Server 共用本轮构建的 sidecar，Cargo 的 deps 目录没有随包运行时。
    #[cfg(test)]
    let harness = crate::supervisor::codex_harness_test_binary();
    #[cfg(not(test))]
    let harness = bundled_executable_path(BundledExecutable::CodexHarness)
        .map_err(|error| format!("codex_harness_bundled_runtime_unavailable:{error}"))?;
    if let Err(error) = validate_executable_file(&harness) {
        match error.kind() {
            std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "codex_harness_bundled_runtime_missing:{}",
                    harness.display()
                ));
            }
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::PermissionDenied => {
                return Err(format!(
                    "codex_harness_bundled_runtime_not_executable:{}",
                    harness.display()
                ));
            }
            _ => {
                return Err(format!("codex_harness_bundled_runtime_unavailable:{error}"));
            }
        }
    }
    if os == OperatingSystem::Windows {
        crate::windows_sandbox::validate_harness_helper_layout(&harness)?;
    }
    Ok(harness.to_string_lossy().into_owned())
}

impl CodexHarnessControl {
    #[cfg(test)]
    pub(crate) fn test_input(stdin: ChildStdin) -> Self {
        Self::new(stdin)
    }

    fn new(stdin: ChildStdin) -> Self {
        Self {
            stdin: Arc::new(Mutex::new(Some(stdin))),
            active_turn: Arc::new(Mutex::new(None)),
            terminal_thread: Arc::new(Mutex::new(None)),
            native_thread: Arc::new(Mutex::new(None)),
            pending_responses: Arc::new(Mutex::new(std::collections::HashMap::new())),
            accepted_steers: Arc::new(Mutex::new(std::collections::HashMap::new())),
            next_request_id: Arc::new(AtomicI64::new(10_000)),
            journal: None,
        }
    }

    pub(crate) fn steer(&self, input: &str, client_message_id: &str) -> Result<(), String> {
        let active_turn = self
            .active_turn
            .lock()
            .map_err(|_| "codex_harness_control_poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "codex_harness_turn_not_active".to_owned())?;
        if let Some(journal) = &self.journal {
            journal.prepare_input(
                client_message_id,
                client_message_id,
                &crate::persistent_threads::digest(&json!({"content":input})),
                true,
                input,
            )?;
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        self.pending_responses
            .lock()
            .map_err(|_| "codex_harness_control_poisoned".to_owned())?
            .insert(
                request_id,
                PendingSteerResponse {
                    sender,
                    client_message_id: client_message_id.to_owned(),
                    input: input.to_owned(),
                },
            );
        let request = json!({
            "method": "turn/steer",
            "id": request_id,
            "params": {
                "threadId": active_turn.thread_id,
                "expectedTurnId": active_turn.turn_id,
                "clientUserMessageId": client_message_id,
                "input": [{ "type": "text", "text": input }]
            }
        });
        if let Err(error) = self.send(&request) {
            let _ = self
                .pending_responses
                .lock()
                .map(|mut pending| pending.remove(&request_id));
            return Err(error);
        }
        match receiver.recv_timeout(STEER_RESPONSE_TIMEOUT) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("codex_harness_steer_timed_out".to_owned()),
        }
    }

    pub(crate) fn resolve_approval(
        &self,
        request_id: &Value,
        method: &str,
        decision: &str,
        requested_permissions: Option<&Value>,
    ) -> Result<(), String> {
        let result = approval_result(method, decision, requested_permissions)?;
        self.send(&json!({ "id": request_id, "result": result }))
    }

    fn send(&self, message: &Value) -> Result<(), String> {
        let mut stdin = self
            .stdin
            .lock()
            .map_err(|_| "codex_harness_control_poisoned".to_owned())?;
        send(stdin.as_mut().ok_or("codex_harness_stdin_closed")?, message)
    }

    pub(crate) fn close_input(&self) -> Result<(), String> {
        self.stdin
            .lock()
            .map_err(|_| "codex_harness_control_poisoned".to_owned())?
            .take();
        Ok(())
    }

    fn set_active_turn(&self, thread_id: &str, turn_id: &str) -> Result<(), String> {
        *self
            .active_turn
            .lock()
            .map_err(|_| "codex_harness_control_poisoned".to_owned())? = Some(ActiveTurn {
            thread_id: thread_id.to_owned(),
            turn_id: turn_id.to_owned(),
        });
        Ok(())
    }

    fn clear_active_turn(&self) {
        if let Ok(mut active_turn) = self.active_turn.lock() {
            *active_turn = None;
        }
    }

    fn resolve_pending_response(&self, message: &Value) -> bool {
        let Some(request_id) = message.get("id").and_then(Value::as_i64) else {
            return false;
        };
        let pending = self
            .pending_responses
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&request_id));
        let Some(pending) = pending else {
            return false;
        };
        let mut result = message.get("error").map_or_else(
            || Ok(()),
            |error| {
                if error["code"] == -32600 && error["message"] == "no active turn to steer" {
                    Err("codex_harness_turn_not_active".to_owned())
                } else {
                    Err(format!(
                        "codex_harness_steer_failed:{}",
                        compact_json(error)
                    ))
                }
            },
        );
        if let Some(journal) = self.journal.as_ref() {
            let recorded = if result.is_ok() {
                self.active_turn
                    .lock()
                    .map_err(|_| "codex_harness_control_poisoned".to_owned())
                    .and_then(|turn| {
                        turn.as_ref()
                            .ok_or_else(|| "codex_harness_turn_not_active".to_owned())
                            .and_then(|turn| {
                                journal.accept_input(
                                    &pending.client_message_id,
                                    &turn.thread_id,
                                    &turn.turn_id,
                                )
                            })
                    })
            } else {
                journal.record(
                    &format!("input:{}", pending.client_message_id),
                    "reject",
                    json!({"reason":"native_rejected"}),
                )
            };
            if let Err(error) = recorded {
                result = Err(error);
            }
        }
        if result.is_ok()
            && let Ok(mut accepted) = self.accepted_steers.lock()
        {
            accepted.insert(pending.client_message_id, pending.input);
        }
        let _ = pending.sender.send(result);
        true
    }

    fn take_accepted_steer(&self, client_message_id: &str) -> Option<String> {
        if let Some(input) = self.accepted_steers.lock().ok()?.remove(client_message_id) {
            return Some(input);
        }
        self.pending_responses
            .lock()
            .ok()?
            .values()
            .find(|pending| pending.client_message_id == client_message_id)
            .map(|pending| pending.input.clone())
    }
}

fn prepare_persistent_application_adapter(
    request: &CodexHarnessRequest,
    progress: &ProgressCallback,
) -> Result<(), String> {
    let Some((_, slot, validate)) = request.application_adapter.as_ref() else {
        return Ok(());
    };
    let _preparation = slot.preparation_guard();
    let result = (|| {
        validate()?;
        if request
            .application_mcp
            .as_ref()
            .is_some_and(|scope| scope.report_review)
        {
            return Err("enterprise_report_review_removed".to_owned());
        }
        if request
            .application_mcp
            .as_ref()
            .is_some_and(|scope| !scope.read_tools.is_empty() || !scope.tool_names.is_empty())
        {
            return Err("enterprise_application_tools_removed".to_owned());
        }
        let mut adapter =
            ApplicationToolAdapter::new_local(request.application_write.clone(), validate.clone());
        if let Some(scope) = request.application_mcp.as_ref() {
            for tool in &scope.write_tools {
                adapter.add_write(tool.clone())?;
            }
        }
        crate::report_workspace::configure_adapter(&mut adapter, request)?;
        if let Some(contract) = request.report_result.as_ref() {
            contract.add_to_adapter(&mut adapter)?;
        }
        if !request.planning
            && let Some(callback) = request.personal_tools.as_ref()
        {
            adapter.add_personal(personal_tools_definition(), callback.clone())?;
        }
        validate()?;
        Ok(Arc::new(adapter))
    })();
    slot.complete(result.clone());
    if result.is_err() {
        (progress)(CodexHarnessEvent::status(
            "application_snapshot.failed",
            "reply",
            "failed",
            "本地工具准备失败",
        ));
    }
    result.map(|_| ())
}

fn configure_application_adapter(command: &mut Command, endpoint: &str) -> Result<(), String> {
    let url =
        Url::parse(endpoint).map_err(|_| "application_adapter_endpoint_invalid".to_owned())?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("application_adapter_endpoint_invalid".to_owned());
    }
    command
        .args([
            "-c",
            &format!(
                "mcp_servers.cilai_application.url={}",
                serde_json::to_string(endpoint)
                    .map_err(|_| "application_mcp_config_invalid".to_owned())?
            ),
        ])
        .args([
            "-c",
            "mcp_servers.cilai_application.bearer_token_env_var=\"CILAI_PERSONAL_TOOLS_TOKEN\"",
        ])
        .args([
            "-c",
            "mcp_servers.cilai_application.default_tools_approval_mode=\"approve\"",
        ])
        .args([
            "-c",
            "mcp_servers.cilai_application.omit_tools_from=[\"direct\"]",
        ]);
    Ok(())
}

fn configure_runtime_mcp(
    command: &mut Command,
    request: &CodexHarnessRequest,
    browser_endpoint: Option<&str>,
) -> Result<(), String> {
    if request.validation_probe {
        return Ok(());
    }
    if let Some(broker) = &request.user_mcp {
        if request.conversation_kind != crate::conversation_kernel::ConversationKind::Workbench
            || request.planning
        {
            return Err("mcp_purpose_not_authorized".into());
        }
        configure_http_mcp(command, "personal_user", &broker.url, true)?;
        command.env("CILAI_PERSONAL_USER_MCP_BEARER_TOKEN",&broker.bearer)
            .args(["-c","mcp_servers.personal_user.bearer_token_env_var=\"CILAI_PERSONAL_USER_MCP_BEARER_TOKEN\""])
            .args(["-c","mcp_servers.personal_user.default_tools_approval_mode=\"approve\""])
            .args(["-c","mcp_servers.personal_user.tool_timeout_sec=600"])
            .args(["-c","mcp_servers.personal_user.required=true"])
            .args(["-c","mcp_servers.personal_user.startup_readiness=\"connection\""])
            .args(["-c","mcp_servers.personal_user.supports_parallel_tool_calls=false"])
            .args(["-c","mcp_servers.personal_user.omit_tools_from=[\"deferred\"]"]);
    }
    if let Some((endpoint, _, _)) = request.application_adapter.as_ref() {
        command.env("CILAI_PERSONAL_TOOLS_TOKEN", &request.api_key);
        configure_application_adapter(command, endpoint)?;
    } else if request.application_mcp.as_ref().is_some_and(|scope| {
        !scope.read_tools.is_empty()
            || !scope.tool_names.is_empty()
            || !scope.write_tools.is_empty()
    }) {
        return Err("application_adapter_required".to_owned());
    }
    if let Some(endpoint) = browser_endpoint
        && !endpoint.trim().is_empty()
        && browser_mcp_permitted(request)
    {
        configure_http_mcp(command, "cilai_browser", endpoint.trim(), !request.planning)?;
        command
            .args([
                "-c",
                "mcp_servers.cilai_browser.bearer_token_env_var=\"CILAI_BROWSER_MCP_TOKEN\"",
            ])
            // 审批方式跟着员工在对话里选的那一档走，不给浏览器单独开一套
            .args([
                "-c",
                &format!(
                    "mcp_servers.cilai_browser.default_tools_approval_mode=\"{}\"",
                    mcp_tools_approval_mode(&request.approval_mode)
                ),
            ])
            .args([
                "-c",
                "mcp_servers.cilai_browser.omit_tools_from=[\"deferred\"]",
            ]);
        // 对话标识随请求头带给浏览器，宿主据此把标签归到发起它的那次对话上
        if let Some(conversation_id) = request
            .conversation_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            command.args([
                "-c",
                &format!(
                    "mcp_servers.cilai_browser.http_headers={{\"x-cilai-conversation-id\"={},\"x-cilai-browser-origins\"={}}}",
                    serde_json::to_string(conversation_id).map_err(|error| format!("browser_mcp_config_invalid:{error}"))?,
                    serde_json::to_string(&serde_json::to_string(&request.application_mcp.as_ref().map(|scope| scope.browser_origins.clone()).unwrap_or_default()).map_err(|error| format!("browser_mcp_config_invalid:{error}"))?).map_err(|error| format!("browser_mcp_config_invalid:{error}"))?
                ),
            ]);
        }
    }
    Ok(())
}

fn browser_mcp_permitted(request: &CodexHarnessRequest) -> bool {
    let scope = request.application_mcp.as_ref();
    let browser_origins = scope.is_some_and(|scope| !scope.browser_origins.is_empty());
    browser_origins
        || (request.conversation_kind == crate::conversation_kernel::ConversationKind::Workbench
            && scope.is_none_or(|scope| !scope.restrict_to_applications))
}

fn expected_runtime_mcp(
    request: &CodexHarnessRequest,
) -> std::collections::BTreeMap<String, String> {
    let mut expected = std::collections::BTreeMap::new();
    if request.validation_probe {
        return expected;
    }
    if let Some(broker) = &request.user_mcp {
        expected.insert("personal_user".into(), broker.url.clone());
    }
    if let Some((endpoint, _, _)) = &request.application_adapter {
        expected.insert("cilai_application".into(), endpoint.clone());
    }
    if browser_mcp_permitted(request)
        && let Ok(endpoint) = std::env::var("CILAI_BROWSER_MCP_URL")
        && !endpoint.trim().is_empty()
    {
        expected.insert("cilai_browser".into(), endpoint.trim().into());
    }
    expected
}

/// 发送链路里 harness 启动与协议握手的分段计时，与宿主记录的发送各阶段写在同一处诊断输出。
struct StageClock {
    started: std::time::Instant,
    last: std::time::Instant,
    command: &'static str,
    scope_id: String,
    parts: Vec<String>,
}

impl StageClock {
    fn new(command: &'static str, scope_id: &str) -> Self {
        let now = std::time::Instant::now();
        Self {
            started: now,
            last: now,
            command,
            scope_id: scope_id.to_owned(),
            parts: Vec::new(),
        }
    }

    fn mark(&mut self, name: &str) {
        let now = std::time::Instant::now();
        self.parts.push(format!(
            "{name}_ms={}",
            now.duration_since(self.last).as_millis()
        ));
        eprintln!(
            "ai_send_timing command={} scope_id={} stage={name} stage_ms={} total_ms={}",
            self.command,
            self.scope_id,
            now.duration_since(self.last).as_millis(),
            now.duration_since(self.started).as_millis()
        );
        self.last = now;
    }

    fn log(&self, command: &str) {
        eprintln!("ai_send_timing command={command} {}", self.parts.join(" "));
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn spawn_codex_harness(
    mut request: CodexHarnessRequest,
    progress: ProgressCallback,
    approval: ApprovalCallback,
    artifact: ArtifactCallback,
    completed: impl FnOnce(Result<CodexHarnessResult, String>) + Send + 'static,
    keep_alive: (impl Send + 'static, Option<Arc<HarnessProcessState>>),
    mut reuse: Option<HarnessReuse>,
) -> Result<SpawnedCodexHarness, String> {
    let mut clock = StageClock::new("harness_spawn_detail", &request.tool_snapshot_scope_id);
    request.conversation_kind.validate(&request)?;
    if request.approval_mode == "auto-review"
        && request
            .approval_review_model
            .as_deref()
            .is_none_or(|model| model.trim().is_empty())
    {
        return Err("ai_review_model_not_configured".to_owned());
    }
    let had_cached = reuse.as_ref().is_some_and(|reuse| reuse.cached.is_some());
    if !had_cached {
        contexthub_platform_adapter::security::ensure_owner_writable_runtime_tree(
            &request.harness_home,
        )
        .map_err(|error| format!("codex_harness_home_failed:{error}"))?;
    }
    clock.mark("home_acl");
    if request.planning {
        request.application_action = None;
        request.application_result = None;
        request.report_result = None;
        request.application_write = None;
        if let Some(scope) = request.application_mcp.as_mut() {
            scope.write_tools.clear();
        }
    }
    let allow_global_tools =
        !request.validation_probe && request.conversation_kind.allows_global_tools();
    let home_isolation = runtime_home_isolation(&request)?;
    let fingerprint = crate::persistent_threads::digest(&json!({
        "runtime": runtime_fingerprint(&request),
        "filePermissions": home_isolation.as_ref().map(contexthub_platform_adapter::macos_runtime_isolation::ChatgptIsolationSnapshot::digest),
    }));
    let mut cached = reuse.as_mut().and_then(|reuse| reuse.cached.take());
    if let Some(runtime) = cached.as_mut() {
        let alive = runtime
            .child
            .as_mut()
            .ok_or("harness_cached_child_missing")?
            .try_wait()
            .map_err(|error| format!("harness_cached_status_failed:{error}"))?
            .is_none();
        if !alive || runtime.fingerprint != fingerprint {
            eprintln!(
                "ai_send_timing command=harness_runtime scope_id={} stage=retired reason={}",
                request.tool_snapshot_scope_id,
                if alive {
                    "configuration_changed"
                } else {
                    "process_exited"
                }
            );
            runtime.retire()?;
            cached = None;
            contexthub_platform_adapter::security::ensure_owner_writable_runtime_tree(
                &request.harness_home,
            )
            .map_err(|error| format!("codex_harness_home_failed:{error}"))?;
            clock.mark("home_acl_refresh");
        }
    }
    let initialized = cached.is_some();
    if let Some(reuse) = reuse.as_ref() {
        let model = request
            .memory_preferences
            .enabled
            .then_some(request.memory_model.as_deref())
            .flatten();
        if initialized {
            reuse.proxy.configure_native_memory(model)?;
        } else {
            reuse.proxy.begin_runtime(model)?;
        }
    }
    if had_cached && !initialized {
        contexthub_platform_adapter::security::ensure_owner_writable_runtime_tree(
            &request.harness_home,
        )
        .map_err(|error| format!("codex_harness_home_failed:{error}"))?;
    }
    clock.mark("runtime_selection");
    let capability = request
        .model_catalog
        .iter()
        .find(|model| model.model_id == request.model)
        .and_then(|model| model.tool_capabilities.as_ref())
        .ok_or_else(|| "模型尚未声明工具能力，请同步模型目录".to_owned())?;
    capability.validate()?;
    let model_catalog_path = if request.model_provider == crate::model_connections::CHATGPT_PROVIDER
    {
        None
    } else {
        Some(write_model_catalog(&request)?)
    };
    let mut event_log = File::create(&request.event_log_path)
        .map_err(|error| format!("codex_harness_log_failed:{error}"))?;
    if let Some(snapshot) = &home_isolation {
        writeln!(
            event_log,
            "{}",
            json!({"method":"cilai/runtimeIsolation", "params":{"snapshot":snapshot}})
        )
        .map_err(|_| "model_runtime_permissions_audit_failed")?;
    }
    let stderr = File::create(&request.stderr_path)
        .map_err(|error| format!("codex_harness_log_failed:{error}"))?;
    let mut command = if request.execution.is_some() {
        let mut command = Command::new(crate::model_connections::launch_gate_executable()?);
        command.arg("personal-runtime-launch-gate");
        if home_isolation.is_some() {
            command.arg("--protected-home-v1");
        }
        command.arg("--").arg(&request.executable);
        command
    } else {
        Command::new(&request.executable)
    };
    crate::model_connections::isolate_runtime_environment(&mut command, &request.harness_home)?;
    if let Some(isolation) = &home_isolation {
        isolation
            .apply_environment(&mut command)
            .map_err(|_| "model_runtime_home_isolation_invalid")?;
    }
    if let Some(execution) = &request.execution {
        let route = execution.foreground.network()?;
        execution.foreground.apply_native_network(&mut command)?;
        route.apply_native_certificate(&mut command, &request.harness_home)?;
    }
    crate::codex_upgrade::configure_aliases(&mut command, &request.harness_home)?;
    command
        .current_dir(&request.workspace_root)
        .env("CODEX_HOME", &request.harness_home)
        .env("CODEX_SQLITE_HOME", &request.harness_home)
        .env(crate::windows_sandbox::REMOTE_CONTROL_DISABLED_ENV, "1")
        .env(&request.api_key_env, &request.api_key)
        .args(["--listen", "stdio://"])
        .args([
            "--session-source",
            if request.conversation_kind == crate::conversation_kernel::ConversationKind::Workbench
            {
                "vscode"
            } else {
                "app-server"
            },
        ])
        .args([
            "-c",
            &format!(
                "web_search=\"{}\"",
                if allow_global_tools {
                    capability.web_search_mode.as_str()
                } else {
                    "disabled"
                }
            ),
        ])
        .args(["-c", "check_for_update_on_startup=false"])
        .args(["-c", "features.plugins=false"])
        .args(["-c", "skills.bundled.enabled=false"])
        .args(["-c", "features.skill_mcp_dependency_install=false"])
        .args(["-c", "project_doc_max_bytes=0"])
        .args(["-c", &compact_prompt_config()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr));
    configure_validation_probe(&mut command, &request)?;
    if !allow_global_tools && !request.validation_probe {
        configure_scoped_page_tools(&mut command);
    }
    configure_model_connection(&mut command, &request, model_catalog_path.as_deref())?;
    configure_memory(&mut command, &request)?;
    crate::report_workspace::configure_command(&mut command, &request)?;
    let os = current_operating_system();
    crate::windows_sandbox::configure_harness_command(&mut command, os);
    crate::windows_sandbox::configure_harness_skill_creator(
        &mut command,
        os,
        &request.harness_home,
    );
    clock.mark("command_config");
    let browser_endpoint = std::env::var("CILAI_BROWSER_MCP_URL").ok();
    configure_runtime_mcp(&mut command, &request, browser_endpoint.as_deref())?;
    clock.mark("application_mcp");
    contexthub_platform_adapter::process::prepare_command_for_supervision(&mut command);
    let native_thread = cached
        .as_mut()
        .and_then(|cached| cached.native_thread.take());
    let inherited_journal = native_thread.as_ref().map(|native| native.journal.clone());
    let (mut child, mut stdin, mut reader) = if let Some(mut cached) = cached {
        (
            cached.child.take().ok_or("harness_cached_child_missing")?,
            cached.input.take().ok_or("harness_cached_input_missing")?,
            cached
                .reader
                .take()
                .ok_or("harness_cached_reader_missing")?,
        )
    } else {
        if let Some(execution) = &request.execution {
            execution.mark_launch_attempted();
        }
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                if let Some(execution) = &request.execution {
                    execution.mark_spawn_failed();
                }
                return Err(format!("codex_harness_spawn_failed:{error}"));
            }
        };
        let mut child = HarnessChild::new(child, (Box::new(keep_alive.0), keep_alive.1));
        contexthub_platform_adapter::process::supervise_spawned_child(&mut child)
            .map_err(|error| format!("codex_harness_supervision_failed:{error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or("codex_harness_stdin_unavailable")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("codex_harness_stdout_unavailable")?;
        (child, stdin, BufReader::new(stdout))
    };
    clock.mark("process_spawn");
    clock.log("harness_spawn_detail");
    let pid = child.id();
    if let Some(execution) = &request.execution
        && let Err(error) = execution.register_process(pid)
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    if let Some(execution) = &request.execution
        && let Err(error) = child.attach_execution(execution.clone(), Arc::clone(&progress))
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    if !initialized
        && request.execution.is_some()
        && let Err(error) = stdin
            .write_all(b"CILAI_PERSONAL_RUNTIME_PERMIT_V1\n")
            .and_then(|()| stdin.flush())
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("model_execution_launch_failed:{error}"));
    }
    eprintln!(
        "ai_send_timing command=harness_runtime scope_id={} stage={} pid={pid}",
        request.tool_snapshot_scope_id,
        if initialized { "reused" } else { "cold" }
    );
    if let Some(plan) = request.persistent_thread.as_mut() {
        let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid);
        if let Ok(Some(fingerprint)) = fingerprint {
            plan.writer_process = json!({"pid":pid,"startFingerprint":fingerprint});
            plan.journal.writer_process = plan.writer_process.clone();
        } else {
            let _ = child.kill();
            if child.wait().is_ok()
                && let Some(previous) = inherited_journal.as_ref()
            {
                previous.release()?;
            }
            return Err("thread_owner_identity_unavailable".to_owned());
        }
    }
    let mut control = CodexHarnessControl::new(stdin);
    *control
        .native_thread
        .lock()
        .map_err(|_| "harness_native_state_unavailable")? = native_thread;
    control.journal = request
        .persistent_thread
        .as_ref()
        .map(|plan| plan.journal.clone());
    let protocol_control = control.clone();
    std::thread::Builder::new()
        .name(format!("cilai-harness-{pid}"))
        .spawn(move || {
            let mut log = BufWriter::new(event_log);
            // 工具准备与 initialize/thread resume/沙箱探测并行；在 turn/start 前汇合。
            let result = std::thread::scope(|workers| {
                let mut adapter = Some(workers.spawn(|| {
                    let mut clock =
                        StageClock::new("harness_adapter", &request.tool_snapshot_scope_id);
                    let result = prepare_persistent_application_adapter(&request, &progress);
                    clock.mark("prepare");
                    result
                }));
                let result = run_protocol_with_io(
                    &request,
                    |message| protocol_control.send(message),
                    &mut reader,
                    &mut log,
                    &progress,
                    Some(&approval),
                    Some(&artifact),
                    Some(&protocol_control),
                    current_operating_system(),
                    initialized,
                    || {
                        adapter
                            .take()
                            .ok_or("application_adapter_worker_missing")?
                            .join()
                            .map_err(|_| "application_adapter_worker_failed".to_owned())?
                    },
                );
                // 协议提前失败时也收拢准备线程，避免遗留带本轮凭据的后台任务。
                let prepared = adapter.map_or(Ok(()), |worker| {
                    worker
                        .join()
                        .map_err(|_| "application_adapter_worker_failed".to_owned())?
                });
                result.and_then(|value| prepared.map(|()| value))
            });
            let release_thread = || {
                if let Some(plan) = request.persistent_thread.as_ref() {
                    // 准备可能在写入权交接前失败；进程退出后仍须释放它原先保留的写入权。
                    let state = plan.journal.state()?;
                    if let Some(previous) = inherited_journal.as_ref()
                        && state["binding"]["activeRunId"] == previous.run_id
                        && state["binding"]["ownerInstanceId"] == previous.owner_instance_id
                    {
                        previous.release()?;
                    } else {
                        plan.journal.release()?;
                    }
                    (progress)(CodexHarnessEvent::status(
                        "harness.thread_released",
                        "lifecycle",
                        "completed",
                        "本轮执行已结束",
                    ));
                }
                Ok(())
            };
            if let Some(reuse) = reuse.filter(|_| request.memory_preferences.enabled && !request.validation_probe) {
                let reusable_thread = result.as_ref().ok().map(|result| result.thread_id.clone());
                completed(result);
                let mut parked = None;
                let recycled = reusable_thread.and_then(|thread_id| {
                    let candidate = protocol_control.native_thread.lock().ok().and_then(|mut state| state.take());
                    let ready = native_thread_reuse::settle(
                        &thread_id,
                        candidate.is_some(),
                        &mut |message| protocol_control.send(message),
                        &mut reader,
                        &mut log,
                        &progress,
                    )
                    .and_then(|retain| {
                        if retain && let Some(native) = candidate {
                            native.journal.park()?;
                            parked = Some(native);
                        }
                        reuse.proxy.suspend()?;
                        Ok(())
                    });
                    if let Err(error) = ready {
                        if error == "harness_runtime_retire_required" { return None; }
                        (progress)(CodexHarnessEvent::status(
                            "harness.runtime_retire_failed",
                            "lifecycle",
                            "failed",
                            format!("运行时收尾未完成：{error}"),
                        ));
                        return None;
                    }
                    // 旧运行控制器不再拥有输入句柄，迟到的取消或审批不能进入下一轮。
                    protocol_control
                        .stdin
                        .lock()
                        .ok()
                        .and_then(|mut input| input.take())
                });
                if let Some(input) = recycled {
                    let retained = parked.is_some();
                    eprintln!(
                        "ai_send_timing command=harness_runtime scope_id={} stage=idle pid={pid} native_thread_retained={retained}",
                        request.tool_snapshot_scope_id
                    );
                    reuse.lease.recycle(CachedHarness {
                        native_thread: parked,
                        child: Some(child),
                        input: Some(input),
                        reader: Some(reader),
                        fingerprint,
                        proxy: reuse.proxy,
                    });
                    if retained {
                        (progress)(CodexHarnessEvent::status("harness.thread_parked","lifecycle","completed","本轮已结束，会话保持空闲"));
                    }
                    return;
                }
                reuse.proxy.retire();
                let _ = child.kill();
                let reaped = child.wait().is_ok();
                if reaped && release_thread().is_err() {
                    (progress)(CodexHarnessEvent::status(
                        "harness.thread_release_failed",
                        "lifecycle",
                        "failed",
                        "对话状态保存失败，暂时无法继续发送",
                    ));
                }
                return;
            }
            if request.conversation_kind != crate::conversation_kernel::ConversationKind::Workbench
            {
                let input_closed = protocol_control.close_input();
                let reaped = if input_closed.is_err() {
                    child.kill().and_then(|()| child.wait())
                } else {
                    child.wait()
                }.map_err(|_| "thread_owner_identity_unavailable".to_owned());
                let finalized = reaped.and_then(|_| release_thread())
                    .and_then(|()| child.release_executions_after_exit())
                    .and(input_closed);
                completed(match finalized {
                    Ok(()) => result,
                    Err(error) => Err(error),
                });
                return;
            }
            let released = complete_protocol_before_retirement(
                result,
                request
                    .memory_preferences
                    .enabled
                    .then(|| {
                        protocol_control
                            .terminal_thread
                            .lock()
                            .ok()
                            .and_then(|thread| thread.clone())
                    })
                    .flatten(),
                completed,
                &release_thread,
                &mut |message| protocol_control.send(message),
                &mut reader,
                &mut log,
                &progress,
            );
            let _ = child.kill();
            let reaped = child.wait().is_ok();
            if reaped && !released && release_thread().is_err() {
                (progress)(CodexHarnessEvent::status(
                    "harness.thread_release_failed",
                    "lifecycle",
                    "failed",
                    "对话状态保存失败，暂时无法继续发送",
                ));
            }
        })
        .map_err(|error| format!("codex_harness_monitor_failed:{error}"))?;
    Ok(SpawnedCodexHarness { pid, control })
}

#[allow(clippy::too_many_arguments)]
fn complete_protocol_before_retirement(
    result: Result<CodexHarnessResult, String>,
    thread_id: Option<String>,
    completed: impl FnOnce(Result<CodexHarnessResult, String>),
    release_thread: &impl Fn() -> Result<(), String>,
    send_message: &mut impl FnMut(&Value) -> Result<(), String>,
    reader: &mut impl BufRead,
    log: &mut BufWriter<File>,
    progress: &ProgressCallback,
) -> bool {
    completed(result);
    let Some(thread_id) = thread_id else {
        return false;
    };
    // 不把取消订阅当作立即卸载。调用者确认进程退出后再释放写入权。
    let _ = (
        thread_id,
        release_thread,
        send_message,
        reader,
        log,
        progress,
    );
    false
}

fn detach_memory_foreground(
    thread_id: &str,
    send_message: &mut impl FnMut(&Value) -> Result<(), String>,
    reader: &mut impl BufRead,
    log: &mut BufWriter<File>,
    progress: &ProgressCallback,
) -> Result<(), String> {
    send_message(&json!({"method":"thread/unsubscribe","id":13,"params":{"threadId":thread_id}}))?;
    read_response(reader, log, 13, progress, "lifecycle")?;
    Ok(())
}

/// JSON 字符串转义与 TOML 基本字符串兼容（不含 `\/`），可直接作为 `-c` 的取值。
fn compact_prompt_config() -> String {
    format!(
        "compact_prompt={}",
        Value::String(COMPACT_PROMPT.to_owned())
    )
}

fn configure_memory(command: &mut Command, request: &CodexHarnessRequest) -> Result<(), String> {
    let enabled = request.memory_preferences.enabled;
    let model = if enabled {
        Some(
            request
                .memory_model
                .as_deref()
                .filter(|model| !model.trim().is_empty())
                .ok_or("ai_memory_model_not_configured")?,
        )
    } else {
        None
    };
    for value in [
        "memories.version=\"v1\"".to_owned(),
        "memories.dual_write=false".to_owned(),
        format!("features.memories={enabled}"),
        format!("memories.use_memories={enabled}"),
        format!("memories.generate_memories={enabled}"),
        format!(
            "memories.disable_on_external_context={}",
            !request.memory_preferences.from_tool_chats
        ),
    ] {
        command.args(["-c", &value]);
    }
    if let Some(model) = model {
        let model = serde_json::to_string(model)
            .map_err(|error| format!("codex_harness_memory_config_invalid:{error}"))?;
        command.args([
            "-c",
            &format!("memories.extract_model={model}"),
            "-c",
            &format!("memories.consolidation_model={model}"),
        ]);
    }
    Ok(())
}

fn record_fork_history(
    request: &CodexHarnessRequest,
    plan: &crate::persistent_threads::PersistentThread,
    thread: &str,
    action: &str,
) -> Result<(), String> {
    if plan.workbench_history.is_empty() {
        return Ok(());
    }
    let home = request
        .harness_home
        .canonicalize()
        .map_err(|_| "ai_context_home_unavailable")?;
    let response=plan.journal.server.request_result("aiHistory/execute",&json!({"action":action,"payload":{"scope":crate::persistent_threads::digest(&json!(home)),"threadId":thread,"turns":plan.workbench_history}}))?;
    if action == "contextPrepare" && response["value"]["replayed"] == true {
        return Err("ai_context_import_delivery_uncertain".into());
    }
    Ok(())
}

fn configure_http_mcp(
    command: &mut Command,
    name: &str,
    origin: &str,
    enabled: bool,
) -> Result<(), String> {
    let endpoint = Url::parse(origin).map_err(|error| format!("{name}_origin_invalid:{error}"))?;
    let url = serde_json::to_string(endpoint.as_str())
        .map_err(|error| format!("{name}_config_invalid:{error}"))?;
    command
        .args(["-c", &format!("mcp_servers.{name}.url={url}")])
        .args(["-c", &format!("mcp_servers.{name}.enabled={enabled}")])
        .args([
            "-c",
            &format!("mcp_servers.{name}.startup_readiness=\"catalog\""),
        ]);
    Ok(())
}

fn product_model_messages() -> Result<Value, String> {
    let upstream: Value = serde_json::from_str(CODEX_MODEL_CATALOG_SOURCE)
        .map_err(|error| format!("codex_harness_model_catalog_invalid:{error}"))?;
    let mut model_messages = upstream
        .get("models")
        .and_then(Value::as_array)
        .and_then(|models| {
            models
                .iter()
                .find(|model| model.get("slug").and_then(Value::as_str) == Some(CODEX_PROMPT_MODEL))
        })
        .and_then(|model| model.get("model_messages"))
        .cloned()
        .ok_or_else(|| "codex_harness_prompt_model_missing".to_owned())?;
    let template = model_messages
        .get("instructions_template")
        .and_then(Value::as_str)
        .ok_or_else(|| "codex_harness_prompt_template_missing".to_owned())?;
    let behavior = template
        .strip_prefix("You are Codex, ")
        .and_then(|text| text.split_once(". ").map(|(_, behavior)| behavior))
        .ok_or_else(|| "codex_harness_prompt_identity_changed".to_owned())?;
    model_messages["instructions_template"] = Value::String(format!(
        "{PRODUCT_IDENTITY_INSTRUCTIONS}\n\n{}",
        behavior.replace("As Codex,", "As 词来,")
    ));
    Ok(model_messages)
}

fn write_model_catalog(request: &CodexHarnessRequest) -> Result<PathBuf, String> {
    request.conversation_kind.validate(request)?;
    let model_messages = product_model_messages()?;
    let models = request
        .model_catalog
        .iter()
        .enumerate()
        .map(|(index, model)| {
            let supported_reasoning_levels = model
                .supported_reasoning_efforts
                .iter()
                .map(|effort| {
                    json!({
                        "effort": effort,
                        "description": match effort.as_str() {
                            "none" => "No additional reasoning",
                            "minimal" => "Minimal reasoning",
                            "low" => "Lighter reasoning",
                            "medium" => "Balanced reasoning",
                            "high" => "Greater reasoning depth",
                            "xhigh" => "Extra high reasoning depth",
                            "max" => "Maximum reasoning depth",
                            _ => "Model-defined reasoning depth",
                        }
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "slug": model.model_id,
                "display_name": model.display_name,
                "description": format!("词来 通过 {} 提供", request.provider_label),
                "default_reasoning_level": model.default_reasoning_effort,
                "supported_reasoning_levels": supported_reasoning_levels,
                "shell_type": if request.report_result.is_some() || !request.conversation_kind.allows_global_tools() || request.validation_probe { "disabled" } else { "default" },
                "visibility": "list",
                "supported_in_api": true,
                "priority": index,
                "availability_nux": null,
                "upgrade": null,
                "model_messages": model_messages.clone(),
                "include_skills_usage_instructions": !request.skills.is_empty(),
                "include_plugin_usage_instructions": true,
                "include_apps_usage_instructions": false,
                "supports_reasoning_summary_parameter": false,
                "default_reasoning_summary": "none",
                "support_verbosity": false,
                "default_verbosity": null,
                "apply_patch_tool_type": null,
                "web_search_tool_type": "text",
                "truncation_policy": { "mode": "bytes", "limit": if request.report_result.is_some() { model.context_window.map(|window| (window / 2).min(128_000)) } else if request.conversation_id.is_none() && request.application_action.is_some() { model.context_window.map(|window| (window / 2).min(512_000)) } else { Some(10_000) } },
                "supports_parallel_tool_calls": false,
                "supports_image_detail_original": false,
                "context_window": model.context_window,
                "max_context_window": model.context_window,
                "auto_compact_token_limit": null,
                "comp_hash": format!("contexthub-codex-{CODEX_HARNESS_VERSION}-{}", json!(&model.tool_capabilities)),
                "effective_context_window_percent": 95,
                "experimental_supported_tools": [],
                "input_modalities": model.input_modalities,
                "supports_search_tool": model.tool_capabilities.as_ref().is_some_and(|capability| capability.client_tool_search),
                "use_responses_lite": false,
                "auto_review_model_override": request.approval_review_model,
                "model_specialty": null,
                "tool_mode": null,
                "multi_agent_version": null
            })
        })
        .collect::<Vec<_>>();
    let path = request.event_log_path.with_extension("model-catalog.json");
    let bytes = serde_json::to_vec(&json!({ "models": models }))
        .map_err(|error| format!("codex_harness_model_catalog_invalid:{error}"))?;
    fs::write(&path, bytes)
        .map_err(|error| format!("codex_harness_model_catalog_write_failed:{error}"))?;
    Ok(path)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_protocol_for_os(
    request: &CodexHarnessRequest,
    send_message: impl FnMut(&Value) -> Result<(), String>,
    stdout: impl std::io::Read,
    event_log: File,
    progress: &ProgressCallback,
    approval: Option<&ApprovalCallback>,
    artifact: Option<&ArtifactCallback>,
    control: Option<&CodexHarnessControl>,
    os: OperatingSystem,
) -> Result<CodexHarnessResult, String> {
    let mut reader = BufReader::new(stdout);
    let mut log = BufWriter::new(event_log);
    run_protocol_with_io(
        request,
        send_message,
        &mut reader,
        &mut log,
        progress,
        approval,
        artifact,
        control,
        os,
        false,
        || Ok(()),
    )
}

fn insert_additional_context(context: &mut serde_json::Map<String, Value>, key: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    if value.len() <= ADDITIONAL_CONTEXT_PART_BYTES {
        context.insert(key.to_owned(), json!({"value":value,"kind":"application"}));
        return;
    }
    let mut parts = Vec::new();
    let mut start = 0;
    while start < value.len() {
        let mut end = (start + ADDITIONAL_CONTEXT_PART_BYTES).min(value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        parts.push(&value[start..end]);
        start = end;
    }
    context.insert(
        key.to_owned(),
        json!({"value":format!(
            "以下附加上下文按键名 {key}_part_000001 起连续编号分段。请按编号连续阅读，作为同一份完整资料；分段不改变原文及权限。共{}段，正文UTF-8共{}字节，SHA-256为{:x}。",
            parts.len(), value.len(), Sha256::digest(value.as_bytes())
        ),"kind":"application"}),
    );
    for (index, part) in parts.into_iter().enumerate() {
        context.insert(
            format!("{key}_part_{:06}", index + 1),
            json!({"value":part,"kind":"application"}),
        );
    }
}

fn foreground_developer_instructions(
    kind: crate::conversation_kernel::ConversationKind,
    instructions: String,
) -> String {
    if kind != crate::conversation_kernel::ConversationKind::Workbench {
        return instructions;
    }
    format!(
        "以下 <cilai_foreground_context> 中的词来工作台规则、权限描述、应用上下文、产物登记和工作归类要求，仅适用于用户发起的前台会话及其业务操作，不适用于 Codex 官方运行时发起的内部后台记忆整理任务。内部记忆任务的指令、工具和权限以官方运行时提供的内容为准。此作用域区分不扩大前台会话或业务操作的权限。\n\n<cilai_foreground_context>\n{instructions}\n</cilai_foreground_context>"
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_protocol_with_io(
    request: &CodexHarnessRequest,
    mut send_message: impl FnMut(&Value) -> Result<(), String>,
    mut reader: &mut impl BufRead,
    log: &mut BufWriter<File>,
    progress: &ProgressCallback,
    approval: Option<&ApprovalCallback>,
    artifact: Option<&ArtifactCallback>,
    control: Option<&CodexHarnessControl>,
    os: OperatingSystem,
    initialized: bool,
    adapter_ready: impl FnOnce() -> Result<(), String>,
) -> Result<CodexHarnessResult, String> {
    let mut clock = StageClock::new("harness_protocol", &request.tool_snapshot_scope_id);
    // 用户消息只放员工原文；材料说明与本轮输出契约走附加上下文，模型可见但不进入线程里的用户消息
    let mut turn_input = vec![json!({ "type": "text", "text": request.user_input })];
    let response_contract = if let Some(classification) = request.classification.as_ref() {
        if !classification.output_schema.is_object() {
            return Err("classification_schema_invalid".into());
        }
        Some(format!(
            "<workbench_response_contract>\n这是工作台的本轮输出契约，用户请求见本轮用户消息。先完成实际工具操作；如有真实文件产物，调用 cilai_register_artifact 登记，无文件产物的只读回答不要求创建或登记文件。最后必须调用 cilai_submit_workbench_result，参数为符合下方 Schema 的完整对象，包含给用户的完整 Markdown 回复和本轮归类。必须收到成功回执，普通最终正文、围栏 JSON 或声称已提交均不能代替结果提交。本轮工作上下文、任务候选和归类规则以以下当前内容为准：\n{}\n\n本轮正式结果参数 Schema：\n{}\n</workbench_response_contract>",
            classification.developer_instructions, classification.output_schema
        ))
    } else {
        None
    };
    let materials = request.turn_context.trim();
    if materials
        .len()
        .checked_add(response_contract.as_ref().map_or(0, String::len))
        .is_none_or(|bytes| bytes > ADDITIONAL_CONTEXT_TOTAL_BYTES)
    {
        return Err("ai_additional_context_too_large".into());
    }
    let mut additional_context = serde_json::Map::new();
    insert_additional_context(&mut additional_context, "cilai_turn_materials", materials);
    if let Some(contract) = response_contract {
        insert_additional_context(
            &mut additional_context,
            "cilai_response_contract",
            &contract,
        );
    }
    turn_input.extend(
        request
            .skills
            .iter()
            .filter(|skill| skill.explicit)
            .map(|skill| {
                json!({
                    "type": "skill",
                    "name": skill.name,
                    "path": skill.path.to_string_lossy()
                })
            }),
    );
    turn_input.extend(request.attachments.iter().map(|attachment| {
        let path = attachment.path.to_string_lossy().into_owned();
        match attachment.media_type.as_str() {
            "audio" => json!({ "type": "localAudio", "path": path }),
            _ => json!({ "type": "localImage", "path": path }),
        }
    }));
    if initialized {
        clock.mark("initialize_reused");
    } else {
        send_message(&json!({
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": {
                    "name": "contexthub_desktop",
                    "title": "词来工作台",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": { "experimentalApi": true }
            }
        }))?;
        read_response(&mut reader, log, 1, progress, "reply")?;
        clock.mark("initialize");
        send_message(&json!({ "method": "initialized", "params": {} }))?;
    }
    if request.report_result.is_some() {
        send_message(
            &json!({"method":"config/read","id":15,"params":{"includeLayers":false,"cwd":request.workspace_root}}),
        )?;
        let effective_config = read_response(&mut reader, log, 15, progress, "reply")?;
        crate::report_workspace::validate_config(&effective_config, request)?;
    }
    if os != OperatingSystem::Windows && !crate::report_workspace::is_report(request) {
        send_message(&json!({
            "method": "skills/config/write",
            "id": 11,
            "params": {
                "path": request.harness_home.join("skills/.system/skill-creator/SKILL.md"),
                "enabled": false
            }
        }))?;
        let creator_config = read_response(&mut reader, log, 11, progress, "reply")
            .map_err(|error| format!("codex_harness_skill_creator_config_failed:{error}"))?;
        if creator_config.pointer("/result/effectiveEnabled") != Some(&Value::Bool(false)) {
            return Err("codex_harness_skill_creator_config_invalid".to_owned());
        }
    }

    if initialized || !request.skills.is_empty() {
        let extra_roots = authorized_skill_roots(&request.skills)?;
        send_message(&json!({
            "method": "skills/extraRoots/set",
            "id": 9,
            "params": { "extraRoots": extra_roots }
        }))?;
        read_response(&mut reader, log, 9, progress, "reply")
            .map_err(|error| format!("codex_harness_skill_roots_failed:{error}"))?;
    }
    if request.execution.is_some() {
        send_message(
            &json!({"method":"config/read","id":16,"params":{"includeLayers":true,"cwd":request.workspace_root}}),
        )?;
        let config = read_response(&mut reader, log, 16, progress, "reply")
            .map_err(|_| "model_runtime_config_unconfirmed")?;
        crate::model_connections::isolation::validate_configuration(&config)?;
        crate::model_connections::isolation::validate_tool_environment(
            &config,
            &request.harness_home,
        )?;
        if !request.conversation_kind.allows_global_tools() {
            validate_scoped_page_tools(&config)?;
        }
        crate::model_connections::isolation::validate_mcp(&config, &expected_runtime_mcp(request))?;
        send_message(
            &json!({"method":"skills/list","id":17,"params":{"cwds":[request.workspace_root],"forceReload":true}}),
        )?;
        let skills = read_response(&mut reader, log, 17, progress, "reply")
            .map_err(|_| "model_runtime_skills_unconfirmed")?;
        crate::model_connections::isolation::validate_skills(&skills, &request.skills)?;
    }
    if os == OperatingSystem::Windows {
        send_message(&json!({
            "method": "windowsSandbox/readiness",
            "id": 8
        }))?;
        let readiness = read_response(&mut reader, log, 8, progress, "reply")
            .map_err(|error| format!("windows_sandbox_setup_failed:{error}"))?;
        crate::windows_sandbox::require_ready_for_run(
            crate::windows_sandbox::parse_readiness_response(&readiness)
                .map_err(|error| format!("windows_sandbox_setup_failed:{error}"))?,
        )?;
    }
    clock.mark("skills_readiness");

    let model_messages = product_model_messages()?;
    let base_instructions = model_messages
        .get("instructions_template")
        .and_then(Value::as_str)
        .ok_or_else(|| "codex_harness_prompt_template_missing".to_owned())?;
    let artifact_tool = register_artifact_tool();
    let application_action_tool = application_action_tool();
    let mut dynamic_tools = Vec::new();
    if !request.validation_probe
        && request.application_adapter.is_none()
        && request.personal_tools.is_some()
    {
        dynamic_tools.push(personal_tools_definition());
    }
    if !request.validation_probe
        && request.conversation_kind == crate::conversation_kernel::ConversationKind::Workbench
    {
        dynamic_tools.push(artifact_tool.clone());
    }
    if !request.validation_probe
        && request.application_action.is_some()
        && request.application_result.is_none()
    {
        dynamic_tools.push(application_action_tool.clone());
    }
    if !request.validation_probe
        && request.application_adapter.is_none()
        && request.application_write.is_some()
        && let Some(scope) = request.application_mcp.as_ref()
        && !scope.write_tools.is_empty()
    {
        dynamic_tools.push(application_write_tool_namespace(&scope.write_tools));
    }
    if let Some(task) = request.application_result.as_ref() {
        dynamic_tools.push(crate::application_result::tool(task));
    }
    if !request.validation_probe
        && let Some(classification) = request.classification.as_ref()
    {
        dynamic_tools.push(crate::workbench_result::tool(classification));
    }
    let developer_instructions = request.classification.as_ref().map_or_else(
        || request.developer_instructions.clone(),
        |classification| {
            format!(
                "{}\n\n{}\n\n正式结果工具参数 Schema（执行期间按授权调用工具，完成后必须调用 cilai_submit_workbench_result）：\n```json\n{}\n```",
                request.developer_instructions,
                classification.developer_instructions,
                classification.output_schema
            )
        },
    );
    let developer_instructions =
        foreground_developer_instructions(request.conversation_kind, developer_instructions);
    let workspace_root = request.workspace_root.to_string_lossy().into_owned();
    let workspace_roots = request
        .workspace_roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let (approval_policy, approvals_reviewer, sandbox_mode) = turn_approval_settings(
        request.planning || request.validation_probe,
        &request.approval_mode,
        &request.sandbox_mode,
    )?;
    let (method, params) = if let Some(plan) = request
        .persistent_thread
        .as_ref()
        .filter(|plan| plan.fork_before_turn_id.is_some() && !plan.workbench_history_fork)
    {
        (
            "thread/fork",
            json!({
                "threadId": plan.native_thread_id, "beforeTurnId": plan.fork_before_turn_id,
                "model": request.model, "modelProvider": request.model_provider,
                "cwd": workspace_root, "runtimeWorkspaceRoots": workspace_roots,
                "approvalPolicy": approval_policy, "approvalsReviewer": approvals_reviewer,
                "sandbox": sandbox_mode, "baseInstructions": base_instructions,
                "developerInstructions": developer_instructions,
            }),
        )
    } else if let Some(thread_id) = request.resume_thread_id.as_deref().filter(|_| {
        request
            .persistent_thread
            .as_ref()
            .is_none_or(|plan| !plan.workbench_history_fork)
    }) {
        (
            "thread/resume",
            json!({
                "threadId": thread_id,
                "model": request.model,
                "modelProvider": request.model_provider,
                "cwd": workspace_root,
                "runtimeWorkspaceRoots": workspace_roots,
                "approvalPolicy": approval_policy,
                "approvalsReviewer": approvals_reviewer,
                "sandbox": sandbox_mode,
                "baseInstructions": base_instructions,
                "developerInstructions": developer_instructions
            }),
        )
    } else {
        (
            "thread/start",
            json!({
                "model": request.model,
                "modelProvider": request.model_provider,
                "cwd": workspace_root,
                "runtimeWorkspaceRoots": workspace_roots,
                "approvalPolicy": approval_policy,
                "approvalsReviewer": approvals_reviewer,
                "sandbox": sandbox_mode,
                "baseInstructions": base_instructions,
                "developerInstructions": developer_instructions,
                "allowProviderModelFallback": false,
                "serviceName": "contexthub_desktop",
                "experimentalRawEvents": request.application_mcp.is_some(),
                "dynamicTools": dynamic_tools.clone()
            }),
        )
    };
    let native_fingerprint =
        native_thread_reuse::fingerprint(&params, &dynamic_tools, &request.skills);
    let previous = control.and_then(|control| {
        control
            .native_thread
            .lock()
            .ok()
            .and_then(|mut native| native.take())
    });
    let mut retained_response = None;
    if let Some(previous) = previous {
        if method == "thread/resume"
            && previous.fingerprint == native_fingerprint
            && previous
                .response
                .pointer("/result/thread/id")
                .and_then(Value::as_str)
                == request.resume_thread_id.as_deref()
            && request
                .persistent_thread
                .as_ref()
                .is_some_and(|plan| plan.history.is_empty())
        {
            retained_response = Some(previous.response);
        } else {
            let id = previous
                .response
                .pointer("/result/thread/id")
                .and_then(Value::as_str)
                .ok_or("harness_native_thread_invalid")?;
            detach_memory_foreground(id, &mut send_message, &mut reader, log, progress)?;
            previous.journal.release()?;
            clock.mark("thread_configuration_changed");
        }
    }
    let inherited_title = if let Some(plan) = request
        .persistent_thread
        .as_ref()
        .filter(|plan| plan.workbench_history_fork && !plan.cross_runtime_fork)
    {
        send_message(
            &json!({"method":"thread/read","id":9,"params":{"threadId":plan.native_thread_id,"includeTurns":false}}),
        )?;
        let source = read_response(&mut reader, log, 9, progress, "lifecycle")?;
        source
            .pointer("/result/thread/name")
            .and_then(Value::as_str)
            .map(str::to_owned)
    } else {
        None
    };
    if let Some(plan) = request.persistent_thread.as_ref() {
        let kind = match method {
            "thread/start" if plan.workbench_history_fork => "fork",
            "thread/start" => "create",
            "thread/fork" => "fork",
            _ => "resume",
        };
        plan.journal.prepare("thread", kind, json!({"sourceThreadId":plan.native_thread_id,"beforeTurnId":plan.fork_before_turn_id,"migrationItemCount":plan.history.len(),"writerProcess":plan.writer_process}))?;
        plan.journal.record("thread", "dispatch", Value::Null)?;
    }
    clock.mark("turn_prepare");
    let thread_response = if let Some(response) = retained_response {
        clock.mark("thread_reused");
        response
    } else {
        send_message(&json!({ "method": method, "id": 2, "params": params }))?;
        let response = read_response(&mut reader, log, 2, progress, "reply").map_err(|error| {
            reject_known_rpc_error(request.persistent_thread.as_ref(), "thread", error)
        })?;
        clock.mark(method.trim_start_matches("thread/"));
        response
    };
    crate::report_workspace::validate_thread(
        &thread_response,
        request
            .application_mcp
            .as_ref()
            .is_some_and(|scope| scope.report_review),
    )?;
    let thread_id = thread_response
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .ok_or_else(|| "codex_harness_thread_invalid".to_owned())?
        .to_owned();
    if let Some(title) = inherited_title {
        send_message(
            &json!({"method":"thread/name/set","id":10,"params":{"threadId":thread_id,"name":title}}),
        )?;
        read_response(&mut reader, log, 10, progress, "lifecycle")?;
    }
    if let Some(control) = control
        && let Some(plan) = request.persistent_thread.as_ref()
    {
        *control
            .native_thread
            .lock()
            .map_err(|_| "harness_native_state_unavailable")? = Some(NativeThread {
            fingerprint: native_fingerprint,
            response: thread_response.clone(),
            journal: plan.journal.clone(),
        });
    }
    if let Some(plan) = request.persistent_thread.as_ref() {
        if method == "thread/start" {
            // 新线程在沙箱准备失败前也必须可以恢复；该公开接口持久化其原生记录。
            send_message(
                &json!({"method":"thread/name/set","id":13,"params":{"threadId":thread_id,"name":plan.name}}),
            )?;
            read_response(&mut reader, log, 13, progress, "lifecycle")?;
            clock.mark("thread_persist");
        }
        plan.journal.record(
            "thread",
            "acknowledge",
            json!({"nativeThreadId":thread_id,"sourceThreadId":plan.native_thread_id}),
        )?;
        record_fork_history(request, plan, &thread_id, "contextPrepare")?;
        for (index, batch) in crate::persistent_threads::history_batches(&plan.history)?
            .into_iter()
            .enumerate()
        {
            let suffix = format!("history:{index}");
            let items = json!(batch);
            plan.journal.prepare(&suffix,"inject",json!({"batch":index,"digest":crate::persistent_threads::digest(&items),"count":batch.len()}))?;
            plan.journal.record(&suffix, "dispatch", Value::Null)?;
            send_message(
                &json!({"method":"thread/inject_items","id":20,"params":{"threadId":thread_id,"items":items}}),
            )?;
            read_response(&mut reader, log, 20, progress, "reply")
                .map_err(|error| reject_known_rpc_error(Some(plan), &suffix, error))?;
            plan.journal.record(
                &suffix,
                "acknowledge",
                json!({"nativeThreadId":thread_id,"batch":index}),
            )?;
        }
        record_fork_history(request, plan, &thread_id, "contextCommit")?;
    }
    clock.mark("history_inject");
    (progress)(CodexHarnessEvent::status(
        "harness.thread_ready",
        "reply",
        "completed",
        if method == "thread/resume" {
            "模型 Harness 会话已恢复".to_owned()
        } else {
            "模型 Harness 会话已建立".to_owned()
        },
    ));

    if os == OperatingSystem::Windows {
        let command = contexthub_platform_adapter::process::sandbox_probe_command()
            .map_err(|error| format!("windows_sandbox_preflight_failed:{error}"))?;
        let params = crate::windows_sandbox::execution_probe_params(&thread_response, &command)?;
        (progress)(CodexHarnessEvent::status(
            "harness.sandbox_preflight",
            "reply",
            "running",
            "正在检查当前项目的沙箱执行环境",
        ));
        send_message(&json!({"method":"command/exec","id":12,"params":params}))?;
    }

    send_message(&json!({
        "method": "mcpServerStatus/list",
        "id": 7,
        "params": {
            "cursor": null,
            "limit": 100,
            "detail": "toolsAndAuthOnly",
            "threadId": thread_id
        }
    }))?;
    // MCP 状态只供上下文计量，和执行探测无依赖；仍须每轮探测成功后才能发 turn/start。
    let mcp_status = if os == OperatingSystem::Windows {
        read_probe_and_mcp_status(&mut reader, log, progress, &request.tool_snapshot_scope_id)?
    } else {
        read_optional_response(&mut reader, log, 7, progress, "reply")?
    };
    clock.mark("sandbox_and_mcp");
    adapter_ready()?;
    let tool_labels = application_tool_labels(request)?;
    clock.mark("adapter_ready");

    if let Some(plan) = request.persistent_thread.as_ref() {
        plan.journal.prepare_input(
            &plan.message_id,
            &plan.client_message_id,
            &plan.content_digest,
            false,
            &request.user_input,
        )?;
    }
    let turn_params = json!({
        "threadId": thread_id,
        "input": turn_input,
        "dynamicTools": dynamic_tools,
        "additionalContext": (!additional_context.is_empty()).then_some(additional_context),
        "clientUserMessageId": request.persistent_thread.as_ref().map(|plan| &plan.client_message_id),
        "effort": request.reasoning_effort,
        "responsesapiClientMetadata": application_tool_search_metadata(request.application_mcp.is_some())
    });
    if request.report_result.is_some()
        && (request.classification.is_some()
            || request.application_result.is_some()
            || request.application_adapter.is_none())
    {
        return Err("codex_harness_result_contract_conflict".into());
    }
    if let Some(admission) = &request.turn_admission {
        admission()?;
    }
    send_message(&json!({
        "method": "turn/start",
        "id": 3,
        "params": turn_params
    }))?;
    let turn_response = read_response(&mut reader, log, 3, progress, "reply").map_err(|error| {
        reject_known_rpc_error(
            request.persistent_thread.as_ref(),
            &format!(
                "input:{}",
                request
                    .persistent_thread
                    .as_ref()
                    .map_or("", |plan| plan.client_message_id.as_str())
            ),
            error,
        )
    })?;
    clock.mark("turn_start");
    clock.log("harness_protocol");
    let turn_id = turn_response
        .pointer("/result/turn/id")
        .and_then(Value::as_str)
        .ok_or_else(|| "codex_harness_turn_invalid".to_owned())?;
    if let Some(plan) = request.persistent_thread.as_ref() {
        plan.journal
            .accept_input(&plan.client_message_id, &thread_id, turn_id)?;
    }
    if let Some(control) = control {
        control.set_active_turn(&thread_id, turn_id)?;
    }
    let visible_started = std::time::Instant::now();
    let visible_scope = request.tool_snapshot_scope_id.clone();
    let first_visible = AtomicBool::new(false);
    let publish_visible = with_tool_labels(Arc::clone(progress), tool_labels);
    let visible_progress: ProgressCallback = Arc::new(move |event: CodexHarnessEvent| {
        let has_text = event
            .streamed_output
            .as_ref()
            .is_some_and(|text| !text.is_empty());
        if has_text && !first_visible.swap(true, Ordering::Relaxed) {
            eprintln!(
                "ai_send_timing command=harness_reply scope_id={visible_scope} stage=first_visible_text total_ms={}",
                visible_started.elapsed().as_millis()
            );
        }
        publish_visible(event);
    });
    let reply_progress: ProgressCallback = if request.classification.is_some() {
        let publish = Arc::clone(&visible_progress);
        let nonce = request
            .classification
            .as_ref()
            .and_then(|value| value.output_schema.pointer("/properties/nonce/enum/0"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Arc::new(move |mut event: CodexHarnessEvent| {
            if let Some(raw) = event.streamed_output.as_deref() {
                let Ok(Some(text)) = work_classification::visible_reply(
                    raw,
                    nonce.as_deref().unwrap_or_default(),
                    true,
                ) else {
                    return;
                };
                event.streamed_output = Some(text);
            } else if event.kind == "message" {
                let Ok(Some(text)) = work_classification::visible_reply(
                    &event.summary,
                    nonce.as_deref().unwrap_or_default(),
                    false,
                ) else {
                    return;
                };
                event.summary = text;
            }
            publish(event);
        })
    } else {
        visible_progress
    };
    let denied_artifact: ArtifactCallback = Arc::new(|_| Err("tool_scope_denied".to_owned()));
    let reply = read_turn(
        &mut reader,
        log,
        &thread_id,
        turn_id,
        &reply_progress,
        "reply",
        true,
        approval,
        if request.planning {
            Some(&denied_artifact)
        } else {
            artifact
        },
        request
            .application_action
            .as_ref()
            .filter(|_| !request.planning),
        request
            .application_mcp
            .as_ref()
            .zip(request.application_write.as_ref())
            .filter(|_| !request.planning),
        request
            .personal_tools
            .as_ref()
            .filter(|_| !request.planning),
        control,
        request.application_result.as_ref(),
        request.classification.as_ref(),
        &mut send_message,
    );
    if let Some(control) = control {
        control.clear_active_turn();
    }
    let mut reply = reply?;
    if let Some(contract) = request.report_result.as_ref() {
        reply.output = contract.complete(&reply.output)?;
    }
    let (output, classification_output, classification_error) =
        if let Some(classification) = request.classification.as_ref() {
            let nonce = classification
                .output_schema
                .pointer("/properties/nonce/enum/0")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let (output, error) = if let Some(result) = reply.classification_output.as_deref() {
                let output = crate::workbench_result::reply(result)?;
                (output, (classification.validate)(result).err())
            } else {
                (
                    work_classification::visible_reply(&reply.output, nonce, false)?
                        .ok_or("ai_response_envelope_content_invalid")?,
                    Some("ai_response_classification_result_missing".into()),
                )
            };
            (progress)(CodexHarnessEvent::status(
                if error.is_some() {
                    "harness.classification_failed"
                } else {
                    "harness.classification_completed"
                },
                "classification",
                if error.is_some() {
                    "failed"
                } else {
                    "completed"
                },
                if error.is_some() {
                    "本轮工作归类未通过校验"
                } else {
                    "本轮工作归类已生成"
                },
            ));
            (output, reply.classification_output.clone(), error)
        } else {
            (reply.output.clone(), None, None)
        };
    let usage = reply.usage.clone();
    let context_composition = reply.context_tokens.map(|tokens| {
        estimate_context_composition(
            tokens,
            &request.conversation_text,
            &reply.output,
            base_instructions,
            &developer_instructions,
            &json!([artifact_tool, application_action_tool]),
            mcp_status.as_ref(),
            request
                .application_mcp
                .as_ref()
                .map(|_| APPLICATION_MCP_SERVER_NAME),
            // 随包运行时只在新建线程时推送原始事件，恢复和分叉的线程看不到检索结果
            (method == "thread/start").then_some(reply.discovered_tool_tokens),
        )
    });
    Ok(CodexHarnessResult {
        thread_id,
        output,
        reply_usage: reply.usage_reported.then_some(reply.usage),
        classification_usage: None,
        context_tokens: reply.context_tokens,
        context_window: reply.context_window,
        context_composition,
        classification_output,
        classification_error,
        file_changes: reply.file_changes,
        usage,
    })
}

fn send(stdin: &mut impl std::io::Write, message: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *stdin, message)
        .map_err(|error| format!("codex_harness_request_invalid:{error}"))?;
    stdin
        .write_all(b"\n")
        .and_then(|()| stdin.flush())
        .map_err(|error| format!("codex_harness_write_failed:{error}"))
}

fn reject_known_rpc_error(plan: Option<&PersistentThread>, suffix: &str, error: String) -> String {
    if error.starts_with("codex_harness_rpc_failed:")
        && let Some(plan) = plan
        && plan
            .journal
            .record(suffix, "reject", json!({"reason":"native_rejected"}))
            .is_err()
    {
        return "message_delivery_uncertain".to_owned();
    }
    error
}

fn read_probe_and_mcp_status(
    reader: &mut impl BufRead,
    log: &mut BufWriter<File>,
    progress: &ProgressCallback,
    scope_id: &str,
) -> Result<Option<Value>, String> {
    let started = std::time::Instant::now();
    let mut probe_ready = false;
    let mut mcp_ready = false;
    let mut mcp_status = None;
    while !probe_ready || !mcp_ready {
        let message = read_message(reader, log)?;
        match message.get("id").and_then(Value::as_i64) {
            Some(12) if message.get("method").is_none() => {
                if let Some(error) = message.get("error") {
                    return Err(format!(
                        "windows_sandbox_preflight_failed:codex_harness_rpc_failed:{}",
                        compact_json(error)
                    ));
                }
                crate::windows_sandbox::verify_execution_probe(&message)?;
                probe_ready = true;
                eprintln!(
                    "ai_send_timing command=harness_preflight scope_id={scope_id} sandbox_probe_ms={}",
                    started.elapsed().as_millis()
                );
            }
            Some(7) if message.get("method").is_none() => {
                mcp_status = message.get("error").is_none().then_some(message);
                mcp_ready = true;
                eprintln!(
                    "ai_send_timing command=harness_preflight scope_id={scope_id} mcp_status_ms={}",
                    started.elapsed().as_millis()
                );
            }
            _ => {
                handle_notification(&message, progress, "reply");
                fail_on_server_request(&message)?;
            }
        }
    }
    Ok(mcp_status)
}

fn read_response(
    reader: &mut impl BufRead,
    log: &mut BufWriter<File>,
    id: i64,
    progress: &ProgressCallback,
    phase: &str,
) -> Result<Value, String> {
    loop {
        let message = read_message(reader, log)?;
        if message.get("id").and_then(Value::as_i64) == Some(id) {
            if let Some(error) = message.get("error") {
                return Err(format!("codex_harness_rpc_failed:{}", compact_json(error)));
            }
            return Ok(message);
        }
        handle_notification(&message, progress, phase);
        fail_on_server_request(&message)?;
    }
}

fn read_optional_response(
    reader: &mut impl BufRead,
    log: &mut BufWriter<File>,
    id: i64,
    progress: &ProgressCallback,
    phase: &str,
) -> Result<Option<Value>, String> {
    loop {
        let message = read_message(reader, log)?;
        if message.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok(message.get("error").is_none().then_some(message));
        }
        handle_notification(&message, progress, phase);
        fail_on_server_request(&message)?;
    }
}

#[expect(clippy::too_many_arguments, reason = "各参数对应独立的上下文计数来源")]
fn estimate_context_composition(
    total_tokens: u64,
    conversation_text: &str,
    output: &str,
    base_instructions: &str,
    developer_instructions: &str,
    artifact_tool: &Value,
    mcp_status: Option<&Value>,
    deferred_mcp_server: Option<&str>,
    discovered_tool_tokens: Option<u64>,
) -> CodexHarnessContextComposition {
    let conversation_tokens = estimate_tokens(&format!("{conversation_text}\n{output}"));
    let system_prompt_tokens =
        estimate_tokens(&format!("{base_instructions}\n{developer_instructions}"));
    let tool_tokens = estimate_tokens(&compact_json(artifact_tool));
    let skill_tokens = 0;
    let direct_mcp_tokens =
        mcp_tools_text(mcp_status, deferred_mcp_server).map_or(0, |tools| estimate_tokens(&tools));
    // 有延迟检索的应用工具却拿不到检索结果时（恢复线程收不到原始事件），整项未计量
    let discovered_tool_tokens = match (deferred_mcp_server, discovered_tool_tokens) {
        (Some(_), None) => None,
        (_, discovered) => Some(discovered.unwrap_or_default()),
    };
    let mcp_tokens = discovered_tool_tokens.and_then(|discovered| {
        (direct_mcp_tokens > 0 || discovered > 0)
            .then_some(direct_mcp_tokens.saturating_add(discovered))
    });
    let raw = [
        conversation_tokens,
        system_prompt_tokens,
        tool_tokens,
        skill_tokens,
        mcp_tokens.unwrap_or_default(),
    ];
    let raw_total = raw.iter().copied().sum::<u64>();
    let normalized = if raw_total > total_tokens && raw_total > 0 {
        raw.map(|tokens| {
            let scaled = (u128::from(tokens) * u128::from(total_tokens)) / u128::from(raw_total);
            match u64::try_from(scaled) {
                Ok(value) => value,
                Err(_) => unreachable!("scaled context tokens exceed total tokens"),
            }
        })
    } else {
        raw
    };
    let measured_total = normalized.iter().copied().sum::<u64>();
    CodexHarnessContextComposition {
        conversation_tokens: normalized[0],
        system_prompt_tokens: normalized[1],
        tool_tokens: normalized[2],
        skill_tokens: normalized[3],
        mcp_tokens: mcp_tokens.map(|_| normalized[4]),
        internal_tokens: total_tokens.saturating_sub(measured_total),
    }
}

fn estimate_tokens(text: &str) -> u64 {
    let mut tokens = 0_u64;
    let mut ascii_run = 0_u64;
    let flush_ascii = |tokens: &mut u64, ascii_run: &mut u64| {
        if *ascii_run > 0 {
            *tokens = tokens.saturating_add((*ascii_run).div_ceil(4));
            *ascii_run = 0;
        }
    };
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
            ascii_run = ascii_run.saturating_add(1);
        } else {
            flush_ascii(&mut tokens, &mut ascii_run);
            if !character.is_whitespace() {
                tokens = tokens.saturating_add(1);
            }
        }
    }
    flush_ascii(&mut tokens, &mut ascii_run);
    tokens
}

fn mcp_tools_text(status: Option<&Value>, deferred_server: Option<&str>) -> Option<String> {
    let servers = status?.pointer("/result/data")?.as_array()?;
    let tools = servers
        .iter()
        .filter(|server| {
            let server_name = server
                .get("name")
                .or_else(|| server.get("serverName"))
                .and_then(Value::as_str);
            deferred_server.is_none_or(|deferred| server_name != Some(deferred))
        })
        .filter_map(|server| server.get("tools"))
        .cloned()
        .collect::<Vec<_>>();
    serde_json::to_string(&tools).ok()
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn read_turn(
    reader: &mut impl BufRead,
    log: &mut BufWriter<File>,
    thread_id: &str,
    turn_id: &str,
    progress: &ProgressCallback,
    phase: &str,
    stream_output: bool,
    approval: Option<&ApprovalCallback>,
    artifact: Option<&ArtifactCallback>,
    application_action: Option<&ApplicationActionCallback>,
    application_write: Option<(&ApplicationMcpScope, &ApplicationWriteCallback)>,
    personal_tools: Option<&PersonalToolsCallback>,
    control: Option<&CodexHarnessControl>,
    result_task: Option<&crate::app_bridge::ApplicationModelTask>,
    classification: Option<&CodexHarnessClassificationRequest>,
    send_message: &mut impl FnMut(&Value) -> Result<(), String>,
) -> Result<CodexHarnessTurnResult, String> {
    let mut output = String::new();
    let mut accepted_result = None;
    let mut accepted_classification = None;
    let mut fallback_output = String::new();
    let mut file_changes = Vec::new();
    let mut usage = CodexHarnessUsage::default();
    let mut usage_baseline: Option<CodexHarnessUsage> = None;
    let mut context_tokens = None;
    let mut context_window = None;
    let mut discovered_tool_tokens = 0_u64;
    let mut discovered_definitions = HashSet::new();
    let mut streamed_reply_item_id: Option<String> = None;
    let mut streamed_output = String::new();
    loop {
        let message = read_message(reader, log)?;
        if control.is_some_and(|control| control.resolve_pending_response(&message)) {
            continue;
        }
        if let Some(request) = parse_approval_request(&message) {
            let Some(approval) = approval else {
                return Err("codex_harness_unexpected_approval".to_owned());
            };
            approval(request);
            continue;
        }
        if crate::workbench_result::handle(
            &message,
            thread_id,
            turn_id,
            classification,
            &mut accepted_classification,
            send_message,
        )? {
            continue;
        }
        if crate::application_result::handle(
            &message,
            thread_id,
            turn_id,
            result_task,
            &mut accepted_result,
            send_message,
        )? {
            continue;
        }
        if handle_personal_tools_request(
            &message,
            thread_id,
            turn_id,
            personal_tools,
            send_message,
        )? {
            continue;
        }
        if handle_artifact_tool_request(&message, thread_id, turn_id, artifact, send_message)? {
            continue;
        }
        if handle_application_action_request(
            &message,
            thread_id,
            turn_id,
            application_action,
            send_message,
        )? {
            continue;
        }
        if handle_application_write_request(
            &message,
            thread_id,
            turn_id,
            application_write,
            send_message,
        )? {
            continue;
        }
        fail_on_server_request(&message)?;
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = message.get("params").unwrap_or(&Value::Null);
        if phase == "reply"
            && method == "item/started"
            && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
            && params.get("turnId").and_then(Value::as_str) == Some(turn_id)
            && params.pointer("/item/type").and_then(Value::as_str) == Some("userMessage")
            && let Some(client_message_id) =
                params.pointer("/item/clientId").and_then(Value::as_str)
            && let Some(input) =
                control.and_then(|control| control.take_accepted_steer(client_message_id))
        {
            if let Some(journal) = control.and_then(|control| control.journal.as_ref()) {
                journal.accept_input(client_message_id, thread_id, turn_id)?;
            }
            (progress)(CodexHarnessEvent {
                event_type: "turn.steered".to_owned(),
                item_id: params
                    .pointer("/item/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                phase: "reply".to_owned(),
                kind: "status".to_owned(),
                status: "completed".to_owned(),
                summary: "已将补充指令加入当前回复".to_owned(),
                duration_ms: None,
                detail_available: false,
                streamed_output: None,
                steered_input: Some((client_message_id.to_owned(), input)),
                read_paths: Vec::new(),
                context_usage: None,
                created_at: now_millis(),
            });
        }
        // 只在新建线程的回合生效：随包运行时恢复、分叉线程时不推送原始事件。后续回合由
        // 模型供应服务保证不会把同名函数调用交给客户端（否则运行时会一直等待）。
        if method == "rawResponseItem/completed"
            && params.pointer("/item/type").and_then(Value::as_str) == Some("function_call")
            && params.pointer("/item/name").and_then(Value::as_str) == Some("tool_search")
        {
            return Err("application_tool_search_transport_incompatible".to_owned());
        }
        if method == "rawResponseItem/completed"
            && params.pointer("/item/type").and_then(Value::as_str) == Some("tool_search_output")
            && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
            && let Some(tools) = params.pointer("/item/tools").and_then(Value::as_array)
        {
            for tool in tools {
                let definition = compact_json(tool);
                if discovered_definitions.insert(definition.clone()) {
                    discovered_tool_tokens =
                        discovered_tool_tokens.saturating_add(estimate_tokens(&definition));
                }
            }
        }
        if stream_output
            && method == "item/started"
            && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
            && params.get("turnId").and_then(Value::as_str) == Some(turn_id)
            && params.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage")
            && matches!(
                params.pointer("/item/phase").and_then(Value::as_str),
                None | Some("final_answer")
            )
            && let Some(item_id) = params.pointer("/item/id").and_then(Value::as_str)
        {
            streamed_reply_item_id = Some(item_id.to_owned());
            streamed_output.clear();
        }
        if stream_output
            && method == "item/agentMessage/delta"
            && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
            && params.get("turnId").and_then(Value::as_str) == Some(turn_id)
            && params.get("itemId").and_then(Value::as_str) == streamed_reply_item_id.as_deref()
            && let Some(delta) = params.get("delta").and_then(Value::as_str)
            && !delta.is_empty()
        {
            streamed_output.push_str(delta);
            (progress)(CodexHarnessEvent {
                event_type: "item.agentMessage.delta".to_owned(),
                item_id: params
                    .get("itemId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                phase: phase.to_owned(),
                kind: "message".to_owned(),
                status: "started".to_owned(),
                summary: "正在生成回复…".to_owned(),
                duration_ms: None,
                detail_available: false,
                streamed_output: Some(streamed_output.clone()),
                steered_input: None,
                read_paths: Vec::new(),
                context_usage: None,
                created_at: now_millis(),
            });
        }
        if method == "item/completed"
            && params.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage")
            && let Some(text) = params.pointer("/item/text").and_then(Value::as_str)
            && !text.trim().is_empty()
        {
            text.trim().clone_into(&mut fallback_output);
            if params.pointer("/item/phase").and_then(Value::as_str) == Some("final_answer") {
                text.trim().clone_into(&mut output);
            }
        }
        if method == "item/completed" {
            collect_file_changes(
                params.pointer("/item").unwrap_or(&Value::Null),
                &mut file_changes,
            );
        }
        if method == "thread/tokenUsage/updated"
            && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
            && params.get("turnId").and_then(Value::as_str) == Some(turn_id)
            && let (Some(total), Some(last)) = (
                params
                    .pointer("/tokenUsage/total")
                    .filter(|value| value.is_object()),
                params
                    .pointer("/tokenUsage/last")
                    .filter(|value| value.is_object()),
            )
        {
            let total = parse_usage(total);
            let last_usage = parse_usage(last);
            let baseline =
                usage_baseline.get_or_insert_with(|| usage_difference(&total, &last_usage));
            usage = usage_difference(&total, baseline);
            context_tokens = last.get("totalTokens").and_then(Value::as_u64);
            context_window = params
                .pointer("/tokenUsage/modelContextWindow")
                .and_then(Value::as_u64)
                .or(context_window);
            if let Some(tokens) = context_tokens {
                let mut event = CodexHarnessEvent::status(
                    "thread.tokenUsage.updated",
                    phase,
                    "completed",
                    "Token 使用量已更新",
                );
                event.context_usage = Some((tokens, context_window));
                (progress)(event);
                continue;
            }
        }
        handle_notification(&message, progress, phase);
        if method == "turn/completed"
            && params.get("threadId").and_then(Value::as_str) == Some(thread_id)
            && params.pointer("/turn/id").and_then(Value::as_str) == Some(turn_id)
        {
            if let Some(items) = params.pointer("/turn/items").and_then(Value::as_array) {
                for item in items {
                    collect_file_changes(item, &mut file_changes);
                }
            }
            if let Some(control) = control
                && let Ok(mut terminal_thread) = control.terminal_thread.lock()
            {
                *terminal_thread = Some(thread_id.to_owned());
            }
            let status = params.pointer("/turn/status").and_then(Value::as_str);
            if status != Some("completed") {
                return Err(turn_failure_reason(params));
            }
            if result_task.is_some() {
                output =
                    crate::application_result::finish(result_task, accepted_result.take(), output)?;
            }
            if output.trim().is_empty() {
                params
                    .pointer("/turn/items")
                    .and_then(Value::as_array)
                    .and_then(|items| {
                        items
                            .iter()
                            .rev()
                            .find_map(|item| {
                                (item.get("type").and_then(Value::as_str) == Some("agentMessage")
                                    && item.get("phase").and_then(Value::as_str)
                                        == Some("final_answer"))
                                .then(|| item.get("text").and_then(Value::as_str))
                                .flatten()
                            })
                            .or_else(|| {
                                items.iter().rev().find_map(|item| {
                                    (item.get("type").and_then(Value::as_str)
                                        == Some("agentMessage"))
                                    .then(|| item.get("text").and_then(Value::as_str))
                                    .flatten()
                                })
                            })
                    })
                    .unwrap_or_default()
                    .trim()
                    .clone_into(&mut output);
            }
            if output.is_empty() {
                output = fallback_output;
            }
            if output.is_empty()
                && let Some(result) = accepted_classification.as_ref()
            {
                output = crate::workbench_result::reply(result)?;
            }
            if output.is_empty() {
                return Err("codex_harness_empty_response".to_owned());
            }
            if is_tool_output_too_large_response(&output) {
                return Err("codex_harness_tool_output_too_large".to_owned());
            }
            return Ok(CodexHarnessTurnResult {
                output,
                classification_output: accepted_classification,
                file_changes,
                usage,
                usage_reported: usage_baseline.is_some(),
                context_tokens,
                context_window,
                discovered_tool_tokens,
            });
        }
    }
}

fn handle_artifact_tool_request(
    message: &Value,
    thread_id: &str,
    turn_id: &str,
    artifact: Option<&ArtifactCallback>,
    send_message: &mut impl FnMut(&Value) -> Result<(), String>,
) -> Result<bool, String> {
    if message.get("method").and_then(Value::as_str) != Some("item/tool/call")
        || message.pointer("/params/tool").and_then(Value::as_str) != Some(REGISTER_ARTIFACT_TOOL)
    {
        return Ok(false);
    }
    let request_id = message
        .get("id")
        .cloned()
        .ok_or_else(|| "codex_harness_artifact_request_id_missing".to_owned())?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let result = if params.get("threadId").and_then(Value::as_str) != Some(thread_id)
        || params.get("turnId").and_then(Value::as_str) != Some(turn_id)
        || params.get("tool").and_then(Value::as_str) != Some(REGISTER_ARTIFACT_TOOL)
    {
        Err("产物登记请求不属于当前运行".to_owned())
    } else if let Some(path) = params
        .pointer("/arguments/path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        artifact.map_or_else(
            || Err("当前运行未启用产物登记".to_owned()),
            |callback| callback(path),
        )
    } else {
        Err("产物路径不能为空".to_owned())
    };
    let (success, text) = match result {
        Ok(path) => (true, format!("产物已登记：{path}")),
        Err(error) => (false, format!("产物登记失败：{error}")),
    };
    send_message(&json!({
        "id": request_id,
        "result": {
            "contentItems": [{ "type": "inputText", "text": text }],
            "success": success
        }
    }))?;
    Ok(true)
}

fn handle_application_action_request(
    message: &Value,
    thread_id: &str,
    turn_id: &str,
    application_action: Option<&ApplicationActionCallback>,
    send_message: &mut impl FnMut(&Value) -> Result<(), String>,
) -> Result<bool, String> {
    if message.get("method").and_then(Value::as_str) != Some("item/tool/call")
        || message.pointer("/params/tool").and_then(Value::as_str) != Some(APPLICATION_ACTION_TOOL)
    {
        return Ok(false);
    }
    let request_id = message
        .get("id")
        .cloned()
        .ok_or_else(|| "codex_harness_application_action_request_id_missing".to_owned())?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let result = if params.get("threadId").and_then(Value::as_str) != Some(thread_id)
        || params.get("turnId").and_then(Value::as_str) != Some(turn_id)
    {
        Err("页面动作请求不属于当前运行".to_owned())
    } else if let (Some(action_id), Some(arguments)) = (
        params
            .pointer("/arguments/actionId")
            .and_then(Value::as_str),
        params
            .pointer("/arguments/arguments")
            .filter(|value| value.is_object()),
    ) {
        application_action.map_or_else(
            || Err("本轮没有已授权的应用页面上下文".to_owned()),
            |callback| callback(action_id, arguments.clone()),
        )
    } else {
        Err("页面动作标识或参数不合法".to_owned())
    };
    let (success, text) = match result {
        Ok(value) => (
            true,
            serde_json::to_string(&value)
                .map_err(|error| format!("application_action_result_invalid:{error}"))?,
        ),
        Err(error) => (false, format!("页面动作未完成：{error}")),
    };
    send_message(&json!({
        "id": request_id,
        "result": {
            "contentItems": [{ "type": "inputText", "text": text }],
            "success": success
        }
    }))?;
    Ok(true)
}

fn handle_application_write_request(
    message: &Value,
    thread_id: &str,
    turn_id: &str,
    application_write: Option<(&ApplicationMcpScope, &ApplicationWriteCallback)>,
    send_message: &mut impl FnMut(&Value) -> Result<(), String>,
) -> Result<bool, String> {
    if message.get("method").and_then(Value::as_str) != Some("item/tool/call") {
        return Ok(false);
    }
    let dynamic_name = message.pointer("/params/tool").and_then(Value::as_str);
    let Some((scope, callback)) = application_write else {
        return Ok(false);
    };
    let Some(tool) = scope
        .write_tools
        .iter()
        .find(|tool| Some(tool.dynamic_name.as_str()) == dynamic_name)
    else {
        return Ok(false);
    };
    let request_id = message
        .get("id")
        .cloned()
        .ok_or_else(|| "codex_harness_application_write_request_id_missing".to_owned())?;
    let params = message.get("params").unwrap_or(&Value::Null);
    let result = if params.get("threadId").and_then(Value::as_str) != Some(thread_id)
        || params.get("turnId").and_then(Value::as_str) != Some(turn_id)
    {
        Err("应用写入请求不属于当前运行".to_owned())
    } else if let Some(arguments) = params
        .get("arguments")
        .filter(|arguments| arguments.is_object())
    {
        callback(tool, arguments.clone())
    } else {
        Err("应用写入参数不合法".to_owned())
    };
    let (success, text) = match result {
        Ok(value) => (
            true,
            serde_json::to_string(&value)
                .map_err(|error| format!("application_write_result_invalid:{error}"))?,
        ),
        Err(error) => (false, format!("应用写入未完成：{error}")),
    };
    send_message(&json!({
        "id": request_id,
        "result": {
            "contentItems": [{ "type": "inputText", "text": text }],
            "success": success
        }
    }))?;
    Ok(true)
}

fn usage_difference(total: &CodexHarnessUsage, baseline: &CodexHarnessUsage) -> CodexHarnessUsage {
    CodexHarnessUsage {
        input: total.input.saturating_sub(baseline.input),
        cached_input: total.cached_input.saturating_sub(baseline.cached_input),
        output: total.output.saturating_sub(baseline.output),
    }
}

fn turn_approval_settings<'a>(
    planning: bool,
    approval_mode: &str,
    sandbox: &'a str,
) -> Result<(&'static str, &'static str, &'a str), String> {
    if planning {
        Ok(("never", "user", "read-only"))
    } else {
        approval_settings(approval_mode, sandbox)
    }
}

fn approval_settings<'a>(
    approval_mode: &str,
    legacy_sandbox_mode: &'a str,
) -> Result<(&'static str, &'static str, &'a str), String> {
    match approval_mode {
        "request" => Ok(("on-request", "user", "workspace-write")),
        "auto-review" => Ok(("on-request", "auto_review", "workspace-write")),
        "full-access" => Ok(("never", "user", "danger-full-access")),
        "legacy" => Ok(("never", "user", legacy_sandbox_mode)),
        _ => Err("codex_harness_approval_mode_invalid".to_owned()),
    }
}

/*
 * 三种审批方式落到 MCP 工具上的含义。不声明就取 harness 的默认值 auto，而 auto 对没有注解的
 * 工具一律按"有破坏性"处理——那会让读页面这类操作也要员工点一次，与员工选的方式对不上。
 *
 * 请求批准：每次都问，包括只读工具；
 * 帮我批准：只问会留下痕迹的那些，读页面自动放行（由工具自己声明的只读注解判定）；
 * 完全访问：一概不问。
 */
fn mcp_tools_approval_mode(approval_mode: &str) -> &'static str {
    match approval_mode {
        "request" => "prompt",
        "full-access" => "approve",
        _ => "auto",
    }
}

/// MCP 工具调用的审批。Harness 借 elicitation 这条通道发来，靠 `_meta` 里的种类标识区分：
/// 同一个方法也用于服务端向员工提问，那种不是审批，不能当审批放行。
fn parse_mcp_tool_approval_request(message: &Value) -> Option<CodexHarnessApprovalRequest> {
    let params = message.get("params")?;
    let meta = params.get("_meta")?;
    if meta.get("codex_approval_kind").and_then(Value::as_str) != Some("mcp_tool_call") {
        return None;
    }
    let server = params
        .get("serverName")
        .and_then(Value::as_str)
        .unwrap_or("未知服务");
    let tool_params = meta.get("tool_params");
    /*
     * 审批文案要让员工一眼看出"批的是什么"。工具自己的描述最贴切，参数里的地址或文本
     * 才是真正要被批准的东西，两者都带上；缺了就退回服务名，不编造内容。
     */
    let description = meta
        .get("tool_description")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let target = tool_params
        .and_then(Value::as_object)
        .and_then(|fields| {
            fields
                .get("url")
                .or_else(|| fields.get("text"))
                .or_else(|| fields.get("purpose"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let reason = match (description, target) {
        (Some(description), Some(target)) => format!("{description}\n目标：{target}"),
        (Some(description), None) => description.to_owned(),
        (None, Some(target)) => format!("允许 AI 使用 {server} 操作：{target}"),
        (None, None) => format!("允许 AI 使用 {server} 吗"),
    };
    /*
     * 这条请求里没有条目标识——它是服务端请求，不属于任何已发出的条目。用请求标识兜住，
     * 界面据此把卡片挂到当前运行上；没有标识就没法回答，那种消息不该被当成审批。
     */
    let item_id = match message.get("id")? {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => return None,
    };
    // 持久化选项由 harness 声明，员工能选"本会话都允许"与否取决于它，不由我们自己决定
    let persists: Vec<String> = meta
        .get("persist")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut available_decisions = vec!["approved".to_owned(), "rejected".to_owned()];
    if persists.iter().any(|value| value == "session") {
        available_decisions.push("approved_session".to_owned());
    }
    Some(CodexHarnessApprovalRequest {
        request_id: message.get("id")?.clone(),
        method: MCP_TOOL_APPROVAL_METHOD.to_owned(),
        item_id,
        reason,
        command: tool_params.map(compact_json),
        resource: target.map(str::to_owned),
        requested_permissions: None,
        available_decisions,
        requested_at: now_millis(),
    })
}

fn parse_approval_request(message: &Value) -> Option<CodexHarnessApprovalRequest> {
    let method = message.get("method")?.as_str()?;
    if method == MCP_TOOL_APPROVAL_METHOD {
        return parse_mcp_tool_approval_request(message);
    }
    if !matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
    ) {
        return None;
    }
    let params = message.get("params")?;
    let item_id = params.get("itemId")?.as_str()?.to_owned();
    let reason = params
        .get("reason")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(match method {
            "item/commandExecution/requestApproval" => "需要允许执行此命令",
            "item/fileChange/requestApproval" => "需要允许修改此文件",
            _ => "需要扩大本次运行权限",
        })
        .to_owned();
    let resource = params
        .get("grantRoot")
        .or_else(|| params.get("cwd"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let available_decisions = params
        .get("availableDecisions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Some(CodexHarnessApprovalRequest {
        request_id: message.get("id")?.clone(),
        method: method.to_owned(),
        item_id,
        reason,
        command: params
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned),
        resource,
        requested_permissions: params
            .get("permissions")
            .or_else(|| params.get("additionalPermissions"))
            .cloned(),
        available_decisions,
        requested_at: params
            .get("startedAtMs")
            .and_then(Value::as_i64)
            .unwrap_or_else(now_millis),
    })
}

fn is_tool_output_too_large_response(output: &str) -> bool {
    output.trim() == TOOL_OUTPUT_TOO_LARGE_RESPONSE
}

fn collect_file_changes(item: &Value, output: &mut Vec<CodexHarnessFileChange>) {
    if item.get("type").and_then(Value::as_str) != Some("fileChange") {
        return;
    }
    let Some(changes) = item.get("changes").and_then(Value::as_array) else {
        return;
    };
    for change in changes {
        let Some(path) = change.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(kind) = change
            .pointer("/kind/type")
            .and_then(Value::as_str)
            .or_else(|| change.get("kind").and_then(Value::as_str))
        else {
            continue;
        };
        if path.trim().is_empty() {
            continue;
        }
        if let Some(current) = output.iter_mut().find(|current| current.path == path) {
            kind.clone_into(&mut current.kind);
        } else {
            output.push(CodexHarnessFileChange {
                path: path.to_owned(),
                kind: kind.to_owned(),
            });
        }
    }
}

fn read_message(reader: &mut impl BufRead, log: &mut BufWriter<File>) -> Result<Value, String> {
    let mut line = String::new();
    let size = reader
        .read_line(&mut line)
        .map_err(|error| format!("codex_harness_read_failed:{error}"))?;
    if size == 0 {
        return Err("codex_harness_closed".to_owned());
    }
    let message: Value = serde_json::from_str(line.trim())
        .map_err(|error| format!("codex_harness_protocol_invalid:{error}"))?;
    let service_error = safe_service_error(&message);
    let audit = json!({"atMs":SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|elapsed| elapsed.as_millis()),"id":message["id"],"method":message["method"],"threadId":message.pointer("/params/threadId").or_else(||message.pointer("/result/thread/id")),"turnId":message["params"]["turnId"],"itemId":message["params"]["item"]["id"],"itemType":message["params"]["item"]["type"],"classificationCall":message.pointer("/params/tool").and_then(Value::as_str)==Some(crate::workbench_result::RESULT_TOOL),"applicationResultCall":message.pointer("/params/tool").and_then(Value::as_str)==Some(crate::application_result::RESULT_TOOL),"runtimeVersion":message.pointer("/result/thread/cliVersion"),"completedStatus":message.pointer("/params/turn/status"),"errorCode":message["error"]["code"],"serviceError":service_error});
    log.write_all(format!("{audit}\n").as_bytes())
        .and_then(|()| log.flush())
        .map_err(|error| format!("codex_harness_log_failed:{error}"))?;
    Ok(message)
}

fn safe_service_error(
    message: &Value,
) -> Option<crate::model_session_proxy::service_error::ServiceError> {
    [
        "/params/turn/error/message",
        "/params/error/message",
        "/params/message",
        "/error/message",
    ]
    .into_iter()
    .filter_map(|pointer| message.pointer(pointer).and_then(Value::as_str))
    .find_map(crate::model_session_proxy::service_error::ServiceError::from_native_message)
}

fn fail_on_server_request(message: &Value) -> Result<(), String> {
    if message.get("id").is_some() && message.get("method").is_some() {
        return Err(format!(
            "codex_harness_unexpected_approval:{}",
            message
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    Ok(())
}

/// 本轮应用工具的版本名与可读名称对照。版本名由契约摘要生成，员工看不懂，
/// 同时写到运行日志旁，供事后查看步骤详情时使用。
fn application_tool_labels(
    request: &CodexHarnessRequest,
) -> Result<BTreeMap<String, String>, String> {
    let Some((_, slot, _)) = request.application_adapter.as_ref() else {
        return Ok(BTreeMap::new());
    };
    let labels = slot
        .current()
        .as_ref()
        .map(|adapter| adapter.labels())
        .unwrap_or_default();
    if !labels.is_empty() {
        let bytes = serde_json::to_vec(&labels)
            .map_err(|error| format!("application_tool_labels_invalid:{error}"))?;
        fs::write(application_tool_labels_path(&request.event_log_path), bytes)
            .map_err(|error| format!("application_tool_labels_write_failed:{error}"))?;
    }
    Ok(labels)
}

pub(crate) fn application_tool_labels_path(event_log_path: &Path) -> PathBuf {
    event_log_path.with_extension("tool-labels.json")
}

/// 把工具类事件摘要里的版本名换成可读名称，其余事件原样转发
fn with_tool_labels(
    progress: ProgressCallback,
    labels: BTreeMap<String, String>,
) -> ProgressCallback {
    if labels.is_empty() {
        return progress;
    }
    Arc::new(move |mut event: CodexHarnessEvent| {
        if event.kind == "mcp" {
            for (name, label) in &labels {
                if event.summary.contains(name.as_str()) {
                    event.summary = event.summary.replace(name.as_str(), label);
                }
            }
        }
        (progress)(event);
    })
}

/// 上下文超出窗口时运行时的原文是英文，且与其他失败无从区分；
/// 带上固定标记，由对外错误映射给出可操作的中文说明。
fn turn_failure_reason(params: &Value) -> String {
    let message = params
        .pointer("/turn/error/message")
        .and_then(Value::as_str)
        .unwrap_or("模型 Harness 运行未完成");
    let context_window_exceeded = params
        .pointer("/turn/error/codexErrorInfo")
        .and_then(Value::as_str)
        == Some("contextWindowExceeded");
    if context_window_exceeded {
        format!("codex_harness_turn_failed:context_window_exceeded:{message}")
    } else {
        format!("codex_harness_turn_failed:{message}")
    }
}

fn completed_read_paths(method: &str, item: &Value) -> Vec<String> {
    if method != "item/completed"
        || item["type"] != "commandExecution"
        || item["status"] != "completed"
        || item["exitCode"].as_i64() != Some(0)
    {
        return Vec::new();
    }
    let Some(actions) = item["commandActions"].as_array() else {
        return Vec::new();
    };
    actions
        .iter()
        .filter(|action| action["type"] == "read")
        .filter_map(|action| {
            let path = Path::new(action["path"].as_str()?);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                Path::new(item["cwd"].as_str()?).join(path)
            };
            path.canonicalize()
                .ok()
                .map(|value| value.to_string_lossy().into_owned())
        })
        .collect()
}

pub(crate) fn handle_notification(message: &Value, progress: &ProgressCallback, phase: &str) {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return;
    };
    let params = message.get("params").unwrap_or(&Value::Null);
    let item = params.pointer("/item").unwrap_or(&Value::Null);
    if method == "rawResponseItem/completed"
        && let Some(event) = tool_search_progress_event(item, phase)
    {
        (progress)(event);
        return;
    }
    let summary = match method {
        "turn/started" => Some("模型开始处理本轮请求".to_owned()),
        "turn/plan/updated" => plan_summary(params),
        "item/started" => item_summary(item, false),
        "item/completed" => item_summary(item, true),
        "thread/tokenUsage/updated" => Some("Token 使用量已更新".to_owned()),
        "turn/completed" => Some("模型 Harness 本轮运行结束".to_owned()),
        _ => None,
    };
    if let Some(summary) = summary {
        let kind = match method {
            "turn/plan/updated" => "plan",
            "item/started" | "item/completed" => match item.get("type").and_then(Value::as_str) {
                Some("agentMessage") => "message",
                Some("reasoning") => "reasoning",
                Some("commandExecution") => "command",
                Some("fileChange") => "file_change",
                Some("mcpToolCall") => "mcp",
                Some("webSearch") => "web_search",
                Some("contextCompaction") => "context",
                _ => "status",
            },
            _ => "status",
        };
        let status = if method == "item/started" || method == "turn/started" {
            "started"
        } else if matches!(
            item.get("status").and_then(Value::as_str),
            Some("failed" | "declined")
        ) {
            "failed"
        } else {
            "completed"
        };
        (progress)(CodexHarnessEvent {
            event_type: method.replace('/', "."),
            item_id: item
                .get("id")
                .or_else(|| params.get("itemId"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            phase: phase.to_owned(),
            kind: kind.to_owned(),
            status: status.to_owned(),
            summary,
            duration_ms: item.get("durationMs").and_then(Value::as_u64),
            detail_available: matches!(kind, "command" | "mcp" | "file_change" | "web_search"),
            streamed_output: None,
            steered_input: None,
            read_paths: completed_read_paths(method, item),
            context_usage: None,
            created_at: now_millis(),
        });
    }
}

fn tool_search_progress_event(item: &Value, phase: &str) -> Option<CodexHarnessEvent> {
    let item_type = item.get("type").and_then(Value::as_str)?;
    let (event_type, status, summary) = match item_type {
        "tool_search_call" => (
            "tool_search.started",
            "started",
            "正在查找企业工具".to_owned(),
        ),
        "tool_search_output" => {
            let count = item
                .get("tools")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            (
                "tool_search.completed",
                "completed",
                format!("已发现 {count} 个企业工具"),
            )
        }
        _ => return None,
    };
    Some(CodexHarnessEvent {
        event_type: event_type.to_owned(),
        item_id: item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        phase: phase.to_owned(),
        kind: "mcp".to_owned(),
        status: status.to_owned(),
        summary,
        duration_ms: None,
        detail_available: false,
        streamed_output: None,
        steered_input: None,
        read_paths: Vec::new(),
        context_usage: None,
        created_at: now_millis(),
    })
}

fn plan_summary(params: &Value) -> Option<String> {
    let plan = params.get("plan")?.as_array()?;
    if plan.is_empty() {
        return None;
    }
    let completed = plan
        .iter()
        .filter(|step| step.get("status").and_then(Value::as_str) == Some("completed"))
        .count();
    Some(format!("Todo 已完成 {completed}/{}", plan.len()))
}

fn item_summary(item: &Value, completed: bool) -> Option<String> {
    let failed = matches!(
        item.get("status").and_then(Value::as_str),
        Some("failed" | "declined")
    );
    match item.get("type").and_then(Value::as_str)? {
        "agentMessage" => {
            let final_answer = item.get("phase").and_then(Value::as_str) == Some("final_answer");
            if completed && !final_answer {
                item.get("text")
                    .and_then(Value::as_str)
                    .and_then(|text| compact_visible_text(text, 1_200))
                    .or_else(|| Some("已整理执行说明".to_owned()))
            } else if completed {
                Some("已生成回复".to_owned())
            } else {
                Some("正在生成回复".to_owned())
            }
        }
        "reasoning" => {
            if completed {
                item.get("summary")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .and_then(|summary| compact_visible_text(&summary, 1_200))
                    .or_else(|| Some("已分析上下文与约束".to_owned()))
            } else {
                Some("正在分析上下文与约束".to_owned())
            }
        }
        "commandExecution" => Some(command_activity_summary(item, completed, failed)),
        "webSearch" => Some(
            if !completed {
                "正在搜索网页"
            } else if failed {
                "网页搜索失败"
            } else {
                "已搜索网页"
            }
            .to_owned(),
        ),
        "fileChange" => {
            let count = item
                .get("changes")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Some(if !completed {
                "正在编辑文件".to_owned()
            } else if failed {
                "文件编辑失败".to_owned()
            } else if count > 0 {
                format!("已编辑 {count} 个文件")
            } else {
                "已编辑文件".to_owned()
            })
        }
        "mcpToolCall" => {
            let tool = item
                .pointer("/appContext/actionName")
                .or_else(|| item.get("tool"))
                .and_then(Value::as_str)
                .and_then(|value| compact_single_line(value, 80))
                .unwrap_or_else(|| "已授权工具".to_owned());
            Some(if !completed {
                format!("正在使用 {tool}")
            } else if failed {
                format!("{tool} 调用失败")
            } else {
                format!("已使用 {tool}")
            })
        }
        // 运行时压缩完成后另发一条英文提醒（多次压缩会降低准确度），
        // 建议并入这里的完成文案，不按上游原句匹配那条提醒。
        "contextCompaction" => Some(if completed {
            "已整理对话上下文，对话较长时建议新建对话继续".to_owned()
        } else {
            "正在整理对话上下文".to_owned()
        }),
        _ => None,
    }
}

fn command_activity_summary(item: &Value, completed: bool, failed: bool) -> String {
    if completed && item.get("status").and_then(Value::as_str) == Some("declined") {
        return "已拒绝工作区命令".to_owned();
    }
    let action = item
        .get("commandActions")
        .and_then(Value::as_array)
        .and_then(|actions| actions.first());
    let action_type = action
        .and_then(|action| action.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let target = action.and_then(command_action_target);
    let (ongoing, done, failed_label) = match (action_type, target.as_deref()) {
        ("read", Some(target)) => (
            format!("正在读取 {target}"),
            format!("已读取 {target}"),
            format!("读取 {target} 失败"),
        ),
        ("read", None) => (
            "正在读取文件".to_owned(),
            "已读取文件".to_owned(),
            "读取文件失败".to_owned(),
        ),
        ("listFiles", Some(target)) => (
            format!("正在查看 {target}"),
            format!("已查看 {target}"),
            format!("查看 {target} 失败"),
        ),
        ("listFiles", None) => (
            "正在查看工作区".to_owned(),
            "已查看工作区".to_owned(),
            "查看工作区失败".to_owned(),
        ),
        ("search", Some(target)) => (
            format!("正在 {target} 中搜索内容"),
            format!("已在 {target} 中搜索内容"),
            format!("在 {target} 中搜索失败"),
        ),
        ("search", None) => (
            "正在工作区搜索内容".to_owned(),
            "已在工作区搜索内容".to_owned(),
            "在工作区搜索失败".to_owned(),
        ),
        _ => (
            "正在运行工作区命令".to_owned(),
            "已运行工作区命令".to_owned(),
            "工作区命令运行失败".to_owned(),
        ),
    };
    if !completed {
        ongoing
    } else if failed {
        failed_label
    } else {
        done
    }
}

fn command_action_target(action: &Value) -> Option<String> {
    action
        .get("name")
        .and_then(Value::as_str)
        .and_then(|value| compact_single_line(value, 80))
        .or_else(|| {
            let path = action.get("path")?.as_str()?;
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|value| compact_single_line(value, 80))
        })
}

fn compact_single_line(value: &str, max_chars: usize) -> Option<String> {
    compact_visible_text(
        &value.split_whitespace().collect::<Vec<_>>().join(" "),
        max_chars,
    )
}

fn compact_visible_text(value: &str, max_chars: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let mut chars = value.chars();
    let mut text = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        text.push('…');
    }
    Some(text)
}

fn parse_usage(value: &Value) -> CodexHarnessUsage {
    CodexHarnessUsage {
        input: value
            .get("inputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cached_input: value
            .get("cachedInputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output: value
            .get("outputTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

fn compact_json(value: &Value) -> String {
    let text = value.to_string();
    text.chars().take(500).collect()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

fn configure_model_connection(
    command: &mut Command,
    request: &CodexHarnessRequest,
    catalog: Option<&std::path::Path>,
) -> Result<(), String> {
    let provider = &request.model_provider;
    command
        .args(["-c", "cli_auth_credentials_store=\"file\""])
        .args(["-c", "features.secret_auth_storage=false"])
        .args(["-c", "features.system_proxy_fallback=false"])
        .args([
            "-c",
            &format!(
                "model_provider={}",
                serde_json::to_string(provider).map_err(|_| "model_provider_invalid")?
            ),
        ])
        .args([
            "-c",
            &format!("model_providers.{provider}.request_max_retries=0"),
        ])
        .args([
            "-c",
            &format!("model_providers.{provider}.stream_max_retries=0"),
        ]);
    command.args([
        "-c",
        &format!("model_providers.{provider}.supports_websockets=false"),
    ]);
    if provider == crate::model_connections::CHATGPT_PROVIDER {
        command
            .args(["-c", &format!("model_providers.{provider}.name=\"OpenAI\"")])
            .args([
                "-c",
                &format!("model_providers.{provider}.requires_openai_auth=true"),
            ])
            .args([
                "-c",
                &format!("model_providers.{provider}.wire_api=\"responses\""),
            ]);
    } else {
        for (key, value) in [
            ("name", &request.provider_label),
            ("base_url", &request.base_url),
            ("env_key", &request.api_key_env),
        ] {
            command.args([
                "-c",
                &format!(
                    "model_providers.{provider}.{key}={}",
                    serde_json::to_string(value).map_err(|_| "model_provider_invalid")?
                ),
            ]);
        }
        command.args([
            "-c",
            &format!("model_providers.{provider}.wire_api=\"responses\""),
        ]);
        let catalog = catalog.ok_or("model_catalog_required")?;
        command.args([
            "-c",
            &format!(
                "model_catalog_json={}",
                serde_json::to_string(&catalog.to_string_lossy())
                    .map_err(|_| "model_catalog_invalid")?
            ),
        ]);
    }
    Ok(())
}

fn configure_validation_probe(
    command: &mut Command,
    request: &CodexHarnessRequest,
) -> Result<(), String> {
    if !request.validation_probe {
        return Ok(());
    }
    if request.memory_preferences.enabled
        || request.approval_mode != "request"
        || request.sandbox_mode != "read-only"
        || !request.skills.is_empty()
        || !request.attachments.is_empty()
        || request.personal_tools.is_some()
        || request.user_mcp.is_some()
        || request.application_action.is_some()
        || request.application_write.is_some()
        || request.application_adapter.is_some()
        || request.classification.is_some()
        || request.report_result.is_some()
        || request.persistent_thread.is_some()
        || request.resume_thread_id.is_some()
        || request.planning
    {
        return Err("model_validation_scope_invalid".into());
    }
    for feature in [
        "shell_tool",
        "view_image",
        "unified_exec",
        "unified_exec_tty",
        "shell_snapshot",
        "shell_snapshot_v2",
        "js_repl",
        "apply_patch_freeform",
        "multi_agent",
        "multi_agent_v2",
        "image_generation",
        "skill_mcp_dependency_install",
        "skill_search",
        "tool_search",
        "executor_capability_discovery",
    ] {
        command.args(["-c", &format!("features.{feature}=false")]);
    }
    command.args(["-c", "skills.bundled.enabled=false"]);
    configure_scoped_page_tools(command);
    Ok(())
}

const PAGE_DISABLED_NATIVE_FEATURES: &[&str] = &[
    "shell_tool",
    "view_image",
    "unified_exec",
    "unified_exec_tty",
    "shell_snapshot",
    "shell_snapshot_v2",
    "js_repl",
    "code_mode",
    "code_mode_only",
    "apply_patch_freeform",
    "multi_agent",
    "multi_agent_v2",
    "image_generation",
    "request_permissions_tool",
    "goals",
    "skill_mcp_dependency_install",
    "skill_search",
    "executor_capability_discovery",
    "stable_environment_tools",
];

fn configure_scoped_page_tools(command: &mut Command) {
    for feature in PAGE_DISABLED_NATIVE_FEATURES {
        command.args(["-c", &format!("features.{feature}=false")]);
    }
    command
        .args(["-c", "tools.experimental_request_user_input.enabled=false"])
        .args(["-c", "tools.update_plan.enabled=false"]);
}

fn validate_scoped_page_tools(response: &Value) -> Result<(), String> {
    let config = &response["result"]["config"];
    if let Some(feature) = PAGE_DISABLED_NATIVE_FEATURES
        .iter()
        .find(|feature| config["features"][**feature] != false)
    {
        eprintln!("personal_page_native_tools_not_isolated feature={feature}");
        return Err("personal_page_native_tools_not_isolated".into());
    }
    let layers = response["result"]["layers"]
        .as_array()
        .ok_or("personal_page_native_tools_not_isolated")?;
    for tool in ["experimental_request_user_input", "update_plan"] {
        let configured = layers
            .iter()
            .filter(|layer| layer["disabledReason"].is_null())
            .find_map(|layer| layer["config"]["tools"][tool].get("enabled"));
        if configured != Some(&Value::Bool(false)) {
            return Err("personal_page_native_tools_not_isolated".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn protocol_audit_extracts_only_the_controlled_service_marker() {
        let marker = "[CILAI_MODEL_SERVICE_ERROR_V1 code=MODEL_REQUEST_REJECTED upstream_status=400 parameter=tools service_code=unsupported_parameter]";
        for message in [
            serde_json::json!({"params":{"turn":{"error":{"message":marker}}}}),
            serde_json::json!({"params":{"error":{"message":marker}}}),
            serde_json::json!({"params":{"message":marker}}),
            serde_json::json!({"error":{"message":marker}}),
        ] {
            let diagnostic = super::safe_service_error(&message).unwrap();
            assert_eq!(diagnostic.upstream_status, 400);
            assert_eq!(diagnostic.parameter, "tools");
            assert_eq!(diagnostic.code, "unsupported_parameter");
        }
        let message = serde_json::json!({"error":{"message":"Authorization: Bearer PRIVATE_SECRET","code":"PRIVATE_SECRET","serviceError":{"upstreamStatus":422,"parameter":"tools"}}});
        assert!(super::safe_service_error(&message).is_none());
        assert!(
            super::safe_service_error(&serde_json::json!({"params":{"input":marker}})).is_none()
        );
    }

    #[test]
    fn web_search_activity_is_retained_for_memory_source_support() {
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = events.clone();
        let callback: super::ProgressCallback =
            std::sync::Arc::new(move |event| sink.lock().unwrap().push(event));
        super::handle_notification(
            &serde_json::json!({"method":"item/completed","params":{"item":{"type":"webSearch","id":"search-1","status":"completed"}}}),
            &callback,
            "reply",
        );
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "web_search");
        assert!(events[0].detail_available);
        assert_eq!(events[0].item_id.as_deref(), Some("search-1"));
    }
    use super::{
        ADDITIONAL_CONTEXT_PART_BYTES, ADDITIONAL_CONTEXT_TOTAL_BYTES, ApplicationActionCallback,
        ApplicationMcpScope, ApplicationReadToolScope, ApplicationWriteCallback,
        ApplicationWriteToolScope, ArtifactCallback, COMPACT_PROMPT, CodexHarnessAttachment,
        CodexHarnessClassificationRequest, CodexHarnessFileChange, CodexHarnessModel,
        CodexHarnessRequest, CodexHarnessSkill, CodexHarnessUsage, MCP_TOOL_APPROVAL_METHOD,
        ModelToolCapabilities, ProgressCallback, application_tool_search_metadata,
        application_write_model_schema, application_write_tool, application_write_tool_namespace,
        approval_result, approval_settings, compact_prompt_config, configure_application_adapter,
        estimate_context_composition, estimate_tokens, handle_application_action_request,
        handle_application_write_request, handle_artifact_tool_request,
        is_tool_output_too_large_response, item_summary, mcp_tools_approval_mode,
        parse_approval_request, parse_usage, plan_summary, register_artifact_tool,
        run_protocol_for_os, send, turn_failure_reason, write_model_catalog,
    };
    use contexthub_platform_adapter::OperatingSystem;
    use serde_json::Value;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    struct OfficialProtocolProbe {
        child: std::process::Child,
        input: std::process::ChildStdin,
        messages: std::sync::mpsc::Receiver<Value>,
        next_id: u64,
    }

    impl OfficialProtocolProbe {
        fn start(command: &mut std::process::Command) -> Self {
            use std::io::BufRead as _;
            let mut child = command
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let input = child.stdin.take().unwrap();
            let output = child.stdout.take().unwrap();
            let (sender, messages) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(output).lines() {
                    let Ok(line) = line else { break };
                    let Ok(value) = serde_json::from_str(&line) else {
                        break;
                    };
                    if sender.send(value).is_err() {
                        break;
                    }
                }
            });
            Self {
                child,
                input,
                messages,
                next_id: 1,
            }
        }

        fn request(&mut self, method: &str, params: Value) -> Value {
            use std::io::Write as _;
            let id = self.next_id;
            self.next_id += 1;
            let mut message = json!({"id":id,"method":method});
            message["params"] = params;
            writeln!(self.input, "{message}").unwrap();
            self.input.flush().unwrap();
            loop {
                let response = self
                    .messages
                    .recv_timeout(std::time::Duration::from_secs(20))
                    .unwrap();
                if response["id"] == id {
                    assert!(response.get("error").is_none(), "{method}: {response}");
                    return response["result"].clone();
                }
            }
        }

        fn initialized(&mut self) {
            use std::io::Write as _;
            writeln!(
                self.input,
                "{}",
                json!({"method":"initialized","params":{}})
            )
            .unwrap();
            self.input.flush().unwrap();
        }
    }

    impl Drop for OfficialProtocolProbe {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn verify_official_local_mcp(request: &mut CodexHarnessRequest) {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let allowed = Arc::new(AtomicBool::new(true));
        let check = Arc::clone(&allowed);
        let validate: super::ScopeValidator = Arc::new(move || {
            check
                .load(Ordering::SeqCst)
                .then_some(())
                .ok_or_else(|| "probe_scope_revoked".into())
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = Arc::clone(&calls);
        let original_callback = request.personal_tools.replace(Arc::new(move |arguments| {
            captured.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"fixtureAction":arguments["action"]}))
        }));
        let slot = super::AdapterSlot::default();
        let proxy =
            crate::model_session_proxy::local_tools_test_proxy(&request.api_key, slot.clone());
        let original_adapter = request.application_adapter.replace((
            proxy.application_mcp_url.clone(),
            slot.clone(),
            validate,
        ));
        let progress: ProgressCallback = Arc::new(|_| {});
        super::prepare_persistent_application_adapter(request, &progress).unwrap();
        let home = request.harness_home.join("official-mcp-probe");
        std::fs::create_dir_all(&home).unwrap();
        let mut command =
            std::process::Command::new(crate::supervisor::codex_harness_test_binary());
        command.env_clear();
        contexthub_platform_adapter::process::retain_required_system_environment(&mut command)
            .unwrap();
        command
            .current_dir(&home)
            .env("HOME", &home)
            .env("CODEX_HOME", &home)
            .env("CODEX_SQLITE_HOME", &home)
            .env(crate::windows_sandbox::REMOTE_CONTROL_DISABLED_ENV, "1")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "127.0.0.1,localhost")
            .args([
                "--listen",
                "stdio://",
                "-c",
                "check_for_update_on_startup=false",
                "-c",
                "features.plugins=false",
                "-c",
                "features.memories=false",
                "-c",
                "memories.generate_memories=false",
                "-c",
                "memories.use_memories=false",
                "-c",
                "cli_auth_credentials_store=\"file\"",
                "-c",
                "model_provider=\"isolated_mcp_probe\"",
                "-c",
                "model_providers.isolated_mcp_probe.name=\"Local protocol probe\"",
                "-c",
                "model_providers.isolated_mcp_probe.base_url=\"http://127.0.0.1:1/v1\"",
                "-c",
                "model_providers.isolated_mcp_probe.wire_api=\"responses\"",
                "-c",
                "model_providers.isolated_mcp_probe.requires_openai_auth=false",
            ]);
        super::configure_runtime_mcp(&mut command, request, None).unwrap();
        let mut runtime = OfficialProtocolProbe::start(&mut command);
        let initialized = runtime.request("initialize", json!({"clientInfo":{"name":"cilai-personal-mcp-audit","version":"1"},"capabilities":{"experimentalApi":true}}));
        runtime.initialized();
        let catalog = runtime.request("mcpServerStatus/list", json!({"limit":100}));
        let servers = catalog["data"].as_array().unwrap();
        assert_eq!(servers.len(), 1, "{catalog}");
        assert_eq!(servers[0]["name"], "cilai_application");
        let tools = servers[0]["tools"].as_object().unwrap();
        assert_eq!(tools.len(), 1, "{catalog}");
        let name = tools.keys().next().unwrap();
        let thread = runtime.request("thread/start", json!({"cwd":home,"model":"local-probe-model","modelProvider":"isolated_mcp_probe","approvalPolicy":"never","sandbox":"read-only"}));
        let call_params = json!({"threadId":thread["thread"]["id"],"server":"cilai_application","tool":name,"arguments":{"action":"list"}});
        let called = runtime.request("mcpServer/tool/call", call_params.clone());
        assert_eq!(
            called["structuredContent"]["fixtureAction"], "list",
            "{called}"
        );
        allowed.store(false, Ordering::SeqCst);
        let revoked = runtime.request("mcpServer/tool/call", call_params);
        assert_eq!(revoked["isError"], true, "{revoked}");
        assert!(revoked.to_string().contains("probe_scope_revoked"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        eprintln!(
            "official_personal_mcp_probe {}",
            json!({"initialize":initialized,"catalog":catalog,"called":called,"revoked":revoked,"revocationVerified":true,"realModelCall":false,"toolCallback":"explicit-local-fixture"})
        );
        drop(runtime);
        request.application_adapter = original_adapter;
        request.personal_tools = original_callback;
        drop(proxy);
    }

    #[test]
    fn skill_discovery_exposes_only_authorized_packages() {
        let root = std::env::temp_dir().join("cilai-skill-scope");
        let skill = |name: &str| CodexHarnessSkill {
            skill_id: name.to_owned(),
            name: name.to_owned(),
            path: root.join(name).join("SKILL.md"),
            explicit: false,
        };
        let first = skill("local-creator");
        let second = skill("user-skill");
        let roots = super::authorized_skill_roots(&[first.clone(), second, first]).unwrap();
        assert_eq!(
            roots,
            vec![
                root.join("local-creator").to_str().unwrap().to_owned(),
                root.join("user-skill").to_str().unwrap().to_owned(),
            ]
        );
        assert!(!roots.contains(&root.to_str().unwrap().to_owned()));
        assert!(super::authorized_skill_roots(&[]).unwrap().is_empty());
        let mut invalid = skill("invalid");
        for path in [
            PathBuf::from("relative/SKILL.md"),
            root.join("SKILL.txt"),
            root.clone(),
        ] {
            invalid.path = path;
            assert_eq!(
                super::authorized_skill_roots(&[invalid.clone()]).unwrap_err(),
                "codex_harness_skill_root_invalid"
            );
        }
    }

    #[test]
    fn only_the_host_loopback_adapter_delegates_mcp_approval_to_business_confirmation() {
        let mut command = std::process::Command::new("unused");
        configure_application_adapter(&mut command, "http://127.0.0.1:1234/mcp").unwrap();
        let args = command
            .get_args()
            .map(|v| v.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.contains(
            &"mcp_servers.cilai_application.default_tools_approval_mode=\"approve\"".to_owned()
        ));
        assert!(
            args.iter()
                .filter(|v| *v != "-c")
                .all(|v| v.starts_with("mcp_servers.cilai_application."))
        );
        for endpoint in [
            "https://enterprise.example/mcp",
            "http://localhost:1234/mcp",
            "https://127.0.0.1:1234/mcp",
            "http://user@127.0.0.1:1234/mcp",
            "http://127.0.0.1:1234/mcp?scope=other",
            "http://127.0.0.1:1234/mcp#other",
        ] {
            let mut rejected = std::process::Command::new("unused");
            assert_eq!(
                configure_application_adapter(&mut rejected, endpoint).unwrap_err(),
                "application_adapter_endpoint_invalid"
            );
            assert_eq!(rejected.get_args().count(), 0);
        }
    }

    #[test]
    fn closing_shared_control_delivers_eof_without_signalling_child() {
        use std::io::Read as _;
        let mut child = std::process::Command::new("node")
            .args(["-e", "let n=0;process.stdin.on('data',d=>n+=d.length);process.stdin.on('end',()=>process.stdout.write(String(n)))"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn().unwrap();
        let control = super::CodexHarnessControl::new(child.stdin.take().unwrap());
        let clone = control.clone();
        control.send(&json!({"method":"test"})).unwrap();
        clone.close_input().unwrap();
        control.close_input().unwrap();
        assert_eq!(
            control.send(&json!({})),
            Err("codex_harness_stdin_closed".to_owned())
        );
        let mut output = String::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert!(output.parse::<usize>().unwrap() > 0);
    }

    #[test]
    fn read_receipts_require_successful_completed_structured_read_actions() {
        let path = std::env::temp_dir().join(format!("read-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, "material").unwrap();
        let mut item = serde_json::json!({"type":"commandExecution","status":"completed","exitCode":0,"commandActions":[{"type":"read","path":path}]});
        assert!(super::completed_read_paths("item/started", &item).is_empty());
        assert_eq!(
            super::completed_read_paths("item/completed", &item),
            vec![path.canonicalize().unwrap().to_string_lossy().into_owned()]
        );
        item["exitCode"] = serde_json::json!(1);
        assert!(super::completed_read_paths("item/completed", &item).is_empty());
        item["exitCode"] = serde_json::json!(0);
        item["commandActions"] = serde_json::json!([]);
        item["summary"] = serde_json::json!("I read all files");
        assert!(super::completed_read_paths("item/completed", &item).is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn application_page_protocol_keeps_twenty_rounds_and_never_exposes_workbench_tools() {
        let _guard = crate::supervisor::app_server_test_guard();
        let root = std::env::temp_dir().join(format!("page-protocol-{}", uuid::Uuid::new_v4()));
        let data = root.join("test");
        let server = crate::supervisor::AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap();
        let mut request = CodexHarnessRequest {
            turn_admission: None,
            validation_probe: false,
            execution: None,
            memory_model: Some("model-A".into()),
            conversation_kind: crate::conversation_kernel::ConversationKind::ApplicationPage,
            planning: false,
            executable: "unused".to_owned(),
            harness_home: root.clone(),
            tool_snapshot_scope_id: "r1".to_owned(),
            conversation_id: Some("c".to_owned()),
            workspace_root: root.clone(),
            workspace_roots: vec![root.clone()],
            model: "model-A".to_owned(),
            model_provider: "probe".to_owned(),
            provider_label: "Probe".to_owned(),
            base_url: "http://127.0.0.1/v1".to_owned(),
            api_key_env: "PROBE_KEY".to_owned(),
            api_key: "unused".to_owned(),
            context_window: Some(10000),
            reasoning_effort: None,
            model_catalog: vec![],
            developer_instructions: "rules".to_owned(),
            classification: None,
            application_result: None,
            report_result: None,
            user_input: "first".to_owned(),
            turn_context: String::new(),
            conversation_text: String::new(),
            attachments: vec![],
            skills: vec![],
            user_mcp: None,
            personal_tools: None,
            application_action: Some(Arc::new(|_, _| Ok(json!({})))),
            application_write: None,
            application_mcp: Some(ApplicationMcpScope {
                application_id: "app".into(),
                application_name: "应用".into(),
                application_ids: vec!["app".into()],
                restrict_to_applications: true,
                browser_origins: vec![],
                report_review: false,
                tool_names: vec![],
                read_tools: vec![],
                write_tools: vec![],
            }),
            resume_thread_id: None,
            persistent_thread: Some(crate::persistent_threads::PersistentThread {
                journal: crate::persistent_threads::ThreadJournal {
                    writer_process: Value::Null,
                    runtime_scope: None,
                    model_execution_lease_id: None,
                    history_consent_confirmed: false,
                    application_scope: Some(contexthub_protocol::AiThreadApplicationScope {
                        application_id: "app".into(),
                        session_kind:
                            contexthub_protocol::AiApplicationSessionKind::ApplicationPage,
                    }),
                    server: server.clone(),
                    conversation_id: "application:page".to_owned(),
                    run_id: "r1".to_owned(),
                    owner_instance_id: crate::persistent_threads::owner_instance_id(),
                },
                native_thread_id: None,
                fork_before_turn_id: None,
                workbench_history_fork: false,
                cross_runtime_fork: false,
                workbench_history: Vec::new(),
                name: "测试应用对话".into(),
                history: vec![],
                message_id: "m1".to_owned(),
                client_message_id: "m1".to_owned(),
                content_digest: "hash1".to_owned(),
                writer_process: Value::Null,
            }),
            application_adapter: Some((
                "http://127.0.0.1/adapter".into(),
                super::AdapterSlot::default(),
                Arc::new(|| Ok(())),
            )),
            approval_mode: "legacy".to_owned(),
            approval_review_model: Some("model-A".to_owned()),
            memory_preferences: crate::memory::MemoryPreferences::disabled(),
            sandbox_mode: "read-only".to_owned(),
            event_log_path: root.join("events"),
            stderr_path: root.join("stderr"),
        };

        {
            let scope = request.application_mcp.as_mut().unwrap();
            scope.restrict_to_applications = false;
            scope.application_ids.push("authorized-other-app".into());
        }
        assert!(request.conversation_kind.validate(&request).is_err());
        request.application_mcp.as_mut().unwrap().report_review = true;
        assert!(request.conversation_kind.validate(&request).is_err());
        request.application_action = None;
        assert!(request.conversation_kind.validate(&request).is_ok());
        assert!(
            crate::conversation_kernel::ConversationKind::ApplicationTask
                .validate(&request)
                .is_err()
        );
        request.application_mcp.as_mut().unwrap().report_review = false;
        request
            .application_mcp
            .as_mut()
            .unwrap()
            .restrict_to_applications = true;
        request.application_mcp.as_mut().unwrap().application_ids = vec!["app".into()];
        request.application_action = Some(Arc::new(|_, _| Ok(json!({}))));

        for round in 1..=20 {
            let id = format!("m{round}");
            let turn = format!("t{round}");
            let plan = request.persistent_thread.as_mut().unwrap();
            plan.journal.run_id = format!("app_run_{round}");
            plan.message_id.clone_from(&id);
            plan.client_message_id = id.clone();
            plan.content_digest = format!("digest{round}");
            plan.native_thread_id.clone_from(&request.resume_thread_id);
            request.user_input = format!("追问{round}");
            request.turn_context = format!("当前页面修订{round}");
            request.model = if round < 10 { "model-A" } else { "model-B" }.into();
            let responses = [
                json!({"id":1,"result":{}}),
                json!({"id":11,"result":{"effectiveEnabled":false}}),
                json!({"id":9,"result":{}}),
                json!({"id":2,"result":{"thread":{"id":"page-thread"}}}),
                json!({"id":13,"result":{}}),
                json!({"id":7,"result":{"data":[]}}),
                json!({"id":3,"result":{"turn":{"id":turn}}}),
                json!({"method":"item/completed","params":{"threadId":"page-thread","turnId":turn,"item":{"id":"reply","type":"agentMessage","phase":"final_answer","text":"完成"}}}),
                json!({"method":"turn/completed","params":{"threadId":"page-thread","turn":{"id":turn,"status":"completed","items":[]}}}),
            ];
            let wire = responses
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            let mut sent = Vec::new();
            let progress: ProgressCallback = Arc::new(|_| {});
            let result = run_protocol_for_os(
                &request,
                |message| {
                    sent.push(message.clone());
                    Ok(())
                },
                Cursor::new(wire),
                File::create(root.join("wire")).unwrap(),
                &progress,
                None,
                None,
                None,
                OperatingSystem::MacOs,
            )
            .unwrap();
            assert_eq!(result.thread_id, "page-thread");
            let thread = sent
                .iter()
                .find(|m| {
                    m["method"]
                        == if round == 1 {
                            "thread/start"
                        } else {
                            "thread/resume"
                        }
                })
                .unwrap();
            assert_eq!(thread["params"]["developerInstructions"], "rules");
            if round == 1 {
                let definitions = thread["params"]["dynamicTools"].to_string();
                assert!(definitions.contains("cilai_application_action"));
                assert!(!definitions.contains("cilai_register_artifact"));
                assert!(!definitions.contains("cilai_personal_tools"));
            } else {
                assert_eq!(thread["params"]["threadId"], "page-thread");
            }
            let start = sent.iter().find(|m| m["method"] == "turn/start").unwrap();
            assert_eq!(start["params"]["input"][0]["text"], request.user_input);
            assert_eq!(
                start["params"]["additionalContext"]["cilai_turn_materials"]["value"],
                request.turn_context
            );
            assert_eq!(start["params"]["clientUserMessageId"], id);
            request
                .persistent_thread
                .as_ref()
                .unwrap()
                .journal
                .release()
                .unwrap();
            request.resume_thread_id = Some("page-thread".into());
        }
        let journal = &request.persistent_thread.as_ref().unwrap().journal;
        assert_eq!(
            journal.state().unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            20
        );
        request.turn_context = "a".repeat(ADDITIONAL_CONTEXT_TOTAL_BYTES + 1);
        let mut rejected_requests = Vec::new();
        let rejected_progress: ProgressCallback = Arc::new(|_| {});
        assert_eq!(
            run_protocol_for_os(
                &request,
                |message| {
                    rejected_requests.push(message.clone());
                    Ok(())
                },
                Cursor::new(Vec::<u8>::new()),
                File::create(root.join("oversized.jsonl")).unwrap(),
                &rejected_progress,
                None,
                None,
                None,
                OperatingSystem::MacOs,
            )
            .unwrap_err(),
            "ai_additional_context_too_large"
        );
        assert!(rejected_requests.is_empty());
        request.memory_preferences = crate::memory::MemoryPreferences {
            enabled: true,
            from_tool_chats: false,
        };
        assert_eq!(
            request.conversation_kind.validate(&request).unwrap_err(),
            "application_execution_scope_invalid"
        );
        request.memory_preferences = crate::memory::MemoryPreferences::disabled();
        request.conversation_kind = crate::conversation_kernel::ConversationKind::ApplicationTask;
        assert_eq!(
            request.conversation_kind.validate(&request).unwrap_err(),
            "application_task_scope_invalid"
        );
        server.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn persistent_protocol_scopes_foreground_rules_through_create_resume_and_fork() {
        let _guard = crate::supervisor::app_server_test_guard();
        let root = PathBuf::from("/tmp").join(format!("ch-journal-{}", uuid::Uuid::new_v4()));
        let data = root.join("test");
        let server = crate::supervisor::AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap_or_else(|error| panic!("{error}: {:?}", server.status()));
        server.request_result("aiState/replace",&json!({"expectedRevision":0,"state":{"conversations":[{"conversationId":"c"}],"messages":[],"runs":[{"runId":"r1","conversationId":"c"},{"runId":"r2","conversationId":"c"},{"runId":"r3","conversationId":"c"}]}})).unwrap();
        let mut request = CodexHarnessRequest {
            turn_admission: None,
            validation_probe: false,
            execution: None,
            memory_model: Some("model-A".into()),
            conversation_kind: crate::conversation_kernel::ConversationKind::Workbench,
            planning: false,
            executable: "unused".to_owned(),
            harness_home: root.clone(),
            tool_snapshot_scope_id: "r1".to_owned(),
            conversation_id: Some("c".to_owned()),
            workspace_root: root.clone(),
            workspace_roots: vec![root.clone()],
            model: "model-A".to_owned(),
            model_provider: "probe".to_owned(),
            provider_label: "Probe".to_owned(),
            base_url: "http://127.0.0.1/v1".to_owned(),
            api_key_env: "PROBE_KEY".to_owned(),
            api_key: "unused".to_owned(),
            context_window: Some(10000),
            reasoning_effort: None,
            model_catalog: vec![],
            developer_instructions: "rules".to_owned(),
            classification: None,
            application_result: None,
            report_result: None,
            user_input: "first".to_owned(),
            turn_context: String::new(),
            conversation_text: String::new(),
            attachments: vec![],
            skills: vec![],
            user_mcp: None,
            personal_tools: None,
            application_action: None,
            application_write: None,
            application_mcp: None,
            resume_thread_id: None,
            persistent_thread: Some(crate::persistent_threads::PersistentThread {
                journal: crate::persistent_threads::ThreadJournal {
                    writer_process: Value::Null,
                    runtime_scope: None,
                    model_execution_lease_id: None,
                    history_consent_confirmed: false,
                    application_scope: None,
                    server: server.clone(),
                    conversation_id: "c".to_owned(),
                    run_id: "r1".to_owned(),
                    owner_instance_id: crate::persistent_threads::owner_instance_id(),
                },
                native_thread_id: None,
                fork_before_turn_id: None,
                workbench_history_fork: false,
                cross_runtime_fork: false,
                workbench_history: Vec::new(),
                name: "测试持续对话".into(),
                history: vec![
                    json!({"type":"message","role":"user","content":[{"type":"input_text","text":"old-fact"}]}),
                ],
                message_id: "m1".to_owned(),
                client_message_id: "m1".to_owned(),
                content_digest: "hash1".to_owned(),
                writer_process: Value::Null,
            }),
            application_adapter: None,
            approval_mode: "legacy".to_owned(),
            approval_review_model: Some("model-A".to_owned()),
            memory_preferences: crate::memory::MemoryPreferences::disabled(),
            sandbox_mode: "read-only".to_owned(),
            event_log_path: root.join("events"),
            stderr_path: root.join("stderr"),
        };
        for round in 1..=3 {
            let turn = format!("turn-{round}");
            let native_thread = if round == 3 {
                "native-forked"
            } else {
                "native-current"
            };
            let mut responses = vec![
                json!({"id":1,"result":{}}),
                json!({"id":11,"result":{"effectiveEnabled":false}}),
                json!({"id":9,"result":{}}),
                json!({"id":2,"result":{"thread":{"id":native_thread}}}),
            ];
            if round == 1 {
                responses.push(json!({"id":13,"result":{}}));
                responses.push(json!({"id":20,"result":{}}));
            }
            responses.extend([json!({"id":7,"result":{"data":[]}}),json!({"id":3,"result":{"turn":{"id":turn}}}),json!({"method":"item/completed","params":{"threadId":native_thread,"turnId":turn,"item":{"id":"reply","type":"agentMessage","phase":"final_answer","text":"done"}}}),json!({"method":"turn/completed","params":{"threadId":native_thread,"turn":{"id":turn,"status":"completed","items":[]}}})]);
            let wire = responses
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            let mut sent = vec![];
            let progress: ProgressCallback = Arc::new(|_| {});
            let result = run_protocol_for_os(
                &request,
                |value| {
                    sent.push(value.clone());
                    Ok(())
                },
                Cursor::new(wire),
                File::create(root.join(format!("protocol-{round}"))).unwrap(),
                &progress,
                None,
                None,
                None,
                OperatingSystem::MacOs,
            )
            .unwrap();
            assert_eq!(result.thread_id, native_thread);
            let start = sent
                .iter()
                .find(|item| item["method"] == "turn/start")
                .unwrap();
            assert_eq!(start["params"]["input"].as_array().unwrap().len(), 1);
            assert_eq!(start["params"]["input"][0]["text"], request.user_input);
            if request.turn_context.is_empty() {
                assert!(start["params"]["additionalContext"].is_null());
            } else {
                assert_eq!(
                    start["params"]["additionalContext"]["cilai_turn_materials"],
                    json!({ "value": request.turn_context, "kind": "application" })
                );
                assert!(
                    start["params"]["additionalContext"]["cilai_response_contract"]["value"]
                        .as_str()
                        .unwrap()
                        .contains("workbench_response_contract")
                );
            }
            assert_eq!(start["params"]["clientUserMessageId"], format!("m{round}"));
            assert_eq!(
                sent.iter()
                    .filter(|item| item["method"] == "thread/inject_items")
                    .count(),
                usize::from(round == 1)
            );
            let method = match round {
                1 => "thread/start",
                2 => "thread/resume",
                _ => "thread/fork",
            };
            let thread_request = sent.iter().find(|item| item["method"] == method).unwrap();
            assert_eq!(thread_request["params"]["sandbox"], "read-only");
            let developer = thread_request["params"]["developerInstructions"]
                .as_str()
                .unwrap();
            let scoped = developer
                .split_once("<cilai_foreground_context>\n")
                .unwrap()
                .1;
            let body = scoped
                .strip_suffix("\n</cilai_foreground_context>")
                .unwrap();
            assert!(developer.contains("不适用于 Codex 官方运行时发起的内部后台记忆整理任务"));
            assert!(body.starts_with("rules"));
            if round > 1 {
                assert!(body.contains("归类说明"));
                assert!(body.contains("cilai_submit_workbench_result"));
                let schema_text = body
                    .rsplit_once("\n```json\n")
                    .unwrap()
                    .1
                    .strip_suffix("\n```")
                    .unwrap();
                let schema: Value = serde_json::from_str(schema_text).unwrap();
                assert_eq!(
                    schema,
                    request.classification.as_ref().unwrap().output_schema
                );
            }
            if round == 3 {
                assert_eq!(thread_request["params"]["threadId"], "native-current");
                assert_eq!(thread_request["params"]["beforeTurnId"], "turn-1");
            }
            let plan = request.persistent_thread.as_mut().unwrap();
            let state = plan.journal.state().unwrap();
            assert_eq!(state["binding"]["nativeThreadId"], native_thread);
            assert_eq!(state["messages"].as_array().unwrap().len(), round);
            plan.journal.release().unwrap();
            assert_eq!(plan.journal.state().unwrap()["binding"]["status"], "ready");
            plan.journal.run_id = format!("r{}", round + 1);
            plan.native_thread_id = Some("native-current".to_owned());
            plan.fork_before_turn_id = (round == 2).then(|| "turn-1".to_owned());
            plan.history.clear();
            plan.message_id = format!("m{}", round + 1);
            plan.client_message_id = format!("m{}", round + 1);
            request.resume_thread_id = Some("native-current".to_owned());
            request.model = "model-B".to_owned();
            request.user_input = "second".to_owned();
            request.turn_context = "本次消息引用的文件：材料.md".to_owned();
            request.classification = Some(CodexHarnessClassificationRequest {
                developer_instructions: "归类说明".to_owned(),
                output_schema: json!({"type":"object"}),
                validate: Arc::new(|_| Ok(())),
            });
        }
        server.shutdown().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn token_usage_uses_the_latest_turn_breakdown() {
        assert_eq!(
            parse_usage(&json!({
                "inputTokens": 120,
                "cachedInputTokens": 40,
                "outputTokens": 30,
                "totalTokens": 150,
                "reasoningOutputTokens": 0
            })),
            CodexHarnessUsage {
                input: 120,
                cached_input: 40,
                output: 30,
            }
        );
    }

    #[test]
    fn context_composition_keeps_the_reported_total_as_the_boundary() {
        let status = json!({
            "result": {"data": [{"tools": {"read": {
                "description": "读取内容",
                "inputSchema": {"type": "object"}
            }}}]}
        });
        let composition = estimate_context_composition(
            1_000,
            "用户：分析项目",
            "已完成分析",
            "系统规则",
            "工作台规则",
            &register_artifact_tool(),
            Some(&status),
            None,
            Some(0),
        );
        assert!(estimate_tokens("分析 project context") > 0);
        assert!(composition.conversation_tokens > 0);
        assert!(composition.system_prompt_tokens > 0);
        assert!(composition.tool_tokens > 0);
        assert!(composition.mcp_tokens.unwrap() > 0);
        assert_eq!(composition.skill_tokens, 0);
        assert_eq!(
            composition.conversation_tokens
                + composition.system_prompt_tokens
                + composition.tool_tokens
                + composition.skill_tokens
                + composition.mcp_tokens.unwrap()
                + composition.internal_tokens,
            1_000
        );
    }

    #[test]
    fn deferred_application_tools_are_not_counted_as_initial_mcp_context() {
        let status = json!({
            "result": {"data": [
                {"name": "cilai_application", "tools": {"search_archive": {
                    "description": "搜索环保档案",
                    "inputSchema": {"type": "object"}
                }}},
                {"name": "cilai_collaboration", "tools": {"list_tasks": {
                    "description": "查看任务",
                    "inputSchema": {"type": "object"}
                }}}
            ]}
        });
        let direct_only = super::mcp_tools_text(Some(&status), Some("cilai_application")).unwrap();
        assert!(!direct_only.contains("search_archive"));
        assert!(direct_only.contains("list_tasks"));
    }

    #[test]
    fn discovered_application_tools_are_counted_after_search() {
        let composition = estimate_context_composition(
            1_000,
            "用户：搜索档案",
            "搜索完成",
            "系统规则",
            "工作台规则",
            &register_artifact_tool(),
            None,
            Some("cilai_application"),
            Some(48),
        );
        assert_eq!(composition.mcp_tokens, Some(48));
    }

    #[test]
    fn resumed_threads_mark_application_tool_definitions_as_unmetered() {
        // 恢复线程时运行时不发原始事件，本轮检索到的和历史里继承的工具定义都看不到；
        // 只算开局可见的那部分会显得像是准确值，所以整项标为未计量，计入内部部分。
        let status = json!({
            "result": {"data": [{"name": "cilai_collaboration", "tools": {"list_tasks": {
                "description": "查看任务",
                "inputSchema": {"type": "object"}
            }}}]}
        });
        let composition = estimate_context_composition(
            1_000,
            "用户：再查一次",
            "查询完成",
            "系统规则",
            "工作台规则",
            &register_artifact_tool(),
            Some(&status),
            Some("cilai_application"),
            None,
        );
        assert_eq!(composition.mcp_tokens, None);
        assert_eq!(
            composition.conversation_tokens
                + composition.system_prompt_tokens
                + composition.tool_tokens
                + composition.skill_tokens
                + composition.internal_tokens,
            1_000
        );
    }

    #[test]
    fn raw_tool_search_items_produce_visible_progress_without_exposing_schema() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let received = Arc::clone(&events);
        let progress: ProgressCallback =
            Arc::new(move |event| received.lock().unwrap().push(event));
        super::handle_notification(
            &json!({
                "method": "rawResponseItem/completed",
                "params": {"item": {
                    "type": "tool_search_call",
                    "call_id": "search-1",
                    "execution": "client",
                    "arguments": {"query": "environmental archive"}
                }}
            }),
            &progress,
            "reply",
        );
        super::handle_notification(
            &json!({
                "method": "rawResponseItem/completed",
                "params": {"item": {
                    "type": "tool_search_output",
                    "call_id": "search-1",
                    "status": "completed",
                    "execution": "client",
                    "tools": [{"name": "search_archive", "parameters": {"secret": true}}]
                }}
            }),
            &progress,
            "reply",
        );
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, "tool_search.started");
        assert_eq!(events[0].summary, "正在查找企业工具");
        assert_eq!(events[1].event_type, "tool_search.completed");
        assert_eq!(events[1].summary, "已发现 1 个企业工具");
        assert!(!events[1].summary.contains("secret"));
    }

    #[test]
    fn function_shaped_tool_search_fails_before_the_harness_handler_can_hang() {
        let root = std::env::temp_dir().join(format!(
            "contexthub-tool-search-protocol-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let lines = json!({
            "method": "rawResponseItem/completed",
            "params": {
                "threadId": "thread",
                "turnId": "turn",
                "item": {
                    "type": "function_call",
                    "name": "tool_search",
                    "arguments": "{\"query\":\"RTO\"}",
                    "call_id": "search-1"
                }
            }
        })
        .to_string()
            + "\n";
        let mut log = std::io::BufWriter::new(File::create(root.join("events.jsonl")).unwrap());
        let progress: ProgressCallback = Arc::new(|_| {});
        let mut send_message = |_: &serde_json::Value| -> Result<(), String> { Ok(()) };
        let error = super::read_turn(
            &mut Cursor::new(lines),
            &mut log,
            "thread",
            "turn",
            &progress,
            "reply",
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &mut send_message,
        )
        .unwrap_err();
        assert_eq!(error, "application_tool_search_transport_incompatible");
        drop(log);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn application_tool_search_metadata_exposes_the_turn_policy() {
        assert_eq!(
            application_tool_search_metadata(true),
            json!({
                "cilai_application_tool_search": "enabled",
                "cilai_application_tool_search_max_calls": "3",
                "cilai_application_tool_search_max_results": "8",
                "cilai_application_tool_search_max_applications": "3"
            })
        );
        assert_eq!(
            application_tool_search_metadata(false)["cilai_application_tool_search"],
            "disabled"
        );
    }

    #[test]
    fn turn_usage_excludes_history_and_deduplicates_cumulative_events() {
        let root = std::env::temp_dir().join(format!("contexthub-usage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let event = |thread: &str, turn: &str, input, total_input| {
            json!({
                "method": "thread/tokenUsage/updated",
                "params": { "threadId": thread, "turnId": turn, "tokenUsage": {
                    "last": {"inputTokens": input, "outputTokens": 2, "cachedInputTokens": 1, "totalTokens": input + 2},
                    "total": {"inputTokens": total_input, "outputTokens": total_input / 5, "cachedInputTokens": total_input / 10},
                    "modelContextWindow": 249_036
                }}
            })
        };
        let lines = [
            event("thread", "old-turn", 1000, 9000),
            event("thread", "turn", 10, 110),
            event("other-thread", "turn", 1000, 9000),
            event("thread", "turn", 20, 130),
            event("thread", "turn", 20, 130),
            json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"completed","items":[{"type":"agentMessage","phase":"final_answer","text":"完成"}]}}}),
        ].map(|value| value.to_string()).join("\n") + "\n";
        let mut log = std::io::BufWriter::new(File::create(root.join("events.jsonl")).unwrap());
        let observed = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::clone(&observed);
        let progress: ProgressCallback = Arc::new(move |event| {
            if let Some(context_usage) = event.context_usage {
                events.lock().unwrap().push(context_usage);
            }
        });
        let mut send_message = |_: &serde_json::Value| -> Result<(), String> { Ok(()) };
        let result = super::read_turn(
            &mut Cursor::new(lines),
            &mut log,
            "thread",
            "turn",
            &progress,
            "reply",
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &mut send_message,
        )
        .unwrap();
        assert_eq!(result.usage.input, 30);
        assert_eq!(result.usage.output, 6);
        assert_eq!(result.usage.cached_input, 3);
        assert_eq!(result.context_tokens, Some(22));
        assert_eq!(result.context_window, Some(249_036));
        assert_eq!(
            *observed.lock().unwrap(),
            vec![
                (12, Some(249_036)),
                (22, Some(249_036)),
                (22, Some(249_036))
            ]
        );
        assert!(result.usage_reported);
        drop(log);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_token_event_does_not_invent_context_usage() {
        let root = std::env::temp_dir().join(format!("contexthub-usage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut log = std::io::BufWriter::new(File::create(root.join("events.jsonl")).unwrap());
        let lines = json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"completed","items":[{"type":"agentMessage","text":"完成"}]}}}).to_string() + "\n";
        let progress: ProgressCallback = Arc::new(|_| {});
        let mut send_message = |_: &serde_json::Value| -> Result<(), String> { Ok(()) };
        let result = super::read_turn(
            &mut Cursor::new(lines),
            &mut log,
            "thread",
            "turn",
            &progress,
            "reply",
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &mut send_message,
        )
        .unwrap();
        assert!(!result.usage_reported);
        assert_eq!(result.context_tokens, None);
        assert_eq!(result.context_window, None);
        drop(log);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn final_answer_deltas_are_published_as_one_growing_output() {
        let root = std::env::temp_dir().join(format!("contexthub-stream-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let lines = [
            json!({"method":"item/started","params":{"threadId":"thread","turnId":"turn","item":{"id":"answer","type":"agentMessage","phase":"final_answer","text":""}}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":"answer","delta":"你"}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":"answer","delta":"好"}}),
            json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{"id":"answer","type":"agentMessage","phase":"final_answer","text":"你好"}}}),
            json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"completed","items":[]}}}),
        ].map(|value| value.to_string()).join("\n") + "\n";
        let mut log = std::io::BufWriter::new(File::create(root.join("events.jsonl")).unwrap());
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = Arc::clone(&events);
        let progress: ProgressCallback =
            Arc::new(move |event| received.lock().unwrap().push(event));
        let mut send_message = |_: &serde_json::Value| -> Result<(), String> { Ok(()) };

        let result = super::read_turn(
            &mut Cursor::new(lines),
            &mut log,
            "thread",
            "turn",
            &progress,
            "reply",
            true,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &mut send_message,
        )
        .unwrap();

        let streamed = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| event.streamed_output.clone())
            .collect::<Vec<_>>();
        assert_eq!(streamed, vec!["你", "你好"]);
        assert_eq!(result.output, "你好");
        drop(log);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_message_phase_streams_each_current_reply_without_mixing_commentary() {
        let root = std::env::temp_dir().join(format!("contexthub-stream-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let lines = [
            json!({"method":"item/started","params":{"threadId":"thread","turnId":"turn","item":{"id":"commentary","type":"agentMessage","phase":"commentary","text":""}}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":"commentary","delta":"过程说明"}}),
            json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{"id":"commentary","type":"agentMessage","phase":"commentary","text":"过程说明"}}}),
            json!({"method":"item/started","params":{"threadId":"thread","turnId":"turn","item":{"id":"legacy-preamble","type":"agentMessage","phase":null,"text":""}}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":"legacy-preamble","delta":"正在读取"}}),
            json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{"id":"legacy-preamble","type":"agentMessage","phase":null,"text":"正在读取"}}}),
            json!({"method":"item/started","params":{"threadId":"thread","turnId":"turn","item":{"id":"legacy-answer","type":"agentMessage","phase":null,"text":""}}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":"legacy-answer","delta":"你"}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread","turnId":"turn","itemId":"legacy-answer","delta":"好"}}),
            json!({"method":"item/completed","params":{"threadId":"thread","turnId":"turn","item":{"id":"legacy-answer","type":"agentMessage","phase":null,"text":"你好"}}}),
            json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"turn","status":"completed","items":[]}}}),
        ].map(|value| value.to_string()).join("\n") + "\n";
        let mut log = std::io::BufWriter::new(File::create(root.join("events.jsonl")).unwrap());
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = Arc::clone(&events);
        let progress: ProgressCallback =
            Arc::new(move |event| received.lock().unwrap().push(event));
        let mut send_message = |_: &serde_json::Value| -> Result<(), String> { Ok(()) };

        let result = super::read_turn(
            &mut Cursor::new(lines),
            &mut log,
            "thread",
            "turn",
            &progress,
            "reply",
            true,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &mut send_message,
        )
        .unwrap();

        let streamed = events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| event.streamed_output.clone())
            .collect::<Vec<_>>();
        assert_eq!(streamed, vec!["正在读取", "你", "你好"]);
        assert_eq!(result.output, "你好");
        drop(log);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn tool_events_do_not_expose_command_contents() {
        let item = json!({ "type": "commandExecution", "command": "secret command" });
        assert_eq!(
            item_summary(&item, true).as_deref(),
            Some("已运行工作区命令")
        );
    }

    #[test]
    fn rejected_command_activity_preserves_rejection_summary() {
        let item = json!({
            "id": "declined-command",
            "type": "commandExecution",
            "status": "declined",
            "command": "cat private-content",
            "commandActions": [{"type": "read", "name": "report.md"}]
        });
        assert_eq!(
            item_summary(&item, true).as_deref(),
            Some("已拒绝工作区命令")
        );
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = Arc::clone(&events);
        let progress: ProgressCallback =
            Arc::new(move |event| received.lock().unwrap().push(event));
        super::handle_notification(
            &json!({"method": "item/completed", "params": {"item": item}}),
            &progress,
            "reply",
        );
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "已拒绝工作区命令");
        assert_eq!(events[0].status, "failed");
        assert_eq!(
            item_summary(
                &json!({
                    "type": "commandExecution", "status": "failed",
                    "commandActions": [{"type": "read", "name": "report.md"}]
                }),
                true
            )
            .as_deref(),
            Some("读取 report.md 失败")
        );
    }

    #[test]
    fn visible_activity_uses_structured_actions_and_explicit_summaries_only() {
        assert_eq!(
            item_summary(
                &json!({
                    "type": "commandExecution",
                    "status": "completed",
                    "command": "cat /Users/test/private/report.md",
                    "commandActions": [{
                        "type": "read",
                        "path": "/Users/test/private/report.md",
                        "name": "report.md"
                    }]
                }),
                true
            )
            .as_deref(),
            Some("已读取 report.md")
        );
        assert_eq!(
            item_summary(
                &json!({
                    "type": "reasoning",
                    "summary": ["我先检查文件结构和已有内容。"],
                    "content": ["不应展示的内部推理"]
                }),
                true
            )
            .as_deref(),
            Some("我先检查文件结构和已有内容。")
        );
        assert_eq!(
            item_summary(
                &json!({
                    "type": "agentMessage",
                    "phase": "commentary",
                    "text": "接下来验证修改后的界面行为。"
                }),
                true
            )
            .as_deref(),
            Some("接下来验证修改后的界面行为。")
        );
    }

    #[test]
    fn application_write_tool_uses_exact_bound_tool_and_returns_receipt() {
        let scope = ApplicationMcpScope {
            application_id: "com.cilai.environmental.archive".to_owned(),
            application_name: "环保档案".to_owned(),
            application_ids: vec!["com.cilai.environmental.archive".to_owned()],
            restrict_to_applications: true,
            browser_origins: Vec::new(),
            report_review: false,
            tool_names: vec!["search_documents".to_owned()],
            read_tools: vec![ApplicationReadToolScope {
                application_id: "com.cilai.environmental.archive".to_owned(),
                tool_name: "search_documents".to_owned(),
            }],
            write_tools: vec![ApplicationWriteToolScope {
                application_id: "com.cilai.environmental.archive".to_owned(),
                application_name: "环保档案".to_owned(),
                dynamic_name: "cilai_write_submit_review_123".to_owned(),
                tool_name: "submit_review".to_owned(),
                title: "提交审核".to_owned(),
                description: "提交指定档案版本进入审核".to_owned(),
                input_schema: json!({"type": "object"}),
                risk_level: "write".to_owned(),
                timeout_ms: Some(30_000),
                confirmation_view: None,
            }],
        };
        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        let callback_received = Arc::clone(&received);
        let callback: ApplicationWriteCallback = Arc::new(move |tool, arguments| {
            callback_received
                .lock()
                .unwrap()
                .push((tool.tool_name.clone(), arguments));
            Ok(json!({"invocationId": "invocation-1", "status": "succeeded"}))
        });
        let mut responses = Vec::new();
        let handled = handle_application_write_request(
            &json!({
                "id": 51,
                "method": "item/tool/call",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "tool": "cilai_write_submit_review_123",
                    "arguments": {"versionId": "version-1", "expectedDocumentRevision": 3}
                }
            }),
            "thread-1",
            "turn-1",
            Some((&scope, &callback)),
            &mut |response| {
                responses.push(response.clone());
                Ok(())
            },
        )
        .unwrap();

        assert!(handled);
        assert_eq!(received.lock().unwrap()[0].0, "submit_review");
        assert_eq!(received.lock().unwrap()[0].1["expectedDocumentRevision"], 3);
        assert_eq!(responses[0].pointer("/result/success"), Some(&json!(true)));
        assert!(
            responses[0]
                .pointer("/result/contentItems/0/text")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|text| text.contains("invocation-1"))
        );
    }

    #[test]
    fn application_write_tool_hides_host_generated_fields_from_the_model() {
        let tool = ApplicationWriteToolScope {
            application_id: "com.cilai.environmental.archive".to_owned(),
            application_name: "环保档案".to_owned(),
            dynamic_name: "cilai_write_submit_review_123".to_owned(),
            tool_name: "submit_review".to_owned(),
            title: "提交审核".to_owned(),
            description: "提交指定档案版本进入审核".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "operationId": { "type": "string" },
                    "versionId": { "type": "string" }
                },
                "required": ["operationId", "versionId"],
                "additionalProperties": false
            }),
            risk_level: "write".to_owned(),
            timeout_ms: Some(30_000),
            confirmation_view: Some(json!({ "hostGeneratedFields": ["operationId"] })),
        };
        let schema = application_write_model_schema(&tool);
        assert!(schema.pointer("/properties/operationId").is_none());
        assert!(schema.pointer("/properties/versionId").is_some());
        assert_eq!(schema["required"], json!(["versionId"]));
        let definition = application_write_tool(&tool);
        assert_eq!(definition["deferLoading"], true);
        assert!(
            definition["description"]
                .as_str()
                .is_some_and(|description| description
                    .contains("环保档案（com.cilai.environmental.archive）"))
        );
        let namespace = application_write_tool_namespace(&[tool]);
        assert_eq!(namespace["type"], "namespace");
        assert_eq!(namespace["name"], "cilai_application_writes");
        assert_eq!(namespace["tools"][0]["deferLoading"], true);
    }

    #[test]
    fn tool_events_keep_lightweight_identity_and_phase() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = Arc::clone(&events);
        let progress: ProgressCallback =
            Arc::new(move |event| received.lock().unwrap().push(event));
        super::handle_notification(
            &json!({"method":"item/completed","params":{"item":{"id":"command-1","type":"commandExecution","status":"completed","durationMs":126,"command":"cat secret","commandActions":[{"type":"read","path":"/tmp/report.md","command":"cat secret","name":"report.md"}]}}}),
            &progress,
            "reply",
        );
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].item_id.as_deref(), Some("command-1"));
        assert_eq!(events[0].phase, "reply");
        assert_eq!(events[0].kind, "command");
        assert_eq!(events[0].status, "completed");
        assert_eq!(events[0].duration_ms, Some(126));
        assert!(events[0].detail_available);
        assert_eq!(events[0].summary, "已读取 report.md");
        assert!(!events[0].summary.contains("cat secret"));
    }

    #[test]
    fn application_tool_steps_show_display_names_instead_of_versioned_names() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = Arc::clone(&events);
        let inner: ProgressCallback = Arc::new(move |event| received.lock().unwrap().push(event));
        let progress = super::with_tool_labels(
            inner,
            std::collections::BTreeMap::from([(
                "a_24aafd2c58c46c190b8e_48c47ce4cebdd343d579".to_owned(),
                "环保档案 · 查询档案记录".to_owned(),
            )]),
        );
        for method in ["item/started", "item/completed"] {
            super::handle_notification(
                &json!({"method": method, "params": {"item": {
                    "id": "tool-1", "type": "mcpToolCall", "status": "completed",
                    "server": "cilai_application", "tool": "a_24aafd2c58c46c190b8e_48c47ce4cebdd343d579"
                }}}),
                &progress,
                "reply",
            );
        }
        let events = events.lock().unwrap();
        assert_eq!(events[0].summary, "正在使用 环保档案 · 查询档案记录");
        assert_eq!(events[1].summary, "已使用 环保档案 · 查询档案记录");
    }

    #[test]
    fn context_compaction_is_reported_as_its_own_visible_step() {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = Arc::clone(&events);
        let progress: ProgressCallback =
            Arc::new(move |event| received.lock().unwrap().push(event));
        for method in ["item/started", "item/completed"] {
            super::handle_notification(
                &json!({"method": method, "params": {"item": {"id": "compact-1", "type": "contextCompaction"}}}),
                &progress,
                "reply",
            );
        }
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .all(|event| event.kind == "context"
                    && event.item_id.as_deref() == Some("compact-1"))
        );
        assert_eq!(events[0].status, "started");
        assert_eq!(events[0].summary, "正在整理对话上下文");
        assert_eq!(events[1].status, "completed");
        assert_eq!(
            events[1].summary,
            "已整理对话上下文，对话较长时建议新建对话继续"
        );
    }

    #[test]
    fn compaction_prompt_forbids_repeating_completed_operations() {
        let argument = compact_prompt_config();
        let value = argument
            .strip_prefix("compact_prompt=")
            .expect("压缩提示词必须以配置项名开头");
        let prompt: String = serde_json::from_str(value).expect("配置值必须是合法的带引号字符串");
        assert_eq!(prompt, COMPACT_PROMPT);
        assert!(prompt.contains("已完成的操作"));
        assert!(prompt.contains("不得重复执行"));
        assert!(!value.contains("\\/"));
    }

    #[test]
    fn completion_does_not_release_writer_before_process_exit_or_call_private_protocol() {
        let path = std::env::temp_dir().join(format!("retirement-{}", uuid::Uuid::new_v4()));
        let mut log = std::io::BufWriter::new(File::create(&path).unwrap());
        let mut reader = Cursor::new(Vec::<u8>::new());
        let progress: ProgressCallback = Arc::new(|_| {});
        let delivered = std::cell::Cell::new(false);
        assert!(!super::complete_protocol_before_retirement(
            Err("stopped".into()),
            Some("t".into()),
            |result| {
                assert!(result.is_err());
                delivered.set(true);
            },
            &|| panic!("writer belongs to live process"),
            &mut |_| panic!("no shutdown RPC or memory flush"),
            &mut reader,
            &mut log,
            &progress
        ));
        assert!(delivered.get());
        drop(log);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn context_window_overflow_keeps_a_stable_failure_marker() {
        assert_eq!(
            turn_failure_reason(&json!({"turn": {"status": "failed", "error": {
                "message": "Codex ran out of room in the model's context window.",
                "codexErrorInfo": "contextWindowExceeded"
            }}})),
            "codex_harness_turn_failed:context_window_exceeded:Codex ran out of room in the model's context window."
        );
        assert_eq!(
            turn_failure_reason(&json!({"turn": {"status": "failed", "error": {
                "message": "upstream busy",
                "codexErrorInfo": "serverOverloaded"
            }}})),
            "codex_harness_turn_failed:upstream busy"
        );
        assert_eq!(
            turn_failure_reason(&json!({"turn": {"status": "interrupted"}})),
            "codex_harness_turn_failed:模型 Harness 运行未完成"
        );
    }

    #[test]
    fn plan_updates_report_todo_progress_only_when_a_list_exists() {
        assert_eq!(
            plan_summary(&json!({"plan": [
                {"step": "分析", "status": "completed"},
                {"step": "修改", "status": "inProgress"}
            ]}))
            .as_deref(),
            Some("Todo 已完成 1/2")
        );
        assert_eq!(plan_summary(&json!({"plan": []})), None);
    }

    #[test]
    fn gateway_tool_output_limit_is_not_treated_as_an_assistant_reply() {
        assert!(is_tool_output_too_large_response(
            "output exceeds maximal length 1000000, original too long."
        ));
        assert!(!is_tool_output_too_large_response("正常回复"));
    }

    #[test]
    fn parallel_preflight_accepts_either_response_order_and_keeps_probe_mandatory() {
        let path = std::env::temp_dir().join(format!("preflight-{}", uuid::Uuid::new_v4()));
        let progress: super::ProgressCallback = Arc::new(|_| {});
        for messages in [
            vec![
                json!({"id":7,"result":{"data":[]}}),
                json!({"id":12,"result":{"exitCode":0}}),
            ],
            vec![
                json!({"id":12,"result":{"exitCode":0}}),
                json!({"id":7,"result":{"data":[]}}),
            ],
        ] {
            let data = format!("{}\n{}\n", messages[0], messages[1]);
            let mut reader = std::io::Cursor::new(data);
            let mut log = std::io::BufWriter::new(File::create(&path).unwrap());
            assert!(
                super::read_probe_and_mcp_status(&mut reader, &mut log, &progress, "test")
                    .unwrap()
                    .is_some()
            );
        }
        for result in [json!({"exitCode":5}), json!({})] {
            let mut reader = std::io::Cursor::new(format!(
                "{}\n{}\n",
                json!({"id":7,"result":{"data":[]}}),
                json!({"id":12,"result":result})
            ));
            let mut log = std::io::BufWriter::new(File::create(&path).unwrap());
            assert!(
                super::read_probe_and_mcp_status(&mut reader, &mut log, &progress, "test").is_err()
            );
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "单个用例核对完整模型目录及全部路由字段"
    )]
    fn generated_model_catalog_preserves_routing_and_uses_product_identity() {
        let root = std::env::temp_dir().join(format!("contexthub-models-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let request = CodexHarnessRequest {
            turn_admission: None,
            validation_probe: false,
            execution: None,
            memory_model: Some("model-A".into()),
            conversation_kind: crate::conversation_kernel::ConversationKind::Workbench,
            planning: false,
            executable: "cilai-harness".to_owned(),
            harness_home: root.clone(),
            tool_snapshot_scope_id: "conversation_test_catalog".to_owned(),
            conversation_id: None,
            workspace_root: PathBuf::from("/tmp"),
            workspace_roots: vec![PathBuf::from("/tmp")],
            model: "Qwen-test".to_owned(),
            model_provider: "qwen-test".to_owned(),
            provider_label: "Qwen test".to_owned(),
            base_url: "http://127.0.0.1/v1".to_owned(),
            api_key_env: "TEST_KEY".to_owned(),
            api_key: "secret".to_owned(),
            context_window: Some(262_144),
            reasoning_effort: Some("high".to_owned()),
            model_catalog: vec![CodexHarnessModel {
                model_id: "Qwen-test".to_owned(),
                display_name: "Qwen Test".to_owned(),
                context_window: Some(262_144),
                supported_reasoning_efforts: vec!["low".to_owned(), "high".to_owned()],
                default_reasoning_effort: Some("low".to_owned()),
                tool_capabilities: Some(ModelToolCapabilities {
                    version: 1,
                    profile: "responses".to_owned(),
                    function_calling: true,
                    client_tool_search: true,
                    custom_tools: true,
                    web_search_mode: "cached".to_owned(),
                }),
                input_modalities: vec!["text".to_owned(), "image".to_owned()],
            }],
            developer_instructions: "平台规则".to_owned(),
            classification: None,
            application_result: None,
            report_result: None,
            user_input: "用户问题".to_owned(),
            turn_context: String::new(),
            conversation_text: "用户：用户问题".to_owned(),
            attachments: Vec::new(),
            skills: vec![CodexHarnessSkill {
                skill_id: "skill_automatic".to_owned(),
                name: "自动技能".to_owned(),
                path: root.join("skills/skill_automatic/SKILL.md"),
                explicit: false,
            }],
            user_mcp: None,
            personal_tools: None,
            application_action: None,
            application_write: None,
            application_mcp: None,
            resume_thread_id: None,
            persistent_thread: None,
            application_adapter: None,
            approval_mode: "legacy".to_owned(),
            approval_review_model: Some("Qwen-review-choice".to_owned()),
            memory_preferences: crate::memory::MemoryPreferences::disabled(),
            sandbox_mode: "read-only".to_owned(),
            event_log_path: root.join("events.jsonl"),
            stderr_path: root.join("stderr.log"),
        };

        let mut request = request;
        verify_official_local_mcp(&mut request);
        let original_runtime = super::runtime_fingerprint(&request);
        request.user_input.push_str("下一轮");
        request.tool_snapshot_scope_id.push_str("-next");
        assert_eq!(super::runtime_fingerprint(&request), original_runtime);
        request.approval_mode = "auto-review".into();
        assert_ne!(super::runtime_fingerprint(&request), original_runtime);
        request.approval_mode = "legacy".into();
        assert_eq!(super::runtime_fingerprint(&request), original_runtime);
        request.memory_preferences = crate::memory::MemoryPreferences {
            enabled: true,
            from_tool_chats: true,
        };
        let mut command = std::process::Command::new("harness");
        super::configure_memory(&mut command, &request).unwrap();
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.contains(&"features.memories=true".to_owned()));
        assert!(args.contains(&"memories.generate_memories=true".to_owned()));
        assert!(args.contains(&"memories.use_memories=true".to_owned()));
        assert!(args.contains(&"memories.extract_model=\"model-A\"".to_owned()));
        assert!(args.contains(&"memories.consolidation_model=\"model-A\"".to_owned()));
        request.memory_preferences = crate::memory::MemoryPreferences::disabled();
        let mut disabled = std::process::Command::new("harness");
        super::configure_memory(&mut disabled, &request).unwrap();
        let args = disabled
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.contains(&"features.memories=false".to_owned()));
        assert!(args.contains(&"memories.generate_memories=false".to_owned()));
        assert!(args.contains(&"memories.use_memories=false".to_owned()));

        let catalog: serde_json::Value =
            serde_json::from_slice(&std::fs::read(write_model_catalog(&request).unwrap()).unwrap())
                .unwrap();
        assert_eq!(
            catalog.pointer("/models/0/auto_review_model_override"),
            Some(&json!("Qwen-review-choice"))
        );
        assert_eq!(catalog.pointer("/models/0/slug"), Some(&json!("Qwen-test")));
        assert_eq!(
            catalog.pointer("/models/0/default_reasoning_level"),
            Some(&json!("low"))
        );
        assert_eq!(
            catalog.pointer("/models/0/supported_reasoning_levels"),
            Some(&json!([
                {"effort": "low", "description": "Lighter reasoning"},
                {"effort": "high", "description": "Greater reasoning depth"}
            ]))
        );
        assert_eq!(
            catalog.pointer("/models/0/context_window"),
            Some(&json!(262_144))
        );
        assert_eq!(
            catalog.pointer("/models/0/include_skills_usage_instructions"),
            Some(&json!(true))
        );
        assert_eq!(
            catalog.pointer("/models/0/supports_search_tool"),
            Some(&json!(true))
        );
        assert!(
            catalog
                .pointer("/models/0/model_messages/instructions_template")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|instructions| instructions
                    .starts_with(super::PRODUCT_IDENTITY_INSTRUCTIONS)
                    && !instructions.contains("GPT-5")
                    && !instructions.contains("Codex"))
        );

        assert_eq!(
            catalog.pointer("/models/0/truncation_policy/limit"),
            Some(&json!(10_000))
        );
        request.application_action = Some(Arc::new(|_, _| Ok(json!({}))));
        let application_catalog: serde_json::Value =
            serde_json::from_slice(&std::fs::read(write_model_catalog(&request).unwrap()).unwrap())
                .unwrap();
        assert_eq!(
            application_catalog.pointer("/models/0/truncation_policy/limit"),
            Some(&json!(131_072))
        );
        request.conversation_id = Some("ordinary-conversation".to_owned());
        let conversation_catalog: serde_json::Value =
            serde_json::from_slice(&std::fs::read(write_model_catalog(&request).unwrap()).unwrap())
                .unwrap();
        assert_eq!(
            conversation_catalog.pointer("/models/0/truncation_policy/limit"),
            Some(&json!(10_000))
        );
        request.report_result = Some(
            serde_json::from_value(json!({
                "schemaVersion":1,"clientMessageId":"client","inputDigest":"digest",
                "reportId":"00000000-0000-0000-0000-000000000001",
                "versionId":"00000000-0000-0000-0000-000000000002"
            }))
            .unwrap(),
        );
        let report_catalog: serde_json::Value =
            serde_json::from_slice(&std::fs::read(write_model_catalog(&request).unwrap()).unwrap())
                .unwrap();
        assert_eq!(
            report_catalog.pointer("/models/0/truncation_policy/limit"),
            Some(&json!(128_000))
        );
        request.report_result = None;
        request.conversation_id = None;
        request.application_action = None;
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn app_server_protocol_starts_and_resumes_with_product_identity() {
        for (resume_thread_id, classification_valid, planning, fenced_reply) in [
            (None, true, false, false),
            (None, true, true, false),
            (Some("thread-1".to_owned()), true, true, false),
            (Some("thread-1".to_owned()), true, false, false),
            (None, false, false, false),
            (Some("thread-1".to_owned()), false, false, false),
            (None, false, false, true),
            (Some("thread-1".to_owned()), false, false, true),
        ] {
            let root =
                std::env::temp_dir().join(format!("contexthub-harness-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let skill_path = root
                .join("run-skills")
                .join("skill_weekly-r1")
                .join("SKILL.md");
            std::fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
            std::fs::write(&skill_path, "---\nname: 周报整理\n---\n\n整理周报\n").unwrap();
            let raw_reply = if fenced_reply {
                "```json\n{\"schemaVersion\":1,\"nonce\":\"classification-test\",\"reply\":\"模型直接回复\",\"action\":\"fixed\"}\n```"
            } else if classification_valid {
                r#"{"schemaVersion":1,"nonce":"classification-test","reply":"模型直接回复","action":"fixed"}"#
            } else {
                "模型直接回复 {\"schemaVersion\":1,\"nonce\":\"classification-test\",\"reply\":\"模型直接回复\",\"decision\":\"keep\"}"
            };
            let first_delta = if fenced_reply {
                "```json\n{\"schemaVersion\":1,\"nonce\":\"classification-test\",\"reply\":\"模型"
            } else if classification_valid {
                "{\"schemaVersion\":1,\"nonce\":\"classification-test\",\"reply\":\"模型"
            } else {
                "模型"
            };
            let last_delta = &raw_reply[first_delta.len()..];
            let response_lines = [
            json!({"id": 1, "result": {"userAgent": "test"}}),
            json!({"id": 11, "result": {"effectiveEnabled": false}}),
            json!({"id": 9, "result": {}}),
            json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}),
            json!({"id": 7, "result": {"data": [{"name": "test", "tools": {"read": {"description": "读取内容", "inputSchema": {"type": "object"}}}}], "nextCursor": null}}),
            json!({"id": 3, "result": {"turn": {"id": "turn-1"}}}),
            json!({"method": "item/completed", "params": {"threadId": "thread-1", "turnId": "turn-1", "item": {"id": "command-1", "type": "commandExecution", "status": "completed", "exitCode": 0, "command": "Write-Output '完成'", "aggregatedOutput": "完成"}}}),
            json!({"method": "item/completed", "params": {"threadId": "thread-1", "turnId": "turn-1", "item": {"id": "file-1", "type": "fileChange", "status": "completed", "changes": [{"path": "产物/报告.md", "kind": {"type": "add"}, "diff": ""}]}}}),
            json!({"id":99,"method":"item/tool/call","params":{"threadId":"thread-1","turnId":"turn-1","tool":crate::workbench_result::RESULT_TOOL,"arguments":if classification_valid {serde_json::from_str::<Value>(raw_reply).unwrap()} else {json!({"nonce":"wrong"})}}}),
            json!({"method":"item/started","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"message-1","type":"agentMessage","phase":"final_answer","text":""}}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"message-1","delta":first_delta}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"message-1","delta":last_delta}}),
            json!({"method": "item/completed", "params": {"threadId": "thread-1", "turnId": "turn-1", "item": {"id": "message-1", "type": "agentMessage", "phase": "final_answer", "text": raw_reply}}}),
            json!({"method": "thread/tokenUsage/updated", "params": {"threadId": "thread-1", "turnId": "turn-1", "tokenUsage": {"last": {"inputTokens": 12, "cachedInputTokens": 3, "outputTokens": 5, "reasoningOutputTokens": 0, "totalTokens": 17}, "total": {"inputTokens": 12, "cachedInputTokens": 3, "outputTokens": 5, "reasoningOutputTokens": 0, "totalTokens": 17}}}}),
            json!({"method": "turn/completed", "params": {"threadId": "thread-1", "turn": {"id": "turn-1", "status": "completed", "items": []}}}),
        ]
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\n";
            let mut requests = Vec::new();
            let events = Arc::new(Mutex::new(Vec::new()));
            let capture = Arc::clone(&events);
            let progress: ProgressCallback = Arc::new(move |event| {
                capture.lock().unwrap().push(event);
            });
            let request = CodexHarnessRequest {
                turn_admission: None,
                validation_probe: false,
                execution: None,
                memory_model: Some("model-A".into()),
                conversation_kind: crate::conversation_kernel::ConversationKind::Workbench,
                planning,
                executable: "codex".to_owned(),
                harness_home: root.clone(),
                tool_snapshot_scope_id: "conversation_test_protocol".to_owned(),
                conversation_id: None,
                workspace_root: PathBuf::from("/tmp"),
                workspace_roots: vec![PathBuf::from("/tmp"), PathBuf::from("/tmp/project-second")],
                model: "Qwen-test".to_owned(),
                model_provider: "qwen-test".to_owned(),
                provider_label: "Qwen test".to_owned(),
                base_url: "http://127.0.0.1:1/v1".to_owned(),
                api_key_env: "TEST_KEY".to_owned(),
                api_key: "secret".to_owned(),
                context_window: Some(1024),
                reasoning_effort: Some("high".to_owned()),
                model_catalog: vec![CodexHarnessModel {
                    model_id: "Qwen-test".to_owned(),
                    display_name: "Qwen Test".to_owned(),
                    context_window: Some(1024),
                    supported_reasoning_efforts: vec!["low".to_owned(), "high".to_owned()],
                    default_reasoning_effort: Some("low".to_owned()),
                    tool_capabilities: Some(ModelToolCapabilities {
                        version: 1,
                        profile: "responses".to_owned(),
                        function_calling: true,
                        client_tool_search: true,
                        custom_tools: true,
                        web_search_mode: "cached".to_owned(),
                    }),
                    input_modalities: vec!["text".to_owned(), "image".to_owned()],
                }],
                developer_instructions: "平台规则".to_owned(),
                classification: Some(CodexHarnessClassificationRequest {
                    developer_instructions: "本轮同时输出回复和归类".to_owned(),
                    output_schema: json!({ "type": "object", "properties": { "nonce": { "enum": ["classification-test"] } } }),
                    validate: Arc::new(move |output| {
                        (classification_valid
                                && serde_json::from_str::<Value>(output).ok() == Some(json!({"schemaVersion":1,"nonce":"classification-test","reply":"模型直接回复","action":"fixed"})))
                                .then_some(())
                                .ok_or_else(|| "objective_missing".to_owned())
                    }),
                }),
                application_result: None,
                report_result: None,
                user_input: "用户问题".to_owned(),
                turn_context: format!("历史来源资料：{}", "完整合成历史页面资料".repeat(4_000)),
                conversation_text: "用户：用户问题".to_owned(),
                attachments: vec![CodexHarnessAttachment {
                    media_type: "image".to_owned(),
                    path: PathBuf::from("/tmp/example.png"),
                }],
                skills: vec![CodexHarnessSkill {
                    skill_id: "skill_weekly".to_owned(),
                    name: "周报整理".to_owned(),
                    path: skill_path.clone(),
                    explicit: true,
                }],
                user_mcp: None,
                personal_tools: None,
                application_action: None,
                application_write: None,
                application_mcp: None,
                resume_thread_id: resume_thread_id.clone(),
                persistent_thread: None,
                application_adapter: None,
                approval_mode: "full-access".to_owned(),
                approval_review_model: Some("Qwen-test".to_owned()),
                memory_preferences: crate::memory::MemoryPreferences::disabled(),
                sandbox_mode: "danger-full-access".to_owned(),
                event_log_path: root.join("events.jsonl"),
                stderr_path: root.join("stderr.log"),
            };
            let result = run_protocol_for_os(
                &request,
                |message| send(&mut requests, message),
                Cursor::new(response_lines),
                File::create(root.join("protocol.jsonl")).unwrap(),
                &progress,
                None,
                None,
                None,
                OperatingSystem::MacOs,
            )
            .unwrap();
            assert_eq!(result.thread_id, "thread-1");
            let streamed = events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|event| event.streamed_output.clone())
                .collect::<Vec<_>>();
            assert_eq!(streamed, vec!["模型", "模型直接回复"]);
            assert!(
                events
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|event| !event.summary.contains("\"action\""))
            );
            assert_eq!(result.output, "模型直接回复");
            assert!(events.lock().unwrap().iter().any(|event| {
                event.item_id.as_deref() == Some("command-1")
                    && event.kind == "command"
                    && event.status == "completed"
            }));
            assert_eq!(
                result.file_changes,
                vec![CodexHarnessFileChange {
                    path: "产物/报告.md".to_owned(),
                    kind: "add".to_owned(),
                }]
            );
            assert_eq!(
                result.classification_output.as_deref(),
                classification_valid
                    .then(|| serde_json::from_str::<Value>(raw_reply)
                        .unwrap()
                        .to_string())
                    .as_deref()
            );
            assert_eq!(
                result.classification_error.as_deref(),
                if classification_valid {
                    None
                } else {
                    Some("ai_response_classification_result_missing")
                }
            );
            assert_eq!(result.usage.input, 12);
            assert_eq!(result.usage.cached_input, 3);
            assert_eq!(result.usage.output, 5);
            assert_eq!(result.reply_usage.as_ref().unwrap().input, 12);
            assert!(result.classification_usage.is_none());
            assert_eq!(result.context_tokens, Some(17));
            let composition = result.context_composition.as_ref().unwrap();
            assert_eq!(
                composition.conversation_tokens
                    + composition.system_prompt_tokens
                    + composition.tool_tokens
                    + composition.skill_tokens
                    + composition.mcp_tokens.unwrap()
                    + composition.internal_tokens,
                17
            );
            let requests = String::from_utf8(requests).unwrap();
            assert_eq!(
                requests
                    .lines()
                    .filter(|line| line.contains("\"method\":\"turn/start\""))
                    .count(),
                1
            );
            let thread_request: serde_json::Value = requests
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .find(|request| request.get("id") == Some(&json!(2)))
                .unwrap();
            assert_eq!(
                thread_request["method"],
                if resume_thread_id.is_some() {
                    "thread/resume"
                } else {
                    "thread/start"
                }
            );
            assert_eq!(thread_request["params"]["model"], "Qwen-test");
            assert_eq!(
                thread_request["params"]["sandbox"],
                if planning {
                    "read-only"
                } else {
                    "danger-full-access"
                }
            );
            assert_eq!(thread_request["params"]["approvalPolicy"], "never");
            if resume_thread_id.is_none() {
                assert_eq!(thread_request["params"]["experimentalRawEvents"], false);
            }
            assert_eq!(
                thread_request["params"]["runtimeWorkspaceRoots"],
                json!(["/tmp", "/tmp/project-second"])
            );
            let base = thread_request["params"]["baseInstructions"]
                .as_str()
                .unwrap();
            assert!(base.starts_with(super::PRODUCT_IDENTITY_INSTRUCTIONS));
            assert!(!base.contains("You are Codex"));
            assert!(!base.contains("GPT-5"));
            assert!(base.contains("# Working with the user"));
            assert!(requests.contains("\"method\":\"skills/extraRoots/set\""));
            let skill_roots: Value = requests
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .find(|request| request["method"] == "skills/extraRoots/set")
                .unwrap();
            assert_eq!(
                skill_roots["params"]["extraRoots"],
                json!([skill_path.parent().unwrap()])
            );
            let creator_config: Value = requests
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .find(|request| request.get("id") == Some(&json!(11)))
                .expect("bundled creator disabled before thread starts");
            assert_eq!(creator_config["method"], "skills/config/write");
            assert_eq!(creator_config["params"]["enabled"], false);
            assert_eq!(
                creator_config["params"]["path"],
                json!(root.join("skills/.system/skill-creator/SKILL.md"))
            );
            assert!(requests.contains("run-skills"));
            assert!(requests.contains("\"type\":\"skill\""));
            assert!(requests.contains("\"name\":\"周报整理\""));
            assert!(requests.contains("SKILL.md"));
            assert!(!requests.contains("windowsSandbox/readiness"));
            assert!(requests.contains("\"experimentalApi\":true"));
            if resume_thread_id.is_none() {
                assert!(requests.contains("\"name\":\"cilai_register_artifact\""));
                let classification_tool = thread_request["params"]["dynamicTools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|tool| tool["name"] == crate::workbench_result::RESULT_TOOL)
                    .expect("classification tool registered");
                assert_eq!(
                    classification_tool["inputSchema"]["properties"]["nonce"]["enum"],
                    json!(["classification-test"])
                );
            } else {
                assert!(
                    !thread_request["params"]
                        .as_object()
                        .unwrap()
                        .contains_key("dynamicTools")
                );
            }
            assert!(
                thread_request["params"]["developerInstructions"]
                    .as_str()
                    .unwrap()
                    .contains("cilai_submit_workbench_result")
            );
            let developer = thread_request["params"]["developerInstructions"]
                .as_str()
                .unwrap();
            let body = developer
                .split_once("<cilai_foreground_context>\n")
                .unwrap()
                .1
                .strip_suffix("\n</cilai_foreground_context>")
                .unwrap();
            assert!(body.starts_with("平台规则"));
            assert!(body.contains("本轮同时输出回复和归类"));
            assert!(body.contains("cilai_submit_workbench_result"));
            let schema_text = body
                .rsplit_once("\n```json\n")
                .unwrap()
                .1
                .strip_suffix("\n```")
                .unwrap();
            let schema: Value = serde_json::from_str(schema_text).unwrap();
            assert_eq!(
                schema,
                request.classification.as_ref().unwrap().output_schema
            );
            let reply_turn = requests
                .lines()
                .find(|line| line.contains("\"id\":3"))
                .expect("reply turn request exists");
            assert!(!reply_turn.contains("outputSchema"));
            assert!(reply_turn.contains("workbench_response_contract"));
            let reply_params: Value = serde_json::from_str(reply_turn).unwrap();
            let additional = reply_params["params"]["additionalContext"]
                .as_object()
                .unwrap();
            let current_contract = additional["cilai_response_contract"]["value"]
                .as_str()
                .unwrap();
            assert!(current_contract.contains("给用户的完整 Markdown 回复和本轮归类"));
            assert!(current_contract.contains("本轮工作上下文、任务候选和归类规则"));
            assert!(current_contract.contains("无文件产物的只读回答不要求创建或登记文件"));
            let expected = format!("历史来源资料：{}", "完整合成历史页面资料".repeat(4_000));
            let mut parts = additional
                .iter()
                .filter(|(key, _)| key.starts_with("cilai_turn_materials_part_"))
                .collect::<Vec<_>>();
            parts.sort_by_key(|(key, _)| *key);
            assert!(parts.len() > 1);
            let mut assembled = String::new();
            for (index, (key, entry)) in parts.iter().enumerate() {
                assert_eq!(**key, format!("cilai_turn_materials_part_{:06}", index + 1));
                assert_eq!(entry["kind"], "application");
                let text = entry["value"].as_str().unwrap();
                assert!(text.len() <= ADDITIONAL_CONTEXT_PART_BYTES);
                assembled.push_str(text);
            }
            assert_eq!(assembled, expected);
            let manifest = additional["cilai_turn_materials"]["value"]
                .as_str()
                .unwrap();
            assert!(manifest.contains(&format!("共{}段", parts.len())));
            assert!(manifest.contains(&format!("共{}字节", expected.len())));
            assert!(manifest.contains(&format!("{:x}", Sha256::digest(expected.as_bytes()))));
            assert!(
                thread_request["params"]["developerInstructions"]
                    .as_str()
                    .unwrap()
                    .contains("正式结果工具参数 Schema")
            );
            assert!(
                thread_request["params"]["developerInstructions"]
                    .as_str()
                    .unwrap()
                    .contains("\"type\":\"object\"")
            );
            assert!(reply_turn.contains("\"effort\":\"high\""));
            assert!(reply_turn.contains("\"cilai_application_tool_search\":\"disabled\""));
            assert!(reply_turn.contains("\"cilai_application_tool_search_max_calls\":\"3\""));
            assert!(reply_turn.contains("\"cilai_application_tool_search_max_results\":\"8\""));
            assert!(reply_turn.contains("\"type\":\"localImage\""));
            assert!(reply_turn.contains("/tmp/example.png"));
            assert_eq!(
                requests
                    .lines()
                    .filter(|line| {
                        line.contains("\"cilai_request_purpose\":\"work_classification\"")
                    })
                    .count(),
                0
            );
            assert!(!requests.contains("contexthub_desktop_classification"));
            assert_eq!(requests.matches("\"method\":\"turn/start\"").count(), 1);
            assert!(!requests.contains("\"id\":4"));
            assert!(!requests.contains("\"id\":5"));
            assert!(!requests.contains("\"id\":6"));
            assert!(!requests.contains("secret"));
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "单个用例覆盖 Windows 沙箱各状态的启动与恢复门禁"
    )]
    fn windows_readiness_gate_controls_thread_start_or_resume() {
        for (status, expected_error) in [
            ("ready", String::new()),
            ("notConfigured", "windows_sandbox_not_configured".to_owned()),
            (
                "updateRequired",
                "windows_sandbox_update_required".to_owned(),
            ),
            (
                "unexpected",
                "windows_sandbox_setup_failed:windows_sandbox_readiness_invalid:".to_owned(),
            ),
        ] {
            for resume_thread_id in [None, Some("thread-existing".to_owned())] {
                let root = std::env::temp_dir().join(format!(
                    "contexthub-windows-preflight-{}",
                    uuid::Uuid::new_v4()
                ));
                std::fs::create_dir_all(&root).unwrap();
                let response_lines = [
                    json!({"id": 1, "result": {"userAgent": "test"}}),
                    json!({"id": 8, "result": {"status": status}}),
                ]
                .into_iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                    + "\n";
                let request = CodexHarnessRequest {
                    turn_admission: None,
                    validation_probe: false,
                    execution: None,
                    memory_model: Some("model-A".into()),
                    conversation_kind: crate::conversation_kernel::ConversationKind::Workbench,
                    planning: false,
                    executable: "codex-app-server.exe".to_owned(),
                    harness_home: root.clone(),
                    tool_snapshot_scope_id: "conversation_test_windows".to_owned(),
                    conversation_id: None,
                    workspace_root: root.clone(),
                    workspace_roots: vec![root.clone()],
                    model: "Qwen-test".to_owned(),
                    model_provider: "qwen-test".to_owned(),
                    provider_label: "Qwen test".to_owned(),
                    base_url: "http://127.0.0.1/v1".to_owned(),
                    api_key_env: "TEST_KEY".to_owned(),
                    api_key: "secret".to_owned(),
                    context_window: Some(1024),
                    reasoning_effort: None,
                    model_catalog: vec![CodexHarnessModel {
                        model_id: "Qwen-test".to_owned(),
                        display_name: "Qwen Test".to_owned(),
                        context_window: Some(1024),
                        supported_reasoning_efforts: Vec::new(),
                        default_reasoning_effort: None,
                        tool_capabilities: Some(ModelToolCapabilities {
                            version: 1,
                            profile: "responses".to_owned(),
                            function_calling: true,
                            client_tool_search: true,
                            custom_tools: true,
                            web_search_mode: "cached".to_owned(),
                        }),
                        input_modalities: vec!["text".to_owned(), "image".to_owned()],
                    }],
                    developer_instructions: "平台规则".to_owned(),
                    classification: None,
                    application_result: None,
                    report_result: None,
                    user_input: "用户问题".to_owned(),
                    turn_context: String::new(),
                    conversation_text: "用户：用户问题".to_owned(),
                    attachments: Vec::new(),
                    skills: Vec::new(),
                    user_mcp: None,
                    personal_tools: None,
                    application_action: None,
                    application_write: None,
                    application_mcp: None,
                    resume_thread_id,
                    persistent_thread: None,
                    application_adapter: None,
                    approval_mode: "legacy".to_owned(),
                    approval_review_model: Some("Qwen-test".to_owned()),
                    memory_preferences: crate::memory::MemoryPreferences::disabled(),
                    sandbox_mode: "read-only".to_owned(),
                    event_log_path: root.join("events.jsonl"),
                    stderr_path: root.join("stderr.log"),
                };
                let mut requests = Vec::new();
                let progress: ProgressCallback = Arc::new(|_| {});
                let error = run_protocol_for_os(
                    &request,
                    |message| send(&mut requests, message),
                    Cursor::new(response_lines),
                    File::create(root.join("protocol.jsonl")).unwrap(),
                    &progress,
                    None,
                    None,
                    None,
                    OperatingSystem::Windows,
                )
                .unwrap_err();
                assert!(
                    error.starts_with(&expected_error),
                    "unexpected error: {error}"
                );
                let requests = String::from_utf8(requests).unwrap();
                let methods = requests
                    .lines()
                    .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                    .map(|request| {
                        request
                            .get("method")
                            .and_then(serde_json::Value::as_str)
                            .unwrap()
                            .to_owned()
                    })
                    .collect::<Vec<_>>();
                let mut expected_methods =
                    vec!["initialize", "initialized", "windowsSandbox/readiness"];
                if status == "ready" {
                    expected_methods.push(if request.resume_thread_id.is_some() {
                        "thread/resume"
                    } else {
                        "thread/start"
                    });
                }
                assert_eq!(methods, expected_methods);
                if status == "ready"
                    && contexthub_platform_adapter::current_operating_system()
                        == OperatingSystem::Windows
                {
                    for exit_code in [Some(0), Some(5), None] {
                        let probe = exit_code.map_or_else(
                            || json!({"id":12,"error":{"message":"helper_unknown_error: setup refresh had errors"}}),
                            |code| json!({"id":12,"result":{"exitCode":code,"stdout":"","stderr":""}}));
                        let lines = [
                            json!({"id":1,"result":{}}),
                            json!({"id":8,"result":{"status":"ready"}}),
                            json!({"id":2,"result":{"thread":{"id":"thread-1"},"cwd":root,"sandbox":{"type":"readOnly","networkAccess":false}}}),
                            probe,
                            json!({"id":7,"result":{"data":[]}}),
                            json!({"id":3,"result":{"turn":{"id":"turn-1"}}}),
                            json!({"method":"item/completed","params":{"threadId":"thread-1","turnId":"turn-1","item":{"id":"message-1","type":"agentMessage","phase":"final_answer","text":"完成"}}}),
                            json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed","items":[]}}}),
                        ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
                        if exit_code == Some(0) {
                            // 即使沙箱和 MCP 都已就绪，准备线程失败也不能启动模型。
                            let mut blocked = Vec::new();
                            let error = super::run_protocol_with_io(
                                &request,
                                |message| send(&mut blocked, message),
                                &mut std::io::BufReader::new(Cursor::new(lines.clone())),
                                &mut std::io::BufWriter::new(
                                    File::create(root.join("adapter-failed.jsonl")).unwrap(),
                                ),
                                &progress,
                                None,
                                None,
                                None,
                                OperatingSystem::Windows,
                                false,
                                || Err("application_adapter_worker_failed".to_owned()),
                            )
                            .unwrap_err();
                            assert_eq!(error, "application_adapter_worker_failed");
                            assert!(
                                String::from_utf8(blocked)
                                    .unwrap()
                                    .lines()
                                    .map(|line| serde_json::from_str::<Value>(line).unwrap())
                                    .all(|command| command["method"] != "turn/start")
                            );
                        }
                        let mut commands = Vec::new();
                        let mut warm_commands = Vec::new();
                        let warm_lines = format!("{}\n{lines}", json!({"id":9,"result":{}}));
                        let warm = super::run_protocol_with_io(
                            &request,
                            |message| send(&mut warm_commands, message),
                            &mut std::io::BufReader::new(Cursor::new(warm_lines)),
                            &mut std::io::BufWriter::new(
                                File::create(root.join("warm-protocol.jsonl")).unwrap(),
                            ),
                            &progress,
                            None,
                            None,
                            None,
                            OperatingSystem::Windows,
                            true,
                            || Ok(()),
                        );
                        let warm_sent = String::from_utf8(warm_commands).unwrap();
                        assert!(!warm_sent.contains("\"method\":\"initialize\""));
                        assert!(warm_sent.contains("\"extraRoots\":[]"));
                        assert!(warm_sent.contains("\"method\":\"command/exec\""));
                        assert_eq!(
                            warm_sent.contains("\"method\":\"turn/start\""),
                            exit_code == Some(0)
                        );
                        assert_eq!(warm.is_ok(), exit_code == Some(0));
                        let result = run_protocol_for_os(
                            &request,
                            |message| send(&mut commands, message),
                            Cursor::new(lines),
                            File::create(root.join("probe-protocol.jsonl")).unwrap(),
                            &progress,
                            None,
                            None,
                            None,
                            OperatingSystem::Windows,
                        );
                        let sent = String::from_utf8(commands).unwrap();
                        assert!(sent.contains("\"method\":\"command/exec\""));
                        let probe_request = sent
                            .lines()
                            .map(|line| serde_json::from_str::<Value>(line).unwrap())
                            .find(|request| request["method"] == "command/exec")
                            .unwrap();
                        assert!(probe_request["params"].get("outputBytesCap").is_none());
                        assert!(probe_request["params"].get("disableOutputCap").is_none());
                        assert_eq!(probe_request["params"]["sandboxPolicy"]["type"], "readOnly");
                        assert_eq!(
                            sent.contains("\"method\":\"turn/start\""),
                            exit_code == Some(0)
                        );
                        if exit_code == Some(0) {
                            assert_eq!(result.unwrap().output, "完成");
                        } else {
                            assert!(
                                result
                                    .unwrap_err()
                                    .starts_with("windows_sandbox_preflight_failed:")
                            );
                        }
                    }
                }
                std::fs::remove_dir_all(root).unwrap();
            }
        }
    }

    #[test]
    fn native_approval_modes_map_to_app_server_policy_and_sandbox() {
        assert_eq!(
            approval_settings("request", "read-only").unwrap(),
            ("on-request", "user", "workspace-write")
        );
        assert_eq!(
            approval_settings("auto-review", "read-only").unwrap(),
            ("on-request", "auto_review", "workspace-write")
        );
        assert_eq!(
            approval_settings("full-access", "read-only").unwrap(),
            ("never", "user", "danger-full-access")
        );
    }

    /// 取一条真实报文：AI 要打开百度，harness 借 elicitation 通道来问。
    fn browser_navigate_elicitation() -> Value {
        json!({
            "id": 0,
            "method": "mcpServer/elicitation/request",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1",
                "serverName": "cilai_browser",
                "mode": "form",
                "message": "Allow the cilai_browser MCP server to run tool \"browser_navigate\"?",
                "requestedSchema": { "type": "object", "properties": {} },
                "_meta": {
                    "codex_approval_kind": "mcp_tool_call",
                    "persist": ["session", "always"],
                    "tool_description": "在受控浏览器中打开指定地址。",
                    "tool_params": { "url": "https://www.baidu.com" }
                }
            }
        })
    }

    /*
     * 这条链路整个缺过一次：harness 问"允许打开百度吗"，运行时把任何服务端请求一律判失败，
     * 于是员工看到的是"运行未能完成 codex_harness_unexpected_approval"，0 token、9 秒，
     * 而真正发生的只是没人回答那一问。
     */
    #[test]
    fn mcp_tool_approval_reaches_the_employee_instead_of_failing_the_run() {
        let request = parse_approval_request(&browser_navigate_elicitation()).unwrap();
        assert_eq!(request.method, MCP_TOOL_APPROVAL_METHOD);
        assert_eq!(request.request_id, json!(0));
        // 员工要看得出批的是哪个地址，而不是只看到一句"允许使用某服务"
        assert!(
            request.reason.contains("https://www.baidu.com"),
            "{}",
            request.reason
        );
        assert_eq!(request.resource.as_deref(), Some("https://www.baidu.com"));
        // harness 声明了本会话可持久化，员工就该有"本会话都允许"这个选项
        assert!(
            request
                .available_decisions
                .iter()
                .any(|item| item == "approved_session")
        );
    }

    #[test]
    fn mcp_tool_approval_ignores_server_questions_that_are_not_approvals() {
        // 同一个方法也用于服务端向员工提问。没有审批种类标识的不是审批，不能当审批放行
        let mut message = browser_navigate_elicitation();
        message["params"]["_meta"] = json!({ "persist": ["session"] });
        assert!(parse_approval_request(&message).is_none());
    }

    #[test]
    fn mcp_tool_approval_answers_in_the_elicitation_shape() {
        let accept = approval_result(MCP_TOOL_APPROVAL_METHOD, "approved", None).unwrap();
        assert_eq!(accept["action"], json!("accept"));
        assert!(accept.get("_meta").is_none(), "只批这一次就不该带上持久化");
        let session = approval_result(MCP_TOOL_APPROVAL_METHOD, "approved_session", None).unwrap();
        assert_eq!(session["action"], json!("accept"));
        assert_eq!(session["_meta"]["persist"], json!("session"));
        let decline = approval_result(MCP_TOOL_APPROVAL_METHOD, "rejected", None).unwrap();
        assert_eq!(decline["action"], json!("decline"));
    }

    #[test]
    fn mcp_tools_follow_the_approval_mode_the_employee_picked() {
        // 不声明就是 harness 的默认值 auto，而 auto 对没有注解的工具一律要审批——
        // 结果是"帮我批准"下连读页面都要点一次，与员工选的那一档对不上
        assert_eq!(mcp_tools_approval_mode("request"), "prompt");
        assert_eq!(mcp_tools_approval_mode("auto-review"), "auto");
        assert_eq!(mcp_tools_approval_mode("full-access"), "approve");
    }

    #[test]
    fn disabled_builtin_mcp_configuration_initializes_real_harness() {
        use std::io::{BufRead as _, Write as _};
        let root = std::env::temp_dir().join(format!("cilai-plan-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let executable = crate::supervisor::codex_harness_test_binary();
        let mut command = std::process::Command::new(executable);
        command
            .current_dir(&root)
            .env("CODEX_HOME", &root)
            .env(crate::windows_sandbox::REMOTE_CONTROL_DISABLED_ENV, "1")
            .args([
                "--listen",
                "stdio://",
                "-c",
                "check_for_update_on_startup=false",
                "-c",
                "features.plugins=false",
            ]);
        super::configure_http_mcp(&mut command, "cilai_browser", &endpoint, false).unwrap();
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines().take(32) {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if value["id"] == 7 {
                    let _ = sender.send(value);
                    break;
                }
            }
        });
        let request = json!({"id":7,"method":"initialize","params":{"clientInfo":{"name":"contexthub-desktop","version":"0.1.0"},"capabilities":{"experimentalApi":true}}});
        writeln!(child.stdin.as_mut().unwrap(), "{request}").unwrap();
        let result = receiver.recv_timeout(std::time::Duration::from_secs(10));
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        reader.join().unwrap();
        let attempted_connection = listener.accept().is_ok();
        std::fs::remove_dir_all(&root).unwrap();
        let response = result
            .unwrap_or_else(|error| panic!("{error}: {}", String::from_utf8_lossy(&output.stderr)));
        assert!(response.get("result").is_some(), "{response}");
        assert!(response.get("error").is_none(), "{response}");
        assert!(!attempted_connection);
    }

    #[test]
    fn app_server_approval_request_keeps_user_visible_details() {
        let request = parse_approval_request(&json!({
            "id": 41,
            "method": "item/commandExecution/requestApproval",
            "params": {
                "itemId": "command-1",
                "reason": "需要检查构建结果",
                "command": "npm test",
                "cwd": "/tmp/workspace",
                "availableDecisions": ["accept", "acceptForSession", "decline"]
            }
        }))
        .unwrap();
        assert_eq!(request.request_id, json!(41));
        assert_eq!(request.command.as_deref(), Some("npm test"));
        assert_eq!(request.resource.as_deref(), Some("/tmp/workspace"));
        assert!(
            request
                .available_decisions
                .iter()
                .any(|item| item == "acceptForSession")
        );
    }

    #[test]
    fn artifact_tool_registers_only_the_active_turn_and_returns_explicit_result() {
        let registered = Arc::new(std::sync::Mutex::new(Vec::new()));
        let received = Arc::clone(&registered);
        let artifact: ArtifactCallback = Arc::new(move |path| {
            received.lock().unwrap().push(path.to_owned());
            Ok(format!("/tmp/{path}"))
        });
        let mut responses = Vec::new();
        let handled = handle_artifact_tool_request(
            &json!({
                "id": 42,
                "method": "item/tool/call",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "callId": "call-1",
                    "namespace": null,
                    "tool": "cilai_register_artifact",
                    "arguments": { "path": "产物/报告.md" }
                }
            }),
            "thread-1",
            "turn-1",
            Some(&artifact),
            &mut |message| send(&mut responses, message),
        )
        .unwrap();

        assert!(handled);
        assert_eq!(registered.lock().unwrap().as_slice(), ["产物/报告.md"]);
        let response: serde_json::Value = serde_json::from_slice(&responses).unwrap();
        assert_eq!(response["id"], 42);
        assert_eq!(response["result"]["success"], true);
        assert_eq!(
            response.pointer("/result/contentItems/0/text"),
            Some(&json!("产物已登记：/tmp/产物/报告.md"))
        );
    }

    #[test]
    fn artifact_tool_ignores_other_dynamic_tools() {
        let mut responses = Vec::new();
        let handled = handle_artifact_tool_request(
            &json!({
                "id": 42,
                "method": "item/tool/call",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "tool": "cilai_write_submit_review_123",
                    "arguments": { "versionId": "version-1" }
                }
            }),
            "thread-1",
            "turn-1",
            None,
            &mut |message| send(&mut responses, message),
        )
        .unwrap();

        assert!(!handled);
        assert!(responses.is_empty());
    }

    #[test]
    fn application_action_tool_only_calls_the_current_turn_callback() {
        let callback: ApplicationActionCallback = Arc::new(|action_id, arguments| {
            assert_eq!(action_id, "project.refresh");
            assert_eq!(arguments["projectId"], "PRJ-001");
            Ok(json!({ "status": "completed", "pageRevision": 2 }))
        });
        let mut responses = Vec::new();
        let handled = handle_application_action_request(
            &json!({
                "id": 43,
                "method": "item/tool/call",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "tool": "cilai_application_action",
                    "arguments": {
                        "actionId": "project.refresh",
                        "arguments": { "projectId": "PRJ-001" }
                    }
                }
            }),
            "thread-1",
            "turn-1",
            Some(&callback),
            &mut |message| send(&mut responses, message),
        )
        .unwrap();

        assert!(handled);
        let response: serde_json::Value = serde_json::from_slice(&responses).unwrap();
        assert_eq!(response["id"], 43);
        assert_eq!(response["result"]["success"], true);
        assert!(
            response["result"]["contentItems"][0]["text"]
                .as_str()
                .unwrap()
                .contains("completed")
        );
    }
}
