use super::{credential, credential_map, credential_vault_root};
use contexthub_platform_adapter::{credential_vault::LocalCredentialVault, security};
use contexthub_protocol::{
    LEGACY_MCP_CREDENTIAL_SERVICE, MCP_CREDENTIAL_SERVICE, McpAuthMode, McpConnection, McpSecretRef,
};
use serde_json::json;
const PROFILE: &str = "11111111111111111111111111111111";
fn connection(service: &str) -> McpConnection {
    serde_json::from_value(json!({"profileId":PROFILE,"connectionId":"22222222222222222222222222222222","revision":1,"authGeneration":1,
        "config":{"name":"合成凭据夹具","source":{"kind":"remote_http","url":"http://127.0.0.1:9/mcp"},"authMode":"bearer","environmentVariables":[],"network":{"mode":"direct","proxyUrl":null}},
        "credentialRef":{"service":service,"account":format!("{PROFILE}:22222222222222222222222222222222:1")},"enabled":false,"deleted":false,"approvedToolNames":[],"purposes":[],"approvedCatalogDigest":null,"validation":null,"updatedAt":1})).unwrap()
}
#[test]
fn legacy_and_foreign_references_never_open_a_credential_store() {
    let runtime = std::path::Path::new("relative-not-admitted/ai-runtime");
    assert_eq!(
        credential(runtime, &connection(LEGACY_MCP_CREDENTIAL_SERVICE))
            .err()
            .unwrap(),
        "mcp_credential_reentry_required"
    );
    assert_eq!(
        credential(
            runtime,
            &connection("cn.cilai.personal.workbench.vault.model")
        )
        .err()
        .unwrap(),
        "mcp_credentials_reference_invalid"
    );
    assert_eq!(
        credential(runtime, &connection(MCP_CREDENTIAL_SERVICE))
            .err()
            .unwrap(),
        "mcp_profile_changed"
    );
}
#[test]
fn own_profile_vault_decodes_exact_bearer_and_environment_maps() {
    let root = crate::ai_runtime::model_test_fixture::fixture_root("mcp-credentials");
    security::ensure_restricted_directory(&root).unwrap();
    let runtime = root.join("ai-runtime");
    let vault = LocalCredentialVault::new(credential_vault_root(&runtime).unwrap()).unwrap();
    let mut conn = connection(MCP_CREDENTIAL_SERVICE);
    let reference = conn.credential_ref.clone().unwrap();
    vault
        .write(
            &reference.service,
            &reference.account,
            br#"{"token":"synthetic-unusable-bearer-917"}"#,
        )
        .unwrap();
    assert_eq!(
        credential(&runtime, &conn).unwrap().unwrap().expose(),
        "synthetic-unusable-bearer-917"
    );
    conn.auth_generation = 2;
    assert_eq!(
        credential(&runtime, &conn).err().unwrap(),
        "mcp_credentials_reference_invalid"
    );
    conn.credential_ref = Some(McpSecretRef {
        service: MCP_CREDENTIAL_SERVICE.into(),
        account: format!("{PROFILE}:22222222222222222222222222222222:2"),
    });
    assert_eq!(
        credential(&runtime, &conn).err().unwrap(),
        "mcp_credentials_required"
    );
    conn.config.auth_mode = McpAuthMode::Environment;
    conn.config.environment_variables = vec!["EXPLICIT_TEST_TOKEN".into()];
    let second = conn.credential_ref.as_ref().unwrap();
    vault
        .write(
            &second.service,
            &second.account,
            br#"{"EXPLICIT_TEST_TOKEN":"synthetic-env-918"}"#,
        )
        .unwrap();
    assert_eq!(
        credential_map(&runtime, &conn).unwrap()["EXPLICIT_TEST_TOKEN"].expose(),
        "synthetic-env-918"
    );
    conn.config.environment_variables = vec!["UNDECLARED".into()];
    assert_eq!(
        credential_map(&runtime, &conn).err().unwrap(),
        "mcp_credential_invalid"
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn no_auth_does_not_require_or_initialize_a_vault() {
    let mut conn = connection(MCP_CREDENTIAL_SERVICE);
    conn.config.auth_mode = McpAuthMode::None;
    conn.credential_ref = None;
    assert!(
        credential_map(std::path::Path::new("unused"), &conn)
            .unwrap()
            .is_empty()
    );
}
