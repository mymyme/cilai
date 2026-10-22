use super::*;
use contexthub_platform_adapter::{credential_vault::LocalCredentialVault, security};
use contexthub_protocol::{
    ModelNetworkAuthentication, ModelNetworkBasicCredentials, ModelNetworkMode,
};

struct Fixture {
    root: std::path::PathBuf,
    vault: LocalCredentialVault,
}
impl Fixture {
    fn new() -> Self {
        let root = crate::ai_runtime::model_test_fixture::fixture_root("network-vault-reader");
        security::ensure_restricted_directory(&root).unwrap();
        let vault = LocalCredentialVault::new(root.join("credentials")).unwrap();
        Self { root, vault }
    }
    fn write(&self, network: &ModelNetworkConfig, password: &str) {
        let reference = network.proxy_secret_ref.as_ref().unwrap();
        let mut bytes = serde_json::to_vec(&ModelNetworkCredentials {
            proxy_basic: Some(ModelNetworkBasicCredentials {
                username: "synthetic-user".into(),
                password: password.into(),
            }),
            service_basic: None,
            headers: std::collections::BTreeMap::default(),
        })
        .unwrap();
        self.vault
            .write(&reference.service, &reference.account, &bytes)
            .unwrap();
        bytes.fill(0);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn network() -> ModelNetworkConfig {
    ModelNetworkConfig {
        mode: ModelNetworkMode::CustomProxy,
        proxy_url: Some("http://127.0.0.1:1".into()),
        proxy_auth: ModelNetworkAuthentication::Basic,
        proxy_secret_ref: Some(ModelSecretRef {
            service: MODEL_NETWORK_CREDENTIAL_SERVICE.into(),
            account: format!("{}:{}:1", "1".repeat(32), "2".repeat(32)),
        }),
        ..Default::default()
    }
}

#[test]
fn explicit_profile_vault_is_required_and_each_route_rechecks_its_original_root() {
    let first = Fixture::new();
    let second = Fixture::new();
    let config = network();
    first.write(&config, "synthetic-first");
    second.write(&config, "synthetic-second");
    let frozen = read(&config, &first.root.join("credentials"))
        .unwrap()
        .unwrap();
    assert_eq!(
        frozen.value.proxy_basic.as_ref().unwrap().password,
        "synthetic-first"
    );
    assert_eq!(
        read(&config, &second.root.join("credentials"))
            .unwrap()
            .unwrap()
            .value
            .proxy_basic
            .as_ref()
            .unwrap()
            .password,
        "synthetic-second"
    );
    second.write(&config, "synthetic-changed-second");
    frozen.validate_current(&config).unwrap();
    first.write(&config, "synthetic-changed-first");
    assert_eq!(
        frozen.validate_current(&config).unwrap_err(),
        "model_network_credentials_changed"
    );
    let reference = config.proxy_secret_ref.as_ref().unwrap();
    first
        .vault
        .delete(&reference.service, &reference.account)
        .unwrap();
    assert_eq!(
        frozen.validate_current(&config).unwrap_err(),
        "model_network_credential_required"
    );
}

#[test]
fn legacy_references_missing_values_and_corrupt_vaults_are_distinct_without_fallback() {
    let f = Fixture::new();
    let mut config = network();
    assert_eq!(
        read(&config, &f.root.join("credentials")).err().unwrap(),
        "model_network_credential_required"
    );
    assert!(!f.root.join("credentials").exists());
    config.proxy_secret_ref.as_mut().unwrap().service = "cn.cilai.personal.workbench.proxy".into();
    assert_eq!(
        read(&config, &f.root.join("credentials")).err().unwrap(),
        "model_credential_reentry_required"
    );
    assert!(!f.root.join("credentials").exists());
    config = network();
    f.write(&config, "synthetic-secret");
    std::fs::remove_file(f.root.join("credentials/master.key")).unwrap();
    assert_eq!(
        read(&config, &f.root.join("credentials")).err().unwrap(),
        "model_network_credential_access_failed"
    );
    assert!(!f.root.join("credentials/master.key").exists());
    assert_eq!(
        super::super::system_proxy::resolve_without_credentials(&config)
            .err()
            .unwrap(),
        "model_credential_vault_root_required"
    );
}
