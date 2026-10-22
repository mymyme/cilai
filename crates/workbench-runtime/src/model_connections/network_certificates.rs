use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub(super) fn certificates(pem: &str) -> Result<Vec<reqwest::Certificate>, String> {
    contexthub_protocol::validate_model_ca_pem_shape(pem).map_err(str::to_owned)?;
    let certificates = reqwest::Certificate::from_pem_bundle(pem.as_bytes())
        .map_err(|_| "model_ca_certificate_invalid")?;
    if certificates.is_empty() {
        return Err("model_ca_certificate_invalid".into());
    }
    Ok(certificates)
}

/// # Errors
/// 证书不是可装载的公开 X.509 CA 时拒绝；不发请求，不改变系统信任库。
pub fn validate_model_ca_certificate(pem: &str) -> Result<(), String> {
    reqwest::Client::builder()
        .no_proxy()
        .tls_certs_merge(certificates(pem)?)
        .build()
        .map_err(|_| "model_ca_certificate_invalid")?;
    Ok(())
}

pub(super) fn apply_native(
    command: &mut Command,
    home: &Path,
    pem: Option<&str>,
) -> Result<(), String> {
    for name in [
        "CODEX_CA_CERTIFICATE",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NODE_EXTRA_CA_CERTS",
        "REQUESTS_CA_BUNDLE",
        "CURL_CA_BUNDLE",
        "SSLKEYLOGFILE",
    ] {
        command.env_remove(name);
    }
    let Some(pem) = pem else {
        return Ok(());
    };
    validate_model_ca_certificate(pem)?;
    if !home.is_absolute()
        || !managed_connection_home(home)
        || home
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
        || home
            .canonicalize()
            .map_err(|_| "model_ca_certificate_home_invalid")?
            != home
    {
        return Err("model_ca_certificate_home_invalid".into());
    }
    contexthub_platform_adapter::security::ensure_restricted_directory(home)
        .map_err(|_| "model_ca_certificate_home_invalid")?;
    let parent = home.join("network-certificates");
    contexthub_platform_adapter::security::ensure_restricted_directory(&parent)
        .map_err(|_| "model_ca_certificate_home_invalid")?;
    let file = persist_certificate(&parent, pem)?;
    command.env("CODEX_CA_CERTIFICATE", file);
    Ok(())
}

fn managed_connection_home(home: &Path) -> bool {
    let mut parts = home.ancestors();
    let leaf = parts.next().and_then(Path::file_name);
    let kind = parts
        .next()
        .and_then(Path::file_name)
        .and_then(std::ffi::OsStr::to_str);
    let scope = parts
        .next()
        .and_then(Path::file_name)
        .and_then(std::ffi::OsStr::to_str);
    let connections = parts.next().and_then(Path::file_name);
    let runtime = parts.next().and_then(Path::file_name);
    let profile = parts
        .next()
        .and_then(Path::file_name)
        .and_then(std::ffi::OsStr::to_str);
    let profiles = parts.next().and_then(Path::file_name);
    let business = parts.next().and_then(Path::file_name);
    let lower_hex = |value: &str, count: usize| {
        value.len() == count
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    leaf == Some(std::ffi::OsStr::new("official-runtime"))
        && matches!(kind, Some("workbench" | "application"))
        && scope.is_some_and(|value| lower_hex(value, 64))
        && connections == Some(std::ffi::OsStr::new("model-connections"))
        && runtime == Some(std::ffi::OsStr::new("ai-runtime"))
        && profile.is_some_and(|value| lower_hex(value, 32))
        && profiles == Some(std::ffi::OsStr::new("profiles"))
        && business == Some(std::ffi::OsStr::new("business"))
        && !home.components().any(|part| part.as_os_str() == ".codex")
}

fn persist_certificate(parent: &Path, pem: &str) -> Result<PathBuf, String> {
    use sha2::{Digest as _, Sha256};
    let digest = format!("{:x}", Sha256::digest(pem.as_bytes()));
    let file = parent.join(format!("{digest}.pem"));
    if file
        .try_exists()
        .map_err(|_| "model_ca_certificate_storage_failed")?
    {
        contexthub_platform_adapter::security::verify_current_user_private_file(&file)
            .map_err(|_| "model_ca_certificate_storage_failed")?;
        if std::fs::symlink_metadata(&file)
            .map_err(|_| "model_ca_certificate_storage_failed")?
            .len()
            != pem.len() as u64
            || std::fs::read(&file).map_err(|_| "model_ca_certificate_storage_failed")?
                != pem.as_bytes()
        {
            return Err("model_ca_certificate_storage_failed".into());
        }
        return Ok(file);
    }
    let mut output = contexthub_platform_adapter::security::create_new_restricted_file(&file)
        .map_err(|_| "model_ca_certificate_storage_failed")?;
    output
        .write_all(pem.as_bytes())
        .map_err(|_| "model_ca_certificate_storage_failed")?;
    output
        .sync_all()
        .map_err(|_| "model_ca_certificate_storage_failed")?;
    contexthub_platform_adapter::filesystem::sync_directory(parent)
        .map_err(|_| "model_ca_certificate_storage_failed")?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_material_malformed_or_empty_bundles_are_never_accepted() {
        for pem in [
            "",
            "PRIVATE_COMPONENT_SECRET",
            "-----BEGIN PRIVATE KEY-----\nPRIVATE_COMPONENT_SECRET\n-----END PRIVATE KEY-----",
            "-----BEGIN CERTIFICATE-----\ninvalid_base64_private\n-----END CERTIFICATE-----",
        ] {
            assert_eq!(
                validate_model_ca_certificate(pem).unwrap_err(),
                "model_ca_certificate_invalid"
            );
        }
    }

    #[test]
    fn unspecified_certificate_removes_inherited_trust_and_key_logging_overrides() {
        let mut command = Command::new("unexecuted-fixture");
        command
            .env("CODEX_CA_CERTIFICATE", "/untrusted-fixture.pem")
            .env("SSL_CERT_FILE", "/other-fixture.pem")
            .env("SSLKEYLOGFILE", "/secret-output-fixture");
        apply_native(&mut command, Path::new("unused"), None).unwrap();
        for name in ["CODEX_CA_CERTIFICATE", "SSL_CERT_FILE", "SSLKEYLOGFILE"] {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == name && value.is_none())
            );
        }
    }

    #[test]
    fn certificate_files_require_a_personal_connection_profile_scope() {
        let relative = format!(
            "business/profiles/{}/ai-runtime/model-connections/{}/workbench/official-runtime",
            "1".repeat(32),
            "a".repeat(64)
        );
        assert!(managed_connection_home(
            &PathBuf::from("/isolated-personal").join(&relative)
        ));
        for root in ["/unmanaged/official-runtime", "/unmanaged/.codex"] {
            assert!(!managed_connection_home(Path::new(root)));
        }
        assert!(!managed_connection_home(
            &PathBuf::from("/isolated-personal/.codex").join(&relative)
        ));
    }

    fn test_route(pem: Option<String>) -> super::super::system_proxy::ResolvedNetwork {
        super::super::system_proxy::resolve_without_credentials(
            &contexthub_protocol::ModelNetworkConfig {
                ca_certificate_pem: pem,
                ..Default::default()
            },
        )
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires the explicitly generated local TLS fixture; never contacts a model service"]
    async fn real_local_tls_keeps_hostname_validation_and_connection_scoped_trust() {
        let config_path = std::env::var_os("CILAI_PERSONAL_CA_FIXTURE").expect("explicit fixture");
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(config_path).unwrap()).unwrap();
        let root = PathBuf::from(config["root"].as_str().unwrap());
        let pem = std::fs::read_to_string(root.join("ca.pem")).unwrap();
        let other_pem = std::fs::read_to_string(root.join("other-ca.pem")).unwrap();
        let url = config["url"].as_str().unwrap();
        let wrong_hostname = config["wrongHostnameUrl"].as_str().unwrap();
        let address = std::net::SocketAddr::new(
            std::net::Ipv4Addr::LOCALHOST.into(),
            reqwest::Url::parse(url).unwrap().port().unwrap(),
        );
        let ca_route = test_route(Some(pem.clone()));
        let client = |route: &super::super::system_proxy::ResolvedNetwork| {
            route
                .apply_http(
                    reqwest::Client::builder()
                        .resolve("wrong-certificate.fixture.test", address)
                        .timeout(std::time::Duration::from_secs(5)),
                )
                .unwrap()
                .build()
                .unwrap()
        };
        let plain_route = test_route(None);
        assert!(client(&plain_route).get(url).send().await.is_err());
        assert_eq!(
            client(&ca_route).get(url).send().await.unwrap().status(),
            200
        );
        let wrong_name_error = client(&ca_route)
            .get(wrong_hostname)
            .send()
            .await
            .unwrap_err();
        let mut error: Option<&dyn std::error::Error> = Some(&wrong_name_error);
        let mut hostname_rejected = false;
        while let Some(current) = error {
            hostname_rejected |= current.to_string().contains("NotValidForName")
                || current.to_string().contains("not valid for name");
            error = current.source();
        }
        assert!(hostname_rejected);
        let other_route = test_route(Some(other_pem));
        assert!(client(&other_route).get(url).send().await.is_err());
        assert!(client(&plain_route).get(url).send().await.is_err());
        assert_ne!(ca_route.snapshot_digest(), plain_route.snapshot_digest());
        let home = root
            .join("business/profiles")
            .join("1".repeat(32))
            .join("ai-runtime/model-connections")
            .join("a".repeat(64))
            .join("workbench/official-runtime");
        std::fs::create_dir_all(&home).unwrap();
        let mut native = Command::new("unexecuted-fixture");
        apply_native(&mut native, &home, Some(&pem)).unwrap();
        let ca_file = native
            .get_envs()
            .find_map(|(key, value)| (key == "CODEX_CA_CERTIFICATE").then_some(value).flatten())
            .map(PathBuf::from)
            .unwrap();
        contexthub_platform_adapter::security::verify_current_user_private_file(&ca_file).unwrap();
        assert_eq!(std::fs::read_to_string(&ca_file).unwrap(), pem);
        std::fs::write(&ca_file, "tampered-public-certificate").unwrap();
        assert_eq!(
            apply_native(&mut Command::new("unexecuted-fixture"), &home, Some(&pem)).unwrap_err(),
            "model_ca_certificate_storage_failed"
        );
        std::fs::write(&ca_file, &pem).unwrap();
        let mut separate = Command::new("unexecuted-fixture");
        apply_native(&mut separate, &home, None).unwrap();
        assert!(
            separate
                .get_envs()
                .any(|(key, value)| key == "CODEX_CA_CERTIFICATE" && value.is_none())
        );
        std::fs::write(
            root.join("runtime-trust-result.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "defaultTrustRejected":true,"explicitCaAccepted":true,"wrongHostnameRejected":true,
                "unrelatedCaRejected":true,"otherConnectionDidNotInherit":true,
                "privateCertificateFileVerified":true,"tamperedFileRejected":true,
                "certificatePath":ca_file,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
