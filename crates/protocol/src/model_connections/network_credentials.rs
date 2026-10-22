//! 网络认证声明与私有 Host 输入；凭据正文不属于持久连接契约。
use super::{
    ModelAuthMode, ModelConnectionConfig, ModelConnectionKind, ModelNetworkConfig, ModelNetworkMode,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// # Errors
/// 网络引用必须属于同一资料、连接及已分配的修订高水位。
pub fn model_network_reference_generation(
    profile: &str,
    connection: &str,
    revision: u32,
    reference: &super::ModelSecretRef,
) -> Result<u32, &'static str> {
    let fields = reference.account.split(':').collect::<Vec<_>>();
    let hex = |value: &str| {
        value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if !matches!(
        reference.service.as_str(),
        super::MODEL_NETWORK_CREDENTIAL_SERVICE | super::LEGACY_MODEL_NETWORK_CREDENTIAL_SERVICE
    ) || !hex(profile)
        || !hex(connection)
        || fields.len() != 3
        || fields[0] != profile
        || fields[1] != connection
    {
        return Err("model_network_credential_scope_invalid");
    }
    fields[2]
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0 && *value <= revision && value.to_string() == fields[2])
        .ok_or("model_network_credential_scope_invalid")
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelNetworkAuthentication {
    #[default]
    None,
    Basic,
}

/// 仅在个人 Host 的私有保存请求及工作台凭据库中传输，不生成公共 schema。
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelNetworkCredentials {
    pub proxy_basic: Option<ModelNetworkBasicCredentials>,
    pub service_basic: Option<ModelNetworkBasicCredentials>,
    pub headers: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelNetworkBasicCredentials {
    pub username: String,
    pub password: String,
}

fn erase(value: &mut String) {
    let mut bytes = std::mem::take(value).into_bytes();
    bytes.fill(0);
}

impl Drop for ModelNetworkBasicCredentials {
    fn drop(&mut self) {
        erase(&mut self.username);
        erase(&mut self.password);
    }
}
impl Drop for ModelNetworkCredentials {
    fn drop(&mut self) {
        for value in self.headers.values_mut() {
            erase(value);
        }
    }
}

#[must_use]
pub fn model_network_requires_credentials(network: &ModelNetworkConfig) -> bool {
    network.proxy_auth != ModelNetworkAuthentication::None
        || network.service_auth != ModelNetworkAuthentication::None
        || !network.header_names.is_empty()
}

#[must_use]
pub fn model_network_header_allowed(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with("proxy-")
        && !name.starts_with("x-cilai-")
        && !matches!(
            name,
            "authorization"
                | "host"
                | "content-length"
                | "content-type"
                | "accept"
                | "accept-encoding"
                | "connection"
                | "transfer-encoding"
                | "upgrade"
                | "trailer"
                | "te"
                | "keep-alive"
                | "cookie"
                | "set-cookie"
                | "idempotency-key"
                | "forwarded"
                | "x-forwarded-host"
                | "x-forwarded-for"
        )
}

/// # Errors
/// 不支持的认证组合、保留头及重复声明均返回明确拒绝。
pub fn validate_model_network_declarations(
    config: &ModelConnectionConfig,
) -> Result<(), &'static str> {
    let network = &config.network;
    if network.proxy_auth != ModelNetworkAuthentication::None
        && network.mode == ModelNetworkMode::Direct
    {
        return Err("model_proxy_auth_route_invalid");
    }
    if network.header_names.len() > 16
        || network
            .header_names
            .iter()
            .any(|name| !model_network_header_allowed(name))
        || network
            .header_names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != network.header_names.len()
    {
        return Err("model_network_header_invalid");
    }
    if network.service_auth != ModelNetworkAuthentication::None
        && (!matches!(
            config.kind,
            ModelConnectionKind::CustomResponses | ModelConnectionKind::Local
        ) || config.auth_mode != ModelAuthMode::None)
    {
        return Err("model_service_auth_conflict");
    }
    if config.kind == ModelConnectionKind::Chatgpt
        && (network.service_auth != ModelNetworkAuthentication::None
            || !network.header_names.is_empty())
    {
        return Err("model_chatgpt_headers_not_supported");
    }
    Ok(())
}

fn valid_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 8_192 && value.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

fn validate_basic(
    basic: Option<&ModelNetworkBasicCredentials>,
    mode: &ModelNetworkAuthentication,
) -> bool {
    match (basic, mode) {
        (None, ModelNetworkAuthentication::None) => true,
        (Some(basic), ModelNetworkAuthentication::Basic) => {
            valid_value(&basic.username)
                && !basic.username.contains(':')
                && valid_value(&basic.password)
        }
        _ => false,
    }
}

/// # Errors
/// 正文必须与已确认声明完全一致，不接受额外头或未声明的认证。
pub fn validate_model_network_credentials(
    network: &ModelNetworkConfig,
    credentials: &ModelNetworkCredentials,
) -> Result<(), &'static str> {
    if !model_network_requires_credentials(network)
        || !validate_basic(credentials.proxy_basic.as_ref(), &network.proxy_auth)
        || !validate_basic(credentials.service_basic.as_ref(), &network.service_auth)
        || credentials.headers.len() != network.header_names.len()
        || network.header_names.iter().any(|name| {
            !model_network_header_allowed(name)
                || !credentials
                    .headers
                    .get(name)
                    .is_some_and(|value| valid_value(value))
        })
    {
        return Err("model_network_credential_invalid");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_secrets_require_exact_declarations_and_reject_reserved_or_injected_headers() {
        let mut network = ModelNetworkConfig {
            header_names: vec!["x-service-key".into()],
            ..Default::default()
        };
        let mut credentials: ModelNetworkCredentials = serde_json::from_value(serde_json::json!({
            "proxyBasic": null, "serviceBasic": null, "headers":{"x-service-key":"synthetic-only"}
        }))
        .unwrap();
        validate_model_network_credentials(&network, &credentials).unwrap();
        credentials
            .headers
            .insert("authorization".into(), "private".into());
        assert!(validate_model_network_credentials(&network, &credentials).is_err());
        credentials.headers.remove("authorization");
        credentials
            .headers
            .insert("x-service-key".into(), "bad\r\nHost: other".into());
        assert!(validate_model_network_credentials(&network, &credentials).is_err());
        for forbidden in [
            "Authorization",
            "authorization",
            "host",
            "content-length",
            "proxy-authorization",
            "cookie",
            "x-cilai-token",
            "x-key\n",
        ] {
            assert!(!model_network_header_allowed(forbidden));
        }
        network.header_names.push("authorization".into());
        assert!(validate_model_network_credentials(&network, &credentials).is_err());
    }
}
