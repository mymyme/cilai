//! One immutable, explicit route for Host HTTP and the official runtime child.
use contexthub_platform_adapter::system_proxy::{SystemProxyFeature, SystemProxySnapshot};
use contexthub_protocol::{ModelNetworkConfig, ModelNetworkMode};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::process::Command;

const LOOPBACK_BYPASS: [&str; 3] = ["127.0.0.0/8", "::1", "localhost"];
const PROXY_ENVIRONMENT: [&str; 13] = [
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "SOCKS_PROXY",
    "socks_proxy",
    "FTP_PROXY",
    "ftp_proxy",
    "REQUEST_METHOD",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Route {
    Direct,
    Manual {
        http_proxy: Option<String>,
        https_proxy: Option<String>,
        no_proxy: String,
    },
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolvedNetwork {
    route: Route,
    snapshot_digest: String,
    #[serde(skip)]
    ca_certificate_pem: Option<String>,
    #[serde(skip)]
    credentials: Option<std::sync::Arc<super::network_credentials::Credentials>>,
    #[serde(skip)]
    network: ModelNetworkConfig,
}

impl std::fmt::Debug for ResolvedNetwork {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedNetwork")
            .field("route", &self.route)
            .field("snapshot_digest", &self.snapshot_digest)
            .finish_non_exhaustive()
    }
}

pub(crate) fn resolve(
    network: &ModelNetworkConfig,
    vault_root: &std::path::Path,
) -> Result<ResolvedNetwork, String> {
    let mut route = resolve_route(network)?;
    route.credentials =
        super::network_credentials::read(network, vault_root)?.map(std::sync::Arc::new);
    Ok(route)
}

pub(crate) fn resolve_without_credentials(
    network: &ModelNetworkConfig,
) -> Result<ResolvedNetwork, String> {
    if contexthub_protocol::model_network_requires_credentials(network)
        || network.proxy_secret_ref.is_some()
    {
        return Err("model_credential_vault_root_required".into());
    }
    resolve_route(network)
}

pub(crate) fn direct_network() -> Result<ResolvedNetwork, String> {
    resolved(Route::Direct, &"direct")
}

fn resolve_route(network: &ModelNetworkConfig) -> Result<ResolvedNetwork, String> {
    let mut route = match network.mode {
        ModelNetworkMode::Direct => resolved(Route::Direct, &"direct"),
        ModelNetworkMode::SystemProxy => {
            let mut snapshot = contexthub_platform_adapter::system_proxy::snapshot()?;
            if network.proxy_auth == contexthub_protocol::ModelNetworkAuthentication::Basic {
                snapshot
                    .unsupported_features
                    .retain(|feature| *feature != SystemProxyFeature::Authentication);
            }
            resolve_system_snapshot(&snapshot)
        }
        ModelNetworkMode::CustomProxy => {
            let proxy = proxy_url(
                network
                    .proxy_url
                    .as_deref()
                    .ok_or("model_proxy_url_required")?,
            )?;
            let route = Route::Manual {
                http_proxy: Some(proxy.clone()),
                https_proxy: Some(proxy),
                no_proxy: LOOPBACK_BYPASS.join(","),
            };
            resolved(route, &"custom_proxy")
        }
    }?;
    route.network = network.clone();
    if let Some(pem) = &network.ca_certificate_pem {
        super::network_certificates::validate_model_ca_certificate(pem)?;
        route.snapshot_digest = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    &route.snapshot_digest,
                    format!("{:x}", Sha256::digest(pem.as_bytes()))
                ))
                .map_err(|_| "model_network_snapshot_failed")?
            )
        );
        route.ca_certificate_pem = Some(pem.clone());
    }
    Ok(route)
}

fn resolved(route: Route, source: &impl Serialize) -> Result<ResolvedNetwork, String> {
    let bytes =
        serde_json::to_vec(&(source, &route)).map_err(|_| "model_network_snapshot_failed")?;
    Ok(ResolvedNetwork {
        route,
        snapshot_digest: format!("{:x}", Sha256::digest(bytes)),
        ca_certificate_pem: None,
        credentials: None,
        network: ModelNetworkConfig::default(),
    })
}

pub(crate) fn resolve_system_snapshot(
    snapshot: &SystemProxySnapshot,
) -> Result<ResolvedNetwork, String> {
    if let Some(feature) = snapshot.unsupported_features.first() {
        return Err(match feature {
            SystemProxyFeature::Pac => "model_system_proxy_pac_not_supported",
            SystemProxyFeature::AutoDiscovery => "model_system_proxy_auto_discovery_not_supported",
            SystemProxyFeature::Scoped => "model_system_proxy_scoped_not_supported",
            SystemProxyFeature::SimpleHostnames => "model_system_proxy_bypass_not_supported",
            SystemProxyFeature::Authentication => "model_proxy_credentials_required",
        }
        .into());
    }
    if snapshot.socks_proxy.is_some() {
        return Err("model_system_proxy_socks_not_supported".into());
    }
    let plain_route = snapshot.http_proxy.as_deref().map(proxy_url).transpose()?;
    let tls_route = snapshot.https_proxy.as_deref().map(proxy_url).transpose()?;
    if plain_route.is_none() && tls_route.is_none() {
        return resolved(Route::Direct, snapshot);
    }
    let mut bypass = LOOPBACK_BYPASS.map(str::to_owned).to_vec();
    for entry in &snapshot.bypass {
        let entry = bypass_entry(entry)?;
        if !bypass.contains(&entry) {
            bypass.push(entry);
        }
    }
    let route = Route::Manual {
        http_proxy: plain_route,
        https_proxy: tls_route,
        no_proxy: bypass.join(","),
    };
    resolved(route, snapshot)
}

fn proxy_url(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "model_proxy_url_invalid")?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("model_proxy_url_credentials_forbidden".into());
    }
    if !matches!(url.scheme(), "http" | "https") {
        return Err("model_proxy_scheme_not_supported".into());
    }
    if url.host_str().is_none()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default().is_none_or(|port| port == 0)
        || value.chars().any(char::is_control)
        || value.trim() != value
    {
        return Err("model_proxy_url_invalid".into());
    }
    Ok(url.to_string())
}

fn bypass_entry(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value == "*" {
        return Ok(value.into());
    }
    if let Ok(ip) = value.parse::<IpAddr>() {
        return Ok(ip.to_string());
    }
    if let Some((address, prefix)) = value.split_once('/') {
        let address = address
            .parse::<IpAddr>()
            .map_err(|_| "model_system_proxy_bypass_not_supported")?;
        let prefix = prefix
            .parse::<u32>()
            .map_err(|_| "model_system_proxy_bypass_not_supported")?;
        let canonical = match address {
            IpAddr::V4(ip) if prefix <= 32 => {
                u32::from(ip) & u32::MAX.checked_shl(32 - prefix).unwrap_or(0) == u32::from(ip)
            }
            IpAddr::V6(ip) if prefix <= 128 => {
                u128::from(ip) & u128::MAX.checked_shl(128 - prefix).unwrap_or(0) == u128::from(ip)
            }
            _ => false,
        };
        if canonical {
            return Ok(format!("{address}/{prefix}"));
        }
    }
    Err("model_system_proxy_bypass_not_supported".into())
}

impl ResolvedNetwork {
    pub(super) fn selected_tls_proxy(&self) -> Option<&str> {
        match &self.route {
            Route::Manual { https_proxy, .. } => https_proxy.as_deref(),
            Route::Direct => None,
        }
    }
    pub(super) fn ca_certificate(&self) -> Option<&str> {
        self.ca_certificate_pem.as_deref()
    }
    pub(super) fn proxy_basic(&self) -> Option<&contexthub_protocol::ModelNetworkBasicCredentials> {
        self.credentials
            .as_ref()
            .and_then(|credentials| credentials.value.proxy_basic.as_ref())
    }
    pub(crate) fn requires_native_bridge(&self) -> bool {
        self.proxy_basic().is_some() && self.selected_tls_proxy().is_some()
    }
    pub(crate) fn validate_credentials(&self) -> Result<(), String> {
        if let Some(credentials) = &self.credentials {
            credentials.validate_current(&self.network)?;
        }
        Ok(())
    }

    pub(crate) fn declares_header(&self, name: &str) -> bool {
        self.network
            .header_names
            .iter()
            .any(|header| header == name)
    }
    pub(crate) fn has_credentials(&self) -> bool {
        self.credentials.is_some()
    }

    pub(crate) fn decorate(
        &self,
        request: reqwest::RequestBuilder,
        token: &str,
    ) -> Result<reqwest::RequestBuilder, String> {
        self.validate_credentials()?;
        match &self.credentials {
            Some(credentials) => credentials.apply(request, token),
            None => Ok(request),
        }
    }
    pub(crate) fn snapshot_digest(&self) -> &str {
        &self.snapshot_digest
    }

    pub(crate) fn apply_http(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> Result<reqwest::ClientBuilder, String> {
        let mut builder = builder.no_proxy();
        if let Some(pem) = &self.ca_certificate_pem {
            builder = builder.tls_certs_merge(super::network_certificates::certificates(pem)?);
        }
        if let Route::Manual {
            http_proxy,
            https_proxy,
            no_proxy,
        } = &self.route
        {
            let bypass = reqwest::NoProxy::from_string(no_proxy);
            if let Some(proxy) = http_proxy {
                let mut proxy = reqwest::Proxy::http(proxy)
                    .map_err(|_| "model_proxy_url_invalid")?
                    .no_proxy(bypass.clone());
                if let Some(basic) = self
                    .credentials
                    .as_ref()
                    .and_then(|c| c.value.proxy_basic.as_ref())
                {
                    proxy = proxy.basic_auth(&basic.username, &basic.password);
                }
                builder = builder.proxy(proxy);
            }
            if let Some(proxy) = https_proxy {
                let mut proxy = reqwest::Proxy::https(proxy)
                    .map_err(|_| "model_proxy_url_invalid")?
                    .no_proxy(bypass);
                if let Some(basic) = self
                    .credentials
                    .as_ref()
                    .and_then(|c| c.value.proxy_basic.as_ref())
                {
                    proxy = proxy.basic_auth(&basic.username, &basic.password);
                }
                builder = builder.proxy(proxy);
            }
        }
        Ok(builder)
    }

    pub(crate) fn apply_native(&self, command: &mut Command) {
        for name in PROXY_ENVIRONMENT {
            command.env_remove(name);
        }
        command.args([
            "-c",
            "features.respect_system_proxy=false",
            "-c",
            "features.system_proxy_fallback=false",
        ]);
        match &self.route {
            Route::Direct => {
                command.env("NO_PROXY", "*").env("no_proxy", "*");
            }
            Route::Manual {
                http_proxy,
                https_proxy,
                no_proxy,
            } => {
                for (names, value) in [
                    (["HTTP_PROXY", "http_proxy"], http_proxy),
                    (["HTTPS_PROXY", "https_proxy"], https_proxy),
                ] {
                    if let Some(value) = value {
                        for name in names {
                            command.env(name, value);
                        }
                    }
                }
                command.env("NO_PROXY", no_proxy).env("no_proxy", no_proxy);
            }
        }
    }

    pub(crate) fn apply_native_certificate(
        &self,
        command: &mut Command,
        home: &std::path::Path,
    ) -> Result<(), String> {
        super::network_certificates::apply_native(command, home, self.ca_certificate_pem.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_platform_adapter::system_proxy::SystemProxySource;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    fn snapshot() -> SystemProxySnapshot {
        SystemProxySnapshot {
            source: SystemProxySource::MacOsSystemConfiguration,
            http_proxy: Some("http://127.0.0.1:7890".into()),
            https_proxy: Some("http://127.0.0.1:7891".into()),
            socks_proxy: None,
            bypass: Vec::new(),
            unsupported_features: Vec::new(),
        }
    }
    fn environment(command: &Command, name: &str) -> Option<String> {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
    }
    fn client(route: &ResolvedNetwork) -> reqwest::Client {
        route
            .apply_http(
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(3))
                    .redirect(reqwest::redirect::Policy::none())
                    .retry(reqwest::retry::never()),
            )
            .unwrap()
            .build()
            .unwrap()
    }
    #[test]
    fn frozen_route_does_not_re_read_system_or_inherit_proxy_environment() {
        let original = snapshot();
        let route = resolve_system_snapshot(&original).unwrap();
        let mut changed = original;
        changed.http_proxy = Some("http://127.0.0.1:7999".into());
        assert_ne!(
            route.snapshot_digest(),
            resolve_system_snapshot(&changed).unwrap().snapshot_digest()
        );
        let mut command = Command::new("unused-fixture-command");
        for name in PROXY_ENVIRONMENT {
            command.env(name, "http://inherited.invalid:1");
        }
        route.apply_native(&mut command);
        assert_eq!(
            environment(&command, "HTTP_PROXY").as_deref(),
            Some("http://127.0.0.1:7890/")
        );
        assert_eq!(
            environment(&command, "HTTPS_PROXY").as_deref(),
            Some("http://127.0.0.1:7891/")
        );
        assert_eq!(
            environment(&command, "http_proxy"),
            environment(&command, "HTTP_PROXY")
        );
        assert_eq!(
            environment(&command, "no_proxy"),
            environment(&command, "NO_PROXY")
        );
        assert_eq!(
            environment(&command, "NO_PROXY").as_deref(),
            Some("127.0.0.0/8,::1,localhost")
        );
        for name in ["ALL_PROXY", "all_proxy", "SOCKS_PROXY", "REQUEST_METHOD"] {
            assert!(environment(&command, name).is_none());
        }
        assert!(
            command
                .get_args()
                .any(|arg| arg == "features.respect_system_proxy=false")
        );
    }
    #[test]
    fn unsupported_system_features_and_ambiguous_bypass_are_explicit_errors() {
        for (feature, expected) in [
            (
                SystemProxyFeature::Pac,
                "model_system_proxy_pac_not_supported",
            ),
            (
                SystemProxyFeature::AutoDiscovery,
                "model_system_proxy_auto_discovery_not_supported",
            ),
            (
                SystemProxyFeature::Scoped,
                "model_system_proxy_scoped_not_supported",
            ),
            (
                SystemProxyFeature::SimpleHostnames,
                "model_system_proxy_bypass_not_supported",
            ),
            (
                SystemProxyFeature::Authentication,
                "model_proxy_credentials_required",
            ),
        ] {
            let mut value = snapshot();
            value.unsupported_features = vec![feature];
            assert_eq!(resolve_system_snapshot(&value).unwrap_err(), expected);
        }
        for bypass in [
            "*.local",
            "example.com",
            "<local>",
            "host:443",
            "169.254/16",
            "192.168.1.1/24",
            "",
            "0.0.0.0/33",
        ] {
            let mut value = snapshot();
            value.bypass = vec![bypass.into()];
            assert_eq!(
                resolve_system_snapshot(&value).unwrap_err(),
                "model_system_proxy_bypass_not_supported"
            );
        }
        let mut value = snapshot();
        value.socks_proxy = Some("socks5://127.0.0.1:7892".into());
        assert_eq!(
            resolve_system_snapshot(&value).unwrap_err(),
            "model_system_proxy_socks_not_supported"
        );
    }
    #[test]
    fn only_successfully_observed_system_direct_is_resolved_direct() {
        let mut value = snapshot();
        value.http_proxy = None;
        value.https_proxy = None;
        value.bypass = vec!["*.local".into()];
        let direct = resolve_system_snapshot(&value).unwrap();
        let mut command = Command::new("unused-fixture-command");
        direct.apply_native(&mut command);
        assert_eq!(environment(&command, "NO_PROXY").as_deref(), Some("*"));
        assert!(environment(&command, "HTTP_PROXY").is_none());
        value.unsupported_features.push(SystemProxyFeature::Pac);
        assert_eq!(
            resolve_system_snapshot(&value).unwrap_err(),
            "model_system_proxy_pac_not_supported"
        );
    }
    #[test]
    fn system_http_only_does_not_silently_supply_an_https_or_all_proxy() {
        let mut value = snapshot();
        value.https_proxy = None;
        value.bypass = vec![
            "192.168.0.0/16".into(),
            "2001:db8::/32".into(),
            "127.0.0.0/8".into(),
        ];
        let route = resolve_system_snapshot(&value).unwrap();
        let mut command = Command::new("unused-fixture-command");
        route.apply_native(&mut command);
        assert!(environment(&command, "HTTPS_PROXY").is_none());
        assert!(environment(&command, "ALL_PROXY").is_none());
        assert_eq!(
            environment(&command, "NO_PROXY").as_deref(),
            Some("127.0.0.0/8,::1,localhost,192.168.0.0/16,2001:db8::/32")
        );
    }
    #[test]
    fn custom_routes_reject_authentication_and_unsupported_schemes_without_exposing_values() {
        for (url, expected) in [
            (
                "http://user:synthetic-secret@proxy.invalid:80",
                "model_proxy_url_credentials_forbidden",
            ),
            (
                "socks5://proxy.invalid:1080",
                "model_proxy_scheme_not_supported",
            ),
            ("http://proxy.invalid/path", "model_proxy_url_invalid"),
            ("http://proxy.invalid:0", "model_proxy_url_invalid"),
        ] {
            let config = ModelNetworkConfig {
                mode: ModelNetworkMode::CustomProxy,
                proxy_url: Some(url.into()),
                proxy_secret_ref: None,
                ca_certificate_pem: None,
                ..Default::default()
            };
            let error = resolve_without_credentials(&config).unwrap_err();
            assert_eq!(error, expected);
            assert!(!error.contains("synthetic-secret"));
        }
    }
    async fn responder() -> (String, tokio::task::JoinHandle<String>) {
        responder_with(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]").await
    }
    async fn responder_with(response: &'static [u8]) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut chunk = [0_u8; 1_024];
                let count = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    socket.read(&mut chunk),
                )
                .await
                .unwrap()
                .unwrap();
                assert!(count > 0 && bytes.len() + count <= 16_384);
                bytes.extend_from_slice(&chunk[..count]);
            }
            let request = String::from_utf8(bytes).unwrap();
            socket.write_all(response).await.unwrap();
            request
        });
        (format!("http://{address}"), task)
    }
    #[tokio::test]
    async fn explicit_http_proxy_and_loopback_bypass_use_the_frozen_route() {
        let (proxy, observed) = responder().await;
        let mut value = snapshot();
        value.http_proxy = Some(proxy);
        value.https_proxy = None;
        let route = resolve_system_snapshot(&value).unwrap();
        let response = client(&route)
            .get("http://model.invalid/v1/models")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(
            observed
                .await
                .unwrap()
                .starts_with("GET http://model.invalid/v1/models HTTP/1.1\r\n")
        );
        let (origin, observed) = responder().await;
        let config = ModelNetworkConfig {
            mode: ModelNetworkMode::CustomProxy,
            proxy_url: Some("http://127.0.0.1:9".into()),
            proxy_secret_ref: None,
            ca_certificate_pem: None,
            ..Default::default()
        };
        assert_eq!(
            client(&resolve_without_credentials(&config).unwrap())
                .get(format!("{origin}/direct"))
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::OK
        );
        assert!(
            observed
                .await
                .unwrap()
                .starts_with("GET /direct HTTP/1.1\r\n")
        );
    }

    #[tokio::test]
    async fn https_uses_the_selected_connect_proxy_and_does_not_supply_credentials() {
        use std::error::Error as _;
        let (proxy, observed) = responder_with(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        let mut value = snapshot();
        value.http_proxy = Some("http://127.0.0.1:9".into());
        value.https_proxy = Some(proxy);
        let route = resolve_system_snapshot(&value).unwrap();
        let error = client(&route)
            .get("https://model.invalid/v1/models")
            .send()
            .await
            .unwrap_err();
        let request = observed.await.unwrap();
        assert!(request.starts_with("CONNECT model.invalid:443 HTTP/1.1\r\n"));
        assert!(
            !request
                .to_ascii_lowercase()
                .contains("proxy-authorization:")
        );
        let mut source = error.source();
        let mut auth_required = false;
        while let Some(error) = source {
            auth_required |= error.to_string().contains("proxy authorization required");
            source = error.source();
        }
        assert!(auth_required);
    }

    fn isolated_environment(mode: &str) {
        let owner = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../.personal-build/target-runtime-audit/fixtures")
            .join(format!("proxy-env-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&owner).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.env_clear();
        contexthub_platform_adapter::process::retain_required_system_environment(&mut command)
            .unwrap();
        command.args(["--exact", "model_connections::system_proxy::tests::external_environment_cannot_override_the_frozen_host_route", "--nocapture"])
            .env("CILAI_PERSONAL_PROXY_ROUTE_TEST", mode)
            .env("HOME", &owner).env("USERPROFILE", &owner).env("XDG_CONFIG_HOME", owner.join("config")).env("XDG_DATA_HOME", owner.join("data"));
        for name in [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
        ] {
            command.env(name, "http://127.0.0.1:9");
        }
        let no_proxy = if mode == "proxy" { "*" } else { "" };
        command.env("NO_PROXY", no_proxy).env("no_proxy", no_proxy);
        let output = command.output().unwrap();
        std::fs::remove_dir_all(owner).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[tokio::test]
    async fn external_environment_cannot_override_the_frozen_host_route() {
        let Ok(mode) = std::env::var("CILAI_PERSONAL_PROXY_ROUTE_TEST") else {
            for mode in ["proxy", "direct"] {
                isolated_environment(mode);
            }
            return;
        };
        let (endpoint, observed) = responder().await;
        let (route, target, expected) = match mode.as_str() {
            "proxy" => {
                let mut value = snapshot();
                value.http_proxy = Some(endpoint);
                value.https_proxy = None;
                (
                    resolve_system_snapshot(&value).unwrap(),
                    "http://model.invalid/explicit".to_owned(),
                    "GET http://model.invalid/explicit HTTP/1.1\r\n",
                )
            }
            "direct" => (
                resolve_without_credentials(&ModelNetworkConfig::default()).unwrap(),
                format!("{endpoint}/direct"),
                "GET /direct HTTP/1.1\r\n",
            ),
            _ => panic!("unknown isolated route fixture"),
        };
        assert_eq!(
            client(&route).get(target).send().await.unwrap().status(),
            reqwest::StatusCode::OK
        );
        assert!(observed.await.unwrap().starts_with(expected));
    }
}
