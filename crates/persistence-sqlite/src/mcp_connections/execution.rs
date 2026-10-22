use super::connection;
use super::{
    MCP_LAUNCH_PROTOCOL, McpExecutionFinishParams, McpExecutionIntent, McpExecutionLease,
    McpExecutionParams, McpExecutionPrepareParams, McpExecutionRegisterParams, McpExecutionState,
    McpSource, McpValidationRecord, McpValidationReportParams, McpValidationState,
    PersistenceError, PersonalWriterAbortParams, PersonalWriterLease, PersonalWriterLeaseParams,
    PersonalWriterMode, PersonalWriterPrepareParams, PersonalWriterRegisterParams,
    PersonalWriterState, Transaction, Value, cleanup, current, decode, digest, encode, expected,
    get, identifier, json, list, next, put, reject, require_current_credential, text, unique,
};
use crate::profile_writers;

fn writer(
    tx: &Transaction<'_>,
    profile: &str,
    fence: &str,
    lease: &McpExecutionLease,
    now: i64,
) -> Result<PersonalWriterLease, PersistenceError> {
    decode(profile_writers::writer_rpc(
        tx,
        "profileWriter/read",
        json!({"leaseId":lease.writer_lease_id}),
        profile,
        fence,
        now,
    )?)
}
fn lease(
    tx: &Transaction<'_>,
    profile: &str,
    fence: &str,
    params: &McpExecutionParams,
    now: i64,
    current_revision: bool,
) -> Result<McpExecutionLease, PersistenceError> {
    let lease = get::<McpExecutionLease>(tx, profile, "lease", &params.lease_id)?
        .ok_or_else(|| reject("mcp_execution_not_found"))?;
    if lease.snapshot_digest != params.snapshot_digest || lease.instance_fence != fence {
        return Err(reject("mcp_execution_snapshot_changed"));
    }
    if lease.state == McpExecutionState::Finished {
        return Err(reject("mcp_execution_finished"));
    }
    let writer = writer(tx, profile, fence, &lease, now)?;
    if writer.state == PersonalWriterState::Released || writer.owner != lease.owner {
        return Err(reject("mcp_execution_writer_released"));
    }
    if current_revision {
        let conn = current(tx, profile, &lease.connection.connection_id)?;
        expected(&conn, lease.connection.revision)?;
        if lease.intent == McpExecutionIntent::ToolCall
            && (!conn.enabled
                || lease
                    .purpose
                    .as_ref()
                    .is_none_or(|purpose| !conn.purposes.contains(purpose))
                || lease
                    .tool_name
                    .as_ref()
                    .is_none_or(|name| !conn.approved_tool_names.contains(name))
                || conn.approved_catalog_digest != lease.connection.approved_catalog_digest)
        {
            return Err(reject("mcp_tool_authorization_revoked"));
        }
        cleanup::require_unreserved(tx, profile, conn.credential_ref.as_ref())?;
    }
    Ok(lease)
}
fn prepare(
    tx: &Transaction<'_>,
    profile: &str,
    fence: &str,
    params: McpExecutionPrepareParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let conn = current(tx, profile, &params.connection_id)?;
    expected(&conn, params.expected_revision)?;
    require_current_credential(profile, &conn)?;
    if params.launch_protocol != MCP_LAUNCH_PROTOCOL {
        return Err(reject("mcp_launch_protocol_required"));
    }
    cleanup::require_unreserved(tx, profile, conn.credential_ref.as_ref())?;
    match params.intent {
        McpExecutionIntent::Validation
            if !params.source_consent || params.tool_name.is_some() || params.purpose.is_some() =>
        {
            return Err(reject("mcp_source_consent_required"));
        }
        McpExecutionIntent::ToolCall => {
            if !conn.enabled
                || params
                    .purpose
                    .as_ref()
                    .is_none_or(|purpose| !conn.purposes.contains(purpose))
                || params
                    .tool_name
                    .as_ref()
                    .is_none_or(|name| !conn.approved_tool_names.contains(name))
                || conn.approved_catalog_digest.is_none()
                || params.catalog_digest != conn.approved_catalog_digest
            {
                return Err(reject("mcp_tool_not_authorized"));
            }
        }
        McpExecutionIntent::Validation => (),
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let writer = profile_writers::prepare_writer(
        tx,
        &PersonalWriterPrepareParams {
            client_operation_id: format!("mcp:{id}"),
            kind: "user_mcp".into(),
            resource_id: conn.connection_id.clone(),
            resource_revision: conn.revision.to_string(),
            owner: params.owner.clone(),
            launch_protocol: MCP_LAUNCH_PROTOCOL,
            process_mode: match &conn.config.source {
                McpSource::LocalCommand { .. } => PersonalWriterMode::LocalProcess,
                McpSource::RemoteHttp { .. } => PersonalWriterMode::RemoteRequest,
            },
        },
        profile,
        fence,
        now,
    )?;
    let snapshot_digest = digest(
        &json!({"connection":conn,"intent":params.intent,"purpose":params.purpose,"toolName":params.tool_name,"owner":params.owner,"writerLeaseId":writer.lease_id,"instanceFence":fence}),
    );
    let result = McpExecutionLease {
        lease_id: id,
        instance_fence: fence.into(),
        snapshot_digest,
        connection: conn,
        intent: params.intent,
        purpose: params.purpose,
        tool_name: params.tool_name,
        owner: params.owner,
        processes: Vec::new(),
        state: McpExecutionState::Prepared,
        writer_lease_id: writer.lease_id,
        outcome: None,
        reason_code: None,
        prepared_at: now,
    };
    put(tx, profile, "lease", &result.lease_id, &result, now)?;
    encode(&result)
}
fn register(
    tx: &Transaction<'_>,
    profile: &str,
    fence: &str,
    params: McpExecutionRegisterParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut result = lease(
        tx,
        profile,
        fence,
        &McpExecutionParams {
            lease_id: params.lease_id,
            snapshot_digest: params.snapshot_digest,
        },
        now,
        false,
    )?;
    if !matches!(
        result.connection.config.source,
        McpSource::LocalCommand { .. }
    ) {
        return Err(reject("mcp_execution_process_unexpected"));
    }
    profile_writers::register_writer_process(
        tx,
        &PersonalWriterRegisterParams {
            lease_id: result.writer_lease_id.clone(),
            process: params.process.clone(),
        },
        profile,
        fence,
        now,
    )?;
    if !result.processes.contains(&params.process) {
        result.processes.push(params.process);
    }
    result.state = McpExecutionState::Registered;
    put(tx, profile, "lease", &result.lease_id, &result, now)?;
    encode(&result)
}
fn finish(
    tx: &Transaction<'_>,
    profile: &str,
    fence: &str,
    params: McpExecutionFinishParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let current = get::<McpExecutionLease>(tx, profile, "lease", &params.lease_id)?
        .ok_or_else(|| reject("mcp_execution_not_found"))?;
    if current.snapshot_digest != params.snapshot_digest {
        return Err(reject("mcp_execution_snapshot_changed"));
    }
    if current.state == McpExecutionState::Finished {
        return encode(&current);
    }
    let mut result = lease(
        tx,
        profile,
        fence,
        &McpExecutionParams {
            lease_id: params.lease_id,
            snapshot_digest: params.snapshot_digest,
        },
        now,
        false,
    )?;
    if !matches!(
        params.outcome.as_str(),
        "success"
            | "failed"
            | "cancelled"
            | "outcome_unconfirmed"
            | "source_changed"
            | "catalog_changed"
    ) {
        return Err(reject("mcp_execution_outcome_invalid"));
    }
    if params
        .reason_code
        .as_deref()
        .is_some_and(|code| !identifier(code))
    {
        return Err(reject("mcp_execution_outcome_invalid"));
    }
    profile_writers::release_writer(
        tx,
        &PersonalWriterLeaseParams {
            lease_id: result.writer_lease_id.clone(),
            owner: Some(result.owner.clone()),
            local_transport_finished: true,
            remote_outcome_unconfirmed: params.request_sent
                && params.outcome == "outcome_unconfirmed",
        },
        profile,
        fence,
        now,
    )?;
    if params.request_sent && params.outcome == "outcome_unconfirmed" {
        let mut conn = connection(tx, profile, &result.connection.connection_id)?;
        if !conn.deleted
            && conn.enabled
            && conn.config == result.connection.config
            && conn.auth_generation == result.connection.auth_generation
        {
            conn.enabled = false;
            conn.approved_tool_names.clear();
            conn.purposes.clear();
            conn.approved_catalog_digest = None;
            conn.revision = next(conn.revision)?;
            conn.updated_at = now;
            put(tx, profile, "connection", &conn.connection_id, &conn, now)?;
        }
    }
    result.state = McpExecutionState::Finished;
    result.outcome = Some(params.outcome);
    result.reason_code = params.reason_code;
    put(tx, profile, "lease", &result.lease_id, &result, now)?;
    encode(&result)
}
fn abort(
    tx: &Transaction<'_>,
    profile: &str,
    fence: &str,
    params: &McpExecutionParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut result = lease(tx, profile, fence, params, now, false)?;
    if result.state != McpExecutionState::Prepared || !result.processes.is_empty() {
        return Err(reject("mcp_execution_already_launched"));
    }
    profile_writers::abort_writer_unlaunched(
        tx,
        &PersonalWriterAbortParams {
            lease_id: result.writer_lease_id.clone(),
            owner: result.owner.clone(),
        },
        profile,
        fence,
        now,
    )?;
    result.state = McpExecutionState::Finished;
    result.outcome = Some("cancelled".into());
    put(tx, profile, "lease", &result.lease_id, &result, now)?;
    encode(&result)
}
fn validate_report(params: &McpValidationReportParams) -> Result<(), PersistenceError> {
    if params.tools.len() > 256
        || !unique(
            &params
                .tools
                .iter()
                .map(|tool| &tool.name)
                .collect::<Vec<_>>(),
        )
        || encode(&params.tools)?.to_string().len() > 1024 * 1024
    {
        return Err(reject("mcp_catalog_invalid"));
    }
    if params.tools.iter().any(|tool| {
        !identifier(&tool.name)
            || tool
                .description
                .as_deref()
                .is_some_and(|description| description.len() > 8192)
            || !tool.input_schema.is_object()
            || tool.input_schema["type"] != "object"
    }) {
        return Err(reject("mcp_catalog_invalid"));
    }
    if params
        .server_name
        .as_deref()
        .is_some_and(|name| !text(name, 240))
        || params
            .server_version
            .as_deref()
            .is_some_and(|version| !text(version, 240))
        || params
            .reason_code
            .as_deref()
            .is_some_and(|code| !identifier(code))
    {
        return Err(reject("mcp_validation_record_invalid"));
    }
    if params.state == McpValidationState::Passed
        && (!matches!(
            params.protocol_version.as_deref(),
            Some("2024-11-05" | "2025-03-26" | "2025-06-18")
        ) || params
            .source_fingerprint
            .as_deref()
            .is_none_or(|value| !text(value, 256)))
    {
        return Err(reject("mcp_validation_unconfirmed"));
    }

    Ok(())
}
fn report(
    tx: &Transaction<'_>,
    profile: &str,
    fence: &str,
    mut params: McpValidationReportParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    validate_report(&params)?;
    let active = lease(
        tx,
        profile,
        fence,
        &McpExecutionParams {
            lease_id: params.lease_id,
            snapshot_digest: params.snapshot_digest,
        },
        now,
        true,
    )?;
    if active.intent != McpExecutionIntent::Validation {
        return Err(reject("mcp_validation_scope_invalid"));
    }
    params.tools.sort_by(|a, b| a.name.cmp(&b.name));
    let catalog_digest = if params.state == McpValidationState::Passed {
        Some(digest(&encode(&params.tools)?))
    } else {
        None
    };
    let mut record = McpValidationRecord {
        validation_id: active.lease_id,
        connection_id: active.connection.connection_id.clone(),
        revision: active.connection.revision,
        state: params.state,
        reason_code: params.reason_code,
        protocol_version: params.protocol_version,
        server_name: params.server_name,
        server_version: params.server_version,
        source_fingerprint: params.source_fingerprint,
        tools: params.tools,
        catalog_digest,
        validated_at: now,
    };
    let mut conn = current(tx, profile, &record.connection_id)?;
    let source_changed = conn
        .validation
        .as_ref()
        .is_some_and(|old| old.source_fingerprint != record.source_fingerprint);
    if conn.enabled
        && (record.state != McpValidationState::Passed
            || conn.approved_catalog_digest != record.catalog_digest
            || source_changed)
    {
        conn.enabled = false;
        conn.approved_tool_names.clear();
        conn.purposes.clear();
        conn.approved_catalog_digest = None;
        conn.revision = next(conn.revision)?;
        record.revision = conn.revision;
    }
    conn.validation = Some(record.clone());
    conn.updated_at = now;
    put(tx, profile, "connection", &conn.connection_id, &conn, now)?;
    encode(&record)
}
pub(super) fn route(
    tx: &Transaction<'_>,
    profile: &str,
    fence: &str,
    method: &str,
    params: Value,
    now: i64,
) -> Result<Value, PersistenceError> {
    match method {
        "mcpExecution/list" => encode(&list::<McpExecutionLease>(tx, profile, "lease")?),
        "mcpExecution/prepare" => prepare(tx, profile, fence, decode(params)?, now),
        "mcpExecution/registerProcess" => register(tx, profile, fence, decode(params)?, now),
        "mcpExecution/validate" => encode(&lease(tx, profile, fence, &decode(params)?, now, true)?),
        "mcpExecution/finish" => finish(tx, profile, fence, decode(params)?, now),
        "mcpExecution/abortUnlaunched" => abort(tx, profile, fence, &decode(params)?, now),
        "mcpExecution/reportValidation" => report(tx, profile, fence, decode(params)?, now),
        _ => Err(reject("mcp_method_unknown")),
    }
}
