//! 个人连接、用途绑定和不可变执行快照；不承载凭据正文。
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

pub const PERSONAL_DATA_VERSION: u32 = 2;
pub const MODEL_CONNECTION_PROTOCOL: u32 = 1;
pub const MODEL_ENTRY_PREFIX: &str = "personalModel:";
pub const MODEL_ENTRY_OWNER: &str = "personal_model";
pub const MODEL_CREDENTIAL_SERVICE: &str = "cn.cilai.personal.workbench.vault.model";
pub const MODEL_NETWORK_CREDENTIAL_SERVICE: &str =
    "cn.cilai.personal.workbench.vault.model-network";
pub const LEGACY_MODEL_CREDENTIAL_SERVICE: &str = "cn.cilai.personal.workbench.model";
pub const LEGACY_MODEL_NETWORK_CREDENTIAL_SERVICE: &str = "cn.cilai.personal.workbench.proxy";

#[must_use]
pub fn is_legacy_model_credential_reference(reference: &ModelSecretRef) -> bool {
    matches!(
        reference.service.as_str(),
        LEGACY_MODEL_CREDENTIAL_SERVICE | LEGACY_MODEL_NETWORK_CREDENTIAL_SERVICE
    )
}
pub const MODEL_LAUNCH_PROTOCOL: u32 = 1;

mod network_credentials;
pub use network_credentials::*;

#[must_use]
pub fn model_runtime_home_scope(
    profile_id: &str,
    connection_id: &str,
    auth_generation: u32,
) -> String {
    format!("{profile_id}:{connection_id}:{auth_generation}")
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionKind {
    OpenaiKey,
    Chatgpt,
    CustomResponses,
    Local,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelAuthMode {
    None,
    ApiKey,
    RuntimeManagedChatgpt,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
pub enum LocalModelRuntime {
    #[serde(rename = "ollama")]
    Ollama,
    #[serde(rename = "lmstudio")]
    LmStudio,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSecretRef {
    pub service: String,
    pub account: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelNetworkMode {
    #[default]
    Direct,
    SystemProxy,
    CustomProxy,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelNetworkConfig {
    #[serde(default)]
    pub mode: ModelNetworkMode,
    pub proxy_url: Option<String>,
    pub proxy_secret_ref: Option<ModelSecretRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ca_certificate_pem: Option<String>,
    #[serde(default)]
    pub proxy_auth: ModelNetworkAuthentication,
    #[serde(default)]
    pub service_auth: ModelNetworkAuthentication,
    #[serde(default)]
    pub header_names: Vec<String>,
}

/// # Errors
/// 仅接受有界的公开 CA 证书 PEM；密钥、其他 PEM 对象和混杂内容均拒绝。
pub fn validate_model_ca_pem_shape(pem: &str) -> Result<(), &'static str> {
    if pem.len() > 131_072 || pem.is_empty() {
        return Err("model_ca_certificate_invalid");
    }
    let mut remaining = pem.trim();
    let mut count = 0;
    while !remaining.is_empty() {
        let body = remaining
            .strip_prefix("-----BEGIN CERTIFICATE-----")
            .ok_or("model_ca_certificate_invalid")?;
        let (body, rest) = body
            .split_once("-----END CERTIFICATE-----")
            .ok_or("model_ca_certificate_invalid")?;
        if body.trim().len() < 16
            || !body.bytes().all(|b| {
                b.is_ascii_alphanumeric()
                    || matches!(b, b'+' | b'/' | b'=' | b' ' | b'\t' | b'\r' | b'\n')
            })
        {
            return Err("model_ca_certificate_invalid");
        }
        count += 1;
        if count > 32 {
            return Err("model_ca_certificate_invalid");
        }
        remaining = rest.trim();
    }
    if count == 0 {
        return Err("model_ca_certificate_invalid");
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionConfig {
    pub name: String,
    pub kind: ModelConnectionKind,
    pub base_url: Option<String>,
    pub local_runtime: Option<LocalModelRuntime>,
    pub auth_mode: ModelAuthMode,
    pub enabled: bool,
    #[serde(default)]
    pub network: ModelNetworkConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnection {
    pub profile_id: String,
    pub connection_id: String,
    pub config_revision: u32,
    pub auth_generation: u32,
    #[serde(default)]
    pub auth_scopes: Vec<ModelAuthScope>,
    pub config: ModelConnectionConfig,
    pub secret_ref: Option<ModelSecretRef>,
    pub deleted: bool,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelCredentialAction {
    #[default]
    Keep,
    Replace,
    Clear,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionSavePrepareParams {
    pub operation_id: String,
    pub connection_id: Option<String>,
    pub expected_config_revision: Option<u32>,
    pub config: ModelConnectionConfig,
    pub credential_action: ModelCredentialAction,
    #[serde(default)]
    pub network_credential_action: ModelCredentialAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionChangeState {
    Prepared,
    Committed,
    Aborted,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelCredentialCleanupState {
    NotRequired,
    Pending,
    Confirmed,
    #[default]
    Unconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionChange {
    pub intent: ModelConnectionChangeIntent,
    pub operation_id: String,
    pub request_digest: String,
    pub expected_config_revision: Option<u32>,
    pub connection: ModelConnection,
    pub credential_action: ModelCredentialAction,
    #[serde(default)]
    pub network_credential_action: ModelCredentialAction,
    pub state: ModelConnectionChangeState,
    #[serde(default)]
    pub credential_cleanup: ModelCredentialCleanupState,
    #[ts(type = "number")]
    pub prepared_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionCommitParams {
    pub operation_id: String,
    /// Host 在工作台凭据库写入并读回核验后提交此引用；不表示认证/推理已通过。
    pub credential_ref: Option<ModelSecretRef>,
    #[serde(default)]
    pub network_credential_ref: Option<ModelSecretRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCredentialCleanupAckParams {
    pub operation_id: String,
    pub credential_ref: ModelSecretRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelCredentialCleanupPlanState {
    Pending,
    Confirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCredentialCleanupPlan {
    pub cleanup_id: String,
    pub profile_id: String,
    pub connection_id: String,
    pub auth_generation: u32,
    pub credential_ref: ModelSecretRef,
    pub reference_digest: String,
    pub state: ModelCredentialCleanupPlanState,
    #[ts(type = "number")]
    pub prepared_at: i64,
    #[ts(type = "number | null")]
    pub confirmed_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCredentialCleanupCandidate {
    pub cleanup_id: String,
    pub profile_id: String,
    pub connection_id: String,
    pub auth_generation: u32,
    pub credential_ref: ModelSecretRef,
    pub source_operation_ids: Vec<String>,
    pub deleted_connection: bool,
    pub blocked_references: Vec<ModelConnectionReference>,
    pub active_lease_ids: Vec<String>,
    pub can_prepare: bool,
    pub reference_digest: String,
    pub state: ModelCredentialCleanupState,
    pub plan: Option<ModelCredentialCleanupPlan>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCredentialCleanupPrepareParams {
    pub cleanup_id: String,
    pub expected_reference_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCredentialCleanupConfirmParams {
    pub cleanup_id: String,
    pub expected_reference_digest: String,
    pub credential_ref: ModelSecretRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelOperationParams {
    pub operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionParams {
    pub connection_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelEvidenceSource {
    UserDeclared,
    ServiceDeclared,
    RuntimeCatalog,
    Observed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapabilityState {
    Unverified,
    Supported,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCapabilityEvidence {
    pub state: ModelCapabilityState,
    pub source: ModelEvidenceSource,
    pub evidence_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelDescriptor {
    pub descriptor_id: String,
    pub connection_id: String,
    pub model_id: String,
    pub display_name: String,
    pub revision: u32,
    pub config_revision: u32,
    pub auth_generation: u32,
    pub source: ModelEvidenceSource,
    pub capabilities: BTreeMap<String, ModelCapabilityEvidence>,
    #[ts(type = "number | null")]
    pub context_window_tokens: Option<u64>,
    #[serde(default)]
    pub input_modalities: Vec<String>,
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<String>,
    pub runtime_version: Option<String>,
    pub service_version: Option<String>,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelDescriptorSaveParams {
    pub connection_id: String,
    pub model_id: String,
    pub display_name: String,
    pub expected_revision: Option<u32>,
    #[ts(type = "number | null")]
    pub context_window_tokens: Option<u64>,
    #[serde(default)]
    pub input_modalities: Vec<String>,
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelDescriptorObservation {
    pub model_id: String,
    pub display_name: String,
    pub source: ModelEvidenceSource,
    pub capabilities: BTreeMap<String, ModelCapabilityEvidence>,
    #[ts(type = "number | null")]
    pub context_window_tokens: Option<u64>,
    #[serde(default)]
    pub input_modalities: Vec<String>,
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<String>,
    pub runtime_version: Option<String>,
    pub service_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelDescriptorObserveParams {
    pub connection_id: String,
    pub expected_config_revision: u32,
    pub expected_auth_generation: u32,
    pub models: Vec<ModelDescriptorObservation>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelPurpose {
    Foreground,
    Automation,
    Review,
    Memory,
    PageAi,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelSelectionPolicy {
    Explicit,
    InheritDefault,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSelection {
    pub connection_id: String,
    pub model_id: String,
    pub reasoning_effort: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelHistoryConsent {
    pub service_name: String,
    pub history_scope: String,
    pub may_incur_cost: bool,
    #[ts(type = "number")]
    pub confirmed_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelPurposeBinding {
    pub binding_id: String,
    pub purpose: ModelPurpose,
    pub scope_id: Option<String>,
    pub policy: ModelSelectionPolicy,
    pub selection: Option<ModelSelection>,
    pub enabled: bool,
    pub consent: Option<ModelHistoryConsent>,
    pub revision: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelPurposeSaveParams {
    pub binding: ModelPurposeBinding,
    pub expected_revision: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelValidationLayer {
    Configuration,
    Network,
    Authentication,
    Protocol,
    Catalog,
    Inference,
    Streaming,
    Tools,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelValidationOutcome {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelValidationPrepareParams {
    #[serde(default)]
    pub runtime_kind: ModelRuntimeKind,
    pub connection_id: String,
    pub model_id: Option<String>,
    pub layer: ModelValidationLayer,
    pub expected_config_revision: u32,
    pub expected_auth_generation: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelValidationRecord {
    pub runtime_kind: ModelRuntimeKind,
    pub validation_id: String,
    pub connection_id: String,
    pub model_id: Option<String>,
    pub layer: ModelValidationLayer,
    pub config_revision: u32,
    pub auth_generation: u32,
    pub outcome: Option<ModelValidationOutcome>,
    pub completion_revision: Option<u32>,
    pub reason: Option<String>,
    pub runtime_version: Option<String>,
    pub service_version: Option<String>,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelValidationReportParams {
    pub validation_id: String,
    pub outcome: ModelValidationOutcome,
    pub reason: Option<String>,
    pub runtime_version: Option<String>,
    pub service_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelValidationExecutionParams {
    pub validation_id: String,
    pub owner: ModelExecutionOwner,
    pub launch_protocol: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionPrepareParams {
    pub owner: Option<ModelExecutionOwner>,
    #[serde(default)]
    pub launch_protocol: u32,
    pub purpose: ModelPurpose,
    pub binding_id: Option<String>,
    pub selection: Option<ModelSelection>,
    pub conversation_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionSnapshot {
    pub validation_id: Option<String>,
    pub runtime_kind: ModelRuntimeKind,
    pub profile_id: String,
    pub instance_fence: String,
    pub connection_id: String,
    pub kind: ModelConnectionKind,
    pub config_revision: u32,
    pub auth_generation: u32,
    pub purpose: ModelPurpose,
    pub model_id: String,
    pub reasoning_effort: Option<String>,
    pub base_url: Option<String>,
    pub local_runtime: Option<LocalModelRuntime>,
    pub auth_mode: ModelAuthMode,
    pub network: ModelNetworkConfig,
    pub secret_ref: Option<ModelSecretRef>,
    pub runtime_home_scope: String,
    pub descriptor_revision: u32,
    pub capabilities: BTreeMap<String, ModelCapabilityEvidence>,
    #[ts(type = "number | null")]
    pub context_window_tokens: Option<u64>,
    #[serde(default)]
    pub input_modalities: Vec<String>,
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<String>,
    pub binding_revision: Option<u32>,
    pub binding_id: Option<String>,
    pub conversation_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionLease {
    pub lease_id: String,
    pub snapshot: ModelExecutionSnapshot,
    pub snapshot_digest: String,
    pub released: bool,
    pub owner: ModelExecutionOwner,
    pub processes: Vec<ModelExecutionProcess>,
    pub execution_started: bool,
    pub launch_protocol: u32,
    #[ts(type = "number")]
    pub created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionLeaseParams {
    pub lease_id: String,
    pub snapshot_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionListParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionMutation {
    Disable,
    Delete,
    Disconnect,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionPreflightParams {
    pub connection_id: String,
    pub action: ModelConnectionMutation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionReference {
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionPreflight {
    pub connection_id: String,
    pub action: ModelConnectionMutation,
    pub config_revision: u32,
    pub impact_digest: String,
    pub references: Vec<ModelConnectionReference>,
    pub active_lease_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionMutationParams {
    pub connection_id: String,
    pub expected_config_revision: u32,
    pub impact_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConnectionSetEnabledParams {
    pub connection_id: String,
    pub expected_config_revision: u32,
    pub enabled: bool,
    pub impact_digest: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalProfileManifest {
    pub schema_version: u32,
    pub profile_id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelThreadRuntimeScope {
    pub profile_id: String,
    pub connection_id: String,
    pub config_revision: u32,
    pub auth_generation: u32,
    pub runtime_home_scope: String,
    pub runtime_kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionOwner {
    pub instance_id: String,
    pub pid: u32,
    pub start_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionProcess {
    pub pid: u32,
    pub start_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionRegisterProcessParams {
    pub lease_id: String,
    pub snapshot_digest: String,
    pub process: ModelExecutionProcess,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionAbortUnlaunchedParams {
    pub lease_id: String,
    pub snapshot_digest: String,
    pub owner: ModelExecutionOwner,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelExecutionRecoveryState {
    Released,
    OwnerAlive,
    ProcessAlive,
    Unconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionLeaseDiagnostic {
    pub lease_id: String,
    pub owner: ModelExecutionOwner,
    pub processes: Vec<ModelExecutionProcess>,
    pub state: ModelExecutionRecoveryState,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionRecoverParams {
    pub connection_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelExecutionRecoverResult {
    pub connection_id: String,
    pub leases: Vec<ModelExecutionLeaseDiagnostic>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelRuntimeKind {
    #[default]
    Workbench,
    Application,
}

impl ModelRuntimeKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workbench => "workbench",
            Self::Application => "application",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionChangeIntent {
    ConnectionSave,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelAuthScopeState {
    Active,
    Retired,
    SignedOut,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthScope {
    pub runtime_kind: ModelRuntimeKind,
    pub auth_generation: u32,
    pub state: ModelAuthScopeState,
    pub operation_id: String,
}

#[must_use]
pub fn model_purpose_runtime_kind(purpose: &ModelPurpose) -> ModelRuntimeKind {
    if *purpose == ModelPurpose::PageAi {
        ModelRuntimeKind::Application
    } else {
        ModelRuntimeKind::Workbench
    }
}

#[must_use]
pub fn model_active_auth_generation(
    connection: &ModelConnection,
    kind: ModelRuntimeKind,
) -> Option<u32> {
    if connection.config.kind != ModelConnectionKind::Chatgpt {
        return Some(connection.auth_generation);
    }
    connection
        .auth_scopes
        .iter()
        .find(|s| s.runtime_kind == kind && s.state == ModelAuthScopeState::Active)
        .map(|s| s.auth_generation)
}

#[must_use]
pub fn model_validation_auth_generation(
    connection: &ModelConnection,
    kind: ModelRuntimeKind,
    layer: &ModelValidationLayer,
) -> Option<u32> {
    if matches!(
        layer,
        ModelValidationLayer::Configuration
            | ModelValidationLayer::Network
            | ModelValidationLayer::Catalog
    ) {
        Some(0)
    } else {
        model_active_auth_generation(connection, kind)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelAuthIntent {
    ChatgptLogin,
    ChatgptLogout,
    ChatgptStatus,
    ModelCatalog,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelAuthChangeState {
    Prepared,
    InProgress,
    Completed,
    Cancelled,
    Uncertain,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelAuthStepState {
    Pending,
    Started,
    Confirmed,
    SignedOut,
    Failed,
    Uncertain,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthStep {
    pub runtime_kind: ModelRuntimeKind,
    pub auth_generation: u32,
    pub state: ModelAuthStepState,
    pub login_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RootAuthChange {
    pub operation_id: String,
    pub profile_id: String,
    pub connection_id: String,
    pub intent: ModelAuthIntent,
    pub state: ModelAuthChangeState,
    pub expected_config_revision: u32,
    pub target_generation: u32,
    pub runtime_kind: Option<ModelRuntimeKind>,
    pub old_scopes: Vec<ModelAuthScope>,
    pub steps: Vec<ModelAuthStep>,
    pub owner: ModelExecutionOwner,
    pub instance_fence: String,
    pub launch_protocol: u32,
    pub processes: Vec<ModelAuthProcess>,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthStartPrepareParams {
    pub operation_id: String,
    pub connection_id: String,
    pub expected_config_revision: u32,
    pub runtime_kind: ModelRuntimeKind,
    pub owner: Option<ModelExecutionOwner>,
    pub launch_protocol: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthLogoutPrepareParams {
    pub operation_id: String,
    pub connection_id: String,
    pub expected_config_revision: u32,
    pub owner: Option<ModelExecutionOwner>,
    pub launch_protocol: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthProcess {
    pub runtime_kind: ModelRuntimeKind,
    pub auth_generation: u32,
    pub process: ModelExecutionProcess,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthRegisterProcessParams {
    pub operation_id: String,
    pub runtime_kind: ModelRuntimeKind,
    pub auth_generation: u32,
    pub process: ModelExecutionProcess,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthRecoverParams {
    pub operation_id: String,
    pub owner: ModelExecutionOwner,
    pub launch_protocol: u32,
    #[serde(default)]
    pub allow_unstarted_login_cleanup: bool,
    #[serde(default)]
    pub allow_unstarted_readonly_cleanup: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthAbortUnlaunchedParams {
    pub operation_id: String,
    pub owner: ModelExecutionOwner,
    pub launch_protocol: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthStepParams {
    pub operation_id: String,
    pub runtime_kind: ModelRuntimeKind,
    pub auth_generation: u32,
    pub state: ModelAuthStepState,
    pub login_id: Option<String>,
    pub reason: Option<String>,
}

#[cfg(test)]
mod certificate_contract_tests {
    use super::*;

    #[test]
    fn old_network_config_keeps_system_trust_and_rejects_tls_bypass_fields() {
        let original = serde_json::json!({"mode":"direct","proxyUrl":null,"proxySecretRef":null});
        let config: ModelNetworkConfig = serde_json::from_value(original.clone()).unwrap();
        assert!(config.ca_certificate_pem.is_none());
        assert_eq!(config.proxy_auth, ModelNetworkAuthentication::None);
        assert_eq!(config.service_auth, ModelNetworkAuthentication::None);
        assert!(config.header_names.is_empty());
        let mut expected = original.clone();
        expected["proxyAuth"] = serde_json::json!("none");
        expected["serviceAuth"] = serde_json::json!("none");
        expected["headerNames"] = serde_json::json!([]);
        assert_eq!(serde_json::to_value(&config).unwrap(), expected);
        let mut bypass = original;
        bypass["acceptInvalidCertificates"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ModelNetworkConfig>(bypass).is_err());
    }

    #[test]
    fn certificate_field_cannot_carry_private_keys_or_mixed_pem_objects() {
        let cert =
            "-----BEGIN CERTIFICATE-----\nQUJDREVGR0hJSktMTU5PUA==\n-----END CERTIFICATE-----";
        assert!(validate_model_ca_pem_shape(cert).is_ok());
        for invalid in [
            cert.replace("CERTIFICATE", "PRIVATE KEY"),
            format!(
                "{cert}\n-----BEGIN PRIVATE KEY-----\nQUJDREVGR0hJSktMTU5PUA==\n-----END PRIVATE KEY-----"
            ),
            format!("{cert}\nprivate-component-secret"),
            cert.repeat(33),
            " ".repeat(131_073),
        ] {
            assert_eq!(
                validate_model_ca_pem_shape(&invalid).unwrap_err(),
                "model_ca_certificate_invalid"
            );
        }
    }
}
