use super::{
    LEGACY_MCP_CREDENTIAL_SERVICE, McpChangeState, McpConnection, McpConnectionChange,
    McpCredentialCleanupConfirmParams, McpCredentialCleanupParams, McpCredentialCleanupPlan,
    McpCredentialCleanupState, McpExecutionLease, McpExecutionState, McpSecretRef,
    PersistenceError, Transaction, Value, change, decode, digest, encode, get, json, list, put,
    reject,
};

pub(super) fn require_unreserved(
    tx: &Transaction<'_>,
    profile: &str,
    reference: Option<&McpSecretRef>,
) -> Result<(), PersistenceError> {
    let Some(reference) = reference else {
        return Ok(());
    };
    if list::<McpCredentialCleanupPlan>(tx, profile, "cleanup")?
        .iter()
        .any(|plan| {
            &plan.credential_ref == reference && plan.state == McpCredentialCleanupState::Prepared
        })
    {
        return Err(reject("mcp_credential_cleanup_pending"));
    }
    Ok(())
}
fn sources(
    tx: &Transaction<'_>,
    profile: &str,
    reference: &McpSecretRef,
) -> Result<Vec<String>, PersistenceError> {
    if !reference.is_scoped_to(profile) {
        return Err(reject("mcp_credentials_reference_invalid"));
    }
    if list::<McpConnection>(tx, profile, "connection")?
        .iter()
        .any(|conn| !conn.deleted && conn.credential_ref.as_ref() == Some(reference))
        || list::<McpConnectionChange>(tx, profile, "change")?
            .iter()
            .any(|change| {
                change.state == McpChangeState::Prepared
                    && change.connection.credential_ref.as_ref() == Some(reference)
            })
        || list::<McpExecutionLease>(tx, profile, "lease")?
            .iter()
            .any(|lease| {
                lease.state != McpExecutionState::Finished
                    && lease.connection.credential_ref.as_ref() == Some(reference)
            })
    {
        return Err(reject("mcp_credential_in_use"));
    }
    let mut ids = list::<McpConnectionChange>(tx, profile, "change")?
        .into_iter()
        .filter(|change| {
            change.state != McpChangeState::Prepared
                && change.cleanup_references.contains(reference)
                && !change.cleaned_references.contains(reference)
        })
        .map(|change| change.operation_id)
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    Ok(ids)
}
fn prepare(
    tx: &Transaction<'_>,
    profile: &str,
    params: McpCredentialCleanupParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    if params.credential_ref.service == LEGACY_MCP_CREDENTIAL_SERVICE {
        return Err(reject("mcp_credential_legacy_cleanup_required"));
    }
    let operation = change(tx, profile, &params.operation_id)?;
    if !operation
        .cleanup_references
        .contains(&params.credential_ref)
    {
        return Err(reject("mcp_credential_cleanup_not_requested"));
    }
    let prior = list::<McpCredentialCleanupPlan>(tx, profile, "cleanup")?
        .into_iter()
        .find(|plan| plan.credential_ref == params.credential_ref);
    if let Some(plan) = prior.as_ref()
        && plan.state == McpCredentialCleanupState::Confirmed
    {
        return encode(plan);
    }
    let source_operation_ids = sources(tx, profile, &params.credential_ref)?;
    if source_operation_ids.is_empty() || !source_operation_ids.contains(&params.operation_id) {
        return Err(reject("mcp_credential_cleanup_not_required"));
    }
    let reference_digest = digest(
        &json!({"profileId":profile,"credentialRef":params.credential_ref,"sourceOperationIds":source_operation_ids}),
    );
    if let Some(plan) = prior {
        if plan.reference_digest != reference_digest {
            return Err(reject("mcp_credential_cleanup_references_changed"));
        }
        return encode(&plan);
    }
    let plan = McpCredentialCleanupPlan {
        cleanup_id: uuid::Uuid::new_v4().simple().to_string(),
        profile_id: profile.into(),
        credential_ref: params.credential_ref,
        state: McpCredentialCleanupState::Prepared,
        source_operation_ids,
        reference_digest,
    };
    put(tx, profile, "cleanup", &plan.cleanup_id, &plan, now)?;
    encode(&plan)
}
fn confirm(
    tx: &Transaction<'_>,
    profile: &str,
    params: &McpCredentialCleanupConfirmParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut plan = get::<McpCredentialCleanupPlan>(tx, profile, "cleanup", &params.cleanup_id)?
        .ok_or_else(|| reject("mcp_credential_cleanup_not_found"))?;
    if plan.credential_ref.service == LEGACY_MCP_CREDENTIAL_SERVICE {
        return Err(reject("mcp_credential_legacy_cleanup_required"));
    }
    if plan.reference_digest != params.expected_reference_digest {
        return Err(reject("mcp_credential_cleanup_references_changed"));
    }
    if plan.state == McpCredentialCleanupState::Confirmed {
        return encode(&plan);
    }
    let source_operation_ids = sources(tx, profile, &plan.credential_ref)?;
    let actual = digest(
        &json!({"profileId":profile,"credentialRef":plan.credential_ref,"sourceOperationIds":source_operation_ids}),
    );
    if actual != plan.reference_digest {
        return Err(reject("mcp_credential_cleanup_references_changed"));
    }
    for id in &source_operation_ids {
        let mut operation = change(tx, profile, id)?;
        operation
            .cleaned_references
            .push(plan.credential_ref.clone());
        put(tx, profile, "change", id, &operation, now)?;
    }
    plan.state = McpCredentialCleanupState::Confirmed;
    put(tx, profile, "cleanup", &plan.cleanup_id, &plan, now)?;
    encode(&plan)
}
pub(super) fn route(
    tx: &Transaction<'_>,
    profile: &str,
    method: &str,
    params: Value,
    now: i64,
) -> Result<Value, PersistenceError> {
    match method {
        "mcpCredentialCleanup/list" => {
            encode(&list::<McpCredentialCleanupPlan>(tx, profile, "cleanup")?)
        }
        "mcpCredentialCleanup/prepare" => prepare(tx, profile, decode(params)?, now),
        "mcpCredentialCleanup/confirm" => confirm(tx, profile, &decode(params)?, now),
        _ => Err(reject("mcp_method_unknown")),
    }
}
