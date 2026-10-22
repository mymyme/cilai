use contexthub_protocol::{
    MODEL_CREDENTIAL_SERVICE, ModelConnection, ModelConnectionChange, ModelConnectionChangeState,
    ModelConnectionCommitParams, ModelConnectionSavePrepareParams, ModelCredentialAction,
    ModelCredentialCleanupPlan, ModelCredentialCleanupPlanState,
    ModelCredentialCleanupPrepareParams, ModelCredentialCleanupState, ModelSecretRef,
};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
#[path = "model_network_credentials.rs"]
mod network_credentials;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveRequest {
    prepare: ModelConnectionSavePrepareParams,
    secret: Option<String>,
    network_secret: Option<contexthub_protocol::ModelNetworkCredentials>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_protocol::{
        ModelAuthMode, ModelConnectionChangeIntent, ModelConnectionConfig, ModelConnectionKind,
        ModelNetworkConfig,
    };
    use std::cell::{Cell, RefCell};
    mod network_tests;

    const PROFILE: &str = "11111111111111111111111111111111";

    #[test]
    fn legacy_pending_save_is_cancelled_before_manual_cleanup_is_reported() {
        let mut change = prepared();
        change.connection.secret_ref.as_mut().unwrap().service =
            contexthub_protocol::LEGACY_MODEL_CREDENTIAL_SERVICE.into();
        let store = TestStore::default();
        let aborted = Cell::new(false);
        let result = cancel_change(PROFILE, &change, false, &store, &|method, _| {
            assert_eq!(method, "modelConnection/saveAbort");
            aborted.set(true);
            let mut cancelled = change.clone();
            cancelled.state = ModelConnectionChangeState::Aborted;
            cancelled.credential_cleanup = ModelCredentialCleanupState::Unconfirmed;
            Ok(serde_json::to_value(cancelled).unwrap())
        });
        assert_eq!(
            result.unwrap_err(),
            "model_credential_legacy_cleanup_required"
        );
        assert!(aborted.get());
        assert_eq!(
            store.reads.get() + store.writes.get() + store.deletes.get(),
            0
        );
        assert_eq!(
            commit_change(
                PROFILE,
                &change,
                Some("synthetic-only"),
                &store,
                &|_, _| panic!("legacy commit")
            )
            .unwrap_err(),
            "model_credential_reentry_required"
        );
    }

    #[test]
    fn invalid_resumed_scope_cannot_write_a_valid_network_credential() {
        let mut change = prepared();
        change.connection.secret_ref.as_mut().unwrap().service =
            contexthub_protocol::LEGACY_MODEL_CREDENTIAL_SERVICE.into();
        change.network_credential_action = ModelCredentialAction::Replace;
        change.connection.config.network.header_names = vec!["x-test-key".into()];
        change.connection.config.network.proxy_secret_ref = Some(ModelSecretRef {
            service: contexthub_protocol::MODEL_NETWORK_CREDENTIAL_SERVICE.into(),
            account: format!("{PROFILE}:{}:1", change.connection.connection_id),
        });
        let credentials = serde_json::from_value(serde_json::json!({"proxyBasic":null,"serviceBasic":null,"headers":{"x-test-key":"synthetic-only"}})).unwrap();
        let store = TestStore::default();
        let error = finish_prepared_change(
            PROFILE,
            &change,
            Some("synthetic-only"),
            Some(&credentials),
            &store,
            &|_, _| panic!("invalid resumed scope must not commit"),
        )
        .unwrap_err();
        assert!(error.starts_with("model_credential_reentry_required;operationId="));
        assert_eq!(
            store.reads.get() + store.writes.get() + store.deletes.get(),
            0
        );
    }

    #[test]
    fn local_model_store_commits_after_readback_and_reopens_without_system_storage() {
        let parent =
            std::env::temp_dir().join(format!("cilai-host-local-vault-{}", uuid::Uuid::new_v4()));
        contexthub_platform_adapter::security::ensure_restricted_directory(&parent).unwrap();
        let root = std::fs::canonicalize(&parent).unwrap().join("credentials");
        let store = LocalCredentials(root.clone());
        let change = prepared();
        commit_change(
            PROFILE,
            &change,
            Some("synthetic-host-model-value"),
            &store,
            &|method, _| {
                assert_eq!(method, "modelConnection/saveCommit");
                Ok(serde_json::to_value(&change.connection).unwrap())
            },
        )
        .unwrap();
        let reference = change.connection.secret_ref.as_ref().unwrap();
        let reopened = LocalCredentials(root);
        let mut value = reopened.read(reference).unwrap().unwrap();
        assert_eq!(value, b"synthetic-host-model-value");
        value.fill(0);
        clear_credential(reference, &reopened).unwrap();
        assert!(reopened.read(reference).unwrap().is_none());
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn invalid_ca_cannot_touch_credentials_or_commit_a_prepared_connection() {
        let mut change = prepared();
        change.connection.config.network.ca_certificate_pem = Some(
            "-----BEGIN PRIVATE KEY-----\nPRIVATE_SYNTHETIC_CERT_MATERIAL\n-----END PRIVATE KEY-----".into(),
        );
        let store = TestStore::default();
        let calls = Cell::new(0);
        let error = commit_change(
            PROFILE,
            &change,
            Some("synthetic-component-key"),
            &store,
            &|_, _| {
                calls.set(calls.get() + 1);
                Ok(Value::Null)
            },
        )
        .unwrap_err();
        assert_eq!(error, "model_ca_certificate_invalid");
        assert_eq!(
            store.reads.get() + store.writes.get() + store.deletes.get(),
            0
        );
        assert_eq!(calls.get(), 0);
        assert!(!error.contains("PRIVATE_SYNTHETIC_CERT_MATERIAL"));
    }

    #[test]
    fn saved_metadata_is_preserved_when_catalog_publication_fails() {
        let saved = serde_json::json!({"connectionId":"saved-connection","configRevision":2});
        let receipt = finish_metadata_change("model_connection_save", saved.clone(), || {
            Err("internal transport details must not escape".to_owned())
        });
        assert_eq!(receipt["result"], saved);
        assert_eq!(
            receipt["catalogRefreshError"],
            "model_metadata_saved_catalog_refresh_failed"
        );
        assert!(!receipt.to_string().contains("internal transport details"));
        let read = finish_metadata_change("model_connection_list", serde_json::json!([]), || {
            panic!("a metadata read must not publish a change")
        });
        assert_eq!(read, serde_json::json!([]));
    }

    fn prepared() -> ModelConnectionChange {
        ModelConnectionChange {
            intent: ModelConnectionChangeIntent::ConnectionSave,
            operation_id: "33333333333333333333333333333333".to_owned(),
            request_digest: "digest".to_owned(),
            expected_config_revision: None,
            connection: ModelConnection {
                profile_id: PROFILE.to_owned(),
                connection_id: "22222222222222222222222222222222".to_owned(),
                config_revision: 1,
                auth_generation: 1,
                auth_scopes: Vec::new(),
                config: ModelConnectionConfig {
                    name: "个人连接".to_owned(),
                    kind: ModelConnectionKind::OpenaiKey,
                    base_url: None,
                    local_runtime: None,
                    auth_mode: ModelAuthMode::ApiKey,
                    enabled: true,
                    network: ModelNetworkConfig::default(),
                },
                secret_ref: Some(ModelSecretRef {
                    service: MODEL_CREDENTIAL_SERVICE.to_owned(),
                    account: format!("{PROFILE}:22222222222222222222222222222222:1"),
                }),
                deleted: false,
                created_at: 1,
                updated_at: 1,
            },
            credential_action: ModelCredentialAction::Replace,
            network_credential_action: ModelCredentialAction::Keep,
            state: ModelConnectionChangeState::Prepared,
            credential_cleanup: ModelCredentialCleanupState::NotRequired,
            prepared_at: 1,
        }
    }

    #[derive(Default)]
    struct TestStore {
        fail_write: bool,
        fail_delete: bool,
        wrong_readback: bool,
        writes: Cell<usize>,
        reads: Cell<usize>,
        deletes: Cell<usize>,
        value: RefCell<Vec<u8>>,
    }

    impl CredentialStore for TestStore {
        fn write(&self, _: &ModelSecretRef, value: &[u8]) -> Result<(), String> {
            self.writes.set(self.writes.get() + 1);
            if self.fail_write {
                return Err("model_credential_write_failed".to_owned());
            }
            self.value.replace(value.to_vec());
            Ok(())
        }
        fn read(&self, _: &ModelSecretRef) -> Result<Option<Vec<u8>>, String> {
            self.reads.set(self.reads.get() + 1);
            if self.value.borrow().is_empty() {
                return Ok(None);
            }
            Ok(Some(if self.wrong_readback {
                b"other".to_vec()
            } else {
                self.value.borrow().clone()
            }))
        }
        fn delete(&self, _: &ModelSecretRef) -> Result<(), String> {
            self.deletes.set(self.deletes.get() + 1);
            if self.fail_delete {
                return Err("model_credential_delete_failed".to_owned());
            }
            self.value.borrow_mut().clear();
            Ok(())
        }
    }

    #[test]
    fn failed_system_write_or_readback_never_commits_metadata() {
        for store in [
            TestStore {
                fail_write: true,
                ..TestStore::default()
            },
            TestStore {
                wrong_readback: true,
                ..TestStore::default()
            },
        ] {
            let commits = Cell::new(0);
            let result = commit_change(
                PROFILE,
                &prepared(),
                Some("synthetic-local-component-secret"),
                &store,
                &|_, _| {
                    commits.set(commits.get() + 1);
                    Ok(Value::Null)
                },
            );
            assert!(result.is_err());
            assert_eq!(commits.get(), 0);
        }
    }

    #[test]
    fn saved_secret_is_read_back_before_commit_and_not_in_commit_payload() {
        let store = TestStore::default();
        let change = prepared();
        let result = commit_change(
            PROFILE,
            &change,
            Some("synthetic-local-component-secret"),
            &store,
            &|method, params| {
                assert_eq!(method, "modelConnection/saveCommit");
                assert_eq!(store.writes.get(), 1);
                assert!(
                    !params
                        .to_string()
                        .contains("synthetic-local-component-secret")
                );
                assert_eq!(params["credentialRef"]["service"], MODEL_CREDENTIAL_SERVICE);
                Ok(serde_json::json!({"saved":true}))
            },
        )
        .unwrap();
        assert_eq!(result["saved"], true);
    }

    #[test]
    fn another_profile_or_arbitrary_system_reference_is_rejected_before_reading() {
        let store = TestStore::default();
        for wrong_service in [true, false] {
            let mut change = prepared();
            if wrong_service {
                change.connection.secret_ref.as_mut().unwrap().service = "Codex Auth".to_owned();
            } else {
                change.connection.profile_id = "44444444444444444444444444444444".to_owned();
            }
            assert_eq!(
                commit_change(
                    PROFILE,
                    &change,
                    Some("synthetic-local-component-secret"),
                    &store,
                    &|_, _| panic!("must not commit")
                )
                .unwrap_err(),
                "model_credential_scope_invalid"
            );
        }
        assert_eq!(store.writes.get(), 0);
    }

    #[test]
    fn a_committed_operation_does_not_repeat_system_writes() {
        let mut change = prepared();
        change.state = ModelConnectionChangeState::Committed;
        let store = TestStore::default();
        let result = commit_change(
            PROFILE,
            &change,
            Some("synthetic-local-component-secret"),
            &store,
            &|_, _| panic!("must not recommit"),
        )
        .unwrap();
        assert_eq!(result["connectionId"], change.connection.connection_id);
        assert_eq!(store.writes.get(), 0);
    }
    #[test]
    fn a_prepared_credential_is_never_overwritten_by_a_different_resume_value() {
        let store = TestStore::default();
        store.value.replace(b"saved-first-value".to_vec());
        let commits = Cell::new(0);
        let call = |_: &str, _: &Value| {
            commits.set(commits.get() + 1);
            Ok(Value::Null)
        };
        assert_eq!(
            commit_change(PROFILE, &prepared(), Some("different-value"), &store, &call)
                .unwrap_err(),
            "model_credential_pending_value_conflict"
        );
        assert_eq!(store.value.borrow().as_slice(), b"saved-first-value");
        assert_eq!(store.writes.get(), 0);
        assert_eq!(commits.get(), 0);
        commit_change(
            PROFILE,
            &prepared(),
            Some("saved-first-value"),
            &store,
            &call,
        )
        .unwrap();
        assert_eq!(store.writes.get(), 0);
        assert_eq!(commits.get(), 1);
    }

    #[test]
    fn commit_winning_an_abort_race_preserves_the_active_system_credential() {
        let store = TestStore::default();
        store.value.replace(b"active-value".to_vec());
        let result = cancel_change(PROFILE, &prepared(), false, &store, &|method, _| {
            assert_eq!(method, "modelConnection/saveAbort");
            Err("model_connection_change_not_prepared".to_owned())
        });
        assert!(result.is_err());
        assert_eq!(store.value.borrow().as_slice(), b"active-value");
    }

    #[test]
    fn cancelled_credential_cleanup_can_fail_and_resume_without_reopening_the_save() {
        let mut change = prepared();
        change.state = ModelConnectionChangeState::Aborted;
        change.credential_cleanup = ModelCredentialCleanupState::Pending;
        let store = TestStore {
            fail_delete: true,
            ..TestStore::default()
        };
        store.value.replace(b"pending-cleanup".to_vec());
        assert_eq!(
            cancel_change(PROFILE, &change, true, &store, &|_, _| panic!(
                "failed cleanup cannot acknowledge or save"
            ))
            .unwrap_err(),
            "model_credential_delete_failed"
        );
        assert_eq!(store.value.borrow().as_slice(), b"pending-cleanup");
        let store = TestStore::default();
        store.value.replace(b"pending-cleanup".to_vec());
        let result = cancel_change(PROFILE, &change, true, &store, &|method, params| {
            assert_eq!(method, "modelConnection/credentialCleanupAck");
            assert_eq!(params["operationId"], change.operation_id);
            let mut confirmed = change.clone();
            confirmed.credential_cleanup = ModelCredentialCleanupState::Confirmed;
            Ok(serde_json::to_value(confirmed).unwrap())
        })
        .unwrap();
        assert_eq!(result["credentialCleanup"], "confirmed");
        assert!(store.value.borrow().is_empty());
        assert_eq!(store.writes.get(), 0);
    }

    fn cleanup_plan() -> ModelCredentialCleanupPlan {
        let connection = prepared().connection;
        ModelCredentialCleanupPlan {
            cleanup_id: "55555555555555555555555555555555".to_owned(),
            profile_id: connection.profile_id,
            connection_id: connection.connection_id,
            auth_generation: connection.auth_generation,
            credential_ref: connection.secret_ref.unwrap(),
            reference_digest: "exact-reference-digest".to_owned(),
            state: ModelCredentialCleanupPlanState::Pending,
            prepared_at: 1,
            confirmed_at: None,
        }
    }

    fn cleanup_request() -> Value {
        let plan = cleanup_plan();
        serde_json::json!({
            "cleanupId": plan.cleanup_id,
            "expectedReferenceDigest": plan.reference_digest,
        })
    }

    #[test]
    fn historical_cleanup_rejects_another_scope_or_changed_confirmation_before_system_access() {
        for invalid in 0..5 {
            let mut plan = cleanup_plan();
            match invalid {
                0 => plan.profile_id = "66666666666666666666666666666666".to_owned(),
                1 => plan.credential_ref.service = "Codex Auth".to_owned(),
                2 => plan.credential_ref.account.push_str(":other"),
                3 => plan.cleanup_id = "77777777777777777777777777777777".to_owned(),
                _ => plan.reference_digest = "changed-reference-digest".to_owned(),
            }
            let store = TestStore::default();
            let calls = Cell::new(0);
            let result =
                cleanup_historical_credential(PROFILE, &cleanup_request(), &store, &|method, _| {
                    calls.set(calls.get() + 1);
                    assert_eq!(method, "modelCredentialCleanup/prepare");
                    Ok(serde_json::to_value(&plan).unwrap())
                });
            assert!(result.is_err());
            assert_eq!(calls.get(), 1);
            assert_eq!(store.deletes.get(), 0);
            assert_eq!(store.reads.get(), 0);
        }
    }

    #[test]
    fn confirmed_historical_cleanup_does_not_repeat_system_access() {
        let mut plan = cleanup_plan();
        plan.state = ModelCredentialCleanupPlanState::Confirmed;
        plan.confirmed_at = Some(2);
        let store = TestStore::default();
        let result =
            cleanup_historical_credential(PROFILE, &cleanup_request(), &store, &|method, _| {
                assert_eq!(method, "modelCredentialCleanup/prepare");
                Ok(serde_json::to_value(&plan).unwrap())
            })
            .unwrap();
        assert_eq!(result["state"], "confirmed");
        assert_eq!(store.deletes.get(), 0);
        assert_eq!(store.reads.get(), 0);
    }

    #[test]
    fn historical_cleanup_acknowledges_only_after_system_deletion_and_readback() {
        let store = TestStore::default();
        store.value.replace(b"synthetic-historical-value".to_vec());
        let plan = cleanup_plan();
        let result = cleanup_historical_credential(
            PROFILE,
            &cleanup_request(),
            &store,
            &|method, params| match method {
                "modelCredentialCleanup/prepare" => Ok(serde_json::to_value(&plan).unwrap()),
                "modelCredentialCleanup/ack" => {
                    assert!(store.value.borrow().is_empty());
                    assert_eq!(store.deletes.get(), 1);
                    assert_eq!(store.reads.get(), 1);
                    assert_eq!(params["cleanupId"], plan.cleanup_id);
                    assert_eq!(params["expectedReferenceDigest"], plan.reference_digest);
                    assert_eq!(
                        params["credentialRef"],
                        serde_json::to_value(&plan.credential_ref).unwrap()
                    );
                    let mut confirmed = plan.clone();
                    confirmed.state = ModelCredentialCleanupPlanState::Confirmed;
                    confirmed.confirmed_at = Some(2);
                    Ok(serde_json::to_value(confirmed).unwrap())
                }
                _ => panic!("historical cleanup cannot save a connection"),
            },
        )
        .unwrap();
        assert_eq!(result["state"], "confirmed");
        assert_eq!(store.writes.get(), 0);
    }

    #[test]
    fn failed_historical_system_deletion_never_acknowledges_the_plan() {
        let store = TestStore {
            fail_delete: true,
            ..TestStore::default()
        };
        store.value.replace(b"synthetic-historical-value".to_vec());
        let calls = Cell::new(0);
        let result =
            cleanup_historical_credential(PROFILE, &cleanup_request(), &store, &|method, _| {
                calls.set(calls.get() + 1);
                assert_eq!(method, "modelCredentialCleanup/prepare");
                Ok(serde_json::to_value(cleanup_plan()).unwrap())
            });
        assert_eq!(result.unwrap_err(), "model_credential_delete_failed");
        assert_eq!(calls.get(), 1);
        assert_eq!(store.reads.get(), 0);
        assert_eq!(
            store.value.borrow().as_slice(),
            b"synthetic-historical-value"
        );
    }

    #[test]
    fn credential_mutations_are_excluded_across_host_processes() {
        let directory = std::env::temp_dir().join(format!(
            "cilai-personal-credential-lock-{}",
            uuid::Uuid::new_v4()
        ));
        contexthub_platform_adapter::security::ensure_restricted_directory(&directory).unwrap();
        let guard = contexthub_platform_adapter::security::try_lock_restricted_file(
            &directory.join("model-credential-operation.lock"),
        )
        .unwrap()
        .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "model_connections::tests::credential_guard_other_host_probe",
            ])
            .env("CILAI_PERSONAL_TEST_CREDENTIAL_DIR", &directory)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        drop(guard);
        let mut aborted = prepared();
        aborted.state = ModelConnectionChangeState::Aborted;
        let calls = Cell::new(0);
        let result = handle(
            "model_connection_resume",
            &serde_json::json!({"request":{"operationId":aborted.operation_id,"secret":null}}),
            PROFILE,
            &directory,
            |_, _| {
                calls.set(calls.get() + 1);
                Ok(serde_json::to_value(&aborted).unwrap())
            },
        );
        assert!(
            result
                .unwrap_err()
                .contains("model_connection_change_aborted")
        );
        assert_eq!(calls.get(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[ignore = "由跨进程凭据锁测试使用独立目录启动"]
    fn credential_guard_other_host_probe() {
        let directory = std::env::var_os("CILAI_PERSONAL_TEST_CREDENTIAL_DIR")
            .expect("isolated parent directory required");
        let result = handle(
            "model_connection_resume",
            &Value::Null,
            PROFILE,
            Path::new(&directory),
            |_, _| panic!("locked mutation must not reach RPC or system credentials"),
        );
        assert_eq!(result.unwrap_err(), "model_credential_operation_in_use");
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResumeRequest {
    operation_id: String,
    secret: Option<String>,
    network_secret: Option<contexthub_protocol::ModelNetworkCredentials>,
}

trait CredentialStore {
    fn write(&self, reference: &ModelSecretRef, secret: &[u8]) -> Result<(), String>;
    fn read(&self, reference: &ModelSecretRef) -> Result<Option<Vec<u8>>, String>;
    fn delete(&self, reference: &ModelSecretRef) -> Result<(), String>;
}

struct LocalCredentials(std::path::PathBuf);

impl LocalCredentials {
    fn vault(
        &self,
    ) -> Result<contexthub_platform_adapter::credential_vault::LocalCredentialVault, String> {
        contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(self.0.clone())
            .map_err(|_| "model_credential_vault_unavailable".into())
    }
}

impl CredentialStore for LocalCredentials {
    fn write(&self, reference: &ModelSecretRef, secret: &[u8]) -> Result<(), String> {
        self.vault()?
            .write(&reference.service, &reference.account, secret)
            .map_err(|_| "model_credential_write_failed".to_owned())
    }

    fn read(&self, reference: &ModelSecretRef) -> Result<Option<Vec<u8>>, String> {
        self.vault()?
            .read(&reference.service, &reference.account)
            .map_err(|_| "model_credential_read_failed".to_owned())
    }

    fn delete(&self, reference: &ModelSecretRef) -> Result<(), String> {
        self.vault()?
            .delete(&reference.service, &reference.account)
            .map_err(|_| "model_credential_delete_failed".to_owned())
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "model_connection_response_invalid".to_owned())
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_reference(profile_id: &str, connection: &ModelConnection) -> Result<(), String> {
    if let Some(reference) = &connection.config.network.proxy_secret_ref {
        contexthub_protocol::model_network_reference_generation(
            profile_id,
            &connection.connection_id,
            connection.config_revision,
            reference,
        )?;
    }
    if connection.secret_ref.as_ref().is_some_and(|reference| {
        !matches!(
            reference.service.as_str(),
            MODEL_CREDENTIAL_SERVICE | contexthub_protocol::LEGACY_MODEL_CREDENTIAL_SERVICE
        )
    }) {
        return Err("model_credential_scope_invalid".into());
    }
    validate_scoped_reference(
        profile_id,
        &connection.profile_id,
        &connection.connection_id,
        connection.auth_generation,
        connection.secret_ref.as_ref(),
    )
}

fn validate_scoped_reference(
    profile_id: &str,
    owner_profile_id: &str,
    connection_id: &str,
    generation: u32,
    reference: Option<&ModelSecretRef>,
) -> Result<(), String> {
    if !valid_id(profile_id)
        || owner_profile_id != profile_id
        || !valid_id(connection_id)
        || generation == 0
    {
        return Err("model_credential_scope_invalid".to_owned());
    }
    if let Some(reference) = reference {
        let expected = format!("{profile_id}:{connection_id}:{generation}");
        if !matches!(
            reference.service.as_str(),
            MODEL_CREDENTIAL_SERVICE
                | contexthub_protocol::MODEL_NETWORK_CREDENTIAL_SERVICE
                | contexthub_protocol::LEGACY_MODEL_CREDENTIAL_SERVICE
                | contexthub_protocol::LEGACY_MODEL_NETWORK_CREDENTIAL_SERVICE
        ) || reference.account != expected
        {
            return Err("model_credential_scope_invalid".to_owned());
        }
    }
    Ok(())
}

fn clear_credential(
    reference: &ModelSecretRef,
    store: &impl CredentialStore,
) -> Result<(), String> {
    if contexthub_protocol::is_legacy_model_credential_reference(reference) {
        return Err("model_credential_legacy_cleanup_required".into());
    }
    store.delete(reference)?;
    if let Some(mut remaining) = store.read(reference)? {
        remaining.fill(0);
        return Err("model_credential_delete_unconfirmed".to_owned());
    }
    Ok(())
}

fn cleanup_historical_credential(
    profile_id: &str,
    request: &Value,
    store: &impl CredentialStore,
    rpc: &impl Fn(&str, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let params: ModelCredentialCleanupPrepareParams = decode(request.clone())?;
    let plan: ModelCredentialCleanupPlan = decode(rpc("modelCredentialCleanup/prepare", request)?)?;
    validate_scoped_reference(
        profile_id,
        &plan.profile_id,
        &plan.connection_id,
        plan.auth_generation,
        Some(&plan.credential_ref),
    )?;
    if plan.cleanup_id != params.cleanup_id
        || plan.reference_digest != params.expected_reference_digest
    {
        return Err("model_credential_cleanup_stale".to_owned());
    }
    if plan.state == ModelCredentialCleanupPlanState::Confirmed {
        return serde_json::to_value(plan)
            .map_err(|_| "model_connection_response_invalid".to_owned());
    }
    clear_credential(&plan.credential_ref, store)?;
    rpc(
        "modelCredentialCleanup/ack",
        &serde_json::json!({
            "cleanupId":plan.cleanup_id,"expectedReferenceDigest":plan.reference_digest,"credentialRef":plan.credential_ref,
        }),
    )
}

fn validate_prepared_credentials(
    profile_id: &str,
    change: &ModelConnectionChange,
) -> Result<(), String> {
    validate_reference(profile_id, &change.connection)?;
    if change
        .connection
        .secret_ref
        .as_ref()
        .is_some_and(contexthub_protocol::is_legacy_model_credential_reference)
        || change
            .connection
            .config
            .network
            .proxy_secret_ref
            .as_ref()
            .is_some_and(contexthub_protocol::is_legacy_model_credential_reference)
    {
        return Err("model_credential_reentry_required".into());
    }
    match change.state {
        ModelConnectionChangeState::Aborted => {
            return Err("model_connection_change_aborted".to_owned());
        }
        ModelConnectionChangeState::Committed => {
            return Ok(());
        }
        ModelConnectionChangeState::Prepared => {}
    }
    if let Some(pem) = &change.connection.config.network.ca_certificate_pem {
        contexthub_workbench_runtime::model_connections::validate_model_ca_certificate(pem)?;
    }
    Ok(())
}

fn commit_change(
    profile_id: &str,
    change: &ModelConnectionChange,
    secret: Option<&str>,
    store: &impl CredentialStore,
    rpc: &impl Fn(&str, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    validate_prepared_credentials(profile_id, change)?;
    if change.state == ModelConnectionChangeState::Committed {
        return serde_json::to_value(&change.connection)
            .map_err(|_| "model_connection_response_invalid".into());
    }
    let credential_ref = match change.credential_action {
        ModelCredentialAction::Replace => {
            let reference = change
                .connection
                .secret_ref
                .as_ref()
                .ok_or("model_credential_scope_invalid")?;
            let secret = secret
                .filter(|value| {
                    !value.trim().is_empty()
                        && value.len() <= 16_384
                        && !value.contains(['\r', '\n', '\0'])
                })
                .ok_or("model_credential_required")?;
            if let Some(mut existing) = store.read(reference)? {
                let matches = existing == secret.as_bytes();
                existing.fill(0);
                if !matches {
                    return Err("model_credential_pending_value_conflict".to_owned());
                }
            } else {
                store.write(reference, secret.as_bytes())?;
            }
            let mut saved = store
                .read(reference)?
                .ok_or("model_credential_readback_failed")?;
            let matches = saved == secret.as_bytes();
            saved.fill(0);
            if !matches {
                return Err("model_credential_readback_failed".to_owned());
            }
            Some(reference.clone())
        }
        ModelCredentialAction::Keep | ModelCredentialAction::Clear => {
            if secret.is_some() {
                return Err("model_credential_action_invalid".to_owned());
            }
            change.connection.secret_ref.clone()
        }
    };
    rpc(
        "modelConnection/saveCommit",
        &serde_json::to_value(ModelConnectionCommitParams {
            operation_id: change.operation_id.clone(),
            credential_ref,
            network_credential_ref: change.connection.config.network.proxy_secret_ref.clone(),
        })
        .map_err(|_| "model_connection_request_invalid")?,
    )
}

fn cancel_change(
    profile_id: &str,
    change: &ModelConnectionChange,
    cleanup_only: bool,
    store: &impl CredentialStore,
    rpc: &impl Fn(&str, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    validate_reference(profile_id, &change.connection)?;
    let aborted = match change.state {
        ModelConnectionChangeState::Prepared if !cleanup_only => {
            decode::<ModelConnectionChange>(rpc(
                "modelConnection/saveAbort",
                &serde_json::json!({"operationId":change.operation_id}),
            )?)?
        }
        ModelConnectionChangeState::Aborted => change.clone(),
        _ => return Err("model_connection_change_not_prepared".to_owned()),
    };
    validate_reference(profile_id, &aborted.connection)?;
    if aborted.state != ModelConnectionChangeState::Aborted
        || aborted.operation_id != change.operation_id
    {
        return Err("model_connection_response_invalid".to_owned());
    }
    if aborted.credential_action == ModelCredentialAction::Replace
        && matches!(
            aborted.credential_cleanup,
            ModelCredentialCleanupState::Pending | ModelCredentialCleanupState::Unconfirmed
        )
    {
        let reference = aborted
            .connection
            .secret_ref
            .as_ref()
            .ok_or("model_credential_scope_invalid")?;
        clear_credential(reference, store)?;
        return rpc(
            "modelConnection/credentialCleanupAck",
            &serde_json::json!({
                "operationId":aborted.operation_id,"credentialRef":reference,
            }),
        );
    }
    serde_json::to_value(aborted).map_err(|_| "model_connection_response_invalid".to_owned())
}

fn finish_prepared_change(
    profile_id: &str,
    change: &ModelConnectionChange,
    secret: Option<&str>,
    network_secret: Option<&contexthub_protocol::ModelNetworkCredentials>,
    store: &impl CredentialStore,
    rpc: &impl Fn(&str, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let result = (|| {
        validate_prepared_credentials(profile_id, change)?;
        network_credentials::write(profile_id, change, network_secret, store)?;
        commit_change(profile_id, change, secret, store, rpc)
    })();
    result.map_err(|reason| format!("{reason};operationId={}", change.operation_id))
}

fn credential_operation_guard(
    command: &str,
    profile_id: &str,
    data_dir: &Path,
) -> Result<Option<contexthub_platform_adapter::security::RestrictedFileLock>, String> {
    if matches!(
        command,
        "model_connection_save"
            | "model_connection_resume"
            | "model_connection_abort"
            | "model_connection_credential_cleanup"
            | "model_credential_cleanup_execute"
    ) {
        if !valid_id(profile_id) {
            return Err("model_credential_scope_invalid".to_owned());
        }
        Ok(Some(
            contexthub_platform_adapter::security::try_lock_restricted_file(
                &data_dir.join("model-credential-operation.lock"),
            )
            .map_err(|_| "model_credential_operation_lock_failed")?
            .ok_or("model_credential_operation_in_use")?,
        ))
    } else {
        Ok(None)
    }
}

pub fn handle(
    command: &str,
    args: &Value,
    profile_id: &str,
    data_dir: &Path,
    rpc: impl Fn(&str, &Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let _credential_guard = credential_operation_guard(command, profile_id, data_dir)?;
    let store = LocalCredentials(data_dir.join("credentials"));
    let request = args.get("request").cloned().unwrap_or(Value::Null);
    match command {
        "model_credential_cleanup_execute" => {
            cleanup_historical_credential(profile_id, &request, &store, &rpc)
        }
        "model_connection_save" => {
            let request: SaveRequest =
                serde_json::from_value(request).map_err(|_| "model_connection_request_invalid")?;
            if let Some(pem) = &request.prepare.config.network.ca_certificate_pem {
                contexthub_workbench_runtime::model_connections::validate_model_ca_certificate(
                    pem,
                )?;
            }
            let change: ModelConnectionChange = decode(rpc(
                "modelConnection/savePrepare",
                &serde_json::to_value(request.prepare)
                    .map_err(|_| "model_connection_request_invalid")?,
            )?)?;
            finish_prepared_change(
                profile_id,
                &change,
                request.secret.as_deref(),
                request.network_secret.as_ref(),
                &store,
                &rpc,
            )
        }
        "model_connection_resume" => {
            let request: ResumeRequest =
                serde_json::from_value(request).map_err(|_| "model_connection_request_invalid")?;
            let change: ModelConnectionChange = decode(rpc(
                "modelConnection/changeStatus",
                &serde_json::json!({"operationId": request.operation_id}),
            )?)?;
            finish_prepared_change(
                profile_id,
                &change,
                request.secret.as_deref(),
                request.network_secret.as_ref(),
                &store,
                &rpc,
            )
        }
        "model_connection_abort" | "model_connection_credential_cleanup" => {
            let change: ModelConnectionChange =
                decode(rpc("modelConnection/changeStatus", &request)?)?;
            let aborted = cancel_change(
                profile_id,
                &change,
                command == "model_connection_credential_cleanup",
                &store,
                &rpc,
            )?;
            network_credentials::cleanup_aborted(
                profile_id,
                &decode(aborted.clone())?,
                &store,
                &rpc,
            )?;
            Ok(aborted)
        }
        _ => {
            let method = match command {
                "model_connection_list" => "modelConnection/list",
                "model_credential_cleanup_list" => "modelCredentialCleanup/list",
                "model_connection_change_list" => "modelConnection/changeList",
                "model_connection_auth_changes" => "modelAuth/list",
                "model_connection_recover_leases" => "modelExecution/recover",
                "model_connection_get" => "modelConnection/get",
                "model_connection_change_status" => "modelConnection/changeStatus",
                "model_connection_preflight" => "modelConnection/preflight",
                "model_connection_set_enabled" => "modelConnection/setEnabled",
                "model_connection_delete" => "modelConnection/deleteCommit",
                "model_descriptor_list" => "modelCatalog/read",
                "model_descriptor_save" => "modelDescriptor/save",
                "model_purpose_list" => "modelPurpose/list",
                "model_purpose_save" => "modelPurpose/save",
                "model_validation_list" => "modelValidation/list",
                _ => return Err("model_connection_command_not_allowed".to_owned()),
            };
            rpc(method, &request)
        }
    }
}

pub fn finish_metadata_change(
    command: &str,
    result: Value,
    refresh: impl FnOnce() -> Result<(), String>,
) -> Value {
    if matches!(
        command,
        "model_connection_save"
            | "model_connection_resume"
            | "model_connection_set_enabled"
            | "model_connection_delete"
            | "model_descriptor_save"
            | "model_purpose_save"
    ) {
        let error = refresh()
            .err()
            .map(|_| "model_metadata_saved_catalog_refresh_failed");
        serde_json::json!({"result":result,"catalogRefreshError":error})
    } else {
        result
    }
}
