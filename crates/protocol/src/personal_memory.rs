//! 个人记忆授权状态与跨全部受管原生作用域的显式删除操作。
use crate::{ModelExecutionOwner, PersonalDataQuiescence};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryPreferences {
    pub enabled: bool,
    pub from_tool_chats: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryEnableConfirmation {
    pub consent_digest: String,
    pub from_tool_chats: bool,
    #[ts(type = "number")]
    pub confirmed_at: i64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryAuthorization {
    pub preferences: PersonalMemoryPreferences,
    pub confirmation: Option<PersonalMemoryEnableConfirmation>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryParams {}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalMemoryResetState {
    Prepared,
    Applying,
    Completed,
    Incomplete,
    Cancelled,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetScope {
    pub scope_id: String,
    pub native_home_relative_path: String,
    pub scope_kind: String,
    pub visible_memory_file_count: Option<u32>,
    #[ts(type = "number | null")]
    pub visible_memory_logical_bytes: Option<u64>,
    pub inspection_issue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub retained_native_paths: Option<Vec<String>>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalMemoryResetOutcome {
    Pending,
    Reset,
    Failed,
    Unconfirmed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetScopeResult {
    pub scope_id: String,
    pub outcome: PersonalMemoryResetOutcome,
    pub writer_lease_id: Option<String>,
    pub official_response_received: bool,
    pub process_exit_confirmed: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetOperation {
    pub operation_id: String,
    pub profile_id: String,
    pub state: PersonalMemoryResetState,
    pub preview_digest: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub scopes: Vec<PersonalMemoryResetScope>,
    pub scope_results: Vec<PersonalMemoryResetScopeResult>,
    pub blockers: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetPrepareParams {
    pub client_operation_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetParams {
    pub operation_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetConfirmParams {
    pub operation_id: String,
    pub preview_digest: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetStartParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub quiescence: Option<PersonalDataQuiescence>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetPermit {
    pub operation_id: String,
    pub preview_digest: String,
    pub scope_id: String,
    pub native_home_relative_path: String,
    pub native_home_fingerprint: String,
    pub permit_token: String,
    pub owner: ModelExecutionOwner,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetStartResult {
    pub operation: PersonalMemoryResetOperation,
    pub permits: Vec<PersonalMemoryResetPermit>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetWriterParams {
    pub permit: PersonalMemoryResetPermit,
    pub client_operation_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetScopeCompleteParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub scope_id: String,
    pub writer_lease_id: Option<String>,
    pub owner: ModelExecutionOwner,
    #[ts(type = "unknown | null")]
    pub official_response: Option<Value>,
    pub failure_reason: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetFinishParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub owner: ModelExecutionOwner,
}
