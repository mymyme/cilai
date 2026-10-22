//! 用户 MCP 的连接验证与逐次工具授权。秘密仅在此受信传输层读取。
pub mod broker;
#[cfg(test)]
mod credential_tests;
mod local;
mod remote;
#[cfg(test)]
pub(crate) mod tests;
mod transport;

use crate::ai_runtime::AiRuntimeManager;
use crate::supervisor::AppServerSupervisor;
use contexthub_protocol::{
    LEGACY_MCP_CREDENTIAL_SERVICE, MCP_LAUNCH_PROTOCOL, McpAuthMode, McpConnection,
    McpExecutionFinishParams, McpExecutionIntent, McpExecutionLease, McpExecutionParams,
    McpExecutionPrepareParams, McpMutationParams, McpPurpose, McpSnapshotParams, McpSource,
    McpToolDefinition, McpToolSnapshot, McpValidateParams, McpValidationRecord,
    McpValidationReportParams, McpValidationState,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

fn rpc<P: Serialize, R: DeserializeOwned>(
    server: &AppServerSupervisor,
    method: &str,
    params: &P,
) -> Result<R, String> {
    serde_json::from_value(server.request_result(
        method,
        &serde_json::to_value(params).map_err(|_| "mcp_request_invalid")?,
    )?)
    .map_err(|_| "mcp_response_invalid".into())
}
fn digest(value: &impl Serialize) -> Result<String, String> {
    let value = serde_json::to_value(value).map_err(|_| "mcp_record_invalid")?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&value).map_err(|_| "mcp_record_invalid")?)
    ))
}

struct Execution {
    manager: AiRuntimeManager,
    server: AppServerSupervisor,
    data_dir: std::path::PathBuf,
    lease: McpExecutionLease,
    finished: bool,
    request_sent: bool,
    local_started: bool,
    failure_reason: Option<String>,
}
impl Execution {
    fn prepare(
        manager: &AiRuntimeManager,
        params: &McpExecutionPrepareParams,
    ) -> Result<Self, String> {
        let server = manager.personal_model_server()?;
        let data_dir = manager.gateway_data_dir()?;
        let lease = rpc(&server, "mcpExecution/prepare", params)?;
        Ok(Self {
            manager: manager.clone(),
            server,
            data_dir,
            lease,
            finished: false,
            request_sent: false,
            local_started: false,
            failure_reason: None,
        })
    }
    fn validate(&self) -> Result<(), String> {
        if self.manager.gateway_data_dir()? != self.data_dir {
            return Err("mcp_profile_changed".into());
        }
        let current: McpExecutionLease =
            rpc(&self.server, "mcpExecution/validate", &self.params())?;
        if current.connection != self.lease.connection
            || current.snapshot_digest != self.lease.snapshot_digest
        {
            return Err("mcp_execution_snapshot_changed".into());
        }
        Ok(())
    }
    fn params(&self) -> McpExecutionParams {
        McpExecutionParams {
            lease_id: self.lease.lease_id.clone(),
            snapshot_digest: self.lease.snapshot_digest.clone(),
        }
    }
    fn finish(&mut self, outcome: &str) -> Result<(), String> {
        if matches!(
            self.lease.connection.config.source,
            McpSource::LocalCommand { .. }
        ) && self.lease.processes.is_empty()
        {
            if self.local_started {
                let leases: Vec<McpExecutionLease> =
                    rpc(&self.server, "mcpExecution/list", &json!({}))?;
                self.lease = leases
                    .into_iter()
                    .find(|lease| {
                        lease.lease_id == self.lease.lease_id
                            && lease.snapshot_digest == self.lease.snapshot_digest
                            && lease.owner == self.lease.owner
                    })
                    .ok_or("mcp_execution_cleanup_unconfirmed")?;
            }
            if self.lease.processes.is_empty() {
                let _: McpExecutionLease =
                    rpc(&self.server, "mcpExecution/abortUnlaunched", &self.params())?;
                self.finished = true;
                return Ok(());
            }
        }
        let _: McpExecutionLease = rpc(
            &self.server,
            "mcpExecution/finish",
            &McpExecutionFinishParams {
                lease_id: self.lease.lease_id.clone(),
                snapshot_digest: self.lease.snapshot_digest.clone(),
                outcome: outcome.into(),
                request_sent: self.request_sent,
                reason_code: self.failure_reason.clone(),
            },
        )?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for Execution {
    fn drop(&mut self) {
        if !self.finished
            && self
                .finish(if self.request_sent {
                    "outcome_unconfirmed"
                } else {
                    "failed"
                })
                .is_err()
        {
            eprintln!("mcp_execution_cleanup_unconfirmed");
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(transparent)]
struct Secret(String);
impl Secret {
    fn expose(&self) -> &str {
        &self.0
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        let length = self.0.len();
        self.0.replace_range(.., &"\0".repeat(length));
    }
}
fn credential_vault_root(data_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if !data_dir.is_absolute() || data_dir.file_name().is_none_or(|name| name != "ai-runtime") {
        return Err("mcp_profile_changed".into());
    }
    Ok(data_dir
        .parent()
        .ok_or("mcp_profile_changed")?
        .join("credentials"))
}
fn credential(
    data_dir: &std::path::Path,
    connection: &McpConnection,
) -> Result<Option<Secret>, String> {
    let mut values = credential_map(data_dir, connection)?;
    Ok(values.remove("token"))
}
fn credential_map(
    data_dir: &std::path::Path,
    connection: &McpConnection,
) -> Result<std::collections::BTreeMap<String, Secret>, String> {
    if connection.config.auth_mode == McpAuthMode::None {
        if connection.credential_ref.is_some()
            || !connection.config.environment_variables.is_empty()
        {
            return Err("mcp_credentials_reference_invalid".into());
        }
        return Ok(std::collections::BTreeMap::new());
    }
    let expected = match connection.config.auth_mode {
        McpAuthMode::Bearer if connection.config.environment_variables.is_empty() => {
            vec!["token".to_owned()]
        }
        McpAuthMode::Environment if !connection.config.environment_variables.is_empty() => {
            connection.config.environment_variables.clone()
        }
        _ => return Err("mcp_auth_mode_invalid".into()),
    };
    let reference = connection
        .credential_ref
        .as_ref()
        .ok_or("mcp_credentials_required")?;
    if !reference.is_scoped_to(&connection.profile_id)
        || reference.account
            != format!(
                "{}:{}:{}",
                connection.profile_id, connection.connection_id, connection.auth_generation
            )
    {
        return Err("mcp_credentials_reference_invalid".into());
    }
    if reference.service == LEGACY_MCP_CREDENTIAL_SERVICE {
        return Err("mcp_credential_reentry_required".into());
    }
    let vault = contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(
        credential_vault_root(data_dir)?,
    )
    .map_err(|_| "mcp_credential_access_failed")?;
    let mut bytes = vault
        .read(&reference.service, &reference.account)
        .map_err(|_| "mcp_credential_access_failed")?
        .ok_or("mcp_credentials_required")?;
    decode_secret_map(&mut bytes, &expected)
}
fn decode_secret_map(
    bytes: &mut [u8],
    expected: &[String],
) -> Result<std::collections::BTreeMap<String, Secret>, String> {
    if bytes.len() > 64 * 1024 {
        bytes.fill(0);
        return Err("mcp_credential_invalid".into());
    }
    let parsed = serde_json::from_slice::<std::collections::BTreeMap<String, Secret>>(bytes);
    bytes.fill(0);
    let values = parsed.map_err(|_| "mcp_credential_invalid")?;
    if values.len() != expected.len()
        || expected.iter().any(|name| !values.contains_key(name))
        || values
            .values()
            .any(|value| value.expose().is_empty() || value.expose().contains(['\0', '\r', '\n']))
    {
        return Err("mcp_credential_invalid".into());
    }
    Ok(values)
}
fn safe_response<'a>(
    value: &Value,
    secrets: impl Iterator<Item = &'a Secret>,
) -> Result<(), String> {
    let encoded = serde_json::to_string(value).map_err(|_| "mcp_response_invalid")?;
    for secret in secrets {
        let escaped =
            serde_json::to_string(secret.expose()).map_err(|_| "mcp_credential_invalid")?;
        if encoded.contains(secret.expose())
            || escaped.len() > 2 && encoded.contains(&escaped[1..escaped.len() - 1])
        {
            return Err("mcp_response_contains_credentials".into());
        }
    }
    Ok(())
}
#[cfg(test)]
fn decode_bearer(bytes: &mut [u8]) -> Result<Secret, String> {
    if bytes.len() > 64 * 1024 {
        bytes.fill(0);
        return Err("mcp_credential_invalid".into());
    }
    let parsed = serde_json::from_slice::<std::collections::BTreeMap<String, Secret>>(bytes);
    bytes.fill(0);
    let mut values = parsed.map_err(|_| "mcp_credential_invalid")?;
    if values.len() != 1 || !values.contains_key("token") {
        return Err("mcp_credential_invalid".into());
    }
    let secret = values.remove("token").ok_or("mcp_credential_invalid")?;
    if secret.expose().is_empty()
        || secret.expose().len() > 64 * 1024
        || secret.expose().contains(['\0', '\r', '\n'])
    {
        return Err("mcp_credential_invalid".into());
    }
    Ok(secret)
}

impl AiRuntimeManager {
    /// # Errors
    /// 来源未经确认、权限变更、真实握手/目录失败或执行回执无法保存时返回明确错误。
    pub fn personal_mcp_validate(
        &self,
        params: McpValidateParams,
    ) -> Result<McpValidationRecord, String> {
        let _operation = self.begin_capability_operation()?;
        let mut execution = Execution::prepare(
            self,
            &McpExecutionPrepareParams {
                connection_id: params.connection_id,
                expected_revision: params.expected_revision,
                intent: McpExecutionIntent::Validation,
                purpose: None,
                tool_name: None,
                catalog_digest: None,
                source_consent: params.source_consent,
                owner: crate::model_connections::execution_owner()?,
                launch_protocol: MCP_LAUNCH_PROTOCOL,
            },
        )?;
        let result = crate::async_support::block_on(async {
            execution.validate()?;
            let connection = execution.lease.connection.clone();
            let mut session = transport::Session::open(&mut execution, &connection)?;
            let evidence = session.discover(&mut execution).await?;
            Ok::<_, String>(evidence)
        });
        execution.validate()?;
        let (state, reason_code, evidence) = match result {
            Ok(evidence) => (McpValidationState::Passed, None, Some(evidence)),
            Err(code) => (McpValidationState::Failed, Some(code), None),
        };
        let report = McpValidationReportParams {
            lease_id: execution.lease.lease_id.clone(),
            snapshot_digest: execution.lease.snapshot_digest.clone(),
            state: state.clone(),
            reason_code,
            protocol_version: evidence.as_ref().map(|e| e.protocol.clone()),
            server_name: evidence.as_ref().map(|e| e.name.clone()),
            server_version: evidence.as_ref().map(|e| e.version.clone()),
            source_fingerprint: evidence.as_ref().map(|e| e.fingerprint.clone()),
            tools: evidence.map_or_else(Vec::new, |e| e.tools),
        };
        let record = rpc(&execution.server, "mcpExecution/reportValidation", &report)?;
        execution.finish(if state == McpValidationState::Passed {
            "success"
        } else {
            "failed"
        })?;
        Ok(record)
    }

    /// # Errors
    /// 仅返回当前资料、用途和已授权目录的不可变快照。
    pub fn personal_mcp_snapshot(
        &self,
        params: &McpSnapshotParams,
    ) -> Result<McpToolSnapshot, String> {
        rpc(&self.personal_model_server()?, "mcpTool/snapshot", params)
    }
    /// # Errors
    /// 真实读取本用途已启用且显式授权的工具，读取失败不转为空目录。
    pub fn personal_mcp_selected_snapshot(
        &self,
        purpose: McpPurpose,
    ) -> Result<McpToolSnapshot, String> {
        let connections: Vec<McpConnection> = rpc(
            &self.personal_model_server()?,
            "mcpConnection/list",
            &json!({}),
        )?;
        let connection_ids = connections
            .into_iter()
            .filter(|c| c.enabled && c.purposes.contains(&purpose))
            .map(|c| c.connection_id)
            .collect();
        self.personal_mcp_snapshot(&McpSnapshotParams {
            purpose,
            connection_ids,
        })
    }
}

/// 每次工具调用由当前轮次的正式权限处理器决定；服务端 annotations 不赋予自动执行权。
pub type ToolApproval = Arc<
    dyn Fn(
            &McpConnection,
            &McpToolDefinition,
            &Value,
            &dyn Fn(&Value) -> Result<Value, String>,
        ) -> Result<Value, String>
        + Send
        + Sync,
>;

pub(crate) fn call_tool(
    manager: &AiRuntimeManager,
    snapshot: &McpToolSnapshot,
    connection_id: &str,
    tool_name: &str,
    arguments: &Value,
    approve: &ToolApproval,
) -> Result<Value, String> {
    let _operation = manager.begin_capability_operation()?;
    let conn = snapshot
        .connections
        .iter()
        .find(|conn| conn.connection_id == connection_id)
        .ok_or("mcp_tool_not_authorized")?;
    let current = manager.personal_mcp_snapshot(&McpSnapshotParams {
        purpose: snapshot.purpose.clone(),
        connection_ids: snapshot
            .connections
            .iter()
            .map(|conn| conn.connection_id.clone())
            .collect(),
    })?;
    if &current != snapshot {
        return Err("mcp_tool_snapshot_changed".into());
    }
    let definition = conn
        .validation
        .as_ref()
        .and_then(|v| v.tools.iter().find(|tool| tool.name == tool_name))
        .filter(|tool| conn.approved_tool_names.contains(&tool.name))
        .ok_or("mcp_tool_not_authorized")?;
    let invoked = std::sync::atomic::AtomicBool::new(false);
    approve(conn, definition, arguments, &|revised| {
        if invoked.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Err("mcp_tool_call_already_started".into());
        }
        let current = manager.personal_mcp_snapshot(&McpSnapshotParams {
            purpose: snapshot.purpose.clone(),
            connection_ids: snapshot
                .connections
                .iter()
                .map(|c| c.connection_id.clone())
                .collect(),
        })?;
        if &current != snapshot {
            return Err("mcp_tool_snapshot_changed".into());
        }
        if !revised.is_object()
            || serde_json::to_vec(revised)
                .map_err(|_| "mcp_tool_arguments_invalid")?
                .len()
                > 1024 * 1024
        {
            return Err("mcp_tool_arguments_invalid".into());
        }
        crate::app_bridge::validate_tool_arguments(&definition.input_schema, revised)
            .map_err(|_| "mcp_tool_arguments_invalid")?;
        execute_tool(manager, snapshot, conn, tool_name, revised)
    })
}
fn execute_tool(
    manager: &AiRuntimeManager,
    snapshot: &McpToolSnapshot,
    conn: &McpConnection,
    tool_name: &str,
    arguments: &Value,
) -> Result<Value, String> {
    let mut execution = Execution::prepare(
        manager,
        &McpExecutionPrepareParams {
            connection_id: conn.connection_id.clone(),
            expected_revision: conn.revision,
            intent: McpExecutionIntent::ToolCall,
            purpose: Some(snapshot.purpose.clone()),
            tool_name: Some(tool_name.into()),
            catalog_digest: conn.approved_catalog_digest.clone(),
            source_consent: false,
            owner: crate::model_connections::execution_owner()?,
            launch_protocol: MCP_LAUNCH_PROTOCOL,
        },
    )?;
    let result: Result<Value, String> = crate::async_support::block_on(async {
        let mut session = transport::Session::open(&mut execution, conn)?;
        let evidence = session.discover(&mut execution).await?;
        if Some(&digest(&evidence.tools)?) != conn.approved_catalog_digest.as_ref()
            || conn
                .validation
                .as_ref()
                .is_none_or(|v| v.source_fingerprint.as_deref() != Some(&evidence.fingerprint))
        {
            return Err("mcp_catalog_changed".into());
        }
        let result = session
            .request(
                &mut execution,
                "tools/call",
                json!({"name":tool_name,"arguments":arguments}),
                true,
            )
            .await?;
        validate_tool_result(&result)?;
        Ok(result)
    });
    match result {
        Ok(value) => {
            execution.finish(if value["isError"] == true {
                "failed"
            } else {
                "success"
            })?;
            Ok(value)
        }
        Err(reason) => {
            execution.failure_reason = Some(
                if reason.len() <= 128
                    && reason
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
                {
                    reason.clone()
                } else {
                    "mcp_lifecycle_unconfirmed".into()
                },
            );
            if matches!(
                reason.as_str(),
                "mcp_catalog_changed"
                    | "mcp_source_changed"
                    | "mcp_file_permissions_changed"
                    | "mcp_original_home_protection_changed"
            ) {
                let _: McpConnection = rpc(
                    &execution.server,
                    "mcpConnection/disable",
                    &McpMutationParams {
                        connection_id: conn.connection_id.clone(),
                        expected_revision: conn.revision,
                    },
                )?;
            }
            let unknown = execution.request_sent;
            execution.finish(if unknown {
                "outcome_unconfirmed"
            } else {
                "failed"
            })?;
            if unknown {
                Err("mcp_tool_outcome_unconfirmed".into())
            } else {
                Err(reason)
            }
        }
    }
}

fn validate_tool_result(result: &Value) -> Result<(), String> {
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .ok_or("mcp_tool_response_invalid")?;
    if !result.is_object()
        || result.get("isError").is_some_and(|v| !v.is_boolean())
        || result
            .get("structuredContent")
            .is_some_and(|v| !v.is_object())
    {
        return Err("mcp_tool_response_invalid".into());
    }
    let string = |value: &Value, key: &str| value.get(key).is_some_and(Value::is_string);
    for item in content {
        let valid = match item["type"].as_str() {
            Some("text") => string(item, "text"),
            Some("image" | "audio") => string(item, "data") && string(item, "mimeType"),
            Some("resource_link") => string(item, "uri") && string(item, "name"),
            Some("resource") => {
                item["resource"].is_object()
                    && string(&item["resource"], "uri")
                    && (string(&item["resource"], "text") || string(&item["resource"], "blob"))
            }
            _ => false,
        };
        if !valid {
            return Err("mcp_tool_response_invalid".into());
        }
    }
    Ok(())
}
