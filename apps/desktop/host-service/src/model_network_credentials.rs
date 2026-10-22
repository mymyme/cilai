//! 网络凭据只由私有 Host 请求写入本地凭据库，DAL 仅接收引用。
use super::{
    CredentialStore, ModelConnectionChange, ModelConnectionChangeState, ModelCredentialAction,
    Value, cleanup_historical_credential, decode,
};
use contexthub_protocol::{
    MODEL_NETWORK_CREDENTIAL_SERVICE, ModelCredentialCleanupCandidate, ModelNetworkCredentials,
    validate_model_network_credentials,
};

pub(super) fn write(
    profile: &str,
    change: &ModelConnectionChange,
    credentials: Option<&ModelNetworkCredentials>,
    store: &impl CredentialStore,
) -> Result<(), String> {
    if change.connection.profile_id != profile {
        return Err("model_network_credential_scope_invalid".into());
    }
    if change.state != ModelConnectionChangeState::Prepared {
        return Ok(());
    }
    if change.network_credential_action != ModelCredentialAction::Replace {
        if credentials.is_some() {
            return Err("model_network_credential_action_invalid".into());
        }
        return Ok(());
    }
    let network = &change.connection.config.network;
    let reference = network
        .proxy_secret_ref
        .as_ref()
        .ok_or("model_network_credential_required")?;
    if reference.service != MODEL_NETWORK_CREDENTIAL_SERVICE
        || reference.account
            != format!(
                "{profile}:{}:{}",
                change.connection.connection_id, change.connection.config_revision
            )
    {
        return Err("model_network_credential_scope_invalid".into());
    }
    let credentials = credentials.ok_or("model_network_credential_required")?;
    validate_model_network_credentials(network, credentials)?;
    let mut bytes =
        serde_json::to_vec(credentials).map_err(|_| "model_network_credential_invalid")?;
    let result = save_exact(reference, &bytes, store);
    bytes.fill(0);
    result
}

fn save_exact(
    reference: &contexthub_protocol::ModelSecretRef,
    bytes: &[u8],
    store: &impl CredentialStore,
) -> Result<(), String> {
    if let Some(mut existing) = store.read(reference)? {
        let same = existing == bytes;
        existing.fill(0);
        if !same {
            return Err("model_network_credential_pending_value_conflict".into());
        }
    } else {
        store.write(reference, bytes)?;
    }
    let mut saved = store
        .read(reference)?
        .ok_or("model_network_credential_readback_failed")?;
    let same = saved == bytes;
    saved.fill(0);
    if !same {
        return Err("model_network_credential_readback_failed".into());
    }
    Ok(())
}

pub(super) fn cleanup_aborted(
    profile: &str,
    change: &ModelConnectionChange,
    store: &impl CredentialStore,
    rpc: &impl Fn(&str, &Value) -> Result<Value, String>,
) -> Result<(), String> {
    if change.state != ModelConnectionChangeState::Aborted
        || change.network_credential_action != ModelCredentialAction::Replace
    {
        return Ok(());
    }
    let reference = change
        .connection
        .config
        .network
        .proxy_secret_ref
        .as_ref()
        .ok_or("model_network_credential_scope_invalid")?;
    let candidates: Vec<ModelCredentialCleanupCandidate> =
        decode(rpc("modelCredentialCleanup/list", &serde_json::json!({}))?)?;
    let candidate = candidates
        .iter()
        .find(|c| &c.credential_ref == reference)
        .ok_or("model_credential_cleanup_not_found")?;
    if candidate.state == contexthub_protocol::ModelCredentialCleanupState::Confirmed {
        return Ok(());
    }
    cleanup_historical_credential(
        profile,
        &serde_json::json!({"cleanupId":candidate.cleanup_id,"expectedReferenceDigest":candidate.reference_digest}),
        store,
        rpc,
    )?;
    Ok(())
}
