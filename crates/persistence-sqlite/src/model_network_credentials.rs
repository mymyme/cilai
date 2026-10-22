//! 独立网络凭据代次在连接事务中冻结；本地凭据正文由 Host 管理。
use super::{
    ModelConnection, ModelConnectionConfig, ModelConnectionSavePrepareParams,
    ModelCredentialAction, ModelSecretRef, PersistenceError, Transaction, credentials_cleanup,
    reject, validate_url,
};
use contexthub_protocol::{MODEL_NETWORK_CREDENTIAL_SERVICE, model_network_requires_credentials};

pub(super) fn validate_proxy_url(url: &str) -> Result<(), PersistenceError> {
    let normalized = url
        .strip_prefix("http://")
        .map_or_else(|| url.to_owned(), |rest| format!("https://{rest}"));
    validate_url(&normalized, false)?;
    let authority = normalized.split_once("://").map_or("", |(_, rest)| rest);
    if authority.trim_end_matches('/').contains('/') {
        return Err(reject("model_proxy_config_invalid"));
    }
    Ok(())
}

pub(super) fn prepare(
    tx: &Transaction<'_>,
    profile_id: &str,
    id: &str,
    revision: u32,
    old: Option<&ModelConnection>,
    params: &ModelConnectionSavePrepareParams,
) -> Result<ModelConnectionConfig, PersistenceError> {
    let mut config = params.config.clone();
    let old_network = old.map(|c| &c.config.network);
    let previous = old_network.and_then(|n| n.proxy_secret_ref.as_ref());
    if let Some(reference) = previous {
        contexthub_protocol::model_network_reference_generation(
            profile_id,
            id,
            old.map_or(0, |connection| connection.config_revision),
            reference,
        )
        .map_err(reject)?;
    }
    if config
        .network
        .proxy_secret_ref
        .as_ref()
        .is_some_and(|reference| Some(reference) != previous)
    {
        return Err(reject("model_network_credential_scope_invalid"));
    }
    let required = model_network_requires_credentials(&config.network);
    config.network.proxy_secret_ref = match params.network_credential_action {
        ModelCredentialAction::Keep => {
            if previous.is_some_and(contexthub_protocol::is_legacy_model_credential_reference) {
                return Err(reject("model_credential_reentry_required"));
            }
            let same_declarations = old_network.is_none_or(|n| {
                n.proxy_auth == config.network.proxy_auth
                    && n.service_auth == config.network.service_auth
                    && n.header_names == config.network.header_names
                    && n.mode == config.network.mode
                    && n.proxy_url == config.network.proxy_url
            });
            if required
                && (!same_declarations
                    || previous.is_none()
                    || old.is_some_and(|c| c.config.base_url != config.base_url))
                || !required && previous.is_some()
            {
                return Err(reject("model_network_credential_rotation_required"));
            }
            previous.cloned()
        }
        ModelCredentialAction::Replace => {
            if !required {
                return Err(reject("model_network_credential_action_invalid"));
            }
            Some(ModelSecretRef {
                service: MODEL_NETWORK_CREDENTIAL_SERVICE.into(),
                account: format!("{profile_id}:{id}:{revision}"),
            })
        }
        ModelCredentialAction::Clear => {
            if required {
                return Err(reject("model_network_credential_required"));
            }
            None
        }
    };
    credentials_cleanup::require_available(tx, config.network.proxy_secret_ref.as_ref())?;
    Ok(config)
}
