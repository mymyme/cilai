//! 用户 MCP 的连接、显式授权与执行账本；不承载秘密正文。
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const MCP_CREDENTIAL_SERVICE: &str = "cn.cilai.personal.workbench.vault.mcp";
pub const LEGACY_MCP_CREDENTIAL_SERVICE: &str = "cn.cilai.personal.workbench.mcp";
pub const MCP_LAUNCH_PROTOCOL: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum McpPurpose {
    Foreground,
    Automation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpSource {
    LocalCommand {
        program: String,
        args: Vec<String>,
        cwd: String,
        permissions: McpLocalPermissions,
    },
    RemoteHttp {
        url: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpLocalPermissions {
    pub read_roots: Vec<String>,
    pub write_roots: Vec<String>,
    pub network: bool,
    pub child_processes: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum McpAuthMode {
    None,
    Bearer,
    Environment,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpNetworkConfig {
    pub mode: crate::ModelNetworkMode,
    pub proxy_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConnectionConfig {
    pub name: String,
    pub source: McpSource,
    pub auth_mode: McpAuthMode,
    pub environment_variables: Vec<String>,
    pub network: McpNetworkConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpSecretRef {
    pub service: String,
    pub account: String,
}

impl McpSecretRef {
    #[must_use]
    pub fn is_scoped_to(&self, profile: &str) -> bool {
        fn identifier(value: &str) -> bool {
            value.len() == 32
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        }
        let parts = self.account.split(':').collect::<Vec<_>>();
        matches!(
            self.service.as_str(),
            MCP_CREDENTIAL_SERVICE | LEGACY_MCP_CREDENTIAL_SERVICE
        ) && identifier(profile)
            && parts.len() == 3
            && parts[0] == profile
            && identifier(parts[1])
            && parts[2]
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .is_some_and(|value| value.to_string() == parts[2])
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpToolDefinition {
    pub name: String,
    pub description: Option<String>,
    #[ts(type = "unknown")]
    pub input_schema: Value,
    #[ts(type = "unknown")]
    pub annotations: Option<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum McpValidationState {
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpValidationRecord {
    pub validation_id: String,
    pub connection_id: String,
    pub revision: u32,
    pub state: McpValidationState,
    pub reason_code: Option<String>,
    pub protocol_version: Option<String>,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub source_fingerprint: Option<String>,
    pub tools: Vec<McpToolDefinition>,
    pub catalog_digest: Option<String>,
    #[ts(type = "number")]
    pub validated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConnection {
    pub profile_id: String,
    pub connection_id: String,
    pub revision: u32,
    pub auth_generation: u32,
    pub config: McpConnectionConfig,
    pub credential_ref: Option<McpSecretRef>,
    pub enabled: bool,
    pub deleted: bool,
    pub approved_tool_names: Vec<String>,
    pub purposes: Vec<McpPurpose>,
    pub approved_catalog_digest: Option<String>,
    pub validation: Option<McpValidationRecord>,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum McpCredentialAction {
    Keep,
    Replace,
    Clear,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpSavePrepareParams {
    pub operation_id: String,
    pub connection_id: Option<String>,
    pub expected_revision: Option<u32>,
    pub config: McpConnectionConfig,
    pub credential_action: McpCredentialAction,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum McpChangeState {
    Prepared,
    Committed,
    Aborted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConnectionChange {
    pub operation_id: String,
    pub state: McpChangeState,
    pub expected_revision: Option<u32>,
    pub connection: McpConnection,
    pub credential_action: McpCredentialAction,
    pub cleanup_references: Vec<McpSecretRef>,
    pub cleaned_references: Vec<McpSecretRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpOperationParams {
    pub operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpConnectionParams {
    pub connection_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpSaveCommitParams {
    pub operation_id: String,
    pub credential_ref: Option<McpSecretRef>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpMutationParams {
    pub connection_id: String,
    pub expected_revision: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpGrantParams {
    pub connection_id: String,
    pub expected_revision: u32,
    pub validation_id: String,
    pub approved_tool_names: Vec<String>,
    pub purposes: Vec<McpPurpose>,
    pub source_consent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum McpExecutionIntent {
    Validation,
    ToolCall,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpExecutionPrepareParams {
    pub connection_id: String,
    pub expected_revision: u32,
    pub intent: McpExecutionIntent,
    pub purpose: Option<McpPurpose>,
    pub tool_name: Option<String>,
    pub catalog_digest: Option<String>,
    pub source_consent: bool,
    pub owner: crate::ModelExecutionOwner,
    pub launch_protocol: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum McpExecutionState {
    Prepared,
    Registered,
    Finished,
    Unconfirmed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpExecutionLease {
    pub lease_id: String,
    pub instance_fence: String,
    pub snapshot_digest: String,
    pub connection: McpConnection,
    pub intent: McpExecutionIntent,
    pub purpose: Option<McpPurpose>,
    pub tool_name: Option<String>,
    pub owner: crate::ModelExecutionOwner,
    pub processes: Vec<crate::ModelExecutionProcess>,
    pub state: McpExecutionState,
    pub writer_lease_id: String,
    pub outcome: Option<String>,
    pub reason_code: Option<String>,
    #[ts(type = "number")]
    pub prepared_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpExecutionParams {
    pub lease_id: String,
    pub snapshot_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpExecutionRegisterParams {
    pub lease_id: String,
    pub snapshot_digest: String,
    pub process: crate::ModelExecutionProcess,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpExecutionFinishParams {
    pub lease_id: String,
    pub snapshot_digest: String,
    pub outcome: String,
    pub request_sent: bool,
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpValidationReportParams {
    pub lease_id: String,
    pub snapshot_digest: String,
    pub state: McpValidationState,
    pub reason_code: Option<String>,
    pub protocol_version: Option<String>,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub source_fingerprint: Option<String>,
    pub tools: Vec<McpToolDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpValidateParams {
    pub connection_id: String,
    pub expected_revision: u32,
    pub source_consent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpSnapshotParams {
    pub purpose: McpPurpose,
    pub connection_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpToolSnapshot {
    pub digest: String,
    pub purpose: McpPurpose,
    pub connections: Vec<McpConnection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpCredentialCleanupParams {
    pub operation_id: String,
    pub credential_ref: McpSecretRef,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum McpCredentialCleanupState {
    Prepared,
    Confirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpCredentialCleanupPlan {
    pub cleanup_id: String,
    pub profile_id: String,
    pub credential_ref: McpSecretRef,
    pub state: McpCredentialCleanupState,
    pub source_operation_ids: Vec<String>,
    pub reference_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpCredentialCleanupConfirmParams {
    pub cleanup_id: String,
    pub expected_reference_digest: String,
}

#[cfg(test)]
mod credential_scope_tests {
    use super::*;
    const PROFILE: &str = "11111111111111111111111111111111";
    #[test]
    fn current_and_legacy_references_keep_exact_profile_connection_generation_scope() {
        for service in [MCP_CREDENTIAL_SERVICE, LEGACY_MCP_CREDENTIAL_SERVICE] {
            let reference = McpSecretRef {
                service: service.into(),
                account: format!("{PROFILE}:22222222222222222222222222222222:1"),
            };
            assert!(reference.is_scoped_to(PROFILE));
            assert!(!reference.is_scoped_to("33333333333333333333333333333333"));
        }
        assert_ne!(MCP_CREDENTIAL_SERVICE, LEGACY_MCP_CREDENTIAL_SERVICE);
    }
    #[test]
    fn malformed_or_foreign_references_are_not_credential_authority() {
        for account in [
            format!("{PROFILE}:22222222222222222222222222222222:0"),
            format!("{PROFILE}:22222222222222222222222222222222:01"),
            format!("{PROFILE}:2222222222222222222222222222222A:1"),
            format!("{PROFILE}:22222222222222222222222222222222:1:other"),
        ] {
            assert!(
                !McpSecretRef {
                    service: MCP_CREDENTIAL_SERVICE.into(),
                    account
                }
                .is_scoped_to(PROFILE)
            );
        }
        assert!(
            !McpSecretRef {
                service: "cn.cilai.personal.workbench.vault.model".into(),
                account: format!("{PROFILE}:22222222222222222222222222222222:1")
            }
            .is_scoped_to(PROFILE)
        );
    }
}
