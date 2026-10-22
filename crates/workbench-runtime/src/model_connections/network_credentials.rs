//! 网络认证正文仅从当前资料显式绑定的本地凭据库读取。
use contexthub_protocol::{
    MODEL_NETWORK_CREDENTIAL_SERVICE, ModelNetworkConfig, ModelNetworkCredentials, ModelSecretRef,
    model_network_requires_credentials, validate_model_network_credentials,
};
use sha2::{Digest as _, Sha256};

pub(super) struct Credentials {
    pub value: ModelNetworkCredentials,
    fingerprint: [u8; 32],
    vault_root: std::path::PathBuf,
}

pub(super) fn validate_reference(reference: &ModelSecretRef) -> Result<(), String> {
    if reference.service == "cn.cilai.personal.workbench.proxy" {
        return Err("model_credential_reentry_required".into());
    }
    let parts = reference.account.split(':').collect::<Vec<_>>();
    let hex = |part: &str| {
        part.len() == 32
            && part
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    if reference.service != MODEL_NETWORK_CREDENTIAL_SERVICE
        || parts.len() != 3
        || !hex(parts[0])
        || !hex(parts[1])
        || !parts[2]
            .parse::<u32>()
            .ok()
            .is_some_and(|n| n > 0 && n.to_string() == parts[2])
    {
        return Err("model_network_credential_scope_invalid".into());
    }
    Ok(())
}

pub(super) fn read(
    network: &ModelNetworkConfig,
    vault_root: &std::path::Path,
) -> Result<Option<Credentials>, String> {
    if !model_network_requires_credentials(network) {
        if network.proxy_secret_ref.is_some() {
            return Err("model_network_credential_scope_invalid".into());
        }
        return Ok(None);
    }
    let reference = network
        .proxy_secret_ref
        .as_ref()
        .ok_or("model_network_credential_required")?;
    validate_reference(reference)?;
    let mut bytes = contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(
        vault_root.to_path_buf(),
    )
    .map_err(|_| "model_network_credential_access_failed")?
    .read(&reference.service, &reference.account)
    .map_err(|_| "model_network_credential_access_failed")?
    .ok_or("model_network_credential_required")?;
    let parsed = decode(network, &bytes, vault_root);
    bytes.fill(0);
    parsed.map(Some)
}

fn decode(
    network: &ModelNetworkConfig,
    bytes: &[u8],
    vault_root: &std::path::Path,
) -> Result<Credentials, String> {
    if bytes.len() > 160_000 {
        return Err("model_network_credential_invalid".into());
    }
    let value: ModelNetworkCredentials =
        serde_json::from_slice(bytes).map_err(|_| "model_network_credential_invalid")?;
    validate_model_network_credentials(network, &value)?;
    Ok(Credentials {
        value,
        fingerprint: Sha256::digest(bytes).into(),
        vault_root: vault_root.to_path_buf(),
    })
}

impl Credentials {
    pub(super) fn validate_current(&self, network: &ModelNetworkConfig) -> Result<(), String> {
        let fresh = read(network, &self.vault_root)?.ok_or("model_network_credential_required")?;
        if self.fingerprint != fresh.fingerprint {
            return Err("model_network_credentials_changed".into());
        }
        Ok(())
    }
    pub(super) fn apply(
        &self,
        request: reqwest::RequestBuilder,
        model_token: &str,
    ) -> Result<reqwest::RequestBuilder, String> {
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in &self.value.headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| "model_network_header_invalid")?;
            let mut value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| "model_network_header_invalid")?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        let mut request = request.headers(headers);
        if let Some(basic) = &self.value.service_basic {
            if !model_token.is_empty() {
                return Err("model_service_auth_conflict".into());
            }
            request = request.basic_auth(&basic.username, Some(&basic.password));
        }
        Ok(request)
    }
}

#[cfg(test)]
mod tests;
