use super::*;
use crate::SqlitePersistence;
use serde_json::json;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

const PROFILE: &str = "11111111111141118111111111111111";
const FENCE: &str = "writer-test-instance";
struct Fixture {
    root: PathBuf,
    db: SqlitePersistence,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("cilai-personal-writer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("contexthub.db"), "writer-test").unwrap();
        db.begin_personal_profile_upgrade(PROFILE, FENCE).unwrap();
        db.finish_personal_profile_upgrade(PROFILE, FENCE, "manifest-digest")
            .unwrap();
        db.connection.execute("INSERT INTO personal_data_operations(operation_id,profile_id,operation_kind,client_operation_id,request_fingerprint,preview_digest,record_json,state,created_at,updated_at) VALUES('data-test',?1,'backup','backup-prepare','request','preview','{}','prepared',1,1)",[PROFILE]).unwrap();
        crate::maintenance_operations::register(
            &db.connection,
            &contexthub_protocol::PersonalMaintenanceOperationStatus {
                operation_id: "data-test".into(),
                profile_id: PROFILE.into(),
                kind: "backup".into(),
                state: "prepared".into(),
                preview_digest: "preview".into(),
            },
            1,
        )
        .unwrap();
        Self { root, db }
    }
    fn call(&mut self, method: &str, value: Value) -> Value {
        self.db
            .personal_writer_rpc(method, value, PROFILE, FENCE, 10)
            .unwrap()
    }
    fn reject(&mut self, method: &str, value: Value, reason: &str) {
        assert_eq!(
            self.db
                .personal_writer_rpc(method, value, PROFILE, FENCE, 10)
                .unwrap_err()
                .reason(),
            Some(reason)
        );
    }
    fn prepare(&mut self, mode: &str, operation: &str) -> Value {
        self.call("profileWriter/prepare",json!({"clientOperationId":operation,"kind":"user_mcp","resourceId":"mcp-connection","resourceRevision":"revision-1","owner":owner(std::process::id()),"launchProtocol":1,"processMode":mode}))
    }
    fn maintenance(&mut self) -> Value {
        self.call("personalMaintenance/acquire",json!({"operationId":"data-test","previewDigest":"preview","owner":owner(std::process::id())}))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn owner(pid: u32) -> Value {
    json!({"instanceId":format!("test-{pid}"),"pid":pid,"startFingerprint":fingerprint(pid).unwrap().unwrap()})
}

#[test]
fn maintenance_closes_admission_but_preserves_active_receipts_for_checked_drain() {
    let mut f = Fixture::new();
    let lease = f.prepare("remote_request", "remote-1");
    let fence = f.maintenance();
    f.reject("profileWriter/prepare",json!({"clientOperationId":"blocked-new","kind":"user_mcp","resourceId":"mcp","resourceRevision":"revision-1","owner":owner(std::process::id()),"launchProtocol":1,"processMode":"remote_request"}),"personal_data_maintenance_active");
    assert_eq!(
        f.call("profileWriter/list", json!({}))
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.reject(
        "profileWriter/release",
        json!({"leaseId":lease["leaseId"],"owner":owner(std::process::id())}),
        "personal_writer_transport_unconfirmed",
    );
    let released=f.call("profileWriter/release",json!({"leaseId":lease["leaseId"],"owner":owner(std::process::id()),"localTransportFinished":true,"remoteOutcomeUnconfirmed":true}));
    assert_eq!(released["state"], "released");
    assert_eq!(released["remoteOutcomeUnconfirmed"], true);
    assert!(
        f.call("profileWriter/list", json!({}))
            .as_array()
            .unwrap()
            .is_empty()
    );
    f.reject("personalMaintenance/acquire",json!({"operationId":"data-test","previewDigest":"wrong","owner":owner(std::process::id())}),"personal_data_preview_changed");
    f.call(
        "personalMaintenance/release",
        json!({"fenceId":fence["fenceId"],"owner":owner(std::process::id())}),
    );
    assert_eq!(f.prepare("remote_request", "remote-2")["state"], "active");
}

#[test]
fn unlaunched_abort_is_owner_scoped_and_prepare_replay_does_not_revive_released_work() {
    let mut f = Fixture::new();
    let lease = f.prepare("local_process", "local-1");
    f.reject(
        "profileWriter/release",
        json!({"leaseId":lease["leaseId"]}),
        "personal_writer_process_unconfirmed",
    );
    let mut wrong = owner(std::process::id());
    wrong["instanceId"] = json!("another-host");
    f.reject(
        "profileWriter/abortUnlaunched",
        json!({"leaseId":lease["leaseId"],"owner":wrong}),
        "personal_writer_abort_not_safe",
    );
    assert_eq!(
        f.call(
            "profileWriter/abortUnlaunched",
            json!({"leaseId":lease["leaseId"],"owner":owner(std::process::id())})
        )["state"],
        "released"
    );
    assert_eq!(f.prepare("local_process", "local-1")["state"], "released");
    f.reject("profileWriter/prepare",json!({"clientOperationId":"local-1","kind":"user_mcp","resourceId":"changed-resource","resourceRevision":"revision-1","owner":owner(std::process::id()),"launchProtocol":1,"processMode":"local_process"}),"operation_id_reused");
}

struct Probe(Child);
impl Probe {
    fn start() -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.env_clear();
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        let child = command
            .args([
                "--ignored",
                "--exact",
                "profile_writers::tests::registered_writer_process_probe",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self(child)
    }
    fn stop(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        if self.0.try_wait().unwrap().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
fn local_processes_must_be_registered_and_proven_exited_before_release_or_recovery() {
    let mut f = Fixture::new();
    let mut child = Probe::start();
    let lease = f.prepare("local_process", "local-process");
    let process =
        json!({"pid":child.0.id(),"startFingerprint":fingerprint(child.0.id()).unwrap().unwrap()});
    f.call(
        "profileWriter/registerProcess",
        json!({"leaseId":lease["leaseId"],"process":process}),
    );
    f.reject(
        "profileWriter/abortUnlaunched",
        json!({"leaseId":lease["leaseId"],"owner":owner(std::process::id())}),
        "personal_writer_abort_not_safe",
    );
    f.reject(
        "profileWriter/release",
        json!({"leaseId":lease["leaseId"]}),
        "personal_writer_process_alive",
    );
    child.stop();
    assert_eq!(
        f.call("profileWriter/release", json!({"leaseId":lease["leaseId"]}))["state"],
        "released"
    );
}

#[test]
fn orphan_recovery_checks_both_owner_and_process_and_never_releases_unknown_empty_local_leases() {
    let mut f = Fixture::new();
    let mut original = Probe::start();
    let mut worker = Probe::start();
    let request = json!({"clientOperationId":"orphan","kind":"user_mcp","resourceId":"mcp","resourceRevision":"revision-1","owner":owner(original.0.id()),"launchProtocol":1,"processMode":"local_process"});
    let lease = f.call("profileWriter/prepare", request);
    f.call("profileWriter/registerProcess",json!({"leaseId":lease["leaseId"],"process":{"pid":worker.0.id(),"startFingerprint":fingerprint(worker.0.id()).unwrap().unwrap()}}));
    f.reject(
        "profileWriter/recover",
        json!({"leaseId":lease["leaseId"]}),
        "personal_writer_owner_alive",
    );
    original.stop();
    f.reject(
        "profileWriter/recover",
        json!({"leaseId":lease["leaseId"]}),
        "personal_writer_process_alive",
    );
    worker.stop();
    assert_eq!(
        f.call("profileWriter/recover", json!({"leaseId":lease["leaseId"]}))["state"],
        "released"
    );
    let mut owner_child = Probe::start();
    let empty=f.call("profileWriter/prepare",json!({"clientOperationId":"unknown-empty","kind":"user_mcp","resourceId":"mcp","resourceRevision":"revision-1","owner":owner(owner_child.0.id()),"launchProtocol":1,"processMode":"local_process"}));
    owner_child.stop();
    f.reject(
        "profileWriter/recover",
        json!({"leaseId":empty["leaseId"]}),
        "personal_writer_process_unconfirmed",
    );
}

#[test]
fn abandoned_maintenance_requires_explicit_safe_reconciliation_without_releasing_writers() {
    let mut f = Fixture::new();
    let mut original = Probe::start();
    let lease = f.prepare("remote_request", "running-remote");
    let fence = f.call(
        "personalMaintenance/acquire",
        json!({"operationId":"data-test","previewDigest":"preview","owner":owner(original.0.id())}),
    );
    let params = json!({"fenceId":fence["fenceId"],"owner":owner(std::process::id())});
    f.reject(
        "personalMaintenance/recover",
        params.clone(),
        "personal_maintenance_owner_alive",
    );
    original.stop();
    f.db.connection
        .execute(
            "UPDATE personal_maintenance_operation_registry SET state='capturing'",
            [],
        )
        .unwrap();
    f.reject(
        "personalMaintenance/recover",
        params.clone(),
        "personal_data_operation_unconfirmed",
    );
    f.db.connection
        .execute(
            "UPDATE personal_maintenance_operation_registry SET state='incomplete'",
            [],
        )
        .unwrap();
    assert_eq!(
        f.call("personalMaintenance/recover", params)["state"],
        "released"
    );
    assert_eq!(
        f.call("profileWriter/list", json!({}))[0]["leaseId"],
        lease["leaseId"]
    );
    let next = f.maintenance();
    assert_ne!(fence["fenceId"], next["fenceId"]);
}

#[test]
#[ignore = "由独立进程退出与恢复测试调用"]
fn registered_writer_process_probe() {
    use std::io::Read as _;
    let mut line = [0u8; 1];
    let _ = std::io::stdin().read(&mut line);
}
