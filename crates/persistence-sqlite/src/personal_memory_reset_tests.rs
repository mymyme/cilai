use super::*;
use contexthub_protocol::{
    PersonalMemoryResetPermit, PersonalMemoryResetScope, PersonalMemoryResetScopeResult,
};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
};
const PROFILE: &str = "11111111111111111111111111111111";
const INSTANCE: &str = "memory-test-instance";
struct Fixture {
    root: PathBuf,
    db: SqlitePersistence,
    owner: ModelExecutionOwner,
    record: PersonalMemoryResetRecord,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "cilai-memory-reset-sqlite-{}",
            uuid::Uuid::new_v4().simple()
        ));
        contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        db.begin_personal_profile_upgrade(PROFILE, INSTANCE)
            .unwrap();
        db.finish_personal_profile_upgrade(PROFILE, INSTANCE, "manifest")
            .unwrap();
        let owner = ModelExecutionOwner {
            instance_id: "test-owner".into(),
            pid: std::process::id(),
            start_fingerprint: contexthub_platform_adapter::instance::process_start_fingerprint(
                std::process::id(),
            )
            .unwrap()
            .unwrap(),
        };
        let scope = "memory_scope_1111111111111111111111111111111111111111111111111111111111111111";
        let path = "ai-runtime/cilai-harness/official-runtime";
        let record = PersonalMemoryResetRecord {
            operation: PersonalMemoryResetOperation {
                operation_id: format!("memory_reset_{}", uuid::Uuid::new_v4().simple()),
                profile_id: PROFILE.into(),
                state: PersonalMemoryResetState::Prepared,
                preview_digest: "sha256:test-preview".into(),
                created_at: 1,
                updated_at: 1,
                scopes: vec![PersonalMemoryResetScope {
                    scope_id: scope.into(),
                    native_home_relative_path: path.into(),
                    scope_kind: "default_workbench".into(),
                    visible_memory_file_count: Some(1),
                    visible_memory_logical_bytes: Some(8),
                    inspection_issue: None,
                    retained_native_paths: Some(vec![]),
                }],
                scope_results: vec![PersonalMemoryResetScopeResult {
                    scope_id: scope.into(),
                    outcome: PersonalMemoryResetOutcome::Pending,
                    writer_lease_id: None,
                    official_response_received: false,
                    process_exit_confirmed: false,
                    reason: None,
                }],
                blockers: vec![],
            },
            scopes: vec![PersonalMemoryResetScopeAuthorization {
                scope_id: scope.into(),
                native_home_relative_path: path.into(),
                native_home_fingerprint: "simulated-directory-identity-for-sql-contract".into(),
                permit_token: None,
                writer_lease_id: None,
                writer_client_operation_id: None,
            }],
            owner: None,
            fence_id: None,
            client_operation_id: "prepare-client".into(),
            request_fingerprint: "prepare-fingerprint".into(),
        };
        db.prepare_personal_memory_reset(&record).unwrap();
        Self {
            root,
            db,
            owner,
            record,
        }
    }
    fn start(&mut self) -> PersonalMemoryResetPermit {
        let fence=self.db.personal_writer_rpc("personalMaintenance/acquire",json!({"operationId":self.record.operation.operation_id,"previewDigest":self.record.operation.preview_digest,"owner":self.owner}),PROFILE,INSTANCE,2).unwrap();
        self.record.owner = Some(self.owner.clone());
        self.record.fence_id = Some(fence["fenceId"].as_str().unwrap().into());
        self.record.operation.state = PersonalMemoryResetState::Applying;
        self.record.scopes[0].permit_token = Some("reset_permit_test".into());
        self.db
            .transition_personal_memory_reset(&self.record, &PersonalMemoryResetState::Prepared)
            .unwrap();
        PersonalMemoryResetPermit {
            operation_id: self.record.operation.operation_id.clone(),
            preview_digest: self.record.operation.preview_digest.clone(),
            scope_id: self.record.scopes[0].scope_id.clone(),
            native_home_relative_path: self.record.scopes[0].native_home_relative_path.clone(),
            native_home_fingerprint: self.record.scopes[0].native_home_fingerprint.clone(),
            permit_token: "reset_permit_test".into(),
            owner: self.owner.clone(),
        }
    }
    fn writer(
        &mut self,
        permit: PersonalMemoryResetPermit,
        client: &str,
    ) -> Result<PersonalWriterLease, PersistenceError> {
        self.db.prepare_personal_memory_reset_writer(
            PROFILE,
            INSTANCE,
            &PersonalMemoryResetWriterParams {
                permit,
                client_operation_id: client.into(),
            },
            3,
        )
    }
    fn complete(
        &mut self,
        id: Option<String>,
        response: Option<serde_json::Value>,
    ) -> PersonalMemoryResetRecord {
        self.db
            .complete_personal_memory_reset_scope(
                PROFILE,
                INSTANCE,
                &PersonalMemoryResetScopeCompleteParams {
                    operation_id: self.record.operation.operation_id.clone(),
                    preview_digest: self.record.operation.preview_digest.clone(),
                    scope_id: self.record.scopes[0].scope_id.clone(),
                    writer_lease_id: id,
                    owner: self.owner.clone(),
                    official_response: response,
                    failure_reason: None,
                },
                4,
            )
            .unwrap()
    }
    fn finish(&mut self) -> Result<PersonalMemoryResetRecord, PersistenceError> {
        self.db.finish_personal_memory_reset(
            PROFILE,
            INSTANCE,
            &self.record.operation.operation_id,
            &self.record.operation.preview_digest,
            &self.owner,
            5,
        )
    }
    fn register(&mut self, lease: &PersonalWriterLease, child: &Child) {
        let start = contexthub_platform_adapter::instance::process_start_fingerprint(child.id())
            .unwrap()
            .unwrap();
        self.db.personal_writer_rpc("profileWriter/registerProcess",json!({"leaseId":lease.lease_id,"process":{"pid":child.id(),"startFingerprint":start}}),PROFILE,INSTANCE,4).unwrap();
    }
    fn release(&mut self, lease: &PersonalWriterLease) {
        self.db
            .personal_writer_rpc(
                "profileWriter/release",
                json!({"leaseId":lease.lease_id}),
                PROFILE,
                INSTANCE,
                4,
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
struct OwnerChild {
    process: Child,
    _stdout: BufReader<std::process::ChildStdout>,
}
impl OwnerChild {
    fn new() -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "personal_memory_reset::tests::memory_reset_owner_child",
                "--ignored",
                "--nocapture",
            ])
            .env("CILAI_MEMORY_RESET_OWNER_CHILD", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert!(output.read_line(&mut line).unwrap() > 0);
            if line.contains("memory-reset-owner-ready") {
                break;
            }
        }
        Self {
            process: child,
            _stdout: output,
        }
    }
    fn exit(&mut self) {
        drop(self.process.stdin.take());
        assert!(self.process.wait().unwrap().success());
    }
}
impl Drop for OwnerChild {
    fn drop(&mut self) {
        if self.process.try_wait().unwrap().is_none() {
            self.process.kill().unwrap();
            self.process.wait().unwrap();
        }
    }
}
#[test]
fn generic_writer_cannot_claim_the_memory_reset_kind() {
    let mut f = Fixture::new();
    let error=f.db.personal_writer_rpc("profileWriter/prepare",json!({"clientOperationId":"unapproved","kind":"native_memory_reset","resourceId":"scope","resourceRevision":"preview","owner":f.owner,"launchProtocol":1,"processMode":"local_process"}),PROFILE,INSTANCE,2).unwrap_err();
    assert_eq!(error.reason(), Some("personal_writer_request_invalid"));
}
#[test]
fn permit_is_scope_bound_and_consumed_once() {
    let mut f = Fixture::new();
    let permit = f.start();
    let mut wrong = permit.clone();
    wrong.native_home_relative_path = "ai-runtime/another/official-runtime".into();
    assert_eq!(
        f.writer(wrong, "wrong").unwrap_err().reason(),
        Some("personal_memory_reset_permit_invalid")
    );
    let lease = f.writer(permit.clone(), "first").unwrap();
    assert_eq!(f.writer(permit.clone(), "first").unwrap(), lease);
    assert_eq!(
        f.writer(permit, "second").unwrap_err().reason(),
        Some("personal_memory_reset_permit_consumed")
    );
}
#[test]
fn an_empty_reported_response_without_a_registered_process_cannot_mean_reset() {
    let mut f = Fixture::new();
    let permit = f.start();
    let lease = f.writer(permit, "only-prepared").unwrap();
    let result = f.complete(Some(lease.lease_id), Some(json!({})));
    assert_eq!(
        result.operation.scope_results[0].outcome,
        PersonalMemoryResetOutcome::Unconfirmed
    );
    assert!(!result.operation.scope_results[0].process_exit_confirmed);
    assert_eq!(
        f.finish().unwrap_err().reason(),
        Some("personal_memory_reset_exit_unconfirmed")
    );
}
#[test]
fn response_and_real_registered_exit_are_both_required_to_complete() {
    let mut f = Fixture::new();
    let permit = f.start();
    let lease = f.writer(permit, "actual-child").unwrap();
    let mut child = OwnerChild::new();
    f.register(&lease, &child.process);
    let active = f.complete(Some(lease.lease_id.clone()), Some(json!({})));
    assert_eq!(
        active.operation.scope_results[0].outcome,
        PersonalMemoryResetOutcome::Unconfirmed
    );
    assert_eq!(
        f.finish().unwrap_err().reason(),
        Some("personal_memory_reset_exit_unconfirmed")
    );
    child.exit();
    f.release(&lease);
    let result = f.complete(Some(lease.lease_id), Some(json!({})));
    assert_eq!(
        result.operation.scope_results[0].outcome,
        PersonalMemoryResetOutcome::Reset
    );
    assert!(result.operation.scope_results[0].process_exit_confirmed);
    assert_eq!(
        f.finish().unwrap().operation.state,
        PersonalMemoryResetState::Completed
    );
}
#[test]
fn nonempty_or_missing_official_response_remains_unconfirmed_after_exit() {
    let mut f = Fixture::new();
    let permit = f.start();
    let lease = f.writer(permit, "actual-child").unwrap();
    let mut child = OwnerChild::new();
    f.register(&lease, &child.process);
    child.exit();
    f.release(&lease);
    let result = f.complete(Some(lease.lease_id), Some(json!({"unexpected":true})));
    assert_eq!(
        result.operation.scope_results[0].outcome,
        PersonalMemoryResetOutcome::Unconfirmed
    );
    assert!(result.operation.scope_results[0].process_exit_confirmed);
    assert_eq!(
        f.finish().unwrap().operation.state,
        PersonalMemoryResetState::Incomplete
    );
}
#[test]
fn never_consumed_permit_records_failed_without_inventing_a_lease() {
    let mut f = Fixture::new();
    let _permit = f.start();
    let result = f.complete(None, None);
    assert_eq!(
        result.operation.scope_results[0].outcome,
        PersonalMemoryResetOutcome::Failed
    );
    assert!(result.operation.scope_results[0].writer_lease_id.is_none());
    assert_eq!(
        f.finish().unwrap().operation.state,
        PersonalMemoryResetState::Incomplete
    );
}
#[test]
fn start_changes_only_explicit_memory_authorization_and_preserves_business_marker() {
    let mut f = Fixture::new();
    f.db.connection.execute("INSERT INTO app_metadata(key,value,updated_at) VALUES('memory-reset-business-marker','preserved',1)",[]).unwrap();
    let value = json!({"entryId":"memoryPreferences","field":"memoryPreferences","shape":"scalar","value":{"enabled":true,"fromToolChats":true}});
    f.db.connection.execute("INSERT INTO ai_runtime_entries(record_id,position,record_json,digest,updated_at) VALUES('memoryPreferences',0,?1,'test',1)",[value.to_string()]).unwrap();
    assert!(
        f.db.personal_memory_authorization()
            .unwrap()
            .preferences
            .enabled
    );
    let _permit = f.start();
    assert!(
        !f.db
            .personal_memory_authorization()
            .unwrap()
            .preferences
            .enabled
    );
    assert_eq!(
        f.db.connection
            .query_row(
                "SELECT value FROM app_metadata WHERE key='memory-reset-business-marker'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "preserved"
    );
}
#[test]
#[ignore = "由父测试真实启动、登记与等待退出"]
fn memory_reset_owner_child() {
    if std::env::var("CILAI_MEMORY_RESET_OWNER_CHILD").as_deref() != Ok("1") {
        return;
    }
    println!("memory-reset-owner-ready");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
}
