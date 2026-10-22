//! 本地已安装个人页面的独立 AI 声明与宿主私有绑定。
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalPageRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalPageConfirmationPolicy {
    Always,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageContextType {
    pub context_type: String,
    pub schema_version: String,
    pub data_schema: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageUiAction {
    pub action_id: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub risk_level: PersonalPageRiskLevel,
    pub preconditions: Vec<String>,
    pub expected_effects: Vec<String>,
    pub confirmation_policy: PersonalPageConfirmationPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageTool {
    pub name: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageAiManifest {
    pub schema_version: u32,
    pub context_types: Vec<PersonalPageContextType>,
    pub ui_actions: Vec<PersonalPageUiAction>,
    pub model: bool,
    pub conversation_handoff: bool,
    pub read_tools: Vec<PersonalPageTool>,
    pub write_tools: Vec<PersonalPageTool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageBindingParams {
    pub creation_project_id: String,
    pub expected_package_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageRuntimeBinding {
    pub package_root: String,
    pub data_root: String,
    pub harness_home: String,
    pub package_digest: String,
    #[ts(type = "number")]
    pub revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageBindingSnapshot {
    pub profile_id: String,
    pub creation_project_id: String,
    pub application_id: String,
    pub name: String,
    pub version: String,
    pub package_digest: String,
    pub permission_digest: String,
    pub source_digest: String,
    pub target_type: String,
    pub permissions: Vec<String>,
    pub enabled: bool,
    #[ts(type = "number")]
    pub installation_revision: i64,
    #[ts(type = "number")]
    pub project_revision: i64,
    pub active_data_slot_id: String,
    pub data_revision: String,
    pub page_ai: PersonalPageAiManifest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub runtime_binding: Option<PersonalPageRuntimeBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalPageAuditMethod {
    SessionOpen,
    SessionRead,
    SessionClose,
    ContextPublish,
    ContextRevoke,
    UiActionsRegister,
    UiActionInvoke,
    ModelSessionCreate,
    ModelSessionRead,
    ModelMessageRead,
    ModelRunCreate,
    ModelRunRead,
    ModelRunCancel,
    ModelWriteResolve,
    ConversationHandoffPrepare,
    ConversationHandoffCommit,
    ToolRead,
    ToolWrite,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalPageAuditOutcome {
    Succeeded,
    Rejected,
    Cancelled,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageAuditPrepareParams {
    pub operation_id: String,
    pub binding: PersonalPageBindingParams,
    pub permission_digest: String,
    pub document_id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub request_id: String,
    pub method: PersonalPageAuditMethod,
    pub arguments_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageAuditFinishParams {
    pub operation_id: String,
    pub outcome: PersonalPageAuditOutcome,
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageAuditRecord {
    pub profile_id: String,
    pub request: PersonalPageAuditPrepareParams,
    #[ts(type = "number")]
    pub prepared_at: i64,
    #[ts(type = "number | null")]
    pub finished_at: Option<i64>,
    pub outcome: Option<PersonalPageAuditOutcome>,
    pub reason_code: Option<String>,
    pub read_only_after_restore: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageAuditListParams {
    pub creation_project_id: String,
    pub limit: u32,
    #[ts(type = "number | null")]
    pub before_sequence: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageAuditEvent {
    #[ts(type = "number")]
    pub sequence: i64,
    pub record: PersonalPageAuditRecord,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPageAuditListResult {
    pub items: Vec<PersonalPageAuditEvent>,
    #[ts(type = "number | null")]
    pub next_before_sequence: Option<i64>,
}
