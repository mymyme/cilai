use super::*;
use contexthub_protocol::{
    ModelExecutionProcess, PersonalNamespaceActionReceipt, PersonalNamespaceStorageKind,
};
use serde_json::json;
use sha2::{Digest as _, Sha256};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
};
const PROFILE: &str = "11111111111111111111111111111111";
const INSTANCE: &str = "namespace-test-instance";
fn owner(pid: u32) -> ModelExecutionOwner {
    ModelExecutionOwner {
        instance_id: format!("test-shell-{pid}"),
        pid,
        start_fingerprint: contexthub_platform_adapter::instance::process_start_fingerprint(pid)
            .unwrap()
            .unwrap(),
    }
}
fn child() -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "personal_namespace_storage::tests::namespace_owner_child",
            "--ignored",
        ])
        .env("CILAI_NAMESPACE_CHILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap()
}
struct Fixture {
    root: PathBuf,
    db: SqlitePersistence,
    record: PersonalNamespaceStorageRecord,
    proof: Option<PersonalDataQuiescence>,
}
impl Fixture {
    fn new(shell: Option<ModelExecutionOwner>) -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "cilai-namespace-ledger-{}",
            uuid::Uuid::new_v4().simple()
        ));
        contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        db.begin_personal_profile_upgrade(PROFILE, INSTANCE)
            .unwrap();
        db.finish_personal_profile_upgrade(PROFILE, INSTANCE, "test-manifest")
            .unwrap();
        let record = PersonalNamespaceStorageRecord {
            operation: PersonalNamespaceStorageOperation {
                operation_id: format!("namespace_storage_{}", uuid::Uuid::new_v4().simple()),
                profile_id: PROFILE.into(),
                namespace_path: "contract-only-namespace".into(),
                namespace_identity: "contract-only-identity".into(),
                shell_owner: shell.unwrap_or_else(|| owner(std::process::id())),
                state: PersonalStorageOperationState::Prepared,
                preview_digest: "sha256:test-preview".into(),
                created_at: 1,
                updated_at: 1,
                log_segment_id: Some("22222222222222222222222222222222".into()),
                log_files: vec![],
                browser_session: None,
                actions: vec![PersonalNamespaceActionReceipt {
                    action_id: "action-contract".into(),
                    kind: PersonalNamespaceStorageKind::HostServiceLogs,
                    state: PersonalNamespaceActionState::Planned,
                    outcome: None,
                    reason_code: None,
                    started_at: None,
                    acknowledged_at: None,
                    after_cache_bytes: None,
                }],
                blockers: vec![],
            },
            authorizations: vec![PersonalNamespaceActionAuthorization {
                action_id: "action-contract".into(),
                permit_token: "contract-only-permit".into(),
                writer_lease_id: None,
                acknowledgement: None,
            }],
            quiescence: None,
            client_operation_id: "test-client".into(),
            request_fingerprint: "test-fingerprint".into(),
        };
        db.prepare_personal_namespace_storage(&record, INSTANCE)
            .unwrap();
        Self {
            root,
            db,
            record,
            proof: None,
        }
    }
    fn rpc(&mut self, method: &str, p: Value) -> Result<Value, PersistenceError> {
        self.db.personal_namespace_storage_rpc(
            &format!("personalNamespaceStorage/{method}"),
            p,
            PROFILE,
            INSTANCE,
            5,
        )
    }
    fn confirm(&self) -> Value {
        json!({"operationId":self.record.operation.operation_id,"previewDigest":self.record.operation.preview_digest})
    }
    fn start(&mut self) -> PersonalNamespaceActionParams {
        self.start_with_host(owner(std::process::id()))
    }
    fn start_with_host(&mut self, host: ModelExecutionOwner) -> PersonalNamespaceActionParams {
        let fence=self.db.personal_writer_rpc("personalMaintenance/acquire",json!({"operationId":self.record.operation.operation_id,"previewDigest":self.record.operation.preview_digest,"owner":host}),PROFILE,INSTANCE,2).unwrap();
        let proof = PersonalDataQuiescence {
            protocol_version: 1,
            profile_id: PROFILE.into(),
            owner: host,
            token: fence["fenceId"].as_str().unwrap().into(),
            writer_registry_digest: format!("sha256:{:x}", Sha256::digest(b"[]")),
            covered_writers: [
                "foreground",
                "automation",
                "review",
                "memory",
                "page_ai",
                "authentication",
                "personal_package_mcp",
                "user_mcp",
                "attachments",
                "local_operations",
                "ledger",
                "native_history",
                "runtime_initialization",
            ]
            .map(String::from)
            .to_vec(),
            native_homes: vec![],
        };
        let result = self
            .db
            .start_personal_namespace_storage(
                &PersonalNamespaceStorageStartParams {
                    operation_id: self.record.operation.operation_id.clone(),
                    preview_digest: self.record.operation.preview_digest.clone(),
                    quiescence: proof.clone(),
                },
                PROFILE,
                INSTANCE,
                3,
            )
            .unwrap();
        self.proof = Some(proof);
        let permit = &result.permits[0];
        PersonalNamespaceActionParams {
            operation_id: result.operation.operation_id,
            action_id: permit.action_id.clone(),
            permit_token: permit.permit_token.clone(),
            shell_owner: permit.shell_owner.clone(),
            execution_processes: vec![ModelExecutionProcess {
                pid: permit.shell_owner.pid,
                start_fingerprint: permit.shell_owner.start_fingerprint.clone(),
            }],
        }
    }
    fn acknowledgement(p: &PersonalNamespaceActionParams) -> Value {
        json!({"operationId":p.operation_id,"actionId":p.action_id,"permitToken":p.permit_token,"shellOwner":p.shell_owner,"outcome":"succeeded","reasonCode":null,"afterCacheBytes":null})
    }
    fn active(&mut self) -> Value {
        self.db
            .personal_writer_rpc("profileWriter/list", json!({}), PROFILE, INSTANCE, 5)
            .unwrap()
    }
    fn release_fence(&mut self) {
        let p = self.proof.as_ref().unwrap();
        self.db
            .personal_writer_rpc(
                "personalMaintenance/release",
                json!({"fenceId":p.token,"owner":p.owner}),
                PROFILE,
                INSTANCE,
                7,
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn only_durable_started_then_acknowledged_action_can_complete_and_permit_is_one_use() {
    let mut f = Fixture::new(None);
    let p = f.start();
    assert_eq!(
        f.rpc("beginAction", value(&p).unwrap()).unwrap()["state"],
        "started"
    );
    assert_eq!(
        f.rpc("beginAction", value(&p).unwrap())
            .unwrap_err()
            .reason(),
        Some("personal_namespace_storage_action_already_started")
    );
    let before = f.rpc("get", json!({"operationId":p.operation_id})).unwrap();
    assert!(!before.to_string().contains("permitToken"));
    assert!(!before.to_string().contains("contract-only-permit"));
    let ack = Fixture::acknowledgement(&p);
    let result = f.rpc("ackAction", ack.clone()).unwrap();
    assert_eq!(f.rpc("ackAction", ack).unwrap(), result);
    let mut changed = Fixture::acknowledgement(&p);
    changed["outcome"] = json!("failed");
    changed["reasonCode"] = json!("cache_failed");
    assert_eq!(
        f.rpc("ackAction", changed).unwrap_err().reason(),
        Some("personal_namespace_storage_ack_changed")
    );
    assert_eq!(f.rpc("finish", f.confirm()).unwrap()["state"], "completed");
    assert!(f.active().as_array().unwrap().is_empty());
    f.release_fence();
}
#[test]
fn missing_ack_keeps_writer_and_fence_and_generic_release_cannot_bypass_typed_receipt() {
    let mut f = Fixture::new(None);
    let p = f.start();
    f.rpc("beginAction", value(&p).unwrap()).unwrap();
    let lease = f.active()[0]["leaseId"].clone();
    for method in ["profileWriter/release", "profileWriter/recover"] {
        assert_eq!(
            f.db.personal_writer_rpc(
                method,
                json!({"leaseId":lease,"owner":p.shell_owner,"localTransportFinished":true}),
                PROFILE,
                INSTANCE,
                5
            )
            .unwrap_err()
            .reason(),
            Some("personal_namespace_storage_typed_ack_required")
        );
    }
    assert_eq!(f.rpc("finish", f.confirm()).unwrap()["state"], "incomplete");
    assert_eq!(f.active().as_array().unwrap().len(), 1);
    assert_eq!(
        f.rpc("resolveInterrupted", f.confirm())
            .unwrap_err()
            .reason(),
        Some("personal_namespace_storage_owner_alive")
    );
    let proof = f.proof.as_ref().unwrap();
    assert!(
        f.db.personal_writer_rpc(
            "personalMaintenance/release",
            json!({"fenceId":proof.token,"owner":proof.owner}),
            PROFILE,
            INSTANCE,
            5
        )
        .is_err()
    );
    f.rpc("ackAction", Fixture::acknowledgement(&p)).unwrap();
    assert_eq!(f.rpc("finish", f.confirm()).unwrap()["state"], "completed");
    f.release_fence();
}
#[test]
fn dead_executor_with_a_live_registered_child_remains_protected_then_records_unknown_without_reexecution()
 {
    let mut shell = child();
    let mut utility = child();
    let mut f = Fixture::new(Some(owner(shell.id())));
    let mut p = f.start();
    p.execution_processes.push(ModelExecutionProcess {
        pid: utility.id(),
        start_fingerprint: owner(utility.id()).start_fingerprint,
    });
    f.rpc("beginAction", value(&p).unwrap()).unwrap();
    drop(shell.stdin.take());
    assert!(shell.wait().unwrap().success());
    assert_eq!(
        f.rpc("resolveInterrupted", f.confirm())
            .unwrap_err()
            .reason(),
        Some("personal_namespace_storage_process_alive")
    );
    assert_eq!(f.active().as_array().unwrap().len(), 1);
    drop(utility.stdin.take());
    assert!(utility.wait().unwrap().success());
    let op = f.rpc("resolveInterrupted", f.confirm()).unwrap();
    assert_eq!(op["state"], "incomplete");
    assert_eq!(op["actions"][0]["state"], "unconfirmed");
    assert!(op["actions"][0]["outcome"].is_null());
    assert!(f.active().as_array().unwrap().is_empty());
    f.release_fence();
}
#[test]
fn invalid_owner_permit_and_missing_executor_registration_never_admit_an_action() {
    let mut f = Fixture::new(None);
    let p = f.start();
    let mut bad = p.clone();
    bad.permit_token = "wrong".into();
    assert!(f.rpc("beginAction", value(&bad).unwrap()).is_err());
    bad = p.clone();
    bad.shell_owner.start_fingerprint = "wrong".into();
    assert!(f.rpc("beginAction", value(&bad).unwrap()).is_err());
    bad = p;
    bad.execution_processes.clear();
    assert_eq!(
        f.rpc("beginAction", value(bad).unwrap())
            .unwrap_err()
            .reason(),
        Some("personal_namespace_storage_processes_unconfirmed")
    );
    assert!(f.active().as_array().unwrap().is_empty());
    assert_eq!(f.rpc("cancel", f.confirm()).unwrap()["state"], "cancelled");
    f.release_fence();
}
#[test]
fn failed_ack_is_terminal_transport_evidence_but_never_successful_cleanup() {
    let mut f = Fixture::new(None);
    let p = f.start();
    f.rpc("beginAction", value(&p).unwrap()).unwrap();
    let mut ack = Fixture::acknowledgement(&p);
    ack["outcome"] = json!("failed");
    ack["reasonCode"] = json!("personal_namespace_storage_content_changed");
    f.rpc("ackAction", ack).unwrap();
    let op = f.rpc("finish", f.confirm()).unwrap();
    assert_eq!(op["state"], "incomplete");
    assert_eq!(op["actions"][0]["outcome"], "failed");
    assert!(f.active().as_array().unwrap().is_empty());
    f.release_fence();
}
#[test]
fn cancelled_segment_can_be_explicitly_prepared_again_but_concurrent_preview_is_rejected() {
    let mut f = Fixture::new(None);
    let mut next = f.record.clone();
    next.operation.operation_id = format!("namespace_storage_{}", uuid::Uuid::new_v4().simple());
    next.client_operation_id = "second".into();
    assert!(
        f.db.prepare_personal_namespace_storage(&next, INSTANCE)
            .is_err()
    );
    f.rpc("cancel", f.confirm()).unwrap();
    assert!(
        f.db.prepare_personal_namespace_storage(&next, INSTANCE)
            .is_ok()
    );
}
#[test]
fn browser_missing_ack_is_not_recovered_from_a_static_dead_pid_list() {
    let mut shell = child();
    let mut f = Fixture::new(Some(owner(shell.id())));
    let mut stored = read(&f.db.connection, PROFILE, &f.record.operation.operation_id).unwrap();
    stored.operation.actions[0].kind = PersonalNamespaceStorageKind::BrowserHttpCache;
    save(&f.db.connection, &stored).unwrap();
    let p = f.start();
    f.rpc("beginAction", value(&p).unwrap()).unwrap();
    drop(shell.stdin.take());
    assert!(shell.wait().unwrap().success());
    assert_eq!(
        f.rpc("resolveInterrupted", f.confirm())
            .unwrap_err()
            .reason(),
        Some("personal_namespace_storage_browser_transport_unconfirmed")
    );
    assert_eq!(f.active().as_array().unwrap().len(), 1);
    let op = f.rpc("finish", f.confirm()).unwrap();
    assert_eq!(op["state"], "incomplete");
    assert_eq!(op["actions"][0]["state"], "started");
    assert!(op["actions"][0]["outcome"].is_null());
}
#[test]
fn schema76_backup_removes_action_authority_and_restore_keeps_source_receipt_intact() {
    let f = Fixture::new(None);
    let candidate_dir = f
        .root
        .join("cn.cilai.personal.workbench/business/profiles")
        .join(format!(
            "restore-candidate-{}",
            uuid::Uuid::new_v4().simple()
        ));
    contexthub_platform_adapter::security::ensure_restricted_directory(&candidate_dir).unwrap();
    let path = candidate_dir.join("contexthub.db");
    assert_eq!(f.db.snapshot_personal_data_database(&path).unwrap(), 77);
    let mut candidate = SqlitePersistence::open(&path, "test").unwrap();
    assert!(
        candidate
            .personal_namespace_storage_operations(PROFILE)
            .unwrap()
            .is_empty()
    );
    assert_eq!(candidate.connection.query_row("SELECT COUNT(*) FROM personal_maintenance_operation_registry WHERE operation_id=?1", [&f.record.operation.operation_id], |row| row.get::<_,i64>(0)).unwrap(), 0);
    candidate
        .prepare_restored_personal_profile(PROFILE, "33333333333333333333333333333333")
        .unwrap();
    assert!(
        candidate
            .personal_namespace_storage_operations("33333333333333333333333333333333")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        candidate
            .connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    let original = read(&f.db.connection, PROFILE, &f.record.operation.operation_id).unwrap();
    assert_eq!(original, f.record);
    assert_eq!(
        original.operation.state,
        PersonalStorageOperationState::Prepared
    );
}
#[test]
fn original_shell_can_ack_a_completed_action_after_host_death_and_new_server_generation_without_new_permission()
 {
    let mut host = child();
    let mut f = Fixture::new(None);
    let p = f.start_with_host(owner(host.id()));
    f.rpc("beginAction", value(&p).unwrap()).unwrap();
    drop(host.stdin.take());
    assert!(host.wait().unwrap().success());
    let restarted = "namespace-restarted-server";
    f.db.begin_personal_profile_upgrade(PROFILE, restarted)
        .unwrap();
    f.db.finish_personal_profile_upgrade(PROFILE, restarted, "same-source-profile")
        .unwrap();
    assert!(
        f.db.personal_namespace_storage_rpc(
            "personalNamespaceStorage/beginAction",
            value(&p).unwrap(),
            PROFILE,
            restarted,
            6
        )
        .is_err()
    );
    let ack = Fixture::acknowledgement(&p);
    assert_eq!(
        f.db.personal_namespace_storage_rpc(
            "personalNamespaceStorage/ackAction",
            ack.clone(),
            PROFILE,
            restarted,
            7
        )
        .unwrap()["state"],
        "acknowledged"
    );
    assert_eq!(
        f.db.personal_namespace_storage_rpc(
            "personalNamespaceStorage/ackAction",
            ack,
            PROFILE,
            restarted,
            8
        )
        .unwrap()["state"],
        "acknowledged"
    );
    assert_eq!(
        f.db.personal_namespace_storage_rpc(
            "personalNamespaceStorage/finish",
            f.confirm(),
            PROFILE,
            restarted,
            8
        )
        .unwrap()["state"],
        "completed"
    );
    let fences =
        f.db.personal_writer_rpc("personalMaintenance/list", json!({}), PROFILE, restarted, 9)
            .unwrap();
    assert_eq!(fences[0]["state"], "draining");
    assert!(
        f.db.personal_writer_rpc("profileWriter/list", json!({}), PROFILE, restarted, 9)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    let new_owner = owner(std::process::id());
    f.db.personal_writer_rpc(
        "personalMaintenance/recover",
        json!({"fenceId":f.proof.as_ref().unwrap().token,"owner":new_owner}),
        PROFILE,
        restarted,
        10,
    )
    .unwrap();
}
#[test]
#[ignore = "由真实父测试启动并检查退出"]
fn namespace_owner_child() {
    if std::env::var("CILAI_NAMESPACE_CHILD").as_deref() == Ok("1") {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
    }
}
