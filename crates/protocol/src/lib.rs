#![doc = "`Local App Server` 的 JSON-RPC 协议单一类型源。"]

mod ai_threads;
pub use ai_threads::*;
mod ai_ledger;
pub use ai_ledger::*;
mod model_connections;
pub use model_connections::*;
mod model_usage;
pub use model_usage::*;
mod personal_automation;
pub use personal_automation::*;
mod personal_packages;
pub use personal_packages::*;
mod personal_pages;
pub use personal_pages::*;
mod mcp_connections;
pub use mcp_connections::*;
mod personal_application_data;
mod personal_data;
pub use personal_application_data::*;
mod personal_memory;
pub use personal_memory::*;
mod personal_namespace_storage;
pub use personal_namespace_storage::*;
mod personal_storage;
pub use personal_data::*;
pub use personal_storage::*;
mod profile_writers;
pub use profile_writers::*;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
use ts_rs::TS;

pub const JSON_RPC_VERSION: &str = "2.0";
pub const PROTOCOL_VERSION: &str = "1.0";
pub const PROTOCOL_MIN_VERSION: &str = "1.0";
pub const PROTOCOL_MAX_VERSION: &str = "1.0";
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PAGE_SIZE: u32 = 500;

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;
pub const BUSINESS_ERROR: i32 = -32010;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(untagged)]
pub enum RequestId {
    String(String),
    Integer(i32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub id: RequestId,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientResponse {
    Success { id: RequestId, result: Value },
    Error { id: RequestId, error: ErrorObject },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientMessage {
    Request(Request),
    Notification(Notification),
    Response(ClientResponse),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
    pub platform: String,
    pub arch: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolRange {
    pub min_version: String,
    pub max_version: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default)]
#[allow(clippy::struct_excessive_bools)]
pub struct ClientCapabilities {
    pub interactive: bool,
    pub notifications: bool,
    pub os_notifications: bool,
    pub event_replay: bool,
    pub resource_tickets: bool,
    pub rich_diff: bool,
    pub deep_links: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct ResumeCursor {
    pub last_durable_event_sequence: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub client: ClientInfo,
    pub protocol: ProtocolRange,
    #[serde(default)]
    pub capabilities: ClientCapabilities,
    #[serde(default)]
    pub resume: ResumeCursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
    pub instance_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct ServerCapabilities {
    #[serde(default)]
    pub personal_data_version: Option<u32>,
    #[serde(default)]
    pub model_connection_protocol: Option<u32>,
    #[serde(default)]
    pub personal_profile_id: Option<String>,
    pub local_ipc: bool,
    pub event_replay: bool,
    pub interactive_requests: bool,
    pub resource_tickets: bool,
    pub rich_text_diff: bool,
    pub file_confirmation: bool,
    pub supported_checksum_algorithms: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub connection_id: String,
    pub client_session_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolLimits {
    pub max_message_bytes: u32,
    pub max_page_size: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResumeResult {
    pub requested_last_durable_event_sequence: Option<u32>,
    pub replay_available: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub server: ServerInfo,
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub session: SessionInfo,
    pub limits: ProtocolLimits,
    pub resume: ResumeResult,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceListParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub workspace_id: String,
    pub workspace_kind: String,
    pub display_name: String,
    pub local_root: String,
    pub local_status: String,
    #[ts(type = "number")]
    pub state_revision: i64,
    pub platform_bound: bool,
    pub platform_revision: Option<String>,
    pub team_space_id: Option<String>,
    pub managed_root: Option<String>,
    pub device_id: Option<String>,
    pub baseline_revision: Option<String>,
    pub change_cursor: Option<String>,
    pub permission_role: Option<String>,
    pub permission_revision: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceListResult {
    pub workspaces: Vec<WorkspaceSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillListParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub enabled: bool,
    pub automatic: bool,
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillListResult {
    pub skills: Vec<SkillSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillRunListParams {
    pub run_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillRunRetryParams {
    pub run_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillRunInstallation {
    pub run_id: String,
    pub message_id: Option<String>,
    pub status: String,
    pub skill_id: Option<String>,
    pub name: Option<String>,
    pub error_reason: Option<String>,
    pub error_detail: Option<String>,
    pub retryable: bool,
    pub display_content: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillRunListResult {
    pub installations: Vec<SkillRunInstallation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillGetParams {
    pub skill_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillPackageFile {
    pub path: String,
    pub category: String,
    pub content: String,
    #[ts(type = "number")]
    pub size_bytes: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillPackageFileInput {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub instructions: String,
    pub files: Vec<SkillPackageFile>,
    pub enabled: bool,
    pub automatic: bool,
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillGetResult {
    pub skill: SkillDetail,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillSaveParams {
    pub skill_id: Option<String>,
    pub name: String,
    pub description: String,
    pub instructions: String,
    #[serde(default)]
    pub files: Option<Vec<SkillPackageFileInput>>,
    pub enabled: bool,
    pub automatic: bool,
    #[ts(type = "number | null")]
    pub expected_revision: Option<i64>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillSaveResult {
    pub skill: SkillDetail,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillImportParams {
    pub archive_path: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillImportDirectoryParams {
    pub skill_file_path: String,
    pub workspace_root: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillImportResult {
    pub skill: SkillDetail,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillSetStateParams {
    pub skill_id: String,
    pub enabled: bool,
    pub automatic: bool,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillSetStateResult {
    pub skill: SkillSummary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillDeleteParams {
    pub skill_id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillDeleteResult {
    pub skill_id: String,
    pub deleted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCreationAcceptanceExample {
    pub input: String,
    pub expected: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCreationSourceFile {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCreationProject {
    pub creation_project_id: String,
    pub title: String,
    pub business_goal: String,
    pub target_type: String,
    pub status: String,
    pub conversation_id: Option<String>,
    pub managed_relative_path: String,
    pub requested_permissions: Vec<String>,
    pub acceptance_examples: Vec<CapabilityCreationAcceptanceExample>,
    pub source_digest: Option<String>,
    #[ts(type = "number")]
    pub source_file_count: i64,
    #[ts(type = "number")]
    pub state_revision: i64,
    pub package_digest: Option<String>,
    pub error_reason: Option<String>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCreationCreateParams {
    pub title: String,
    pub business_goal: String,
    pub target_type: String,
    pub conversation_id: Option<String>,
    pub requested_permissions: Vec<String>,
    pub acceptance_examples: Vec<CapabilityCreationAcceptanceExample>,
    #[serde(default)]
    pub source_files: Vec<CapabilityCreationSourceFile>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCreationCreateResult {
    pub project: CapabilityCreationProject,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCreationListParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCreationListResult {
    pub projects: Vec<CapabilityCreationProject>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCreationGetParams {
    pub creation_project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCreationGetResult {
    pub project: CapabilityCreationProject,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCreationPreviewParams {
    pub creation_project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCreationPreviewResult {
    pub creation_project_id: String,
    pub title: String,
    pub html: String,
    pub content_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityCreationInstallParams {
    pub creation_project_id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub package_digest: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCreationInstallResult {
    pub project: CapabilityCreationProject,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalApplicationOpenParams {
    pub creation_project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct PersonalApplicationOpenResult {
    pub creation_project_id: String,
    pub title: String,
    pub html: String,
    pub content_digest: String,
    #[ts(type = "number")]
    pub installation_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillConversationCatalogParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillConversationCatalogItem {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub automatic: bool,
    #[ts(type = "number")]
    pub state_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillConversationCatalogResult {
    pub skills: Vec<SkillConversationCatalogItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkillRuntimeCatalogParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillRuntimeCatalogItem {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub source: String,
    pub managed_path: String,
    pub automatic: bool,
    #[ts(type = "number")]
    pub state_revision: i64,
    pub content_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct SkillRuntimeCatalogResult {
    pub skills: Vec<SkillRuntimeCatalogItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationSkillBinding {
    pub name: String,
    pub content_digest: String,
    pub required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceCreateParams {
    pub local_root: String,
    pub display_name: String,
    pub client_operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCreateResult {
    pub workspace: WorkspaceSummary,
    pub scan_task_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceDirectoryCreateParams {
    pub workspace_id: String,
    pub parent_relative_path: String,
    pub name: String,
    pub client_operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDirectoryCreateResult {
    pub relative_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceTargetValidateParams {
    pub workspace_id: String,
    pub relative_path: Option<String>,
    pub require_write: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTargetValidateResult {
    pub workspace_id: String,
    pub workspace_kind: String,
    pub local_root: String,
    pub team_space_id: Option<String>,
    pub sync_eligible: bool,
    pub permission_role: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceScanParams {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceScanResult {
    pub workspace_id: String,
    #[ts(type = "number")]
    pub generation: i64,
    #[ts(type = "number")]
    pub observed_count: i64,
    #[ts(type = "number")]
    pub state_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceFilesParams {
    pub workspace_id: String,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileItem {
    pub local_file_id: String,
    pub relative_path: String,
    pub file_type: String,
    pub presence_state: String,
    pub display_state: Option<String>,
    /// 本地基线相对平台最新已发布版本的关系。界面据此区分"只有本地改了"和"云端也改了"。
    pub baseline_relation: Option<String>,
    pub local_change_state: Option<String>,
    pub confirmation_state: Option<String>,
    pub conflict_state: Option<String>,
    #[ts(type = "number")]
    pub state_revision: Option<i64>,
    pub checksum_algorithm: Option<String>,
    pub checksum_value: Option<String>,
    pub baseline_version_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFilesResult {
    pub workspace_id: String,
    #[ts(type = "number")]
    pub workspace_revision: i64,
    pub files: Vec<WorkspaceFileItem>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

/// 读取本地工作区里一个目录的直接子项。只读磁盘、不读文件内容，不依赖索引。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceListDirectoryParams {
    pub workspace_id: String,
    /// 相对工作区根的目录路径；为空表示根目录。
    pub relative_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDirectoryEntry {
    pub relative_path: String,
    pub name: String,
    /// `file`、`directory` 或 `symlink`。
    pub file_type: String,
    #[ts(type = "number | null")]
    pub size_bytes: Option<i64>,
    #[ts(type = "number | null")]
    pub modified_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceListDirectoryResult {
    pub workspace_id: String,
    pub relative_path: Option<String>,
    /// 目录在前、同类按名称排序。
    pub entries: Vec<WorkspaceDirectoryEntry>,
    /// 这一层里读不到的条目数。非零时界面要提示这一层不完整。
    #[ts(type = "number")]
    pub issue_count: i64,
}

/// 请求后台立即为工作区建一次索引。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceIndexRequestParams {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIndexRequestResult {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceIndexStatusParams {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIndexStatusResult {
    pub workspace_id: String,
    /// `idle`、`queued`、`scanning`、`hashing`、`committing` 或 `failed`。
    pub phase: String,
    #[ts(type = "number")]
    pub entries_examined: i64,
    #[ts(type = "number")]
    pub files_hashed: i64,
    #[ts(type = "number")]
    pub files_to_hash: i64,
    /// 写入阶段已提交的批数。
    #[ts(type = "number")]
    pub batches_committed: i64,
    /// 写入阶段总批数。
    #[ts(type = "number")]
    pub batches_total: i64,
    /// 失败原因代码，仅 `failed` 时有值。
    pub reason: Option<String>,
}

/// 优先为单个条目建立索引记录，不等整个工作区的索引。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceIndexPathParams {
    pub workspace_id: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIndexPathResult {
    pub workspace_id: String,
    pub local_file_id: String,
    pub relative_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiStateGetParams {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiStateGetResult {
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "unknown")]
    pub state: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiStateReplaceParams {
    #[ts(type = "number")]
    pub expected_revision: i64,
    #[ts(type = "unknown")]
    pub state: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiStateReplaceResult {
    #[ts(type = "number")]
    pub state_revision: i64,
}

/// 一个会话、消息或运行条目在列表中的位置与完整内容。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiStateEntityUpsert {
    #[ts(type = "number")]
    pub position: i64,
    #[ts(type = "unknown")]
    pub value: Value,
}

/// 同一类条目相对上次提交的变化。
///
/// `total` 是应用变化后该类条目应有的总数，服务端据此核对双方基线是否一致：
/// 不一致说明本地记忆的"上次已保存"与库里实际不同，按增量写入只会把错误固化下来。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiStateEntityDelta {
    pub upserts: Vec<AiStateEntityUpsert>,
    pub removed_ids: Vec<String>,
    #[ts(type = "number")]
    pub total: i64,
}

/// AI 运行状态的一次变更。
///
/// `full` 为真时各类条目必须完整给出，库里不在其中的条目一律删除，`payload` 必填；
/// 为假时只给出变化的条目，`payload` 为空表示其余字段未变。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiStateDelta {
    pub full: bool,
    #[ts(type = "unknown")]
    pub payload: Option<Value>,
    pub conversations: AiStateEntityDelta,
    pub messages: AiStateEntityDelta,
    pub runs: AiStateEntityDelta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiStateApplyParams {
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub delta: AiStateDelta,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiStateApplyResult {
    #[ts(type = "number")]
    pub state_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiConversationSetArchivedParams {
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub conversation_ids: Vec<String>,
    pub archived: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiConversationSetArchivedResult {
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "unknown[]")]
    pub conversations: Vec<Value>,
    pub changed_conversation_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiWorkTaskCandidatesParams {
    pub project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiWorkTaskCandidate {
    pub task_id: String,
    pub title: String,
    pub objective: String,
    pub expected_output: String,
    pub completion_criteria: String,
    pub stage: String,
    #[ts(type = "number")]
    pub state_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiWorkTaskCandidatesResult {
    pub project_id: String,
    pub tasks: Vec<AiWorkTaskCandidate>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiTurnClassification {
    pub action: String,
    pub task_action: String,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub objective: Option<String>,
    pub expected_output: Option<String>,
    pub completion_criteria: Option<String>,
    pub conversation_status: Option<String>,
    pub stage: Option<String>,
    pub outcome_title: Option<String>,
    pub outcome_summary: Option<String>,
    pub reason: String,
    pub confidence: f64,
    #[ts(type = "unknown")]
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiTurnFinalizeParams {
    pub ledger: AiLedgerApplyParams,
    pub conversation_id: String,
    pub run_id: String,
    pub project_id: Option<String>,
    pub previous_task_id: Option<String>,
    pub agent_label: String,
    pub new_task_id: Option<String>,
    pub classification: AiTurnClassification,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiTurnFinalizeResult {
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "unknown")]
    pub ledger_receipt: Value,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub stage: Option<String>,
    /// 同一回合此前已提交过，本次未写入状态；调用方不能再把本地状态当作库里的现状。
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectListParams {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub project_id: String,
    pub display_name: String,
    pub goal: String,
    pub status: String,
    pub primary_workspace_id: Option<String>,
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectListResult {
    pub projects: Vec<ProjectSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectGetParams {
    pub project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGetResult {
    pub project: ProjectSummary,
    pub workspace_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectPortfolioListParams {
    pub workspace_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPortfolioPhaseItem {
    pub phase_id: String,
    pub name: String,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPortfolioItem {
    pub project_id: String,
    pub display_name: String,
    pub goal: String,
    pub status: String,
    pub workspace_id: Option<String>,
    pub workspace_display_name: Option<String>,
    pub workspace_ids: Vec<String>,
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub has_plan: bool,
    pub active_phases: Vec<ProjectPortfolioPhaseItem>,
    pub next_phase_name: Option<String>,
    #[ts(type = "number")]
    pub task_total: i64,
    #[ts(type = "number")]
    pub task_completed: i64,
    #[ts(type = "number")]
    pub deliverable_total: i64,
    #[ts(type = "number")]
    pub deliverable_accepted: i64,
    #[ts(type = "number")]
    pub blocker_count: i64,
    #[ts(type = "number")]
    pub decision_count: i64,
    #[ts(type = "number")]
    pub incident_count: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPortfolioListResult {
    pub items: Vec<ProjectPortfolioItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectCreateParams {
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_ids: Vec<String>,
    #[serde(default)]
    pub team_space_ids: Vec<String>,
    pub display_name: String,
    pub goal: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCreateResult {
    pub project: ProjectSummary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectUpdateParams {
    pub project_id: String,
    pub display_name: String,
    pub goal: String,
    /// The revision the editor was showing. A stale revision is refused rather than silently
    /// overwriting someone else's edit, which is also what makes a duplicate submit safe.
    #[ts(type = "number")]
    pub expected_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUpdateResult {
    pub project: ProjectSummary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectDeleteParams {
    pub project_id: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeleteResult {
    pub project_id: String,
    pub released_workspace_ids: Vec<String>,
    pub removed_phases: i64,
    pub removed_tasks: i64,
    pub removed_deliverables: i64,
    pub removed_attention_cases: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceDeleteParams {
    pub workspace_id: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDeleteResult {
    pub workspace_id: String,
    pub removed_files: i64,
    pub removed_events: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectDetachWorkspaceParams {
    pub project_id: String,
    pub workspace_id: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDetachWorkspaceResult {
    pub project: ProjectSummary,
    pub workspace_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectAttachWorkspaceParams {
    pub project_id: String,
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub workspace_ids: Vec<String>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAttachWorkspaceResult {
    pub project: ProjectSummary,
    pub workspace_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliverableListParams {
    pub project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeliverableFileBindingItem {
    pub local_file_id: String,
    pub workspace_id: String,
    pub relative_path: String,
    #[ts(type = "number")]
    pub file_revision: i64,
    pub checksum_algorithm: String,
    pub checksum_value: String,
    pub binding_role: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeliverableCarrierItem {
    pub carrier_id: String,
    pub carrier_type: String,
    pub carrier_role: String,
    pub label: String,
    pub reference: String,
    pub version_label: String,
    pub source_project_id: Option<String>,
    pub source_deliverable_id: Option<String>,
    pub source_version_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeliverableVersionItem {
    pub version_id: String,
    #[ts(type = "number")]
    pub version_no: i64,
    pub status: String,
    pub summary: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub accepted_at: Option<i64>,
    #[ts(type = "number")]
    pub rejected_at: Option<i64>,
    pub bindings: Vec<DeliverableFileBindingItem>,
    pub carriers: Vec<DeliverableCarrierItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliverableCarrierInputItem {
    pub carrier_type: String,
    pub carrier_role: String,
    pub label: String,
    pub reference: String,
    pub version_label: String,
    pub source_version_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeliverableItem {
    pub deliverable_id: String,
    pub project_id: String,
    pub title: String,
    pub deliverable_type: String,
    pub owner_label: String,
    pub acceptance_criteria: String,
    pub status: String,
    pub current_accepted_version_id: Option<String>,
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub versions: Vec<DeliverableVersionItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeliverableListResult {
    pub project_id: String,
    pub deliverables: Vec<DeliverableItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliverableCreateParams {
    pub project_id: String,
    pub title: String,
    pub deliverable_type: String,
    pub owner_label: String,
    pub acceptance_criteria: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeliverableCreateResult {
    pub deliverable: DeliverableItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliverableVersionCreateParams {
    pub deliverable_id: String,
    pub summary: String,
    pub local_file_ids: Vec<String>,
    pub carriers: Vec<DeliverableCarrierInputItem>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeliverableVersionCreateResult {
    pub version: DeliverableVersionItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliverableVersionReviewParams {
    pub deliverable_id: String,
    pub version_id: String,
    pub decision: String,
    pub reviewer_label: String,
    pub comment: String,
    #[ts(type = "number")]
    pub expected_deliverable_revision: i64,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeliverableVersionReviewResult {
    pub deliverable: DeliverableItem,
    pub version: DeliverableVersionItem,
    pub decision: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MyWorkGetParams {
    pub project_id: Option<String>,
    pub focus_group: Option<String>,
    pub limit: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct MyWorkItem {
    pub work_item_id: String,
    pub source_type: String,
    pub source_id: String,
    pub project_id: String,
    pub project_name: String,
    pub primary_workspace_id: Option<String>,
    pub phase_id: Option<String>,
    pub phase_name: Option<String>,
    pub task_id: Option<String>,
    pub deliverable_id: Option<String>,
    pub deliverable_title: Option<String>,
    pub attention_case_id: Option<String>,
    pub title: String,
    pub action_type: String,
    pub focus_group: String,
    pub responsibility_type: String,
    pub responsibility_label: String,
    pub priority: String,
    #[ts(type = "number | null")]
    pub due_at: Option<i64>,
    #[ts(type = "number")]
    pub source_revision: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[serde(default)]
    #[ts(optional)]
    pub blocking_run_id: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub blocking_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct MyWorkGetResult {
    pub items: Vec<MyWorkItem>,
    #[ts(type = "number")]
    pub total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlobalSearchParams {
    pub query: String,
    #[ts(type = "number")]
    pub limit: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchItem {
    pub object_type: String,
    pub object_id: String,
    pub title: String,
    pub subtitle: String,
    pub project_id: Option<String>,
    pub workspace_id: String,
    pub phase_id: Option<String>,
    pub deliverable_id: Option<String>,
    pub local_file_id: Option<String>,
    pub relative_path: Option<String>,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchResult {
    pub items: Vec<GlobalSearchItem>,
    #[ts(type = "number")]
    pub total: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedSearchCursor {
    pub provider_id: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnifiedSearchParams {
    pub query: String,
    pub scope: Vec<String>,
    #[ts(type = "number")]
    pub limit: u32,
    #[serde(default)]
    pub cursors: Vec<UnifiedSearchCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedSearchLocalTarget {
    pub project_id: Option<String>,
    pub workspace_id: String,
    pub phase_id: Option<String>,
    pub deliverable_id: Option<String>,
    pub local_file_id: Option<String>,
    pub relative_path: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedSearchDeepLink {
    pub kind: String,
    pub application_id: String,
    pub target: String,
    pub object_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedSearchItem {
    pub hit_type: String,
    pub provider_id: String,
    pub object_type: String,
    pub object_id: String,
    pub title: String,
    pub summary: String,
    pub match_reason: String,
    pub updated_at: Option<String>,
    pub deep_link: Option<UnifiedSearchDeepLink>,
    pub local_target: Option<UnifiedSearchLocalTarget>,
    #[ts(type = "unknown")]
    pub source_identity: serde_json::Value,
    #[ts(type = "unknown | null")]
    pub business_reference: Option<serde_json::Value>,
    #[ts(type = "unknown | null")]
    pub knowledge_reference: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedSearchGroup {
    pub group_id: String,
    pub provider_id: String,
    pub label: String,
    pub kind: String,
    pub items: Vec<UnifiedSearchItem>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedSearchPartialFailure {
    pub group_id: String,
    pub provider_id: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedSearchResult {
    pub request_id: String,
    pub authorization_revision: Option<String>,
    pub groups: Vec<UnifiedSearchGroup>,
    pub partial_failures: Vec<UnifiedSearchPartialFailure>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AttentionCaseItem {
    pub attention_case_id: String,
    pub case_type: String,
    pub project_id: String,
    pub phase_id: Option<String>,
    pub task_id: Option<String>,
    pub deliverable_id: Option<String>,
    pub title: String,
    pub detail: String,
    pub status: String,
    pub responsibility_type: String,
    pub responsibility_label: String,
    pub reporter_label: String,
    pub priority: String,
    #[ts(type = "number | null")]
    pub due_at: Option<i64>,
    pub agent_run_id: Option<String>,
    pub agent_label: Option<String>,
    pub resolution_summary: Option<String>,
    pub decision_options: Option<String>,
    pub decision_impact: Option<String>,
    pub decision_evidence: Option<String>,
    pub decision_recommendation: Option<String>,
    pub decision_consequence: Option<String>,
    pub decision_action: Option<String>,
    pub decision_action_summary: Option<String>,
    pub decision_actor_label: Option<String>,
    pub decision_delegated_to: Option<String>,
    #[ts(type = "number | null")]
    pub decision_action_at: Option<i64>,
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[ts(type = "number | null")]
    pub resolved_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttentionCaseListParams {
    pub project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AttentionCaseListResult {
    pub project_id: String,
    pub cases: Vec<AttentionCaseItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectBlockerCreateParams {
    pub project_id: String,
    pub phase_id: Option<String>,
    pub task_id: Option<String>,
    pub title: String,
    pub reason: String,
    pub owner_label: String,
    pub reporter_label: String,
    /// 以枚举表达，合法取值随契约发布；非法值在反序列化阶段即被拒绝。
    pub priority: WorkPriority,
    #[ts(type = "number | null")]
    pub due_at: Option<i64>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectBlockerCreateResult {
    pub blocker: AttentionCaseItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionRequestCreateParams {
    pub project_id: String,
    pub phase_id: Option<String>,
    pub deliverable_id: Option<String>,
    pub title: String,
    pub question: String,
    pub options: String,
    pub impact: String,
    pub evidence: String,
    pub recommendation: String,
    pub consequence: String,
    pub decision_owner_label: String,
    pub requester_label: String,
    /// 以枚举表达，合法取值随契约发布；非法值在反序列化阶段即被拒绝。
    pub priority: WorkPriority,
    #[ts(type = "number | null")]
    pub due_at: Option<i64>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRequestCreateResult {
    pub decision_request: AttentionCaseItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentIncidentReportParams {
    pub project_id: String,
    pub phase_id: Option<String>,
    pub task_id: Option<String>,
    pub run_id: String,
    pub agent_label: String,
    pub title: String,
    pub error_summary: String,
    pub responsibility_label: String,
    pub reporter_label: String,
    /// 以枚举表达，合法取值随契约发布；非法值在反序列化阶段即被拒绝。
    pub priority: WorkPriority,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentIncidentReportResult {
    pub incident: AttentionCaseItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttentionCaseTransitionParams {
    pub attention_case_id: String,
    pub target_status: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub actor_label: String,
    pub resolution_summary: String,
    pub decision_action: Option<String>,
    pub delegated_to: Option<String>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AttentionCaseTransitionResult {
    pub attention_case: AttentionCaseItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectProgressGetParams {
    pub project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct BusinessTaskItem {
    pub business_task_id: String,
    pub project_id: String,
    pub phase_id: Option<String>,
    pub title: String,
    pub objective: String,
    pub input_context: String,
    pub expected_output: String,
    pub completion_criteria: String,
    pub assignee_type: String,
    pub assignee_label: String,
    pub priority: String,
    #[ts(type = "number | null")]
    pub due_at: Option<i64>,
    #[ts(type = "number")]
    pub attempt_no: i64,
    pub status: String,
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPhaseItem {
    pub phase_id: String,
    pub plan_id: String,
    pub project_id: String,
    pub name: String,
    pub goal: String,
    pub acceptance_criteria: String,
    pub required_deliverable_id: Option<String>,
    pub required_deliverable_title: Option<String>,
    pub required_deliverable_version_id: Option<String>,
    #[ts(type = "number")]
    pub sort_order: i64,
    pub predecessor_phase_ids: Vec<String>,
    pub is_main: bool,
    pub status: String,
    #[ts(type = "number")]
    pub state_revision: i64,
    pub tasks: Vec<BusinessTaskItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct PhasePlanItem {
    pub plan_id: String,
    pub project_id: String,
    #[ts(type = "number")]
    pub version_no: i64,
    pub status: String,
    pub summary: String,
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub activated_at: Option<i64>,
    pub phases: Vec<ProjectPhaseItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProgressGetResult {
    pub project_id: String,
    pub plans: Vec<PhasePlanItem>,
    pub tasks: Vec<BusinessTaskItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhaseDraftItem {
    pub name: String,
    pub goal: String,
    pub acceptance_criteria: String,
    pub required_deliverable_id: Option<String>,
    pub predecessor_indexes: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhasePlanCreateParams {
    pub project_id: String,
    pub summary: String,
    pub phases: Vec<PhaseDraftItem>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct PhasePlanCreateResult {
    pub plan: PhasePlanItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PhasePlanActivateParams {
    pub plan_id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct PhasePlanActivateResult {
    pub plan: PhasePlanItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectPhaseTransitionParams {
    pub phase_id: String,
    pub target_status: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub actor_label: String,
    pub comment: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPhaseTransitionResult {
    pub phase: ProjectPhaseItem,
}

/// 任务责任主体类型。以枚举而非字符串表达，使合法取值出现在协议契约中，
/// 客户端无需查阅服务端源码，非法值也会在反序列化阶段即被拒绝。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskAssigneeType {
    User,
    Agent,
    Collaboration,
}

impl TaskAssigneeType {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::Collaboration => "collaboration",
        }
    }
}

/// 工作优先级。任务、阻塞、决策请求与智能体事故共用同一套档位。
///
/// 此前只有任务用枚举，其余几处是裸字符串：服务端照样按同一套档位校验，写错却要等到
/// 运行时被整条拒绝。事故上报就因此长期写了一个不在档位里的值，而上报失败在界面侧是
/// 静默的，事故卡从来没有建出来过。以枚举表达后非法值在反序列化阶段即被拒绝。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkPriority {
    Low,
    Medium,
    High,
    Critical,
}

impl WorkPriority {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessTaskCreateParams {
    pub project_id: String,
    pub phase_id: String,
    pub title: String,
    pub objective: String,
    pub input_context: String,
    pub expected_output: String,
    pub completion_criteria: String,
    /// 以枚举表达，合法取值随契约发布；非法值在反序列化阶段即被拒绝。
    pub assignee_type: TaskAssigneeType,
    pub assignee_label: String,
    /// 以枚举表达，理由同 `assignee_type`。
    pub priority: WorkPriority,
    #[ts(type = "number")]
    pub due_at: Option<i64>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct BusinessTaskCreateResult {
    pub task: BusinessTaskItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessTaskGetParams {
    pub business_task_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct BusinessTaskContributionItem {
    pub run_id: String,
    pub conversation_id: String,
    pub conversation_status: String,
    pub stage: Option<String>,
    pub outcome_title: Option<String>,
    pub outcome_summary: Option<String>,
    pub reason: String,
    pub confidence: f64,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct BusinessTaskGetResult {
    pub task: BusinessTaskItem,
    pub contributions: Vec<BusinessTaskContributionItem>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessTaskTransitionParams {
    pub business_task_id: String,
    pub target_status: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub actor_label: String,
    pub comment: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct BusinessTaskTransitionResult {
    pub task: BusinessTaskItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessTaskMoveParams {
    pub business_task_id: String,
    pub target_phase_id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub actor_label: String,
    pub dependency_impact: String,
    pub deliverable_impact: String,
    pub acceptance_impact: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct BusinessTaskMoveResult {
    pub task: BusinessTaskItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventReplayParams {
    #[ts(type = "number | null")]
    pub after_sequence: Option<i64>,
    pub limit: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DurableEventItem {
    #[ts(type = "number")]
    pub sequence: i64,
    pub event_id: String,
    pub event_type: String,
    pub workspace_id: Option<String>,
    pub object_type: Option<String>,
    pub object_id: Option<String>,
    pub payload_json: String,
    pub correlation_id: Option<String>,
    #[ts(type = "number")]
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct EventReplayResult {
    pub events: Vec<DurableEventItem>,
    #[ts(type = "number")]
    pub next_sequence: i64,
    pub has_more: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConflictItem {
    pub conflict_id: String,
    pub local_file_id: String,
    pub conflict_type: String,
    pub state: String,
    pub baseline_checksum: Option<String>,
    pub platform_checksum: Option<String>,
    pub local_checksum: Option<String>,
    #[ts(type = "number")]
    pub state_revision: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PreviewSource {
    Local,
    Cloud,
    Baseline,
    PlatformMock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PreviewFormat {
    Image,
    Docx,
    Pptx,
    Xlsx,
    Xmind,
    Markdown,
    Html,
    Pdf,
    PlainText,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PreviewAvailability {
    Ready,
    Unsupported,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenPreviewParams {
    pub workspace_id: String,
    pub local_file_id: String,
    pub source: PreviewSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenPreviewResult {
    pub source: PreviewSource,
    pub availability: PreviewAvailability,
    pub format: PreviewFormat,
    pub relative_path: String,
    #[ts(type = "number | null")]
    pub size_bytes: Option<u64>,
    #[ts(type = "number | null")]
    pub source_modified_at: Option<i64>,
    pub content_revision: Option<String>,
    #[ts(type = "number | null")]
    pub content_bytes: Option<u64>,
    pub ticket_id: Option<String>,
    #[ts(type = "number | null")]
    pub ticket_expires_at: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadBinaryPreviewParams {
    pub workspace_id: String,
    pub local_file_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReadBinaryPreviewResult {
    pub content_base64: String,
    pub content_revision: String,
    pub content_type: String,
    #[ts(type = "number")]
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReadTextResourceParams {
    pub ticket_id: String,
    #[ts(type = "number")]
    pub offset: u64,
    pub max_bytes: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReadTextResourceResult {
    #[ts(type = "number")]
    pub offset: u64,
    #[ts(type = "number")]
    pub next_offset: u64,
    pub end_of_resource: bool,
    pub content_revision: String,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiffSourceRef {
    pub ticket_id: String,
    pub source: PreviewSource,
    pub content_revision: String,
    pub label: String,
    pub version: Option<String>,
    #[ts(type = "number | null")]
    pub modified_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ComputeTextDiffParams {
    pub left: DiffSourceRef,
    pub right: DiffSourceRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct TextDiffSummary {
    pub added_lines: u32,
    pub deleted_lines: u32,
    pub unchanged_lines: u32,
    pub change_blocks: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ComputeTextDiffResult {
    pub diff_ticket_id: String,
    #[ts(type = "number")]
    pub expires_at: i64,
    pub left: DiffSourceRef,
    pub right: DiffSourceRef,
    pub summary: TextDiffSummary,
    pub total_rows: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReadTextDiffParams {
    pub diff_ticket_id: String,
    pub cursor: u32,
    pub max_rows: u32,
    pub max_bytes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum TextDiffRowKind {
    Equal,
    Delete,
    Insert,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct TextDiffRow {
    pub kind: TextDiffRowKind,
    pub left_line: Option<u32>,
    pub right_line: Option<u32>,
    pub text: String,
    pub has_line_terminator: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReadTextDiffResult {
    pub cursor: u32,
    pub next_cursor: Option<u32>,
    pub rows: Vec<TextDiffRow>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmFileParams {
    pub workspace_id: String,
    pub local_file_id: String,
    pub expected_checksum_algorithm: String,
    pub expected_checksum_value: String,
    pub expected_baseline_version_id: Option<String>,
    pub client_operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmFileResult {
    pub confirmation_id: String,
    pub workspace_id: String,
    pub local_file_id: String,
    pub checksum_algorithm: String,
    pub checksum_value: String,
    pub baseline_version_id: Option<String>,
    pub confirmed_by: String,
    #[ts(type = "number")]
    pub confirmed_at: i64,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationHistoryParams {
    pub workspace_id: String,
    pub local_file_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationHistoryItem {
    pub confirmation_id: String,
    pub checksum_algorithm: String,
    pub checksum_value: String,
    pub baseline_version_id: Option<String>,
    pub confirmed_by: String,
    #[ts(type = "number")]
    pub confirmed_at: i64,
    #[ts(type = "number | null")]
    pub invalidated_at: Option<i64>,
    pub invalidation_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmationHistoryResult {
    pub workspace_id: String,
    pub local_file_id: String,
    pub items: Vec<ConfirmationHistoryItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SuccessResponse {
    pub jsonrpc: String,
    pub id: RequestId,
    pub result: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorResponse {
    pub jsonrpc: String,
    pub id: Option<RequestId>,
    pub error: ErrorObject,
}

/// 构造 JSON-RPC 成功响应。
///
/// # Errors
///
/// 当结果值无法序列化为 JSON 时返回序列化错误。
pub fn success<T: Serialize>(id: RequestId, result: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(&SuccessResponse {
        jsonrpc: JSON_RPC_VERSION.to_owned(),
        id,
        result: serde_json::to_value(result)?,
    })
}

/// 构造 JSON-RPC 错误响应。
///
/// # Panics
///
/// 仅当协议内建错误对象无法序列化时触发；其字段均为受支持的 JSON 类型。
#[must_use]
pub fn error(id: Option<RequestId>, code: i32, message: &str, data: Option<Value>) -> String {
    serde_json::to_string(&ErrorResponse {
        jsonrpc: JSON_RPC_VERSION.to_owned(),
        id,
        error: ErrorObject {
            code,
            message: message.to_owned(),
            data,
        },
    })
    .expect("error response uses serializable protocol values")
}

#[must_use]
pub fn business_error(id: RequestId, reason: &str, message: &str, details: Value) -> String {
    let mut data = json!({"reason": reason, "retryable": false});
    if let (Some(target), Value::Object(source)) = (data.as_object_mut(), details) {
        target.extend(source);
    }
    error(Some(id), BUSINESS_ERROR, message, Some(data))
}

/// 解析并严格校验一条客户端 JSON-RPC 请求或通知。
///
/// # Errors
///
/// 失败时返回已经序列化、可直接发送的 JSON-RPC 标准错误响应。
pub fn parse_client_message(bytes: &[u8]) -> Result<ClientMessage, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| {
        error(
            None,
            PARSE_ERROR,
            "Parse error",
            Some(json!({"reason": "invalid_json"})),
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        error(
            None,
            INVALID_REQUEST,
            "Invalid Request",
            Some(json!({"reason": "request_must_be_object"})),
        )
    })?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some(JSON_RPC_VERSION) {
        return Err(error(
            request_id(object.get("id")),
            INVALID_REQUEST,
            "Invalid Request",
            Some(json!({"reason": "invalid_jsonrpc_version"})),
        ));
    }
    let Some(method_value) = object.get("method") else {
        return parse_client_response(object);
    };
    let method = method_value
        .as_str()
        .filter(|method| !method.is_empty())
        .ok_or_else(|| {
            error(
                request_id(object.get("id")),
                INVALID_REQUEST,
                "Invalid Request",
                Some(json!({"reason": "method_required"})),
            )
        })?
        .to_owned();
    let params = object.get("params").cloned();
    if params
        .as_ref()
        .is_some_and(|params| !params.is_object() && !params.is_array())
    {
        return Err(error(
            request_id(object.get("id")),
            INVALID_REQUEST,
            "Invalid Request",
            Some(json!({"reason": "params_must_be_object_or_array"})),
        ));
    }
    match object.get("id") {
        None => Ok(ClientMessage::Notification(Notification { method, params })),
        Some(id) => parse_request_id(id)
            .map(|id| ClientMessage::Request(Request { id, method, params }))
            .ok_or_else(|| {
                error(
                    None,
                    INVALID_REQUEST,
                    "Invalid Request",
                    Some(json!({"reason": "invalid_id"})),
                )
            }),
    }
}

fn parse_client_response(object: &serde_json::Map<String, Value>) -> Result<ClientMessage, String> {
    let id = object.get("id").and_then(parse_request_id).ok_or_else(|| {
        error(
            None,
            INVALID_REQUEST,
            "Invalid Request",
            Some(json!({"reason": "response_id_required"})),
        )
    })?;
    match (object.get("result"), object.get("error")) {
        (Some(result), None) => Ok(ClientMessage::Response(ClientResponse::Success {
            id,
            result: result.clone(),
        })),
        (None, Some(error_value)) => serde_json::from_value::<ErrorObject>(error_value.clone())
            .map(|error| ClientMessage::Response(ClientResponse::Error { id, error }))
            .map_err(|_| {
                error(
                    None,
                    INVALID_REQUEST,
                    "Invalid Request",
                    Some(json!({"reason": "invalid_error_response"})),
                )
            }),
        _ => Err(error(
            Some(id),
            INVALID_REQUEST,
            "Invalid Request",
            Some(json!({"reason": "response_requires_exactly_one_result_or_error"})),
        )),
    }
}

fn request_id(value: Option<&Value>) -> Option<RequestId> {
    value.and_then(parse_request_id)
}

fn parse_request_id(value: &Value) -> Option<RequestId> {
    match value {
        Value::String(value) => Some(RequestId::String(value.clone())),
        Value::Number(value) => value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .map(RequestId::Integer),
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Frame {
    Message(Vec<u8>),
    TooLarge,
}

#[derive(Debug)]
pub struct JsonlDecoder {
    buffer: Vec<u8>,
    discarding_oversized: bool,
    max_message_bytes: usize,
}

impl JsonlDecoder {
    #[must_use]
    pub fn new(max_message_bytes: usize) -> Self {
        Self {
            buffer: Vec::new(),
            discarding_oversized: false,
            max_message_bytes,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<Frame> {
        let mut frames = Vec::new();
        for &byte in chunk {
            if self.discarding_oversized {
                if byte == b'\n' {
                    self.discarding_oversized = false;
                    frames.push(Frame::TooLarge);
                }
                continue;
            }
            if byte == b'\n' {
                if self.buffer.last() == Some(&b'\r') {
                    self.buffer.pop();
                }
                frames.push(Frame::Message(std::mem::take(&mut self.buffer)));
            } else if self.buffer.len() == self.max_message_bytes {
                self.buffer.clear();
                self.discarding_oversized = true;
            } else {
                self.buffer.push(byte);
            }
        }
        frames
    }
}

impl Default for JsonlDecoder {
    fn default() -> Self {
        Self::new(MAX_MESSAGE_BYTES)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ProtocolVersion {
    major: u32,
    minor: u32,
}

impl ProtocolVersion {
    fn parse(value: &str) -> Option<Self> {
        let (major, minor) = value.split_once('.')?;
        if minor.contains('.') || major.is_empty() || minor.is_empty() {
            return None;
        }
        Some(Self {
            major: major.parse().ok()?,
            minor: minor.parse().ok()?,
        })
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

/// 在客户端声明范围和服务端当前版本之间协商最高可用版本。
///
/// # Errors
///
/// 版本格式、范围次序无效或与服务端版本无交集时返回稳定原因码。
pub fn negotiate_protocol(range: &ProtocolRange) -> Result<String, &'static str> {
    let client_min = ProtocolVersion::parse(&range.min_version).ok_or("invalid_version_format")?;
    let client_max = ProtocolVersion::parse(&range.max_version).ok_or("invalid_version_format")?;
    if client_min > client_max {
        return Err("invalid_version_range");
    }
    let server = ProtocolVersion { major: 1, minor: 0 };
    if client_min <= server && server <= client_max {
        Ok(server.to_string())
    } else {
        Err("protocol_version_unsupported")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_supports_chunking_multiple_frames_and_crlf() {
        let mut decoder = JsonlDecoder::new(64);
        assert!(decoder.push(b"{\"a\":").is_empty());
        assert_eq!(
            decoder.push(b"1}\r\n{}\n"),
            vec![
                Frame::Message(b"{\"a\":1}".to_vec()),
                Frame::Message(b"{}".to_vec())
            ]
        );
    }

    #[test]
    fn decoder_bounds_memory_and_recovers_after_oversized_frame() {
        let mut decoder = JsonlDecoder::new(4);
        assert_eq!(
            decoder.push(b"12345\n{}\n"),
            vec![Frame::TooLarge, Frame::Message(b"{}".to_vec())]
        );
        assert!(decoder.buffer.capacity() <= 8);
    }

    #[test]
    fn parser_distinguishes_request_and_notification() {
        let request =
            parse_client_message(br#"{"jsonrpc":"2.0","id":"r1","method":"ping","params":{}}"#)
                .unwrap();
        assert!(
            matches!(request, ClientMessage::Request(Request { id: RequestId::String(id), .. }) if id == "r1")
        );
        let notification =
            parse_client_message(br#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#)
                .unwrap();
        assert!(matches!(notification, ClientMessage::Notification(_)));
        let response = parse_client_message(
            br#"{"jsonrpc":"2.0","id":"server_request_1","result":{"confirmed":true}}"#,
        )
        .unwrap();
        assert!(matches!(
            response,
            ClientMessage::Response(ClientResponse::Success { id: RequestId::String(id), .. }) if id == "server_request_1"
        ));
    }

    #[test]
    fn negotiation_rejects_invalid_and_disjoint_ranges() {
        assert_eq!(
            negotiate_protocol(&ProtocolRange {
                min_version: "1.0".into(),
                max_version: "1.1".into()
            }),
            Ok("1.0".into())
        );
        assert_eq!(
            negotiate_protocol(&ProtocolRange {
                min_version: "2.0".into(),
                max_version: "2.1".into()
            }),
            Err("protocol_version_unsupported")
        );
        assert_eq!(
            negotiate_protocol(&ProtocolRange {
                min_version: "1".into(),
                max_version: "1.0".into()
            }),
            Err("invalid_version_format")
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileHydrationParams {
    pub workspace_id: String,
    pub relative_path: String,
    pub operation_id: Option<String>,
    #[serde(default)]
    pub cancel: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileHydrationResult {
    pub state: String,
    pub relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}
