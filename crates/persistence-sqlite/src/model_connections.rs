//! 个人连接与用途使用现有逐条账本；凭据正文不进入业务账本。
use super::{PersistenceError, SqlitePersistence, database_error};
use crate::ai_business_ledger::{apply_model_batch, digest, reject};
use contexthub_protocol::{
    MODEL_CREDENTIAL_SERVICE, MODEL_ENTRY_OWNER, MODEL_ENTRY_PREFIX, MODEL_LAUNCH_PROTOCOL,
    ModelAuthAbortUnlaunchedParams, ModelAuthChangeState, ModelAuthIntent,
    ModelAuthLogoutPrepareParams, ModelAuthMode, ModelAuthProcess, ModelAuthRecoverParams,
    ModelAuthRegisterProcessParams, ModelAuthScope, ModelAuthScopeState,
    ModelAuthStartPrepareParams, ModelAuthStep, ModelAuthStepParams, ModelAuthStepState,
    ModelCapabilityEvidence, ModelCapabilityState, ModelConnection, ModelConnectionChange,
    ModelConnectionChangeIntent, ModelConnectionChangeState, ModelConnectionCommitParams,
    ModelConnectionConfig, ModelConnectionKind, ModelConnectionMutation,
    ModelConnectionMutationParams, ModelConnectionParams, ModelConnectionPreflight,
    ModelConnectionPreflightParams, ModelConnectionReference, ModelConnectionSavePrepareParams,
    ModelConnectionSetEnabledParams, ModelCredentialAction, ModelCredentialCleanupAckParams,
    ModelCredentialCleanupState, ModelDescriptor, ModelDescriptorObserveParams,
    ModelDescriptorSaveParams, ModelEvidenceSource, ModelExecutionAbortUnlaunchedParams,
    ModelExecutionLease, ModelExecutionLeaseDiagnostic, ModelExecutionLeaseParams,
    ModelExecutionPrepareParams, ModelExecutionProcess, ModelExecutionRecoverParams,
    ModelExecutionRecoverResult, ModelExecutionRecoveryState, ModelExecutionRegisterProcessParams,
    ModelExecutionSnapshot, ModelNetworkMode, ModelOperationParams, ModelPurpose,
    ModelPurposeBinding, ModelPurposeSaveParams, ModelRuntimeKind, ModelSecretRef, ModelSelection,
    ModelSelectionPolicy, ModelThreadRuntimeScope, ModelValidationExecutionParams,
    ModelValidationLayer, ModelValidationOutcome, ModelValidationPrepareParams,
    ModelValidationRecord, ModelValidationReportParams, RootAuthChange,
    model_active_auth_generation, model_purpose_runtime_kind, model_runtime_home_scope,
    model_validation_auth_generation,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[path = "model_auth.rs"]
mod auth;
#[path = "model_credentials_cleanup.rs"]
mod credentials_cleanup;
#[path = "model_network_credentials.rs"]
mod network_credentials;
#[cfg(test)]
#[path = "model_connections_tests.rs"]
mod tests;

fn encoded<T: Serialize>(value: &T) -> Result<Value, PersistenceError> {
    serde_json::to_value(value).map_err(|_| reject("model_record_invalid"))
}
fn decoded<T: DeserializeOwned>(value: Value) -> Result<T, PersistenceError> {
    serde_json::from_value(value).map_err(|_| reject("model_request_invalid"))
}
fn entry_id(kind: &str, key: &str) -> String {
    format!("{MODEL_ENTRY_PREFIX}{kind}:{key}")
}
fn entry(tx: &Transaction<'_>, kind: &str, key: &str) -> Result<Option<Value>, PersistenceError> {
    let id = entry_id(kind, key);
    let raw: Option<String> = tx
        .query_row(
            "SELECT record_json FROM ai_runtime_entries WHERE record_id=?1",
            [&id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error("read model entry"))?;
    raw.map(|raw| {
        let value: Value =
            serde_json::from_str(&raw).map_err(|_| reject("model_storage_invalid"))?;
        if value["entryId"] != id || value["owner"] != MODEL_ENTRY_OWNER || value["kind"] != kind {
            return Err(reject("model_storage_invalid"));
        }
        Ok(value)
    })
    .transpose()
}
fn get<T: DeserializeOwned>(
    tx: &Transaction<'_>,
    kind: &str,
    key: &str,
) -> Result<Option<T>, PersistenceError> {
    entry(tx, kind, key)?
        .map(|v| {
            serde_json::from_value(v["value"].clone()).map_err(|_| reject("model_storage_invalid"))
        })
        .transpose()
}
fn list<T: DeserializeOwned>(tx: &Transaction<'_>, kind: &str) -> Result<Vec<T>, PersistenceError> {
    let mut statement = tx
        .prepare(
            "SELECT record_json FROM ai_runtime_entries WHERE record_id GLOB ?1 ORDER BY record_id",
        )
        .map_err(database_error("list model entries"))?;
    let rows = statement
        .query_map([format!("{MODEL_ENTRY_PREFIX}{kind}:*")], |row| {
            row.get::<_, String>(0)
        })
        .map_err(database_error("query model entries"))?;
    rows.map(|row| {
        let raw = row.map_err(database_error("read model row"))?;
        let value: Value =
            serde_json::from_str(&raw).map_err(|_| reject("model_storage_invalid"))?;
        if value["owner"] != MODEL_ENTRY_OWNER || value["kind"] != kind {
            return Err(reject("model_storage_invalid"));
        }
        serde_json::from_value(value["value"].clone()).map_err(|_| reject("model_storage_invalid"))
    })
    .collect()
}
fn put_extra<T: Serialize>(
    tx: &Transaction<'_>,
    kind: &str,
    key: &str,
    source: &T,
    extra: Option<(&str, Value)>,
    now: i64,
) -> Result<(), PersistenceError> {
    let id = entry_id(kind, key);
    let expected: Option<String> = tx
        .query_row(
            "SELECT digest FROM ai_runtime_entries WHERE record_id=?1",
            [&id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error("read model write baseline"))?;
    let mut value =
        json!({"entryId":id,"owner":MODEL_ENTRY_OWNER,"kind":kind,"value":encoded(source)?});
    if let Some((key, extra)) = extra {
        value[key] = extra;
    }
    apply_model_batch(
        tx,
        &json!({"commandId":format!("{MODEL_ENTRY_PREFIX}{}",uuid::Uuid::new_v4().simple()),"records":[{"collection":"entries","id":id,"expectedDigest":expected,"position":0,"value":value}]}),
        now,
    )?;
    Ok(())
}
fn put<T: Serialize>(
    tx: &Transaction<'_>,
    kind: &str,
    key: &str,
    source: &T,
    now: i64,
) -> Result<(), PersistenceError> {
    put_extra(tx, kind, key, source, None, now)
}
fn connection(tx: &Transaction<'_>, id: &str) -> Result<ModelConnection, PersistenceError> {
    get(tx, "connection", id)?.ok_or_else(|| reject("model_connection_not_found"))
}
fn alive_connection(tx: &Transaction<'_>, id: &str) -> Result<ModelConnection, PersistenceError> {
    let value = connection(tx, id)?;
    if value.deleted {
        return Err(reject("model_connection_deleted"));
    }
    Ok(value)
}
fn next(value: u32) -> Result<u32, PersistenceError> {
    value
        .checked_add(1)
        .ok_or_else(|| reject("model_revision_exhausted"))
}
fn valid_text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
fn valid_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn descriptor_id(connection_id: &str, model_id: &str) -> String {
    digest(&json!([connection_id, model_id]))
}
fn validate_url(url: &str, local_only: bool) -> Result<(), PersistenceError> {
    if !valid_text(url, 2048) || url.contains(['?', '#', '@', '\\']) {
        return Err(reject("model_endpoint_invalid"));
    }
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| reject("model_endpoint_invalid"))?;
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty()
        || !authority
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".:-[]".contains(&b))
    {
        return Err(reject("model_endpoint_invalid"));
    }
    let host = if authority.starts_with('[') {
        authority
            .split_once(']')
            .map_or("", |(h, _)| h.trim_start_matches('['))
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
    if (scheme != "https" && !(scheme == "http" && loopback)) || (local_only && !loopback) {
        return Err(reject("model_endpoint_tls_required"));
    }
    Ok(())
}
fn validate_config(config: &ModelConnectionConfig) -> Result<(), PersistenceError> {
    contexthub_protocol::validate_model_network_declarations(config).map_err(reject)?;
    if !valid_text(&config.name, 100) {
        return Err(reject("model_connection_name_invalid"));
    }
    match config.kind {
        ModelConnectionKind::OpenaiKey => {
            if config.auth_mode != ModelAuthMode::ApiKey
                || config.local_runtime.is_some()
                || config
                    .base_url
                    .as_deref()
                    .is_some_and(|v| v.trim_end_matches('/') != "https://api.openai.com/v1")
            {
                return Err(reject("model_connection_config_invalid"));
            }
        }
        ModelConnectionKind::Chatgpt => {
            if config.auth_mode != ModelAuthMode::RuntimeManagedChatgpt
                || config.base_url.is_some()
                || config.local_runtime.is_some()
            {
                return Err(reject("model_connection_config_invalid"));
            }
        }
        ModelConnectionKind::CustomResponses | ModelConnectionKind::Local => {
            if config.auth_mode == ModelAuthMode::RuntimeManagedChatgpt
                || (config.kind == ModelConnectionKind::Local) != config.local_runtime.is_some()
            {
                return Err(reject("model_connection_config_invalid"));
            }
            validate_url(
                config
                    .base_url
                    .as_deref()
                    .ok_or_else(|| reject("model_endpoint_missing"))?,
                config.kind == ModelConnectionKind::Local,
            )?;
        }
    }
    if let Some(pem) = &config.network.ca_certificate_pem {
        contexthub_protocol::validate_model_ca_pem_shape(pem).map_err(reject)?;
    }
    match config.network.mode {
        ModelNetworkMode::CustomProxy => network_credentials::validate_proxy_url(
            config
                .network
                .proxy_url
                .as_deref()
                .ok_or_else(|| reject("model_proxy_missing"))?,
        )?,
        _ if config.network.proxy_url.is_some() => {
            return Err(reject("model_proxy_config_invalid"));
        }
        _ => (),
    }
    if let Some(reference) = &config.network.proxy_secret_ref
        && (!matches!(reference.service.as_str(), contexthub_protocol::MODEL_NETWORK_CREDENTIAL_SERVICE | contexthub_protocol::LEGACY_MODEL_NETWORK_CREDENTIAL_SERVICE)
            || !valid_text(&reference.account, 200))
    {
        return Err(reject("model_proxy_secret_scope_invalid"));
    }
    Ok(())
}

fn validate_edit(
    old: Option<&ModelConnection>,
    params: &ModelConnectionSavePrepareParams,
) -> Result<(), PersistenceError> {
    if let Some(old) = old {
        if old.deleted {
            return Err(reject("model_connection_deleted"));
        }
        if old.config.kind != params.config.kind
            || old.config.auth_mode != params.config.auth_mode
            || old.config.local_runtime != params.config.local_runtime
        {
            return Err(reject("model_connection_kind_immutable"));
        }
        if old.config.enabled != params.config.enabled {
            return Err(reject("model_connection_use_set_enabled"));
        }
        if old.config.kind == ModelConnectionKind::Chatgpt
            && params.credential_action != ModelCredentialAction::Keep
        {
            return Err(reject("model_auth_private_operation_required"));
        }
        if old.config.base_url != params.config.base_url
            && params.credential_action == ModelCredentialAction::Keep
        {
            return Err(reject("model_endpoint_change_requires_auth_rotation"));
        }
    }
    Ok(())
}
fn validate_clear(
    tx: &Transaction<'_>,
    params: &ModelConnectionSavePrepareParams,
    id: &str,
    has_previous: bool,
) -> Result<(), PersistenceError> {
    if params.credential_action == ModelCredentialAction::Clear && has_previous {
        let impact = preflight(
            tx,
            &ModelConnectionPreflightParams {
                connection_id: id.to_owned(),
                action: ModelConnectionMutation::Disconnect,
            },
        )?;
        if !impact.active_lease_ids.is_empty()
            || impact
                .references
                .iter()
                .any(|r| r.kind == "active_native_thread")
        {
            return Err(reject("model_connection_in_use"));
        }
    }
    Ok(())
}

fn reserved_revisions(
    pending: &[ModelConnectionChange],
    old: Option<&ModelConnection>,
    id: &str,
) -> (u32, u32) {
    let max_config = pending
        .iter()
        .filter(|c| c.connection.connection_id == id)
        .map(|c| c.connection.config_revision)
        .chain(old.iter().map(|c| c.config_revision))
        .max()
        .unwrap_or(0);
    let max_auth = pending
        .iter()
        .filter(|c| c.connection.connection_id == id)
        .map(|c| c.connection.auth_generation)
        .chain(old.iter().map(|c| c.auth_generation))
        .max()
        .unwrap_or(0);
    (max_config, max_auth)
}

fn validate_pending_change(
    pending: &[ModelConnectionChange],
    id: &str,
    params: &ModelConnectionSavePrepareParams,
) -> Result<(), PersistenceError> {
    if pending.iter().any(|c| {
        c.connection.connection_id == id
            && c.state == ModelConnectionChangeState::Prepared
            && (c.credential_action == ModelCredentialAction::Clear
                || params.credential_action == ModelCredentialAction::Clear)
    }) {
        return Err(reject("model_connection_auth_change_pending"));
    }
    Ok(())
}

fn save_prepare(
    tx: &Transaction<'_>,
    params: ModelConnectionSavePrepareParams,
    profile_id: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    if !valid_text(&params.operation_id, 100) {
        return Err(reject("model_operation_id_invalid"));
    }
    validate_config(&params.config)?;
    if params
        .config
        .network
        .proxy_secret_ref
        .as_ref()
        .is_some_and(|r| !r.account.starts_with(&format!("{profile_id}:")))
    {
        return Err(reject("model_proxy_secret_scope_invalid"));
    }
    let request_digest = digest(&encoded(&params)?);
    if let Some(known) = get::<ModelConnectionChange>(tx, "change", &params.operation_id)? {
        if known.request_digest != request_digest {
            return Err(reject("model_operation_conflict"));
        }
        return encoded(&known);
    }
    let id = params
        .connection_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    if !valid_id(&id) {
        return Err(reject("model_connection_id_invalid"));
    }
    let old = get::<ModelConnection>(tx, "connection", &id)?;
    if auth::pending(tx, &id, None)? {
        return Err(reject("model_connection_auth_change_pending"));
    }
    if get::<RootAuthChange>(tx, "authChange", &params.operation_id)?.is_some() {
        return Err(reject("model_operation_owner_mismatch"));
    }
    if old.as_ref().map(|c| c.config_revision) != params.expected_config_revision {
        return Err(reject("model_connection_revision_conflict"));
    }
    validate_edit(old.as_ref(), &params)?;
    if params.credential_action == ModelCredentialAction::Replace
        && params.config.auth_mode != ModelAuthMode::ApiKey
    {
        return Err(reject("model_credential_action_invalid"));
    }
    let pending = list::<ModelConnectionChange>(tx, "change")?;
    validate_pending_change(&pending, &id, &params)?;
    validate_clear(tx, &params, &id, old.is_some())?;
    let (max_config, max_auth) = reserved_revisions(&pending, old.as_ref(), &id);
    let auth_generation = if params.credential_action == ModelCredentialAction::Keep {
        old.as_ref()
            .map_or_else(|| next(max_auth), |c| Ok(c.auth_generation))?
    } else {
        next(max_auth)?
    };
    let secret_ref = match params.credential_action {
        ModelCredentialAction::Keep => {
            let reference = old.as_ref().and_then(|c| c.secret_ref.clone());
            if reference.as_ref().is_some_and(contexthub_protocol::is_legacy_model_credential_reference) {
                return Err(reject("model_credential_reentry_required"));
            }
            reference
        },
        ModelCredentialAction::Replace => Some(ModelSecretRef {
            service: MODEL_CREDENTIAL_SERVICE.into(),
            account: format!("{profile_id}:{id}:{auth_generation}"),
        }),
        ModelCredentialAction::Clear => None,
    };
    credentials_cleanup::require_available(tx, secret_ref.as_ref())?;
    let config_revision = next(max_config)?;
    let config =
        network_credentials::prepare(tx, profile_id, &id, config_revision, old.as_ref(), &params)?;
    let change = ModelConnectionChange {
        intent: ModelConnectionChangeIntent::ConnectionSave,
        operation_id: params.operation_id,
        request_digest,
        expected_config_revision: params.expected_config_revision,
        connection: ModelConnection {
            profile_id: profile_id.into(),
            connection_id: id,
            config_revision,
            auth_generation,
            auth_scopes: old
                .as_ref()
                .map(|c| c.auth_scopes.clone())
                .unwrap_or_default(),
            config,
            secret_ref,
            deleted: false,
            created_at: old.map_or(now, |c| c.created_at),
            updated_at: now,
        },
        credential_action: params.credential_action,
        network_credential_action: params.network_credential_action,
        state: ModelConnectionChangeState::Prepared,
        credential_cleanup: ModelCredentialCleanupState::NotRequired,
        prepared_at: now,
    };
    put(tx, "change", &change.operation_id, &change, now)?;
    encoded(&change)
}
fn save_commit(
    tx: &Transaction<'_>,
    params: &ModelConnectionCommitParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    if get::<RootAuthChange>(tx, "authChange", &params.operation_id)?.is_some() {
        return Err(reject("model_operation_owner_mismatch"));
    }
    let mut change = get::<ModelConnectionChange>(tx, "change", &params.operation_id)?
        .ok_or_else(|| reject("model_operation_not_found"))?;
    if change.connection.secret_ref != params.credential_ref
        || change.connection.config.network.proxy_secret_ref != params.network_credential_ref
    {
        return Err(reject("model_credential_receipt_mismatch"));
    }
    match change.state {
        ModelConnectionChangeState::Committed => return encoded(&change.connection),
        ModelConnectionChangeState::Aborted => return Err(reject("model_operation_aborted")),
        ModelConnectionChangeState::Prepared => (),
    }
    let current = get::<ModelConnection>(tx, "connection", &change.connection.connection_id)?;
    if current.as_ref().map(|c| c.config_revision) != change.expected_config_revision {
        return Err(reject("model_connection_revision_conflict"));
    }
    credentials_cleanup::require_available(tx, change.connection.secret_ref.as_ref())?;
    credentials_cleanup::require_available(
        tx,
        change.connection.config.network.proxy_secret_ref.as_ref(),
    )?;
    put(
        tx,
        "connection",
        &change.connection.connection_id,
        &change.connection,
        now,
    )?;
    change.state = ModelConnectionChangeState::Committed;
    put(tx, "change", &change.operation_id, &change, now)?;
    encoded(&change.connection)
}
fn change_abort(
    tx: &Transaction<'_>,
    params: &ModelOperationParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    if get::<RootAuthChange>(tx, "authChange", &params.operation_id)?.is_some() {
        return Err(reject("model_operation_owner_mismatch"));
    }
    let mut change = get::<ModelConnectionChange>(tx, "change", &params.operation_id)?
        .ok_or_else(|| reject("model_operation_not_found"))?;
    if change.state == ModelConnectionChangeState::Committed {
        return Err(reject("model_operation_already_committed"));
    }
    if change.state == ModelConnectionChangeState::Aborted {
        return encoded(&change);
    }
    change.state = ModelConnectionChangeState::Aborted;
    change.credential_cleanup = if change.credential_action == ModelCredentialAction::Replace {
        ModelCredentialCleanupState::Pending
    } else {
        ModelCredentialCleanupState::NotRequired
    };
    put(tx, "change", &change.operation_id, &change, now)?;
    encoded(&change)
}

fn change_status(tx: &Transaction<'_>, params: Value) -> Result<Value, PersistenceError> {
    encoded(
        &get::<ModelConnectionChange>(
            tx,
            "change",
            &decoded::<ModelOperationParams>(params)?.operation_id,
        )?
        .ok_or_else(|| reject("model_operation_not_found"))?,
    )
}

fn credential_cleanup_ack(
    tx: &Transaction<'_>,
    params: &ModelCredentialCleanupAckParams,
    profile_id: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    if get::<RootAuthChange>(tx, "authChange", &params.operation_id)?.is_some() {
        return Err(reject("model_operation_owner_mismatch"));
    }
    let mut change = get::<ModelConnectionChange>(tx, "change", &params.operation_id)?
        .ok_or_else(|| reject("model_operation_not_found"))?;
    if change.state != ModelConnectionChangeState::Aborted
        || change.credential_action != ModelCredentialAction::Replace
    {
        return Err(reject("model_credential_cleanup_not_aborted"));
    }
    let expected = ModelSecretRef {
        service: MODEL_CREDENTIAL_SERVICE.into(),
        account: format!(
            "{profile_id}:{}:{}",
            change.connection.connection_id, change.connection.auth_generation
        ),
    };
    if change.connection.profile_id != profile_id
        || change.connection.secret_ref.as_ref() != Some(&expected)
        || params.credential_ref != expected
    {
        return Err(reject("model_credential_cleanup_reference_mismatch"));
    }
    if params.credential_ref.service == contexthub_protocol::LEGACY_MODEL_CREDENTIAL_SERVICE {
        return Err(reject("model_credential_legacy_cleanup_required"));
    }
    if change.credential_cleanup != ModelCredentialCleanupState::Confirmed {
        change.credential_cleanup = ModelCredentialCleanupState::Confirmed;
        put(tx, "change", &change.operation_id, &change, now)?;
    }
    encoded(&change)
}
fn descriptor_save(
    tx: &Transaction<'_>,
    params: ModelDescriptorSaveParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let conn = alive_connection(tx, &params.connection_id)?;
    if !valid_text(&params.model_id, 160) || !valid_text(&params.display_name, 160) {
        return Err(reject("model_descriptor_invalid"));
    }
    let id = descriptor_id(&params.connection_id, &params.model_id);
    let known = get::<ModelDescriptor>(tx, "descriptor", &id)?;
    if known.as_ref().map(|c| c.revision) != params.expected_revision {
        return Err(reject("model_descriptor_revision_conflict"));
    }
    validate_catalog_metadata(
        params.context_window_tokens,
        &params.input_modalities,
        params.default_reasoning_effort.as_ref(),
        &params.supported_reasoning_efforts,
    )?;
    let capabilities = known
        .as_ref()
        .filter(|d| {
            d.config_revision == conn.config_revision && d.auth_generation == conn.auth_generation
        })
        .map(|d| d.capabilities.clone())
        .unwrap_or_default();
    let descriptor = ModelDescriptor {
        descriptor_id: id.clone(),
        connection_id: params.connection_id,
        model_id: params.model_id,
        display_name: params.display_name,
        revision: next(known.map_or(0, |d| d.revision))?,
        config_revision: conn.config_revision,
        auth_generation: conn.auth_generation,
        source: ModelEvidenceSource::UserDeclared,
        capabilities,
        context_window_tokens: params.context_window_tokens,
        input_modalities: params.input_modalities,
        default_reasoning_effort: params.default_reasoning_effort,
        supported_reasoning_efforts: params.supported_reasoning_efforts,
        runtime_version: None,
        service_version: None,
        updated_at: now,
    };
    put(tx, "descriptor", &id, &descriptor, now)?;
    encoded(&descriptor)
}
fn validate_catalog_metadata(
    context: Option<u64>,
    modalities: &[String],
    default_effort: Option<&String>,
    efforts: &[String],
) -> Result<(), PersistenceError> {
    if context.is_some_and(|n| n == 0 || n > 1_000_000_000)
        || modalities.len() > 4
        || modalities
            .iter()
            .any(|v| !matches!(v.as_str(), "text" | "image" | "audio" | "video"))
        || efforts.len() > 32
        || efforts.iter().any(|v| !valid_text(v, 32))
        || default_effort.is_some_and(|v| !efforts.contains(v))
    {
        return Err(reject("model_catalog_record_invalid"));
    }
    Ok(())
}
fn descriptor_observe(
    tx: &Transaction<'_>,
    params: ModelDescriptorObserveParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let conn = alive_connection(tx, &params.connection_id)?;
    if conn.config_revision != params.expected_config_revision
        || conn.auth_generation != params.expected_auth_generation
    {
        return Err(reject("model_catalog_stale"));
    }
    if params.models.len() > 1000 {
        return Err(reject("model_catalog_too_large"));
    }
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for model in params.models {
        if !seen.insert(model.model_id.clone())
            || !valid_text(&model.model_id, 160)
            || !valid_text(&model.display_name, 160)
            || !matches!(
                model.source,
                ModelEvidenceSource::RuntimeCatalog | ModelEvidenceSource::ServiceDeclared
            )
        {
            return Err(reject("model_catalog_record_invalid"));
        }
        validate_catalog_metadata(
            model.context_window_tokens,
            &model.input_modalities,
            model.default_reasoning_effort.as_ref(),
            &model.supported_reasoning_efforts,
        )?;
        if model.capabilities.iter().any(|(name, e)| {
            !valid_text(name, 64) || e.source != model.source || e.evidence_id.is_some()
        }) {
            return Err(reject("model_catalog_capability_invalid"));
        }
        let id = descriptor_id(&conn.connection_id, &model.model_id);
        let old = get::<ModelDescriptor>(tx, "descriptor", &id)?;
        let mut capabilities = model.capabilities;
        if let Some(old) = &old
            && old.config_revision == conn.config_revision
            && old.auth_generation == conn.auth_generation
        {
            for (key, value) in &old.capabilities {
                if value.source == ModelEvidenceSource::Observed {
                    capabilities.insert(key.clone(), value.clone());
                }
            }
        }
        let descriptor = ModelDescriptor {
            descriptor_id: id.clone(),
            connection_id: conn.connection_id.clone(),
            model_id: model.model_id,
            display_name: model.display_name,
            revision: next(old.map_or(0, |d| d.revision))?,
            config_revision: conn.config_revision,
            auth_generation: conn.auth_generation,
            source: model.source,
            capabilities,
            context_window_tokens: model.context_window_tokens,
            input_modalities: model.input_modalities,
            default_reasoning_effort: model.default_reasoning_effort,
            supported_reasoning_efforts: model.supported_reasoning_efforts,
            runtime_version: model.runtime_version,
            service_version: model.service_version,
            updated_at: now,
        };
        put(tx, "descriptor", &id, &descriptor, now)?;
        result.push(descriptor);
    }
    encoded(&result)
}
fn selected_descriptor(
    tx: &Transaction<'_>,
    selection: &ModelSelection,
) -> Result<ModelDescriptor, PersistenceError> {
    get(
        tx,
        "descriptor",
        &descriptor_id(&selection.connection_id, &selection.model_id),
    )?
    .ok_or_else(|| reject("model_descriptor_not_found"))
}
fn purpose_save(
    tx: &Transaction<'_>,
    params: ModelPurposeSaveParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut binding = params.binding;
    if !valid_text(&binding.binding_id, 100)
        || binding
            .scope_id
            .as_ref()
            .is_some_and(|s| !valid_text(s, 160))
    {
        return Err(reject("model_purpose_binding_invalid"));
    }
    let known = get::<ModelPurposeBinding>(tx, "purpose", &binding.binding_id)?;
    if known.as_ref().map(|c| c.revision) != params.expected_revision {
        return Err(reject("model_purpose_revision_conflict"));
    }
    if list::<ModelPurposeBinding>(tx, "purpose")?.iter().any(|b| {
        b.binding_id != binding.binding_id
            && b.purpose == binding.purpose
            && b.scope_id == binding.scope_id
    }) {
        return Err(reject("model_purpose_scope_conflict"));
    }
    let consent_target = if binding.enabled {
        match binding.policy {
            ModelSelectionPolicy::Explicit => {
                let selected = binding
                    .selection
                    .as_ref()
                    .ok_or_else(|| reject("model_selection_missing"))?;
                let conn = alive_connection(tx, &selected.connection_id)?;
                selected_descriptor(tx, selected)?;
                Some(
                    json!({"connectionId":conn.connection_id,"configRevision":conn.config_revision,"authGeneration":model_active_auth_generation(&conn,model_purpose_runtime_kind(&binding.purpose))}),
                )
            }
            ModelSelectionPolicy::InheritDefault => {
                if binding.selection.is_some() || binding.scope_id.is_none() {
                    return Err(reject("model_inheritance_invalid"));
                }
                let defaults = list::<ModelPurposeBinding>(tx, "purpose")?;
                let selected = defaults
                    .iter()
                    .find(|b| {
                        b.purpose == binding.purpose
                            && b.scope_id.is_none()
                            && b.enabled
                            && b.policy == ModelSelectionPolicy::Explicit
                    })
                    .and_then(|b| b.selection.as_ref())
                    .ok_or_else(|| reject("model_default_binding_unavailable"))?;
                let conn = alive_connection(tx, &selected.connection_id)?;
                Some(
                    json!({"connectionId":conn.connection_id,"configRevision":conn.config_revision,"authGeneration":model_active_auth_generation(&conn,model_purpose_runtime_kind(&binding.purpose))}),
                )
            }
        }
    } else {
        None
    };
    if binding.enabled
        && matches!(
            binding.purpose,
            ModelPurpose::Review
                | ModelPurpose::Memory
                | ModelPurpose::Automation
                | ModelPurpose::PageAi
        )
    {
        let consent = binding
            .consent
            .as_ref()
            .ok_or_else(|| reject("model_history_consent_required"))?;
        if !valid_text(&consent.service_name, 160)
            || !valid_text(&consent.history_scope, 256)
            || consent.confirmed_at <= 0
        {
            return Err(reject("model_history_consent_required"));
        }
    }
    binding.revision = next(known.map_or(0, |b| b.revision))?;
    put_extra(
        tx,
        "purpose",
        &binding.binding_id,
        &binding,
        Some(("consentTarget", json!(consent_target))),
        now,
    )?;
    encoded(&binding)
}
fn preflight(
    tx: &Transaction<'_>,
    params: &ModelConnectionPreflightParams,
) -> Result<ModelConnectionPreflight, PersistenceError> {
    let conn = alive_connection(tx, &params.connection_id)?;
    let mut references: Vec<_> = list::<ModelPurposeBinding>(tx, "purpose")?
        .iter()
        .filter(|b| {
            b.selection
                .as_ref()
                .is_some_and(|s| s.connection_id == params.connection_id)
        })
        .map(|b| ModelConnectionReference {
            kind: "purpose_binding".into(),
            id: b.binding_id.clone(),
        })
        .collect();
    references.extend(
        list::<ModelConnectionChange>(tx, "change")?
            .iter()
            .filter(|c| {
                c.state == ModelConnectionChangeState::Prepared
                    && c.connection.connection_id == params.connection_id
            })
            .map(|c| ModelConnectionReference {
                kind: "prepared_change".into(),
                id: c.operation_id.clone(),
            }),
    );
    references.extend(
        list::<RootAuthChange>(tx, "authChange")?
            .iter()
            .filter(|c| {
                c.connection_id == params.connection_id
                    && !matches!(
                        c.state,
                        ModelAuthChangeState::Completed | ModelAuthChangeState::Cancelled
                    )
            })
            .map(|c| ModelConnectionReference {
                kind: "auth_change".into(),
                id: c.operation_id.clone(),
            }),
    );
    let mut statement=tx.prepare("SELECT conversation_id,active_run_id FROM ai_conversation_thread_bindings WHERE json_extract(runtime_scope_json,'$.connectionId')=?1 AND status<>'retired'").map_err(database_error("read model native references"))?;
    let native = statement
        .query_map([&params.connection_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(database_error("query model native references"))?;
    for row in native {
        let (id, active) = row.map_err(database_error("decode model native reference"))?;
        references.push(ModelConnectionReference {
            kind: if active.is_some() {
                "active_native_thread"
            } else {
                "native_thread"
            }
            .into(),
            id,
        });
    }
    let active_lease_ids: Vec<_> = list::<ModelExecutionLease>(tx, "lease")?
        .into_iter()
        .filter(|l| !l.released && l.snapshot.connection_id == params.connection_id)
        .map(|l| l.lease_id)
        .collect();
    let impact_digest = digest(
        &json!({"connection":conn,"action":params.action,"references":references,"activeLeaseIds":active_lease_ids}),
    );
    Ok(ModelConnectionPreflight {
        connection_id: params.connection_id.clone(),
        action: params.action.clone(),
        config_revision: conn.config_revision,
        impact_digest,
        references,
        active_lease_ids,
    })
}
fn mutate(
    tx: &Transaction<'_>,
    params: &ModelConnectionMutationParams,
    action: &ModelConnectionMutation,
    now: i64,
) -> Result<Value, PersistenceError> {
    let impact = preflight(
        tx,
        &ModelConnectionPreflightParams {
            connection_id: params.connection_id.clone(),
            action: action.clone(),
        },
    )?;
    if impact.config_revision != params.expected_config_revision
        || impact.impact_digest != params.impact_digest
    {
        return Err(reject("model_connection_impact_changed"));
    }
    if !impact.active_lease_ids.is_empty()
        || impact
            .references
            .iter()
            .any(|r| r.kind == "active_native_thread")
    {
        return Err(reject("model_connection_in_use"));
    }
    if impact
        .references
        .iter()
        .any(|r| r.kind == "prepared_change")
    {
        return Err(reject("model_connection_change_pending"));
    }
    if impact.references.iter().any(|r| r.kind == "auth_change") {
        return Err(reject("model_connection_auth_change_pending"));
    }
    let mut conn = connection(tx, &params.connection_id)?;
    if conn.config.kind == ModelConnectionKind::Chatgpt
        && conn
            .auth_scopes
            .iter()
            .any(|s| s.state != ModelAuthScopeState::SignedOut)
    {
        return Err(reject("model_auth_logout_required"));
    }
    conn.config_revision = next(conn.config_revision)?;
    conn.config.enabled = false;
    conn.updated_at = now;
    match action {
        ModelConnectionMutation::Delete => conn.deleted = true,
        ModelConnectionMutation::Disconnect => {
            conn.auth_generation = next(conn.auth_generation)?;
            conn.secret_ref = None;
        }
        ModelConnectionMutation::Disable => (),
    }
    put(tx, "connection", &conn.connection_id, &conn, now)?;
    encoded(&conn)
}
fn set_enabled(
    tx: &Transaction<'_>,
    params: ModelConnectionSetEnabledParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    require_no_auth_change(tx, &params.connection_id, None)?;
    if !params.enabled {
        return mutate(
            tx,
            &ModelConnectionMutationParams {
                connection_id: params.connection_id,
                expected_config_revision: params.expected_config_revision,
                impact_digest: params
                    .impact_digest
                    .ok_or_else(|| reject("model_connection_preflight_required"))?,
            },
            &ModelConnectionMutation::Disable,
            now,
        );
    }
    let mut conn = alive_connection(tx, &params.connection_id)?;
    if conn.config_revision != params.expected_config_revision {
        return Err(reject("model_connection_revision_conflict"));
    }
    if !conn.config.enabled {
        conn.config.enabled = true;
        conn.config_revision = next(conn.config_revision)?;
        conn.updated_at = now;
        put(tx, "connection", &conn.connection_id, &conn, now)?;
    }
    encoded(&conn)
}
fn validation_prepare(
    tx: &Transaction<'_>,
    params: ModelValidationPrepareParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    let conn = alive_connection(tx, &params.connection_id)?;
    require_no_auth_change(tx, &params.connection_id, Some(params.runtime_kind))?;
    if !conn.config.enabled {
        return Err(reject("model_connection_disabled"));
    }
    let auth_generation =
        model_validation_auth_generation(&conn, params.runtime_kind, &params.layer)
            .ok_or_else(|| reject("model_auth_required"))?;
    if conn.config_revision != params.expected_config_revision
        || auth_generation != params.expected_auth_generation
    {
        return Err(reject("model_validation_stale"));
    }
    if params
        .model_id
        .as_ref()
        .is_some_and(|id| !valid_text(id, 160))
    {
        return Err(reject("model_descriptor_invalid"));
    }
    let value = ModelValidationRecord {
        runtime_kind: params.runtime_kind,
        validation_id: uuid::Uuid::new_v4().simple().to_string(),
        connection_id: params.connection_id,
        model_id: params.model_id,
        layer: params.layer,
        config_revision: conn.config_revision,
        auth_generation,
        outcome: None,
        completion_revision: None,
        reason: None,
        runtime_version: None,
        service_version: None,
        updated_at: now,
    };
    put(tx, "validation", &value.validation_id, &value, now)?;
    encoded(&value)
}
fn validation_report(
    tx: &Transaction<'_>,
    params: ModelValidationReportParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    if params.reason.as_ref().is_some_and(|v| {
        v.is_empty()
            || v.len() > 128
            || !v
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.:-".contains(&b))
    }) {
        return Err(reject("model_validation_reason_invalid"));
    }
    if [&params.runtime_version, &params.service_version]
        .iter()
        .any(|v| v.as_ref().is_some_and(|v| !valid_text(v, 160)))
    {
        return Err(reject("model_validation_version_invalid"));
    }
    let mut record = get::<ModelValidationRecord>(tx, "validation", &params.validation_id)?
        .ok_or_else(|| reject("model_validation_not_found"))?;
    if list::<ModelExecutionLease>(tx, "lease")?
        .iter()
        .any(|l| !l.released && l.snapshot.validation_id.as_deref() == Some(&record.validation_id))
    {
        return Err(reject("model_validation_execution_in_use"));
    }
    let conn = alive_connection(tx, &record.connection_id)?;
    if conn.config_revision != record.config_revision
        || model_validation_auth_generation(&conn, record.runtime_kind, &record.layer)
            != Some(record.auth_generation)
    {
        return Err(reject("model_validation_stale"));
    }
    if let Some(outcome) = &record.outcome {
        if outcome == &params.outcome
            && record.reason == params.reason
            && record.runtime_version == params.runtime_version
            && record.service_version == params.service_version
        {
            return encoded(&record);
        }
        return Err(reject("model_validation_already_reported"));
    }
    record.outcome = Some(params.outcome.clone());
    record.completion_revision = Some(next(
        list::<ModelValidationRecord>(tx, "validation")?
            .iter()
            .filter_map(|r| r.completion_revision)
            .max()
            .unwrap_or(0),
    )?);
    record.reason = params.reason;
    record.runtime_version = params.runtime_version;
    record.service_version = params.service_version;
    record.updated_at = now;
    put(tx, "validation", &record.validation_id, &record, now)?;
    encoded(&record)
}
fn resolve_binding(
    tx: &Transaction<'_>,
    params: &ModelExecutionPrepareParams,
) -> Result<(ModelSelection, Option<ModelPurposeBinding>), PersistenceError> {
    if let Some(selection) = &params.selection {
        if params.purpose != ModelPurpose::Foreground {
            return Err(reject("model_purpose_binding_required"));
        }
        return Ok((selection.clone(), None));
    }
    let bindings = list::<ModelPurposeBinding>(tx, "purpose")?;
    let binding = if let Some(id) = &params.binding_id {
        bindings.iter().find(|b| &b.binding_id == id)
    } else {
        bindings
            .iter()
            .find(|b| b.purpose == params.purpose && b.scope_id.is_none())
    }
    .ok_or_else(|| reject("model_purpose_binding_required"))?;
    if binding.purpose != params.purpose || !binding.enabled {
        return Err(reject("model_purpose_disabled"));
    }
    let target = if binding.policy == ModelSelectionPolicy::InheritDefault {
        bindings
            .iter()
            .find(|b| {
                b.purpose == params.purpose
                    && b.scope_id.is_none()
                    && b.policy == ModelSelectionPolicy::Explicit
                    && b.enabled
            })
            .ok_or_else(|| reject("model_default_binding_unavailable"))?
    } else {
        binding
    };
    let selection = target
        .selection
        .clone()
        .ok_or_else(|| reject("model_selection_missing"))?;
    Ok((selection, Some(binding.clone())))
}
fn execution_snapshot(
    tx: &Transaction<'_>,
    params: ModelExecutionPrepareParams,
    profile_id: &str,
    instance_fence: &str,
    validation: Option<&ModelValidationRecord>,
) -> Result<ModelExecutionSnapshot, PersistenceError> {
    let (selection, binding) = resolve_binding(tx, &params)?;
    let conn = alive_connection(tx, &selection.connection_id)?;
    let runtime_kind = validation.map_or_else(
        || model_purpose_runtime_kind(&params.purpose),
        |r| r.runtime_kind,
    );
    require_no_auth_change(tx, &selection.connection_id, Some(runtime_kind))?;
    let auth_generation = model_active_auth_generation(&conn, runtime_kind)
        .ok_or_else(|| reject("model_auth_required"))?;
    if !conn.config.enabled {
        return Err(reject("model_connection_disabled"));
    }
    if conn.config.auth_mode == ModelAuthMode::ApiKey && conn.secret_ref.is_none() {
        return Err(reject("model_auth_required"));
    }
    credentials_cleanup::require_available(tx, conn.secret_ref.as_ref())?;
    let descriptor = selected_descriptor(tx, &selection)?;
    if descriptor.config_revision != conn.config_revision {
        return Err(reject("model_descriptor_stale"));
    }
    if let Some(binding) = &binding
        && params.purpose != ModelPurpose::Foreground
    {
        let original = entry(tx, "purpose", &binding.binding_id)?
            .ok_or_else(|| reject("model_purpose_binding_required"))?;
        if original["consentTarget"]
            != json!({"connectionId":conn.connection_id,"configRevision":conn.config_revision,"authGeneration":auth_generation})
        {
            return Err(reject("model_history_consent_stale"));
        }
    }
    let mut capabilities = descriptor.capabilities.clone();
    capabilities.retain(|_, e| e.source != ModelEvidenceSource::Observed);
    let mut observed = list::<ModelValidationRecord>(tx, "validation")?
        .into_iter()
        .filter(|r| {
            r.connection_id == conn.connection_id
                && r.runtime_kind == runtime_kind
                && r.auth_generation == auth_generation
                && r.config_revision == conn.config_revision
                && r.model_id.as_deref() == Some(&selection.model_id)
                && r.outcome.is_some()
                && r.completion_revision.is_some()
        })
        .collect::<Vec<_>>();
    observed.sort_by_key(|r| r.completion_revision);
    for record in observed {
        let name = match record.layer {
            ModelValidationLayer::Inference => "inference",
            ModelValidationLayer::Streaming => "streaming",
            ModelValidationLayer::Tools => "tools",
            _ => continue,
        };
        capabilities.insert(
            name.into(),
            ModelCapabilityEvidence {
                state: if record.outcome == Some(ModelValidationOutcome::Passed) {
                    ModelCapabilityState::Supported
                } else {
                    ModelCapabilityState::Unsupported
                },
                source: ModelEvidenceSource::Observed,
                evidence_id: Some(record.validation_id),
            },
        );
    }
    Ok(ModelExecutionSnapshot {
        validation_id: validation.map(|r| r.validation_id.clone()),
        runtime_kind,
        profile_id: profile_id.into(),
        instance_fence: instance_fence.into(),
        connection_id: conn.connection_id.clone(),
        kind: conn.config.kind,
        config_revision: conn.config_revision,
        auth_generation,
        purpose: params.purpose,
        model_id: selection.model_id,
        reasoning_effort: selection.reasoning_effort,
        base_url: conn.config.base_url,
        local_runtime: conn.config.local_runtime,
        auth_mode: conn.config.auth_mode,
        network: conn.config.network,
        secret_ref: conn.secret_ref,
        runtime_home_scope: model_runtime_home_scope(
            profile_id,
            &conn.connection_id,
            auth_generation,
        ),
        descriptor_revision: descriptor.revision,
        capabilities,
        context_window_tokens: descriptor.context_window_tokens,
        input_modalities: descriptor.input_modalities,
        default_reasoning_effort: descriptor.default_reasoning_effort,
        supported_reasoning_efforts: descriptor.supported_reasoning_efforts,
        binding_revision: binding.as_ref().map(|b| b.revision),
        binding_id: binding.map(|b| b.binding_id),
        conversation_id: params.conversation_id,
    })
}
pub(super) fn execution_prepare(
    tx: &Transaction<'_>,
    params: ModelExecutionPrepareParams,
    profile_id: &str,
    instance_fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    if params.launch_protocol != MODEL_LAUNCH_PROTOCOL {
        return Err(reject("model_execution_launch_protocol_unsupported"));
    }
    let owner = params
        .owner
        .clone()
        .ok_or_else(|| reject("model_execution_owner_required"))?;
    if !valid_text(&owner.instance_id, 200) || !process_alive(owner.pid, &owner.start_fingerprint)?
    {
        return Err(reject("model_execution_owner_unconfirmed"));
    }
    let snapshot = execution_snapshot(tx, params, profile_id, instance_fence, None)?;
    let snapshot_digest = digest(&encoded(&snapshot)?);
    let lease = ModelExecutionLease {
        lease_id: uuid::Uuid::new_v4().simple().to_string(),
        snapshot,
        snapshot_digest,
        released: false,
        owner,
        processes: vec![],
        execution_started: false,
        launch_protocol: MODEL_LAUNCH_PROTOCOL,
        created_at: now,
    };
    put(tx, "lease", &lease.lease_id, &lease, now)?;
    encoded(&lease)
}

fn validation_execution(
    tx: &Transaction<'_>,
    params: ModelValidationExecutionParams,
    profile_id: &str,
    fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let record = get::<ModelValidationRecord>(tx, "validation", &params.validation_id)?
        .ok_or_else(|| reject("model_validation_not_found"))?;
    if record.outcome.is_some()
        || !matches!(
            record.layer,
            ModelValidationLayer::Authentication
                | ModelValidationLayer::Protocol
                | ModelValidationLayer::Inference
                | ModelValidationLayer::Streaming
                | ModelValidationLayer::Tools
        )
    {
        return Err(reject("model_validation_execution_invalid"));
    }
    if params.launch_protocol != MODEL_LAUNCH_PROTOCOL
        || !valid_text(&params.owner.instance_id, 200)
        || !process_alive(params.owner.pid, &params.owner.start_fingerprint)?
    {
        return Err(reject("model_execution_owner_unconfirmed"));
    }
    if list::<ModelExecutionLease>(tx, "lease")?
        .iter()
        .any(|l| l.snapshot.validation_id.as_deref() == Some(&record.validation_id))
    {
        return Err(reject("model_validation_execution_already_prepared"));
    }
    let selection = ModelSelection {
        connection_id: record.connection_id.clone(),
        model_id: record
            .model_id
            .clone()
            .ok_or_else(|| reject("model_validation_model_required"))?,
        reasoning_effort: None,
    };
    let snapshot = execution_snapshot(
        tx,
        ModelExecutionPrepareParams {
            owner: None,
            launch_protocol: 0,
            purpose: ModelPurpose::Foreground,
            binding_id: None,
            selection: Some(selection),
            conversation_id: None,
        },
        profile_id,
        fence,
        Some(&record),
    )?;
    if snapshot.config_revision != record.config_revision
        || snapshot.auth_generation != record.auth_generation
    {
        return Err(reject("model_validation_stale"));
    }
    let lease = ModelExecutionLease {
        lease_id: uuid::Uuid::new_v4().simple().to_string(),
        snapshot_digest: digest(&encoded(&snapshot)?),
        snapshot,
        released: false,
        owner: params.owner,
        processes: Vec::new(),
        execution_started: false,
        launch_protocol: params.launch_protocol,
        created_at: now,
    };
    put(tx, "lease", &lease.lease_id, &lease, now)?;
    encoded(&lease)
}
fn execution_lease(
    tx: &Transaction<'_>,
    params: &ModelExecutionLeaseParams,
    instance_fence: &str,
    release: bool,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut lease = get::<ModelExecutionLease>(tx, "lease", &params.lease_id)?
        .ok_or_else(|| reject("model_execution_lease_not_found"))?;
    if lease.snapshot_digest != params.snapshot_digest {
        return Err(reject("model_execution_snapshot_mismatch"));
    }
    if !release {
        if lease.released {
            return Err(reject("model_execution_lease_released"));
        }
        if lease.snapshot.instance_fence != instance_fence {
            return Err(reject("model_execution_instance_changed"));
        }
        let conn = alive_connection(tx, &lease.snapshot.connection_id)?;
        if !conn.config.enabled {
            return Err(reject("model_connection_disabled"));
        }
        if conn.config_revision != lease.snapshot.config_revision
            || model_active_auth_generation(&conn, lease.snapshot.runtime_kind)
                != Some(lease.snapshot.auth_generation)
        {
            return Err(reject("model_execution_snapshot_stale"));
        }
        require_no_auth_change(tx, &conn.connection_id, Some(lease.snapshot.runtime_kind))?;
        credentials_cleanup::require_available(tx, lease.snapshot.secret_ref.as_ref())?;
    } else if !lease.released {
        if lease.processes.is_empty() {
            return Err(reject("model_execution_ownership_unconfirmed"));
        }
        for process in &lease.processes {
            if process_alive(process.pid, &process.start_fingerprint)? {
                return Err(reject("model_execution_process_alive"));
            }
        }
        lease.released = true;
        retire_binding_owner(tx, &lease, now)?;
        put(tx, "lease", &lease.lease_id, &lease, now)?;
    }
    encoded(&lease)
}

fn retire_binding_owner(
    tx: &Transaction<'_>,
    lease: &ModelExecutionLease,
    now: i64,
) -> Result<(), PersistenceError> {
    tx.execute("UPDATE ai_conversation_thread_bindings SET owner_instance_id=NULL,active_run_id=NULL,writer_parked=0,status='blocked',binding_revision=binding_revision+1,updated_at=?2 WHERE model_execution_lease_id=?1 AND owner_instance_id=?3 AND active_run_id IS NOT NULL",params![lease.lease_id,now,lease.owner.instance_id]).map_err(database_error("retire reaped model thread owner"))?;
    Ok(())
}

pub(super) fn recover_automation_lease(
    tx: &Transaction<'_>,
    bound: &ModelExecutionLease,
    now: i64,
) -> Result<(), PersistenceError> {
    let mut current = get::<ModelExecutionLease>(tx, "lease", &bound.lease_id)?
        .ok_or_else(|| reject("model_execution_lease_not_found"))?;
    if !matches!(
        current.snapshot.purpose,
        ModelPurpose::Automation | ModelPurpose::Review | ModelPurpose::Memory
    ) || current.snapshot != bound.snapshot
        || current.snapshot_digest != bound.snapshot_digest
        || current.owner != bound.owner
        || current.launch_protocol != bound.launch_protocol
    {
        return Err(reject("model_automation_lease_scope_invalid"));
    }
    if current.released {
        return Ok(());
    }
    if process_alive(current.owner.pid, &current.owner.start_fingerprint)? {
        return Err(reject("model_execution_owner_alive"));
    }
    if current.processes.is_empty() {
        return Err(reject("model_execution_ownership_unconfirmed"));
    }
    for process in &current.processes {
        if process_alive(process.pid, &process.start_fingerprint)? {
            return Err(reject("model_execution_process_alive"));
        }
    }
    current.released = true;
    retire_binding_owner(tx, &current, now)?;
    put(tx, "lease", &current.lease_id, &current, now)
}

fn process_alive(pid: u32, fingerprint: &str) -> Result<bool, PersistenceError> {
    if pid == 0 || !valid_text(fingerprint, 512) {
        return Err(reject("model_execution_ownership_unconfirmed"));
    }
    contexthub_platform_adapter::instance::process_start_fingerprint(pid)
        .map(|current| current.as_deref() == Some(fingerprint))
        .map_err(|_| reject("model_execution_ownership_unconfirmed"))
}

fn require_no_auth_change(
    tx: &Transaction<'_>,
    id: &str,
    kind: Option<ModelRuntimeKind>,
) -> Result<(), PersistenceError> {
    if auth::pending(tx, id, kind)?
        || list::<ModelConnectionChange>(tx, "change")?
            .iter()
            .any(|c| {
                c.connection.connection_id == id
                    && c.state == ModelConnectionChangeState::Prepared
                    && c.credential_action == ModelCredentialAction::Clear
            })
    {
        return Err(reject("model_connection_auth_change_pending"));
    }
    Ok(())
}

fn register_process(
    tx: &Transaction<'_>,
    params: ModelExecutionRegisterProcessParams,
    instance_fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let current = execution_lease(
        tx,
        &ModelExecutionLeaseParams {
            lease_id: params.lease_id,
            snapshot_digest: params.snapshot_digest,
        },
        instance_fence,
        false,
        now,
    )?;
    let mut lease: ModelExecutionLease = decoded(current)?;
    if !process_alive(lease.owner.pid, &lease.owner.start_fingerprint)? {
        return Err(reject("model_execution_owner_unconfirmed"));
    }
    if !process_alive(params.process.pid, &params.process.start_fingerprint)? {
        return Err(reject("model_execution_process_unconfirmed"));
    }
    if !lease.processes.contains(&params.process) {
        lease.processes.push(params.process);
    }
    lease.execution_started = true;
    put(tx, "lease", &lease.lease_id, &lease, now)?;
    encoded(&lease)
}

fn abort_unlaunched(
    tx: &Transaction<'_>,
    params: &ModelExecutionAbortUnlaunchedParams,
    fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut lease = get::<ModelExecutionLease>(tx, "lease", &params.lease_id)?
        .ok_or_else(|| reject("model_execution_lease_not_found"))?;
    if lease.snapshot_digest != params.snapshot_digest {
        return Err(reject("model_execution_snapshot_mismatch"));
    }
    if lease.snapshot.instance_fence != fence
        || lease.owner != params.owner
        || !process_alive(lease.owner.pid, &lease.owner.start_fingerprint)?
    {
        return Err(reject("model_execution_owner_unconfirmed"));
    }
    if lease.launch_protocol != MODEL_LAUNCH_PROTOCOL
        || lease.execution_started
        || !lease.processes.is_empty()
    {
        return Err(reject("model_execution_already_launched"));
    }
    lease.released = true;
    put(tx, "lease", &lease.lease_id, &lease, now)?;
    encoded(&lease)
}

pub(crate) fn validate_for_request(
    tx: &Transaction<'_>,
    params: &ModelExecutionLeaseParams,
    instance_fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let value = execution_lease(tx, params, instance_fence, false, now)?;
    let lease: ModelExecutionLease = decoded(value.clone())?;
    if let Some(id) = &lease.snapshot.validation_id {
        let record = get::<ModelValidationRecord>(tx, "validation", id)?
            .ok_or_else(|| reject("model_validation_not_found"))?;
        if record.outcome.is_some()
            || record.config_revision != lease.snapshot.config_revision
            || record.auth_generation != lease.snapshot.auth_generation
            || record.runtime_kind != lease.snapshot.runtime_kind
            || record.model_id.as_deref() != Some(&lease.snapshot.model_id)
        {
            return Err(reject("model_validation_stale"));
        }
    }
    if let Some(binding_id) = &lease.snapshot.binding_id {
        let params = ModelExecutionPrepareParams {
            owner: None,
            launch_protocol: 0,
            purpose: lease.snapshot.purpose.clone(),
            binding_id: Some(binding_id.clone()),
            selection: None,
            conversation_id: lease.snapshot.conversation_id.clone(),
        };
        let (selection, binding) = resolve_binding(tx, &params)?;
        if binding.as_ref().map(|b| b.revision) != lease.snapshot.binding_revision
            || selection.connection_id != lease.snapshot.connection_id
            || selection.model_id != lease.snapshot.model_id
            || selection.reasoning_effort != lease.snapshot.reasoning_effort
        {
            return Err(reject("model_purpose_binding_changed"));
        }
        if lease.snapshot.purpose != ModelPurpose::Foreground {
            let current = entry(tx, "purpose", binding_id)?
                .ok_or_else(|| reject("model_purpose_binding_required"))?;
            if current["consentTarget"]
                != json!({"connectionId":lease.snapshot.connection_id,"configRevision":lease.snapshot.config_revision,"authGeneration":lease.snapshot.auth_generation})
            {
                return Err(reject("model_history_consent_stale"));
            }
        }
    }
    if !lease.execution_started || lease.processes.is_empty() {
        return Err(reject("model_execution_process_not_registered"));
    }
    let mut alive = false;
    for process in &lease.processes {
        alive |= process_alive(process.pid, &process.start_fingerprint)?;
    }
    if !alive {
        return Err(reject("model_execution_process_missing"));
    }
    Ok(value)
}

fn recover_leases(
    tx: &Transaction<'_>,
    params: ModelExecutionRecoverParams,
    now: i64,
) -> Result<Value, PersistenceError> {
    connection(tx, &params.connection_id)?;
    let mut diagnostics = Vec::new();
    for mut lease in list::<ModelExecutionLease>(tx, "lease")?
        .into_iter()
        .filter(|l| !l.released && l.snapshot.connection_id == params.connection_id)
    {
        let mut state = match process_alive(lease.owner.pid, &lease.owner.start_fingerprint) {
            Ok(true) => ModelExecutionRecoveryState::OwnerAlive,
            Ok(false) => ModelExecutionRecoveryState::Released,
            Err(_) => ModelExecutionRecoveryState::Unconfirmed,
        };
        if state == ModelExecutionRecoveryState::Released && lease.processes.is_empty() {
            state = ModelExecutionRecoveryState::Unconfirmed;
        }
        if state == ModelExecutionRecoveryState::Released {
            for process in &lease.processes {
                match process_alive(process.pid, &process.start_fingerprint) {
                    Ok(true) => {
                        state = ModelExecutionRecoveryState::ProcessAlive;
                        break;
                    }
                    Err(_) => {
                        state = ModelExecutionRecoveryState::Unconfirmed;
                        break;
                    }
                    Ok(false) => (),
                }
            }
        }
        let reason = match state {
            ModelExecutionRecoveryState::Released => "model_execution_processes_reaped",
            ModelExecutionRecoveryState::OwnerAlive => "model_execution_owner_alive",
            ModelExecutionRecoveryState::ProcessAlive => "model_execution_process_alive",
            ModelExecutionRecoveryState::Unconfirmed => "model_execution_ownership_unconfirmed",
        };
        if state == ModelExecutionRecoveryState::Released {
            lease.released = true;
            put(tx, "lease", &lease.lease_id, &lease, now)?;
            retire_binding_owner(tx, &lease, now)?;
        }
        diagnostics.push(ModelExecutionLeaseDiagnostic {
            lease_id: lease.lease_id,
            owner: lease.owner,
            processes: lease.processes,
            state,
            reason: reason.into(),
        });
    }
    encoded(&ModelExecutionRecoverResult {
        connection_id: params.connection_id,
        leases: diagnostics,
    })
}

pub(super) fn validate_native_scope(
    tx: &Transaction<'_>,
    scope: &ModelThreadRuntimeScope,
    lease_id: &str,
    conversation_id: &str,
    owner_instance_id: &str,
    writer: &Value,
) -> Result<(), PersistenceError> {
    let lease = get::<ModelExecutionLease>(tx, "lease", lease_id)?
        .ok_or_else(|| reject("model_execution_lease_not_found"))?;
    credentials_cleanup::require_available(tx, lease.snapshot.secret_ref.as_ref())?;
    let fence:String=tx.query_row("SELECT instance_fence FROM personal_profile_compatibility WHERE singleton=1 AND state='ready'",[],|row|row.get(0)).map_err(database_error("read native model fence"))?;
    if lease.released
        || lease.snapshot.validation_id.is_some()
        || lease.snapshot.instance_fence != fence
        || !lease.execution_started
        || scope.profile_id != lease.snapshot.profile_id
        || scope.connection_id != lease.snapshot.connection_id
        || scope.config_revision != lease.snapshot.config_revision
        || scope.auth_generation != lease.snapshot.auth_generation
        || scope.runtime_home_scope != lease.snapshot.runtime_home_scope
        || scope.runtime_kind != lease.snapshot.runtime_kind.as_str()
    {
        return Err(reject("model_thread_scope_invalid"));
    }
    if lease.snapshot.conversation_id.as_deref() != Some(conversation_id)
        || lease.owner.instance_id != owner_instance_id
    {
        return Err(reject("model_thread_owner_scope_mismatch"));
    }
    let writer: ModelExecutionProcess = serde_json::from_value(writer.clone())
        .map_err(|_| reject("model_thread_writer_process_required"))?;
    if !lease.processes.contains(&writer) || !process_alive(writer.pid, &writer.start_fingerprint)?
    {
        return Err(reject("model_thread_writer_process_mismatch"));
    }
    Ok(())
}

impl SqlitePersistence {
    /// 仅供持有资料实例锁的启动路径登记升级，连接条目不会被此操作重写。
    /// # Errors
    /// 资料标识冲突或持久化失败时拒绝。
    pub fn begin_personal_profile_upgrade(
        &mut self,
        profile_id: &str,
        instance_fence: &str,
    ) -> Result<(), PersistenceError> {
        if !valid_id(profile_id) || !valid_text(instance_fence, 200) {
            return Err(reject("model_profile_identity_invalid"));
        }
        let known: Option<String> = self
            .connection
            .query_row(
                "SELECT profile_id FROM personal_profile_compatibility WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(database_error("read personal profile fence"))?;
        if known.as_deref().is_some_and(|id| id != profile_id) {
            return Err(reject("model_profile_identity_mismatch"));
        }
        self.connection.execute("UPDATE personal_profile_compatibility SET profile_id=?1,instance_fence=?2,state='pending' WHERE singleton=1",params![profile_id,instance_fence]).map_err(database_error("prepare personal profile version"))?;
        Ok(())
    }
    /// 发布同一实例持锁完成的资料版本。
    /// # Errors
    /// 资料、实例或升级状态不匹配以及持久化失败时拒绝。
    pub fn finish_personal_profile_upgrade(
        &mut self,
        profile_id: &str,
        instance_fence: &str,
        manifest_digest: &str,
    ) -> Result<(), PersistenceError> {
        let changed=self.connection.execute("UPDATE personal_profile_compatibility SET manifest_digest=?3,state='ready' WHERE singleton=1 AND profile_id=?1 AND instance_fence=?2 AND state='pending'",params![profile_id,instance_fence,manifest_digest]).map_err(database_error("finish personal profile version"))?;
        if changed != 1 {
            return Err(reject("model_profile_upgrade_ownership_changed"));
        }
        Ok(())
    }
    /// 全部个人模型写入限定在同一账本命名空间，调用者不提交秘密正文。
    /// 在当前资料和服务实例范围内执行个人模型账本请求。
    /// # Errors
    /// 资料未就绪、版本不兼容、参数或业务前置条件不满足时拒绝。
    pub fn personal_model_rpc(
        &mut self,
        method: &str,
        params: Value,
        profile_id: &str,
        instance_fence: &str,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("begin model transaction"))?;
        let ready:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_profile_compatibility WHERE singleton=1 AND data_version=2 AND profile_id=?1 AND instance_fence=?2 AND state='ready')",params![profile_id,instance_fence],|r|r.get(0)).map_err(database_error("read model profile readiness"))?;
        if !ready {
            return Err(reject("model_profile_not_ready"));
        }
        if matches!(
            method,
            "modelConnection/savePrepare"
                | "modelConnection/setEnabled"
                | "modelConnection/deleteCommit"
                | "modelDescriptor/save"
                | "modelDescriptor/observe"
                | "modelPurpose/save"
                | "modelValidation/prepare"
                | "modelValidation/prepareExecution"
                | "modelExecution/prepare"
                | "modelAuth/startPrepare"
                | "modelAuth/logoutPrepare"
                | "modelAuth/readPrepare"
                | "modelAuth/catalogPrepare"
        ) {
            crate::profile_writers::require_writer_admission(&tx, profile_id, instance_fence)?;
        }
        let result = route(&tx, method, params, profile_id, instance_fence, now)?;
        tx.commit()
            .map_err(database_error("commit model transaction"))?;
        Ok(result)
    }
}

fn execution_list(
    tx: &Transaction<'_>,
    params: Value,
    profile: &str,
) -> Result<Value, PersistenceError> {
    let _: contexthub_protocol::ModelExecutionListParams = decoded(params)?;
    let leases = list::<ModelExecutionLease>(tx, "lease")?;
    if leases
        .iter()
        .any(|lease| lease.snapshot.profile_id != profile)
    {
        return Err(reject("model_profile_identity_mismatch"));
    }
    encoded(&leases)
}

fn route(
    tx: &Transaction<'_>,
    method: &str,
    params: Value,
    profile_id: &str,
    instance_fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    match method {
        "modelExecution/list" => execution_list(tx, params, profile_id),
        method if method.starts_with("modelCredentialCleanup/") => {
            credentials_cleanup::route(tx, method, params, profile_id, now)
        }
        method if method.starts_with("modelAuth/") => {
            auth::route(tx, method, params, instance_fence, now)
        }
        "modelConnection/list" => encoded(
            &list::<ModelConnection>(tx, "connection")?
                .into_iter()
                .filter(|c| !c.deleted)
                .collect::<Vec<_>>(),
        ),
        "modelConnection/get" => encoded(&connection(
            tx,
            &decoded::<ModelConnectionParams>(params)?.connection_id,
        )?),
        "modelConnection/changeList" => encoded(&list::<ModelConnectionChange>(tx, "change")?),
        "modelConnection/changeStatus" => change_status(tx, params),
        "modelConnection/savePrepare" => save_prepare(tx, decoded(params)?, profile_id, now),
        "modelConnection/saveCommit" => save_commit(tx, &decoded(params)?, now),
        "modelConnection/saveAbort" => change_abort(tx, &decoded(params)?, now),
        "modelConnection/credentialCleanupAck" => {
            credential_cleanup_ack(tx, &decoded(params)?, profile_id, now)
        }
        "modelConnection/preflight" | "modelConnection/deletePrepare" => {
            encoded(&preflight(tx, &decoded(params)?)?)
        }
        "modelConnection/deleteCommit" => {
            mutate(tx, &decoded(params)?, &ModelConnectionMutation::Delete, now)
        }
        "modelConnection/disconnectCommit" => mutate(
            tx,
            &decoded(params)?,
            &ModelConnectionMutation::Disconnect,
            now,
        ),
        "modelConnection/setEnabled" => set_enabled(tx, decoded(params)?, now),
        "modelCatalog/read" => {
            let id = decoded::<ModelConnectionParams>(params)?.connection_id;
            alive_connection(tx, &id)?;
            encoded(
                &list::<ModelDescriptor>(tx, "descriptor")?
                    .into_iter()
                    .filter(|d| d.connection_id == id)
                    .collect::<Vec<_>>(),
            )
        }
        "modelDescriptor/save" => descriptor_save(tx, decoded(params)?, now),
        "modelDescriptor/observe" => descriptor_observe(tx, decoded(params)?, now),
        "modelPurpose/list" => encoded(&list::<ModelPurposeBinding>(tx, "purpose")?),
        "modelPurpose/save" => purpose_save(tx, decoded(params)?, now),
        "modelValidation/list" => {
            let id = decoded::<ModelConnectionParams>(params)?.connection_id;
            alive_connection(tx, &id)?;
            encoded(
                &list::<ModelValidationRecord>(tx, "validation")?
                    .into_iter()
                    .filter(|d| d.connection_id == id)
                    .collect::<Vec<_>>(),
            )
        }
        "modelValidation/prepare" => validation_prepare(tx, decoded(params)?, now),
        "modelValidation/report" => validation_report(tx, decoded(params)?, now),
        "modelValidation/prepareExecution" => {
            validation_execution(tx, decoded(params)?, profile_id, instance_fence, now)
        }
        "modelExecution/prepare" => {
            execution_prepare(tx, decoded(params)?, profile_id, instance_fence, now)
        }
        "modelExecution/resolve" => encoded(&execution_snapshot(
            tx,
            decoded(params)?,
            profile_id,
            instance_fence,
            None,
        )?),
        "modelExecution/validate" => {
            execution_lease(tx, &decoded(params)?, instance_fence, false, now)
        }
        "modelExecution/validateForRequest" => {
            validate_for_request(tx, &decoded(params)?, instance_fence, now)
        }
        "modelExecution/registerProcess" => {
            register_process(tx, decoded(params)?, instance_fence, now)
        }
        "modelExecution/abortUnlaunched" => {
            abort_unlaunched(tx, &decoded(params)?, instance_fence, now)
        }
        "modelExecution/recover" => recover_leases(tx, decoded(params)?, now),
        "modelExecution/release" => {
            execution_lease(tx, &decoded(params)?, instance_fence, true, now)
        }
        _ => Err(reject("model_method_not_found")),
    }
}
