//! 本地凭据删除由 Host 执行；此处保留资料内引用及排他清理回执。
use super::{
    MODEL_CREDENTIAL_SERVICE, ModelConnection, ModelConnectionChange, ModelConnectionChangeState,
    ModelConnectionReference, ModelCredentialAction, ModelCredentialCleanupState,
    ModelExecutionLease, ModelSecretRef, ModelThreadRuntimeScope, PersistenceError, Transaction,
    Value, database_error, decoded, digest, encoded, get, json, list, put, reject, valid_id,
};
use contexthub_protocol::{
    MODEL_NETWORK_CREDENTIAL_SERVICE, ModelCredentialCleanupCandidate,
    ModelCredentialCleanupConfirmParams, ModelCredentialCleanupPlan,
    ModelCredentialCleanupPlanState, ModelCredentialCleanupPrepareParams,
};
use std::collections::BTreeMap;

fn identity(
    profile_id: &str,
    reference: &ModelSecretRef,
) -> Result<(String, u32), PersistenceError> {
    let fields: Vec<_> = reference.account.split(':').collect();
    if !matches!(
        reference.service.as_str(),
        MODEL_CREDENTIAL_SERVICE
            | MODEL_NETWORK_CREDENTIAL_SERVICE
            | contexthub_protocol::LEGACY_MODEL_CREDENTIAL_SERVICE
            | contexthub_protocol::LEGACY_MODEL_NETWORK_CREDENTIAL_SERVICE
    ) || !valid_id(profile_id)
        || fields.len() != 3
        || fields[0] != profile_id
        || !valid_id(fields[1])
    {
        return Err(reject("model_credential_scope_invalid"));
    }
    let generation = fields[2]
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == fields[2])
        .ok_or_else(|| reject("model_credential_scope_invalid"))?;
    Ok((fields[1].into(), generation))
}

fn cleanup_id(reference: &ModelSecretRef) -> Result<String, PersistenceError> {
    Ok(digest(&encoded(reference)?))
}

fn insert_reference(
    references: &mut BTreeMap<String, ModelSecretRef>,
    profile_id: &str,
    reference: &ModelSecretRef,
) -> Result<(), PersistenceError> {
    identity(profile_id, reference)?;
    references.insert(cleanup_id(reference)?, reference.clone());
    Ok(())
}

fn native_references(
    tx: &Transaction<'_>,
    profile_id: &str,
    connection_id: &str,
    generation: Option<u32>,
) -> Result<Vec<ModelConnectionReference>, PersistenceError> {
    let mut statement = tx.prepare("SELECT conversation_id,runtime_scope_json FROM ai_conversation_thread_bindings WHERE owner_instance_id IS NOT NULL OR active_run_id IS NOT NULL OR writer_parked=1 ORDER BY conversation_id").map_err(database_error("read credential native owners"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(database_error("query credential native owners"))?;
    let mut references = Vec::new();
    for row in rows {
        let (id, raw) = row.map_err(database_error("decode credential native owner"))?;
        let scope = raw.and_then(|raw| serde_json::from_str::<ModelThreadRuntimeScope>(&raw).ok());
        let kind = match scope {
            Some(scope) if scope.profile_id == profile_id => {
                if scope.connection_id != connection_id
                    || generation.is_some_and(|generation| scope.auth_generation != generation)
                {
                    continue;
                }
                "active_native_thread"
            }
            _ => "unconfirmed_native_writer",
        };
        references.push(ModelConnectionReference {
            kind: kind.into(),
            id,
        });
    }
    Ok(references)
}

struct Inventory {
    connections: Vec<ModelConnection>,
    changes: Vec<ModelConnectionChange>,
    leases: Vec<ModelExecutionLease>,
    plans: Vec<ModelCredentialCleanupPlan>,
}

impl Inventory {
    fn read(tx: &Transaction<'_>) -> Result<Self, PersistenceError> {
        Ok(Self {
            connections: list(tx, "connection")?,
            changes: list(tx, "change")?,
            leases: list(tx, "lease")?,
            plans: list(tx, "credentialCleanup")?,
        })
    }

    fn references(
        &self,
        profile_id: &str,
    ) -> Result<BTreeMap<String, ModelSecretRef>, PersistenceError> {
        let mut references = BTreeMap::new();
        for connection in self
            .connections
            .iter()
            .chain(self.changes.iter().map(|c| &c.connection))
        {
            if connection.profile_id != profile_id {
                return Err(reject("model_credential_scope_invalid"));
            }
            if let Some(reference) = &connection.secret_ref {
                let (id, generation) = identity(profile_id, reference)?;
                if !matches!(
                    reference.service.as_str(),
                    MODEL_CREDENTIAL_SERVICE | contexthub_protocol::LEGACY_MODEL_CREDENTIAL_SERVICE
                ) || id != connection.connection_id
                    || generation != connection.auth_generation
                {
                    return Err(reject("model_credential_scope_invalid"));
                }
                insert_reference(&mut references, profile_id, reference)?;
            }
            if let Some(reference) = &connection.config.network.proxy_secret_ref {
                let (id, generation) = identity(profile_id, reference)?;
                if !matches!(
                    reference.service.as_str(),
                    MODEL_NETWORK_CREDENTIAL_SERVICE
                        | contexthub_protocol::LEGACY_MODEL_NETWORK_CREDENTIAL_SERVICE
                ) || id != connection.connection_id
                    || generation > connection.config_revision
                {
                    return Err(reject("model_credential_scope_invalid"));
                }
                insert_reference(&mut references, profile_id, reference)?;
            }
        }
        for plan in &self.plans {
            validate_plan(profile_id, plan)?;
            insert_reference(&mut references, profile_id, &plan.credential_ref)?;
        }
        Ok(references)
    }
}

fn validate_plan(
    profile_id: &str,
    plan: &ModelCredentialCleanupPlan,
) -> Result<(), PersistenceError> {
    let (id, generation) = identity(profile_id, &plan.credential_ref)?;
    if plan.profile_id != profile_id
        || plan.connection_id != id
        || plan.auth_generation != generation
        || plan.cleanup_id != cleanup_id(&plan.credential_ref)?
    {
        return Err(reject("model_credential_scope_invalid"));
    }
    Ok(())
}

fn candidate(
    tx: &Transaction<'_>,
    profile_id: &str,
    reference: ModelSecretRef,
    inventory: &Inventory,
) -> Result<ModelCredentialCleanupCandidate, PersistenceError> {
    let (connection_id, auth_generation) = identity(profile_id, &reference)?;
    let id = cleanup_id(&reference)?;
    let changes: Vec<_> = inventory
        .changes
        .iter()
        .filter(|c| references_credential(&c.connection, &reference))
        .collect();
    let source_operation_ids: Vec<_> = changes.iter().map(|c| c.operation_id.clone()).collect();
    let connection = inventory
        .connections
        .iter()
        .find(|c| c.connection_id == connection_id);
    let current = connection.is_some_and(|c| !c.deleted && references_credential(c, &reference));
    let mut blocked_references = native_references(
        tx,
        profile_id,
        &connection_id,
        (reference.service == MODEL_CREDENTIAL_SERVICE).then_some(auth_generation),
    )?;
    if current {
        blocked_references.push(ModelConnectionReference {
            kind: "current_connection".into(),
            id: connection_id.clone(),
        });
    }
    blocked_references.extend(
        changes
            .iter()
            .filter(|c| c.state == ModelConnectionChangeState::Prepared)
            .map(|c| ModelConnectionReference {
                kind: "prepared_change".into(),
                id: c.operation_id.clone(),
            }),
    );
    let active_lease_ids: Vec<_> = inventory
        .leases
        .iter()
        .filter(|l| {
            !l.released
                && (l.snapshot.secret_ref.as_ref() == Some(&reference)
                    || l.snapshot.network.proxy_secret_ref.as_ref() == Some(&reference))
        })
        .map(|l| l.lease_id.clone())
        .collect();
    let plan = inventory.plans.iter().find(|p| p.cleanup_id == id).cloned();
    let confirmed_abort = !changes.is_empty()
        && reference.service == MODEL_CREDENTIAL_SERVICE
        && connection.is_none_or(|c| !references_credential(c, &reference))
        && changes.iter().all(|c| {
            c.state == ModelConnectionChangeState::Aborted
                && c.credential_action == ModelCredentialAction::Replace
                && c.credential_cleanup == ModelCredentialCleanupState::Confirmed
        });
    let state = match plan.as_ref().map(|p| &p.state) {
        Some(ModelCredentialCleanupPlanState::Pending) => ModelCredentialCleanupState::Pending,
        Some(ModelCredentialCleanupPlanState::Confirmed) => ModelCredentialCleanupState::Confirmed,
        None if confirmed_abort => ModelCredentialCleanupState::Confirmed,
        None if current => ModelCredentialCleanupState::NotRequired,
        None => ModelCredentialCleanupState::Unconfirmed,
    };
    let legacy = contexthub_protocol::is_legacy_model_credential_reference(&reference);
    let deleted_connection = connection.is_some_and(|c| c.deleted);
    let reference_digest = digest(
        &json!({"credentialRef":reference,"sourceOperationIds":source_operation_ids,"deletedConnection":deleted_connection,"blockedReferences":blocked_references,"activeLeaseIds":active_lease_ids}),
    );
    Ok(ModelCredentialCleanupCandidate {
        cleanup_id: id,
        profile_id: profile_id.into(),
        connection_id,
        auth_generation,
        credential_ref: reference,
        source_operation_ids,
        deleted_connection,
        can_prepare: !legacy
            && blocked_references.is_empty()
            && active_lease_ids.is_empty()
            && state != ModelCredentialCleanupState::Confirmed,
        blocked_references,
        active_lease_ids,
        reference_digest,
        state,
        plan,
    })
}

fn references_credential(connection: &ModelConnection, reference: &ModelSecretRef) -> bool {
    connection.secret_ref.as_ref() == Some(reference)
        || connection.config.network.proxy_secret_ref.as_ref() == Some(reference)
}

fn candidates(
    tx: &Transaction<'_>,
    profile_id: &str,
) -> Result<Vec<ModelCredentialCleanupCandidate>, PersistenceError> {
    let inventory = Inventory::read(tx)?;
    inventory
        .references(profile_id)?
        .into_values()
        .map(|reference| candidate(tx, profile_id, reference, &inventory))
        .collect()
}

fn selected(
    tx: &Transaction<'_>,
    profile_id: &str,
    id: &str,
) -> Result<ModelCredentialCleanupCandidate, PersistenceError> {
    candidates(tx, profile_id)?
        .into_iter()
        .find(|c| c.cleanup_id == id)
        .ok_or_else(|| reject("model_credential_cleanup_not_found"))
}

fn check_eligible(
    candidate: &ModelCredentialCleanupCandidate,
    expected_digest: &str,
) -> Result<(), PersistenceError> {
    if contexthub_protocol::is_legacy_model_credential_reference(&candidate.credential_ref) {
        return Err(reject("model_credential_legacy_cleanup_required"));
    }
    if candidate.reference_digest != expected_digest {
        return Err(reject("model_credential_cleanup_impact_changed"));
    }
    if !candidate.blocked_references.is_empty() || !candidate.active_lease_ids.is_empty() {
        return Err(reject("model_credential_cleanup_in_use"));
    }
    Ok(())
}

fn prepare(
    tx: &Transaction<'_>,
    params: &ModelCredentialCleanupPrepareParams,
    profile_id: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let candidate = selected(tx, profile_id, &params.cleanup_id)?;
    check_eligible(&candidate, &params.expected_reference_digest)?;
    if let Some(plan) = &candidate.plan
        && plan.state == ModelCredentialCleanupPlanState::Confirmed
    {
        return encoded(plan);
    }
    if candidate.state == ModelCredentialCleanupState::Confirmed {
        return Err(reject("model_credential_cleanup_already_confirmed"));
    }
    let plan = ModelCredentialCleanupPlan {
        cleanup_id: candidate.cleanup_id,
        profile_id: profile_id.into(),
        connection_id: candidate.connection_id,
        auth_generation: candidate.auth_generation,
        credential_ref: candidate.credential_ref,
        reference_digest: candidate.reference_digest,
        state: ModelCredentialCleanupPlanState::Pending,
        prepared_at: candidate.plan.map_or(now, |p| p.prepared_at),
        confirmed_at: None,
    };
    put(tx, "credentialCleanup", &plan.cleanup_id, &plan, now)?;
    encoded(&plan)
}

fn acknowledge(
    tx: &Transaction<'_>,
    params: &ModelCredentialCleanupConfirmParams,
    profile_id: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut plan = get::<ModelCredentialCleanupPlan>(tx, "credentialCleanup", &params.cleanup_id)?
        .ok_or_else(|| reject("model_credential_cleanup_not_prepared"))?;
    validate_plan(profile_id, &plan)?;
    if params.credential_ref != plan.credential_ref
        || params.expected_reference_digest != plan.reference_digest
    {
        return Err(reject("model_credential_cleanup_receipt_mismatch"));
    }
    if contexthub_protocol::is_legacy_model_credential_reference(&plan.credential_ref) {
        return Err(reject("model_credential_legacy_cleanup_required"));
    }
    if plan.state == ModelCredentialCleanupPlanState::Confirmed {
        return encoded(&plan);
    }
    let candidate = selected(tx, profile_id, &params.cleanup_id)?;
    check_eligible(&candidate, &params.expected_reference_digest)?;
    plan.state = ModelCredentialCleanupPlanState::Confirmed;
    plan.confirmed_at = Some(now);
    put(tx, "credentialCleanup", &plan.cleanup_id, &plan, now)?;
    for mut change in list::<ModelConnectionChange>(tx, "change")? {
        if change.state == ModelConnectionChangeState::Aborted
            && change.credential_action == ModelCredentialAction::Replace
            && change.connection.secret_ref.as_ref() == Some(&plan.credential_ref)
        {
            change.credential_cleanup = ModelCredentialCleanupState::Confirmed;
            put(tx, "change", &change.operation_id, &change, now)?;
        }
    }
    encoded(&plan)
}

pub(super) fn require_available(
    tx: &Transaction<'_>,
    reference: Option<&ModelSecretRef>,
) -> Result<(), PersistenceError> {
    if reference.is_some_and(contexthub_protocol::is_legacy_model_credential_reference) {
        return Err(reject("model_credential_reentry_required"));
    }
    if let Some(reference) = reference
        && let Some(plan) =
            get::<ModelCredentialCleanupPlan>(tx, "credentialCleanup", &cleanup_id(reference)?)?
    {
        return Err(reject(
            if plan.state == ModelCredentialCleanupPlanState::Pending {
                "model_credential_cleanup_pending"
            } else {
                "model_credential_reference_cleared"
            },
        ));
    }
    Ok(())
}

pub(super) fn route(
    tx: &Transaction<'_>,
    method: &str,
    params: Value,
    profile_id: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    match method {
        "modelCredentialCleanup/list" => encoded(&candidates(tx, profile_id)?),
        "modelCredentialCleanup/prepare" => prepare(tx, &decoded(params)?, profile_id, now),
        "modelCredentialCleanup/ack" => acknowledge(tx, &decoded(params)?, profile_id, now),
        _ => Err(reject("model_method_not_found")),
    }
}
