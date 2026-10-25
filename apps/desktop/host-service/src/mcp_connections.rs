use contexthub_protocol::{
    LEGACY_MCP_CREDENTIAL_SERVICE, McpAuthMode, McpChangeState, McpConnection, McpConnectionChange,
    McpCredentialAction, McpCredentialCleanupParams, McpCredentialCleanupPlan,
    McpCredentialCleanupState, McpSavePrepareParams, McpSecretRef,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveRequest {
    prepare: McpSavePrepareParams,
    secrets: Option<BTreeMap<String, String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContinueRequest {
    operation_id: String,
    secrets: Option<BTreeMap<String, String>>,
}

trait CredentialStore {
    fn read(&self, reference: &McpSecretRef) -> Result<Option<Vec<u8>>, String>;
    fn write(&self, reference: &McpSecretRef, value: &[u8]) -> Result<(), String>;
    fn delete(&self, reference: &McpSecretRef) -> Result<(), String>;
}

struct LocalCredentials {
    root: std::path::PathBuf,
}
impl LocalCredentials {
    fn vault(
        &self,
    ) -> std::io::Result<contexthub_platform_adapter::credential_vault::LocalCredentialVault> {
        contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(self.root.clone())
    }
}
impl CredentialStore for LocalCredentials {
    fn read(&self, reference: &McpSecretRef) -> Result<Option<Vec<u8>>, String> {
        self.vault()
            .and_then(|vault| vault.read(&reference.service, &reference.account))
            .map_err(|_| "mcp_credential_read_failed".to_owned())
    }
    fn write(&self, reference: &McpSecretRef, value: &[u8]) -> Result<(), String> {
        self.vault()
            .and_then(|vault| vault.write(&reference.service, &reference.account, value))
            .map_err(|_| "mcp_credential_write_failed".to_owned())
    }
    fn delete(&self, reference: &McpSecretRef) -> Result<(), String> {
        self.vault()
            .and_then(|vault| vault.delete(&reference.service, &reference.account))
            .map_err(|_| "mcp_credential_delete_failed".to_owned())
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "mcp_response_invalid".to_owned())
}
fn valid_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn scoped_reference(profile: &str, reference: &McpSecretRef) -> Result<(), String> {
    if !reference.is_scoped_to(profile) {
        return Err("mcp_credential_scope_invalid".into());
    }
    Ok(())
}
fn connection_scope(profile: &str, connection: &McpConnection) -> Result<(), String> {
    if !valid_id(profile)
        || connection.profile_id != profile
        || !valid_id(&connection.connection_id)
    {
        return Err("mcp_credential_scope_invalid".into());
    }
    if let Some(reference) = &connection.credential_ref {
        scoped_reference(profile, reference)?;
        if reference.account
            != format!(
                "{profile}:{}:{}",
                connection.connection_id, connection.auth_generation
            )
        {
            return Err("mcp_credential_scope_invalid".into());
        }
    }
    Ok(())
}

fn secret_bytes(
    connection: &McpConnection,
    secrets: Option<&BTreeMap<String, String>>,
) -> Result<Vec<u8>, String> {
    let secrets = secrets.ok_or("mcp_credentials_required")?;
    let expected: Vec<_> = match connection.config.auth_mode {
        McpAuthMode::Bearer => vec!["token"],
        McpAuthMode::Environment => connection
            .config
            .environment_variables
            .iter()
            .map(String::as_str)
            .collect(),
        McpAuthMode::None => return Err("mcp_credential_action_invalid".into()),
    };
    if expected.is_empty()
        || secrets.len() != expected.len()
        || expected.iter().any(|name| !secrets.contains_key(*name))
        || secrets
            .values()
            .any(|value| value.is_empty() || value.len() > 16_384 || value.contains('\0'))
        || (connection.config.auth_mode == McpAuthMode::Bearer
            && secrets.values().any(|value| value.contains(['\n', '\r'])))
    {
        return Err("mcp_credentials_invalid".into());
    }
    let bytes = serde_json::to_vec(secrets).map_err(|_| "mcp_credentials_invalid")?;
    if bytes.len() > 64 * 1024 {
        return Err("mcp_credentials_invalid".into());
    }
    Ok(bytes)
}

fn commit_change(
    profile: &str,
    change: &McpConnectionChange,
    secrets: Option<&BTreeMap<String, String>>,
    store: &impl CredentialStore,
    rpc: &impl Fn(&str, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    connection_scope(profile, &change.connection)?;
    match change.state {
        McpChangeState::Committed => {
            return serde_json::to_value(&change.connection)
                .map_err(|_| "mcp_response_invalid".into());
        }
        McpChangeState::Aborted => return Err("mcp_change_aborted".into()),
        McpChangeState::Prepared => {}
    }
    if change
        .connection
        .credential_ref
        .as_ref()
        .is_some_and(|reference| reference.service == LEGACY_MCP_CREDENTIAL_SERVICE)
    {
        return Err("mcp_credential_reentry_required".into());
    }
    match change.credential_action {
        McpCredentialAction::Replace => {
            let reference = change
                .connection
                .credential_ref
                .as_ref()
                .ok_or("mcp_credential_scope_invalid")?;
            let mut bytes = secret_bytes(&change.connection, secrets)?;
            let saved: Result<(), String> = (|| {
                if let Some(mut existing) = store.read(reference)? {
                    let matches = existing == bytes;
                    existing.fill(0);
                    if !matches {
                        return Err("mcp_credential_pending_value_conflict".into());
                    }
                } else {
                    store.write(reference, &bytes)?;
                }
                let mut readback = store
                    .read(reference)?
                    .ok_or("mcp_credential_readback_failed")?;
                let matches = readback == bytes;
                readback.fill(0);
                if !matches {
                    return Err("mcp_credential_readback_failed".into());
                }
                Ok(())
            })();
            bytes.fill(0);
            saved?;
        }
        McpCredentialAction::Keep | McpCredentialAction::Clear => {
            if secrets.is_some() {
                return Err("mcp_credential_action_invalid".into());
            }
        }
    }
    rpc(
        "mcpConnection/saveCommit",
        &json!({"operationId":change.operation_id,"credentialRef":change.connection.credential_ref}),
    )
}

fn cleanup(
    profile: &str,
    request: &McpCredentialCleanupParams,
    store: &impl CredentialStore,
    rpc: &impl Fn(&str, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    scoped_reference(profile, &request.credential_ref)?;
    if request.credential_ref.service == LEGACY_MCP_CREDENTIAL_SERVICE {
        return Err("mcp_credential_legacy_cleanup_required".into());
    }
    let plan: McpCredentialCleanupPlan = decode(rpc(
        "mcpCredentialCleanup/prepare",
        &serde_json::to_value(request).map_err(|_| "mcp_request_invalid")?,
    )?)?;
    scoped_reference(profile, &plan.credential_ref)?;
    if plan.profile_id != profile
        || plan.credential_ref != request.credential_ref
        || !plan.source_operation_ids.contains(&request.operation_id)
    {
        return Err("mcp_credential_cleanup_changed".into());
    }
    if plan.state == McpCredentialCleanupState::Confirmed {
        return serde_json::to_value(plan).map_err(|_| "mcp_response_invalid".into());
    }
    store.delete(&plan.credential_ref)?;
    if let Some(mut remaining) = store.read(&plan.credential_ref)? {
        remaining.fill(0);
        return Err("mcp_credential_delete_unconfirmed".into());
    }
    rpc(
        "mcpCredentialCleanup/confirm",
        &json!({"cleanupId":plan.cleanup_id,"expectedReferenceDigest":plan.reference_digest}),
    )
}

pub fn handle(
    command: &str,
    args: &Value,
    profile: &str,
    directory: &Path,
    rpc: impl Fn(&str, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let _guard = if matches!(
        command,
        "mcp_connection_save"
            | "mcp_connection_continue_save"
            | "mcp_connection_cancel_save"
            | "mcp_connection_cleanup"
    ) {
        if !valid_id(profile) {
            return Err("mcp_credential_scope_invalid".into());
        }
        Some(
            contexthub_platform_adapter::security::try_lock_restricted_file(
                &directory.join("model-credential-operation.lock"),
            )
            .map_err(|_| "mcp_credential_operation_lock_failed")?
            .ok_or("mcp_credential_operation_in_use")?,
        )
    } else {
        None
    };
    let store = LocalCredentials {
        root: directory.join("credentials"),
    };
    let request = args.get("request").cloned().unwrap_or(Value::Null);
    match command {
        "mcp_connection_save" => {
            let request: SaveRequest = decode(request)?;
            let change: McpConnectionChange = decode(rpc(
                "mcpConnection/savePrepare",
                &serde_json::to_value(request.prepare).map_err(|_| "mcp_request_invalid")?,
            )?)?;
            commit_change(profile, &change, request.secrets.as_ref(), &store, &rpc)
                .map_err(|reason| format!("{reason};operationId={}", change.operation_id))
        }
        "mcp_connection_continue_save" => {
            let request: ContinueRequest = decode(request)?;
            let change: McpConnectionChange = decode(rpc(
                "mcpConnection/changeStatus",
                &json!({"operationId":request.operation_id}),
            )?)?;
            commit_change(profile, &change, request.secrets.as_ref(), &store, &rpc)
                .map_err(|reason| format!("{reason};operationId={}", change.operation_id))
        }
        "mcp_connection_cancel_save" => {
            let change: McpConnectionChange = decode(rpc("mcpConnection/saveAbort", &request)?)?;
            connection_scope(profile, &change.connection)?;
            if change.state != McpChangeState::Aborted {
                return Err("mcp_change_not_aborted".into());
            }
            serde_json::to_value(change).map_err(|_| "mcp_response_invalid".into())
        }
        "mcp_connection_cleanup" => cleanup(profile, &decode(request)?, &store, &rpc),
        _ => {
            let method = match command {
                "mcp_connection_list" => "mcpConnection/list",
                "mcp_connection_changes" => "mcpConnection/changeList",
                "mcp_connection_get" => "mcpConnection/get",
                "mcp_connection_grant" => "mcpConnection/grant",
                "mcp_connection_disable" => "mcpConnection/disable",
                "mcp_connection_delete" => "mcpConnection/delete",
                _ => return Err("mcp_command_not_allowed".into()),
            };
            rpc(method, &request)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_protocol::MCP_CREDENTIAL_SERVICE;
    use std::cell::{Cell, RefCell};
    const PROFILE: &str = "11111111111111111111111111111111";
    fn change() -> McpConnectionChange {
        decode(json!({"operationId":"33333333333333333333333333333333","state":"prepared","expectedRevision":null,
            "connection":{"profileId":PROFILE,"connectionId":"22222222222222222222222222222222","revision":1,"authGeneration":1,
            "config":{"name":"测试工具","source":{"kind":"remote_http","url":"https://example.invalid/mcp"},"authMode":"bearer","environmentVariables":[],"network":{"mode":"direct","proxyUrl":null}},
            "credentialRef":{"service":MCP_CREDENTIAL_SERVICE,"account":format!("{PROFILE}:22222222222222222222222222222222:1")},
            "enabled":false,"deleted":false,"approvedToolNames":[],"purposes":[],"approvedCatalogDigest":null,"validation":null,"updatedAt":1},
            "credentialAction":"replace","cleanupReferences":[],"cleanedReferences":[]})).unwrap()
    }
    #[derive(Default)]
    struct Store {
        value: RefCell<Option<Vec<u8>>>,
        writes: Cell<usize>,
        reads: Cell<usize>,
        deletes: Cell<usize>,
        fail_delete: bool,
    }
    impl CredentialStore for Store {
        fn read(&self, _: &McpSecretRef) -> Result<Option<Vec<u8>>, String> {
            self.reads.set(self.reads.get() + 1);
            Ok(self.value.borrow().clone())
        }
        fn write(&self, _: &McpSecretRef, value: &[u8]) -> Result<(), String> {
            self.writes.set(self.writes.get() + 1);
            self.value.replace(Some(value.to_vec()));
            Ok(())
        }
        fn delete(&self, _: &McpSecretRef) -> Result<(), String> {
            self.deletes.set(self.deletes.get() + 1);
            if self.fail_delete {
                return Err("mcp_credential_delete_failed".into());
            }
            self.value.replace(None);
            Ok(())
        }
    }
    fn secrets() -> BTreeMap<String, String> {
        BTreeMap::from([("token".into(), "synthetic-non-provider-value".into())])
    }
    #[test]
    fn secret_is_read_back_in_own_namespace_before_metadata_commit() {
        let change = change();
        let store = Store::default();
        commit_change(
            PROFILE,
            &change,
            Some(&secrets()),
            &store,
            &|method, params| {
                assert_eq!(method, "mcpConnection/saveCommit");
                assert_eq!(store.writes.get(), 1);
                assert_eq!(store.reads.get(), 2);
                assert_eq!(params["credentialRef"]["service"], MCP_CREDENTIAL_SERVICE);
                assert!(!params.to_string().contains("synthetic-non-provider-value"));
                Ok(Value::Null)
            },
        )
        .unwrap();
    }
    #[test]
    fn pending_secret_cannot_be_overwritten_and_committed_save_does_not_repeat_vault_write() {
        let mut change = change();
        let store = Store::default();
        store.value.replace(Some(b"other".to_vec()));
        assert_eq!(
            commit_change(PROFILE, &change, Some(&secrets()), &store, &|_, _| panic!())
                .unwrap_err(),
            "mcp_credential_pending_value_conflict"
        );
        assert_eq!(store.writes.get(), 0);
        change.state = McpChangeState::Committed;
        assert_eq!(
            commit_change(PROFILE, &change, Some(&secrets()), &store, &|_, _| panic!()).unwrap()["connectionId"],
            change.connection.connection_id
        );
        assert_eq!(store.writes.get(), 0);
    }
    #[test]
    fn foreign_or_model_namespace_is_rejected_before_any_store_access() {
        for service in ["cn.cilai.personal.workbench.model", "Codex Auth"] {
            let mut change = change();
            change.connection.credential_ref.as_mut().unwrap().service = service.into();
            let store = Store::default();
            assert_eq!(
                commit_change(PROFILE, &change, Some(&secrets()), &store, &|_, _| panic!())
                    .unwrap_err(),
                "mcp_credential_scope_invalid"
            );
            assert_eq!(store.reads.get(), 0);
            assert_eq!(store.writes.get(), 0);
        }
        let change = change();
        let store = Store::default();
        assert!(
            commit_change(
                "44444444444444444444444444444444",
                &change,
                Some(&secrets()),
                &store,
                &|_, _| panic!()
            )
            .is_err()
        );
        assert_eq!(store.reads.get(), 0);
    }
    #[test]
    fn undeclared_secret_fields_and_invalid_bearer_never_write() {
        let change = change();
        for invalid in [
            BTreeMap::from([("another".into(), "value".into())]),
            BTreeMap::from([("token".into(), "line\nbreak".into())]),
        ] {
            let store = Store::default();
            assert!(
                commit_change(PROFILE, &change, Some(&invalid), &store, &|_, _| panic!()).is_err()
            );
            assert_eq!(store.writes.get(), 0);
        }
    }
    fn cleanup_request() -> McpCredentialCleanupParams {
        let change = change();
        McpCredentialCleanupParams {
            operation_id: change.operation_id,
            credential_ref: change.connection.credential_ref.unwrap(),
        }
    }
    fn plan() -> McpCredentialCleanupPlan {
        let request = cleanup_request();
        McpCredentialCleanupPlan {
            cleanup_id: "cleanup-one".into(),
            profile_id: PROFILE.into(),
            credential_ref: request.credential_ref,
            state: McpCredentialCleanupState::Prepared,
            source_operation_ids: vec![request.operation_id],
            reference_digest: "known-reference-digest".into(),
        }
    }
    #[test]
    fn cleanup_reservation_precedes_vault_delete_and_confirmation_requires_read_none() {
        let store = Store::default();
        store.value.replace(Some(b"synthetic".to_vec()));
        cleanup(
            PROFILE,
            &cleanup_request(),
            &store,
            &|method, params| match method {
                "mcpCredentialCleanup/prepare" => {
                    assert_eq!(store.deletes.get(), 0);
                    Ok(serde_json::to_value(plan()).unwrap())
                }
                "mcpCredentialCleanup/confirm" => {
                    assert_eq!(store.deletes.get(), 1);
                    assert_eq!(store.reads.get(), 1);
                    assert_eq!(params["expectedReferenceDigest"], "known-reference-digest");
                    Ok(Value::Null)
                }
                _ => panic!(),
            },
        )
        .unwrap();
    }
    #[test]
    fn unconfirmed_delete_never_confirms_and_confirmed_plan_is_idempotent() {
        let store = Store {
            fail_delete: true,
            ..Store::default()
        };
        assert_eq!(
            cleanup(PROFILE, &cleanup_request(), &store, &|method, _| {
                assert_eq!(method, "mcpCredentialCleanup/prepare");
                Ok(serde_json::to_value(plan()).unwrap())
            })
            .unwrap_err(),
            "mcp_credential_delete_failed"
        );
        assert_eq!(store.reads.get(), 0);
        let store = Store::default();
        let mut confirmed = plan();
        confirmed.state = McpCredentialCleanupState::Confirmed;
        cleanup(PROFILE, &cleanup_request(), &store, &|_, _| {
            Ok(serde_json::to_value(&confirmed).unwrap())
        })
        .unwrap();
        assert_eq!(store.deletes.get(), 0);
        assert_eq!(store.reads.get(), 0);
    }
    #[test]
    fn changed_cleanup_owner_or_reference_never_reaches_local_vault() {
        let store = Store::default();
        let mut changed = plan();
        changed.source_operation_ids.clear();
        assert!(
            cleanup(PROFILE, &cleanup_request(), &store, &|_, _| Ok(
                serde_json::to_value(&changed).unwrap()
            ))
            .is_err()
        );
        assert_eq!(store.deletes.get(), 0);
        assert_eq!(store.reads.get(), 0);
    }
    #[test]
    fn legacy_save_requires_reentry_and_never_reads_or_writes_a_store() {
        let mut change = change();
        change.connection.credential_ref.as_mut().unwrap().service =
            LEGACY_MCP_CREDENTIAL_SERVICE.into();
        let store = Store::default();
        assert_eq!(
            commit_change(PROFILE, &change, Some(&secrets()), &store, &|_, _| panic!())
                .unwrap_err(),
            "mcp_credential_reentry_required"
        );
        assert_eq!(store.reads.get(), 0);
        assert_eq!(store.writes.get(), 0);
    }
    #[test]
    fn legacy_cleanup_is_never_confirmed_by_an_empty_local_store() {
        let mut request = cleanup_request();
        request.credential_ref.service = LEGACY_MCP_CREDENTIAL_SERVICE.into();
        let store = Store::default();
        assert_eq!(
            cleanup(PROFILE, &request, &store, &|_, _| panic!()).unwrap_err(),
            "mcp_credential_legacy_cleanup_required"
        );
        assert_eq!(store.reads.get(), 0);
        assert_eq!(store.deletes.get(), 0);
    }
    #[test]
    fn local_vault_commit_reopens_and_cleanup_deletes_only_its_credential() {
        let root = crate::test_support::fixture_root("fixtures/mcp-vault");
        let reference = cleanup_request().credential_ref;
        let store = LocalCredentials {
            root: root.join("credentials"),
        };
        commit_change(PROFILE, &change(), Some(&secrets()), &store, &|_, _| {
            Ok(Value::Null)
        })
        .unwrap();
        let reopened = LocalCredentials {
            root: root.join("credentials"),
        };
        let mut bytes = reopened.read(&reference).unwrap().unwrap();
        assert_eq!(bytes, serde_json::to_vec(&secrets()).unwrap());
        bytes.fill(0);
        cleanup(
            PROFILE,
            &cleanup_request(),
            &reopened,
            &|method, _| match method {
                "mcpCredentialCleanup/prepare" => Ok(serde_json::to_value(plan()).unwrap()),
                "mcpCredentialCleanup/confirm" => Ok(Value::Null),
                _ => panic!(),
            },
        )
        .unwrap();
        assert_eq!(reopened.read(&reference).unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
