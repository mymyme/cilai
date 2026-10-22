//! 用户 MCP 独立记录与修订；秘密正文保存在工作台本地凭据库，业务记录仅保留作用域引用。
use super::{PersistenceError, SqlitePersistence, database_error};
use crate::ai_business_ledger::{digest, reject};
use contexthub_protocol::{
    LEGACY_MCP_CREDENTIAL_SERVICE, MCP_CREDENTIAL_SERVICE, MCP_LAUNCH_PROTOCOL, McpAuthMode,
    McpChangeState, McpConnection, McpConnectionChange, McpConnectionConfig, McpConnectionParams,
    McpCredentialAction, McpCredentialCleanupConfirmParams, McpCredentialCleanupParams,
    McpCredentialCleanupPlan, McpCredentialCleanupState, McpExecutionFinishParams,
    McpExecutionIntent, McpExecutionLease, McpExecutionParams, McpExecutionPrepareParams,
    McpExecutionRegisterParams, McpExecutionState, McpGrantParams, McpMutationParams,
    McpOperationParams, McpSaveCommitParams, McpSavePrepareParams, McpSecretRef, McpSnapshotParams,
    McpSource, McpToolSnapshot, McpValidationRecord, McpValidationReportParams, McpValidationState,
    ModelNetworkMode, PersonalWriterAbortParams, PersonalWriterLease, PersonalWriterLeaseParams,
    PersonalWriterMode, PersonalWriterPrepareParams, PersonalWriterRegisterParams,
    PersonalWriterState,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

mod cleanup;
mod execution;
#[cfg(test)]
mod tests;

fn encode<T: Serialize>(value: &T) -> Result<Value, PersistenceError> {
    serde_json::to_value(value).map_err(|_| reject("mcp_record_invalid"))
}
fn decode<T: DeserializeOwned>(value: Value) -> Result<T, PersistenceError> {
    serde_json::from_value(value).map_err(|_| reject("mcp_request_invalid"))
}
fn get<T: DeserializeOwned>(
    tx: &Transaction<'_>,
    profile: &str,
    kind: &str,
    id: &str,
) -> Result<Option<T>, PersistenceError> {
    let raw: Option<String> = tx.query_row("SELECT record_json FROM personal_mcp_records WHERE profile_id=?1 AND kind=?2 AND record_id=?3",params![profile,kind,id],|row|row.get(0)).optional().map_err(database_error("read MCP record"))?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(|_| reject("mcp_storage_invalid")))
        .transpose()
}
fn list<T: DeserializeOwned>(
    tx: &Transaction<'_>,
    profile: &str,
    kind: &str,
) -> Result<Vec<T>, PersistenceError> {
    let mut statement=tx.prepare("SELECT record_json FROM personal_mcp_records WHERE profile_id=?1 AND kind=?2 ORDER BY record_id").map_err(database_error("list MCP records"))?;
    statement
        .query_map(params![profile, kind], |row| row.get::<_, String>(0))
        .map_err(database_error("query MCP records"))?
        .map(|row| {
            serde_json::from_str(&row.map_err(database_error("read MCP row"))?)
                .map_err(|_| reject("mcp_storage_invalid"))
        })
        .collect()
}
fn put<T: Serialize>(
    tx: &Transaction<'_>,
    profile: &str,
    kind: &str,
    id: &str,
    value: &T,
    now: i64,
) -> Result<(), PersistenceError> {
    let raw = serde_json::to_string(value).map_err(|_| reject("mcp_record_invalid"))?;
    tx.execute("INSERT INTO personal_mcp_records(profile_id,kind,record_id,record_json,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(profile_id,kind,record_id) DO UPDATE SET record_json=excluded.record_json,updated_at=excluded.updated_at",params![profile,kind,id,raw,now]).map_err(database_error("write MCP record"))?;
    Ok(())
}
fn text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn identifier(value: &str) -> bool {
    text(value, 128)
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
}
fn connection(
    tx: &Transaction<'_>,
    profile: &str,
    id: &str,
) -> Result<McpConnection, PersistenceError> {
    get(tx, profile, "connection", id)?.ok_or_else(|| reject("mcp_connection_not_found"))
}
fn current(
    tx: &Transaction<'_>,
    profile: &str,
    id: &str,
) -> Result<McpConnection, PersistenceError> {
    let conn = connection(tx, profile, id)?;
    if conn.deleted {
        return Err(reject("mcp_connection_deleted"));
    }
    Ok(conn)
}
fn expected(conn: &McpConnection, revision: u32) -> Result<(), PersistenceError> {
    if conn.revision != revision {
        return Err(reject("mcp_revision_changed"));
    }
    Ok(())
}
fn next(value: u32) -> Result<u32, PersistenceError> {
    value
        .checked_add(1)
        .ok_or_else(|| reject("mcp_revision_exhausted"))
}
fn unique<T: Serialize>(values: &[T]) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    values
        .iter()
        .all(|value| serde_json::to_string(value).is_ok_and(|value| seen.insert(value)))
}
fn absolute(value: &str) -> bool {
    text(value, 4096)
        && std::path::Path::new(value).is_absolute()
        && !std::path::Path::new(value)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}
fn remote_url(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or_default();
    text(value, 4096)
        && !authority.is_empty()
        && !authority.contains('@')
        && !value.contains(['?', '#'])
        && (scheme == "https"
            || (scheme == "http"
                && (matches!(authority.split(':').next(), Some("127.0.0.1" | "localhost"))
                    || authority.starts_with("[::1]"))))
}
fn proxy_url(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    let authority = rest.strip_suffix('/').unwrap_or(rest);
    text(value, 4096)
        && matches!(scheme, "http" | "https")
        && !authority.is_empty()
        && !authority.contains(['@', '?', '#', '/'])
}
fn environment_valid(names: &[String]) -> Result<(), PersistenceError> {
    for name in names {
        if !text(name, 128)
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
            || name.as_bytes()[0].is_ascii_digit()
            || [
                "HOME",
                "USERPROFILE",
                "PATH",
                "TMPDIR",
                "TEMP",
                "TMP",
                "CODEX_HOME",
                "CODEX_SQLITE_HOME",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "NO_PROXY",
                "NODE_OPTIONS",
                "RUST_LOG",
                "BASH_ENV",
                "ENV",
                "ZDOTDIR",
                "SHELL",
            ]
            .contains(&name.as_str())
            || ["CILAI_", "DYLD_", "LD_", "XDG_"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        {
            return Err(reject("mcp_environment_invalid"));
        }
    }
    Ok(())
}
fn config_valid(config: &McpConnectionConfig) -> Result<(), PersistenceError> {
    if !text(&config.name, 120)
        || !unique(&config.environment_variables)
        || config.environment_variables.len() > 32
    {
        return Err(reject("mcp_configuration_invalid"));
    }
    environment_valid(&config.environment_variables)?;
    match &config.source {
        McpSource::LocalCommand {
            program,
            args,
            cwd,
            permissions,
        } => {
            if config.network.mode != ModelNetworkMode::Direct || config.network.proxy_url.is_some()
            {
                return Err(reject("mcp_local_proxy_not_supported"));
            }
            if !absolute(program)
                || !absolute(cwd)
                || args.len() > 128
                || args.iter().any(|arg| {
                    arg.len() > 8192 || arg.contains(['\0', '\r', '\n']) || secret_argument(arg)
                })
                || permissions.read_roots.len() + permissions.write_roots.len() > 64
                || permissions
                    .read_roots
                    .iter()
                    .chain(&permissions.write_roots)
                    .any(|path| !absolute(path))
            {
                return Err(reject("mcp_local_source_invalid"));
            }
            if config.auth_mode == McpAuthMode::Bearer {
                return Err(reject("mcp_auth_mode_invalid"));
            }
        }
        McpSource::RemoteHttp { url } => {
            if !remote_url(url) {
                return Err(reject("mcp_remote_source_invalid"));
            }
            if config.auth_mode == McpAuthMode::Environment {
                return Err(reject("mcp_auth_mode_invalid"));
            }
        }
    }
    if (config.auth_mode == McpAuthMode::Environment) == config.environment_variables.is_empty() {
        return Err(reject("mcp_environment_invalid"));
    }
    match config.network.mode {
        ModelNetworkMode::CustomProxy
            if config
                .network
                .proxy_url
                .as_deref()
                .is_none_or(|url| !proxy_url(url)) =>
        {
            return Err(reject("mcp_proxy_invalid"));
        }
        ModelNetworkMode::Direct | ModelNetworkMode::SystemProxy
            if config.network.proxy_url.is_some() =>
        {
            return Err(reject("mcp_proxy_invalid"));
        }
        _ => (),
    }
    Ok(())
}
fn secret_argument(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase().replace('_', "-");
    let flag = normalized.split('=').next().unwrap_or_default();
    [
        "--api-key",
        "--apikey",
        "--token",
        "--access-token",
        "--refresh-token",
        "--password",
        "--secret",
        "--authorization",
        "--bearer",
        "--client-secret",
        "--header",
    ]
    .contains(&flag)
        || value.split('=').next() == Some("-H")
        || normalized.contains("authorization:")
        || normalized.contains("bearer ")
        || normalized.split_once("://").is_some_and(|(_, rest)| {
            rest.split('/').next().unwrap_or_default().contains('@')
                || rest.contains("?token=")
                || rest.contains("?api-key=")
                || rest.contains("&token=")
        })
}
fn no_active(
    tx: &Transaction<'_>,
    profile: &str,
    connection_id: &str,
) -> Result<(), PersistenceError> {
    if list::<McpExecutionLease>(tx, profile, "lease")?
        .iter()
        .any(|lease| {
            lease.connection.connection_id == connection_id
                && lease.state != McpExecutionState::Finished
        })
    {
        return Err(reject("mcp_execution_active"));
    }
    Ok(())
}
fn change(
    tx: &Transaction<'_>,
    profile: &str,
    id: &str,
) -> Result<McpConnectionChange, PersistenceError> {
    get(tx, profile, "change", id)?.ok_or_else(|| reject("mcp_change_not_found"))
}
fn validate_credential_action(
    params: &McpSavePrepareParams,
    previous: Option<&McpConnection>,
) -> Result<(), PersistenceError> {
    let needs_secret = params.config.auth_mode != McpAuthMode::None;
    if needs_secret && params.credential_action == McpCredentialAction::Clear {
        return Err(reject("mcp_credentials_required"));
    }
    if !needs_secret && params.credential_action != McpCredentialAction::Clear {
        return Err(reject("mcp_credentials_unexpected"));
    }
    if needs_secret
        && params.credential_action == McpCredentialAction::Keep
        && previous
            .and_then(|conn| conn.credential_ref.as_ref())
            .is_some_and(|reference| reference.service == LEGACY_MCP_CREDENTIAL_SERVICE)
    {
        return Err(reject("mcp_credential_reentry_required"));
    }
    if needs_secret
        && params.credential_action == McpCredentialAction::Keep
        && previous.is_none_or(|conn| {
            conn.credential_ref.is_none()
                || conn.config.source != params.config.source
                || conn.config.auth_mode != params.config.auth_mode
                || conn.config.environment_variables != params.config.environment_variables
        })
    {
        return Err(reject("mcp_credentials_reentry_required"));
    }

    Ok(())
}
fn require_current_credential(profile: &str, conn: &McpConnection) -> Result<(), PersistenceError> {
    if conn.profile_id != profile {
        return Err(reject("mcp_credentials_reference_invalid"));
    }
    let Some(reference) = conn.credential_ref.as_ref() else {
        return if conn.config.auth_mode == McpAuthMode::None {
            Ok(())
        } else {
            Err(reject("mcp_credentials_required"))
        };
    };
    if !reference.is_scoped_to(&conn.profile_id)
        || reference.account
            != format!(
                "{}:{}:{}",
                conn.profile_id, conn.connection_id, conn.auth_generation
            )
        || conn.config.auth_mode == McpAuthMode::None
    {
        return Err(reject("mcp_credentials_reference_invalid"));
    }
    if reference.service == LEGACY_MCP_CREDENTIAL_SERVICE {
        return Err(reject("mcp_credential_reentry_required"));
    }
    Ok(())
}
fn save_prepare(
    tx: &Transaction<'_>,
    profile: &str,
    params: McpSavePrepareParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    config_valid(&params.config)?;
    if !identifier(&params.operation_id) {
        return Err(reject("mcp_operation_invalid"));
    }
    if let Some(prior) = get::<McpConnectionChange>(tx, profile, "change", &params.operation_id)? {
        if prior.connection.config != params.config
            || prior.expected_revision != params.expected_revision
            || prior.credential_action != params.credential_action
            || params
                .connection_id
                .as_ref()
                .is_some_and(|id| id != &prior.connection.connection_id)
        {
            return Err(reject("mcp_operation_payload_changed"));
        }
        return encode(&prior);
    }
    let previous = params
        .connection_id
        .as_deref()
        .map(|id| current(tx, profile, id))
        .transpose()?;
    match (&previous, params.expected_revision) {
        (Some(conn), Some(revision)) => expected(conn, revision)?,
        (None, None) => (),
        _ => return Err(reject("mcp_revision_required")),
    }
    let connection_id = previous.as_ref().map_or_else(
        || uuid::Uuid::new_v4().simple().to_string(),
        |conn| conn.connection_id.clone(),
    );
    let changes = list::<McpConnectionChange>(tx, profile, "change")?;
    if changes.iter().any(|item| {
        item.state == McpChangeState::Prepared && item.connection.connection_id == connection_id
    }) {
        return Err(reject("mcp_change_pending"));
    }
    validate_credential_action(&params, previous.as_ref())?;
    let high = changes
        .iter()
        .filter(|item| item.connection.connection_id == connection_id)
        .map(|item| item.connection.auth_generation)
        .chain(previous.as_ref().map(|conn| conn.auth_generation))
        .max()
        .unwrap_or(0);
    let generation = if params.credential_action == McpCredentialAction::Keep {
        previous
            .as_ref()
            .ok_or_else(|| reject("mcp_credentials_required"))?
            .auth_generation
    } else {
        next(high)?
    };
    let reference = match params.credential_action {
        McpCredentialAction::Keep => previous
            .as_ref()
            .and_then(|conn| conn.credential_ref.clone()),
        McpCredentialAction::Replace => Some(McpSecretRef {
            service: MCP_CREDENTIAL_SERVICE.into(),
            account: format!("{profile}:{connection_id}:{generation}"),
        }),
        McpCredentialAction::Clear => None,
    };
    cleanup::require_unreserved(tx, profile, reference.as_ref())?;
    let cleanup_references = previous
        .as_ref()
        .and_then(|conn| conn.credential_ref.clone())
        .filter(|old| Some(old) != reference.as_ref())
        .into_iter()
        .collect();
    let conn = McpConnection {
        profile_id: profile.into(),
        connection_id,
        revision: next(previous.as_ref().map_or(0, |conn| conn.revision))?,
        auth_generation: generation,
        config: params.config,
        credential_ref: reference,
        enabled: false,
        deleted: false,
        approved_tool_names: Vec::new(),
        purposes: Vec::new(),
        approved_catalog_digest: None,
        validation: None,
        updated_at: now,
    };
    let value = McpConnectionChange {
        operation_id: params.operation_id,
        state: McpChangeState::Prepared,
        expected_revision: params.expected_revision,
        connection: conn,
        credential_action: params.credential_action,
        cleanup_references,
        cleaned_references: Vec::new(),
    };
    put(tx, profile, "change", &value.operation_id, &value, now)?;
    encode(&value)
}
fn save_commit(
    tx: &Transaction<'_>,
    profile: &str,
    params: &McpSaveCommitParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut operation = change(tx, profile, &params.operation_id)?;
    if operation.state == McpChangeState::Aborted {
        return Err(reject("mcp_change_aborted"));
    }
    if params.credential_ref != operation.connection.credential_ref {
        return Err(reject("mcp_credentials_reference_mismatch"));
    }
    if operation.state == McpChangeState::Committed {
        return encode(&operation.connection);
    }
    require_current_credential(profile, &operation.connection)?;
    no_active(tx, profile, &operation.connection.connection_id)?;
    if let Some(revision) = operation.expected_revision {
        expected(
            &current(tx, profile, &operation.connection.connection_id)?,
            revision,
        )?;
    } else if get::<McpConnection>(
        tx,
        profile,
        "connection",
        &operation.connection.connection_id,
    )?
    .is_some()
    {
        return Err(reject("mcp_connection_exists"));
    }
    cleanup::require_unreserved(tx, profile, operation.connection.credential_ref.as_ref())?;
    operation.state = McpChangeState::Committed;
    put(
        tx,
        profile,
        "connection",
        &operation.connection.connection_id,
        &operation.connection,
        now,
    )?;
    put(
        tx,
        profile,
        "change",
        &operation.operation_id,
        &operation,
        now,
    )?;
    encode(&operation.connection)
}
fn save_abort(
    tx: &Transaction<'_>,
    profile: &str,
    params: &McpOperationParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut operation = change(tx, profile, &params.operation_id)?;
    if operation.state != McpChangeState::Prepared {
        return encode(&operation);
    }
    operation.state = McpChangeState::Aborted;
    operation.cleanup_references = if operation.credential_action == McpCredentialAction::Replace {
        operation
            .connection
            .credential_ref
            .clone()
            .into_iter()
            .collect()
    } else {
        Vec::new()
    };
    put(
        tx,
        profile,
        "change",
        &operation.operation_id,
        &operation,
        now,
    )?;
    encode(&operation)
}
fn grant(
    tx: &Transaction<'_>,
    profile: &str,
    params: McpGrantParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut conn = current(tx, profile, &params.connection_id)?;
    expected(&conn, params.expected_revision)?;
    if !params.source_consent
        || params.approved_tool_names.is_empty()
        || params.purposes.is_empty()
        || !unique(&params.approved_tool_names)
        || !unique(&params.purposes)
    {
        return Err(reject("mcp_permission_confirmation_required"));
    }
    let validation = conn
        .validation
        .as_ref()
        .ok_or_else(|| reject("mcp_validation_required"))?;
    if validation.validation_id != params.validation_id
        || validation.revision != conn.revision
        || validation.state != McpValidationState::Passed
        || validation.catalog_digest.is_none()
    {
        return Err(reject("mcp_validation_stale"));
    }
    if params
        .approved_tool_names
        .iter()
        .any(|name| !validation.tools.iter().any(|tool| &tool.name == name))
    {
        return Err(reject("mcp_tool_not_validated"));
    }
    conn.enabled = true;
    conn.approved_tool_names = params.approved_tool_names;
    conn.purposes = params.purposes;
    conn.approved_catalog_digest = validation.catalog_digest.clone();
    conn.revision = next(conn.revision)?;
    conn.updated_at = now;
    put(tx, profile, "connection", &conn.connection_id, &conn, now)?;
    encode(&conn)
}
fn mutate(
    tx: &Transaction<'_>,
    profile: &str,
    params: &McpMutationParams,
    delete: bool,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut conn = connection(tx, profile, &params.connection_id)?;
    if conn.deleted && delete {
        return encode(&conn);
    }
    expected(&conn, params.expected_revision)?;
    if conn.deleted {
        return Err(reject("mcp_connection_deleted"));
    }
    if delete {
        no_active(tx, profile, &conn.connection_id)?;
    }
    conn.revision = next(conn.revision)?;
    conn.enabled = false;
    conn.updated_at = now;
    if delete {
        conn.deleted = true;
        let refs = conn.credential_ref.take().into_iter().collect();
        let operation = McpConnectionChange {
            operation_id: uuid::Uuid::new_v4().simple().to_string(),
            state: McpChangeState::Committed,
            expected_revision: Some(params.expected_revision),
            connection: conn.clone(),
            credential_action: McpCredentialAction::Clear,
            cleanup_references: refs,
            cleaned_references: Vec::new(),
        };
        put(
            tx,
            profile,
            "change",
            &operation.operation_id,
            &operation,
            now,
        )?;
    }
    put(tx, profile, "connection", &conn.connection_id, &conn, now)?;
    encode(&conn)
}
fn snapshot(
    tx: &Transaction<'_>,
    profile: &str,
    params: McpSnapshotParams,
) -> Result<Value, PersistenceError> {
    if !unique(&params.connection_ids) || params.connection_ids.len() > 64 {
        return Err(reject("mcp_snapshot_invalid"));
    }
    let mut connections = Vec::new();
    for id in params.connection_ids {
        let conn = current(tx, profile, &id)?;
        if !conn.enabled
            || !conn.purposes.contains(&params.purpose)
            || conn.approved_catalog_digest.is_none()
            || conn.approved_tool_names.is_empty()
        {
            return Err(reject("mcp_connection_not_authorized"));
        }
        cleanup::require_unreserved(tx, profile, conn.credential_ref.as_ref())?;
        connections.push(conn);
    }
    connections.sort_by(|a, b| a.connection_id.cmp(&b.connection_id));
    let fingerprint = digest(&json!({"purpose":params.purpose,"connections":connections}));
    encode(&McpToolSnapshot {
        digest: fingerprint,
        purpose: params.purpose,
        connections,
    })
}

impl SqlitePersistence {
    /// # Errors
    /// Rejects invalid profile ownership, maintenance, stale revisions and unsupported requests.
    pub fn personal_mcp_rpc(
        &mut self,
        method: &str,
        params: Value,
        profile: &str,
        fence: &str,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("begin MCP transaction"))?;
        let ready:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_profile_compatibility WHERE singleton=1 AND profile_id=?1 AND instance_fence=?2 AND data_version=2 AND state='ready')",params![profile,fence],|row|row.get(0)).map_err(database_error("read MCP profile readiness"))?;
        if !ready {
            return Err(reject("mcp_profile_not_ready"));
        }
        let existing_completion = matches!(
            method,
            "mcpExecution/registerProcess"
                | "mcpExecution/finish"
                | "mcpExecution/abortUnlaunched"
                | "mcpExecution/validate"
                | "mcpExecution/reportValidation"
        );
        let read_only = matches!(
            method,
            "mcpConnection/list"
                | "mcpConnection/get"
                | "mcpConnection/changeList"
                | "mcpConnection/changeStatus"
                | "mcpExecution/list"
                | "mcpTool/snapshot"
                | "mcpCredentialCleanup/list"
        );
        if !read_only && !existing_completion {
            crate::profile_writers::require_writer_admission(&tx, profile, fence)?;
        }
        let result = match method {
            "mcpConnection/list" => encode(
                &list::<McpConnection>(&tx, profile, "connection")?
                    .into_iter()
                    .filter(|conn| !conn.deleted)
                    .collect::<Vec<_>>(),
            ),
            "mcpConnection/get" => encode(&connection(
                &tx,
                profile,
                &decode::<McpConnectionParams>(params)?.connection_id,
            )?),
            "mcpConnection/changeList" => {
                encode(&list::<McpConnectionChange>(&tx, profile, "change")?)
            }
            "mcpConnection/changeStatus" => encode(&change(
                &tx,
                profile,
                &decode::<McpOperationParams>(params)?.operation_id,
            )?),
            "mcpConnection/savePrepare" => save_prepare(&tx, profile, decode(params)?, now),
            "mcpConnection/saveCommit" => save_commit(&tx, profile, &decode(params)?, now),
            "mcpConnection/saveAbort" => save_abort(&tx, profile, &decode(params)?, now),
            "mcpConnection/grant" => grant(&tx, profile, decode(params)?, now),
            "mcpConnection/disable" => mutate(&tx, profile, &decode(params)?, false, now),
            "mcpConnection/delete" => mutate(&tx, profile, &decode(params)?, true, now),
            "mcpTool/snapshot" => snapshot(&tx, profile, decode(params)?),
            method if method.starts_with("mcpExecution/") => {
                execution::route(&tx, profile, fence, method, params, now)
            }
            method if method.starts_with("mcpCredentialCleanup/") => {
                cleanup::route(&tx, profile, method, params, now)
            }
            _ => Err(reject("mcp_method_unknown")),
        }?;
        tx.commit()
            .map_err(database_error("commit MCP transaction"))?;
        Ok(result)
    }
}
