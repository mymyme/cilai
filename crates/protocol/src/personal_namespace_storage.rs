//! 个人应用命名空间固定日志及既有浏览器 HTTP 缓存的一次执行和持久回执。
use crate::{
    ModelExecutionOwner, ModelExecutionProcess, PersonalDataQuiescence,
    PersonalStorageOperationState,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalNamespaceStorageKind {
    HostServiceLogs,
    TestDiagnosticsLogs,
    BrowserHttpCache,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalNamespaceLogFile {
    HostServiceCurrent,
    HostServicePrevious,
    TestDiagnosticsCurrent,
    TestDiagnosticsPrevious,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceLogSnapshot {
    pub file: PersonalNamespaceLogFile,
    pub exists: bool,
    pub device: Option<String>,
    pub inode: Option<String>,
    pub mode: Option<String>,
    #[ts(type = "number | null")]
    pub logical_bytes: Option<u64>,
    pub modified_nanoseconds: Option<String>,
    pub changed_nanoseconds: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceBrowserSnapshot {
    pub partition: String,
    pub storage_path: String,
    #[ts(type = "number")]
    pub cache_bytes: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceStoragePrepareParams {
    pub client_operation_id: String,
    pub item_kinds: Vec<PersonalNamespaceStorageKind>,
    pub shell_owner: ModelExecutionOwner,
    pub namespace_path: String,
    pub log_segment_id: Option<String>,
    pub log_files: Vec<PersonalNamespaceLogSnapshot>,
    pub browser_session: Option<PersonalNamespaceBrowserSnapshot>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalNamespaceActionState {
    Planned,
    Started,
    Acknowledged,
    Unconfirmed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalNamespaceActionOutcome {
    Succeeded,
    Failed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceActionReceipt {
    pub action_id: String,
    pub kind: PersonalNamespaceStorageKind,
    pub state: PersonalNamespaceActionState,
    pub outcome: Option<PersonalNamespaceActionOutcome>,
    pub reason_code: Option<String>,
    #[ts(type = "number | null")]
    pub started_at: Option<i64>,
    #[ts(type = "number | null")]
    pub acknowledged_at: Option<i64>,
    #[ts(type = "number | null")]
    pub after_cache_bytes: Option<u64>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceStorageOperation {
    pub operation_id: String,
    pub profile_id: String,
    pub namespace_path: String,
    pub namespace_identity: String,
    pub shell_owner: ModelExecutionOwner,
    pub state: PersonalStorageOperationState,
    pub preview_digest: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub log_segment_id: Option<String>,
    pub log_files: Vec<PersonalNamespaceLogSnapshot>,
    pub browser_session: Option<PersonalNamespaceBrowserSnapshot>,
    pub actions: Vec<PersonalNamespaceActionReceipt>,
    pub blockers: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceStorageParams {
    pub operation_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceStorageConfirmParams {
    pub operation_id: String,
    pub preview_digest: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceStorageStartParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub quiescence: PersonalDataQuiescence,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceActionPermit {
    pub action_id: String,
    pub permit_token: String,
    pub kind: PersonalNamespaceStorageKind,
    pub shell_owner: ModelExecutionOwner,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceStorageStartResult {
    pub operation: PersonalNamespaceStorageOperation,
    pub permits: Vec<PersonalNamespaceActionPermit>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceActionParams {
    pub operation_id: String,
    pub action_id: String,
    pub permit_token: String,
    pub shell_owner: ModelExecutionOwner,
    pub execution_processes: Vec<ModelExecutionProcess>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceActionAckParams {
    pub operation_id: String,
    pub action_id: String,
    pub permit_token: String,
    pub shell_owner: ModelExecutionOwner,
    pub outcome: PersonalNamespaceActionOutcome,
    pub reason_code: Option<String>,
    #[ts(type = "number | null")]
    pub after_cache_bytes: Option<u64>,
}
