use super::*;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

#[path = "model_credentials_cleanup_tests.rs"]
mod credential_cleanup;
#[path = "model_network_credentials_tests.rs"]
mod network_credentials;

const PROFILE: &str = "11111111111141118111111111111111";
const FENCE: &str = "srv-personal-test";
struct Fixture {
    root: PathBuf,
    db: SqlitePersistence,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "cilai-personal-model-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("contexthub.db"), "personal-test").unwrap();
        db.begin_personal_profile_upgrade(PROFILE, FENCE).unwrap();
        db.finish_personal_profile_upgrade(PROFILE, FENCE, "manifest-test-digest")
            .unwrap();
        Self { root, db }
    }
    fn call(&mut self, method: &str, value: Value) -> Value {
        self.db
            .personal_model_rpc(method, value, PROFILE, FENCE, 123)
            .unwrap()
    }
    fn reject(&mut self, method: &str, value: Value, reason: &str) {
        assert_eq!(
            self.db
                .personal_model_rpc(method, value, PROFILE, FENCE, 123)
                .unwrap_err()
                .reason(),
            Some(reason)
        );
    }
    fn local(&mut self) -> Value {
        let prepared=self.call("modelConnection/savePrepare",json!({"operationId":uuid::Uuid::new_v4().simple().to_string(),"config":{"name":"本机","kind":"local","baseUrl":"http://127.0.0.1:11434/v1","localRuntime":"ollama","authMode":"none","enabled":true},"credentialAction":"clear"}));
        self.call(
            "modelConnection/saveCommit",
            json!({"operationId":prepared["operationId"]}),
        )
    }
    fn descriptor(&mut self, conn: &Value) {
        self.call("modelDescriptor/save",json!({"connectionId":conn["connectionId"],"modelId":"local-model","displayName":"本机模型"}));
    }
    fn owner() -> Value {
        json!({"instanceId":"host-test","pid":std::process::id(),"startFingerprint":contexthub_platform_adapter::instance::process_start_fingerprint(std::process::id()).unwrap().unwrap()})
    }
    fn lease(&mut self, conn: &Value, owner: &Value) -> Value {
        self.call("modelExecution/prepare",json!({"owner":owner,"launchProtocol":1,"purpose":"foreground","selection":{"connectionId":conn["connectionId"],"modelId":"local-model"}}))
    }
    fn preflight(&mut self, conn: &Value, action: &str) -> Value {
        self.call(
            "modelConnection/preflight",
            json!({"connectionId":conn["connectionId"],"action":action}),
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn staged_save_is_durable_and_never_implies_authentication() {
    let mut f = Fixture::new();
    let request = json!({"operationId":"staged-api","config":{"name":"官方API","kind":"openai_key","authMode":"api_key","enabled":true},"credentialAction":"replace"});
    let prepared = f.call("modelConnection/savePrepare", request.clone());
    assert_eq!(f.call("modelConnection/list", json!({})), json!([]));
    f.reject(
        "modelConnection/saveCommit",
        json!({"operationId":"staged-api"}),
        "model_credential_receipt_mismatch",
    );
    assert_eq!(f.call("modelConnection/savePrepare", request), prepared);
    let mut reopened =
        SqlitePersistence::open(&f.root.join("contexthub.db"), "personal-test").unwrap();
    let status = reopened
        .personal_model_rpc("modelConnection/changeList", json!({}), PROFILE, FENCE, 124)
        .unwrap();
    assert_eq!(status[0]["state"], "prepared");
    let conn = f.call(
        "modelConnection/saveCommit",
        json!({"operationId":"staged-api","credentialRef":prepared["connection"]["secretRef"]}),
    );
    assert_eq!(conn["secretRef"]["service"], MODEL_CREDENTIAL_SERVICE);
    assert_eq!(
        conn["secretRef"]["account"],
        format!("{PROFILE}:{}:1", conn["connectionId"].as_str().unwrap())
    );
    assert_eq!(
        f.call(
            "modelValidation/list",
            json!({"connectionId":conn["connectionId"]})
        ),
        json!([])
    );
}

#[test]
fn execution_list_retains_lease_evidence_after_connection_deletion_and_refuses_unknown_profile() {
    let mut f = Fixture::new();
    let conn = f.local();
    f.descriptor(&conn);
    let owner = Fixture::owner();
    let active = f.lease(&conn, &owner);
    let ended = f.lease(&conn, &owner);
    f.call(
        "modelExecution/abortUnlaunched",
        json!({
            "leaseId":ended["leaseId"],"snapshotDigest":ended["snapshotDigest"],"owner":owner
        }),
    );
    let mut deleted: ModelConnection = decoded(conn).unwrap();
    deleted.deleted = true;
    deleted.config.enabled = false;
    let tx = f.db.connection.transaction().unwrap();
    put(&tx, "connection", &deleted.connection_id, &deleted, 124).unwrap();
    tx.commit().unwrap();
    assert_eq!(f.call("modelConnection/list", json!({})), json!([]));
    let listed = f.call("modelExecution/list", json!({}));
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["leaseId"] == active["leaseId"] && v["released"] == false)
    );
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["leaseId"] == ended["leaseId"] && v["released"] == true)
    );
    f.reject(
        "modelExecution/list",
        json!({"connectionId":deleted.connection_id}),
        "model_request_invalid",
    );
    let mut unknown: ModelExecutionLease = decoded(active).unwrap();
    unknown.snapshot.profile_id = "unknown-profile".into();
    let tx = f.db.connection.transaction().unwrap();
    put(&tx, "lease", &unknown.lease_id, &unknown, 125).unwrap();
    tx.commit().unwrap();
    f.reject(
        "modelExecution/list",
        json!({}),
        "model_profile_identity_mismatch",
    );
}

fn prepare_api(f: &mut Fixture, operation: &str) -> Value {
    f.call("modelConnection/savePrepare", json!({"operationId":operation,"config":{"name":"API","kind":"openai_key","authMode":"api_key","enabled":true},"credentialAction":"replace"}))
}

#[test]
fn cancelled_credential_cleanup_is_durable_scoped_and_cannot_restore_a_save() {
    let mut f = Fixture::new();
    let prepared = prepare_api(&mut f, "cancel-api");
    let reference = prepared["connection"]["secretRef"].clone();
    let ack = json!({"operationId":"cancel-api","credentialRef":reference});
    f.reject(
        "modelConnection/credentialCleanupAck",
        ack.clone(),
        "model_credential_cleanup_not_aborted",
    );
    let aborted = f.call(
        "modelConnection/saveAbort",
        json!({"operationId":"cancel-api"}),
    );
    assert_eq!(aborted["state"], "aborted");
    assert_eq!(aborted["credentialCleanup"], "pending");
    f.reject(
        "modelConnection/saveCommit",
        ack.clone(),
        "model_operation_aborted",
    );
    f.reject(
        "modelConnection/credentialCleanupAck",
        json!({"operationId":"unknown","credentialRef":reference}),
        "model_operation_not_found",
    );
    for wrong in [
        json!({"service":"another-service","account":reference["account"]}),
        json!({"service":reference["service"],"account":"another-profile:connection:1"}),
    ] {
        f.reject(
            "modelConnection/credentialCleanupAck",
            json!({"operationId":"cancel-api","credentialRef":wrong}),
            "model_credential_cleanup_reference_mismatch",
        );
    }
    let mut reopened =
        SqlitePersistence::open(&f.root.join("contexthub.db"), "personal-test").unwrap();
    let changes = reopened
        .personal_model_rpc("modelConnection/changeList", json!({}), PROFILE, FENCE, 124)
        .unwrap();
    assert_eq!(changes[0], aborted);
    let confirmed = f.call("modelConnection/credentialCleanupAck", ack.clone());
    assert_eq!(confirmed["credentialCleanup"], "confirmed");
    assert_eq!(
        f.call("modelConnection/credentialCleanupAck", ack),
        confirmed
    );
    assert_eq!(
        f.call(
            "modelConnection/saveAbort",
            json!({"operationId":"cancel-api"})
        ),
        confirmed
    );
    assert_eq!(f.call("modelConnection/list", json!({})), json!([]));
    assert_eq!(f.call("modelConnection/savePrepare", json!({"operationId":"cancel-api","config":prepared["connection"]["config"],"credentialAction":"replace"})), confirmed);
}

#[test]
fn committed_and_nonreplacement_operations_cannot_acknowledge_credential_cleanup() {
    let mut f = Fixture::new();
    let prepared = prepare_api(&mut f, "committed-api");
    let ack =
        json!({"operationId":"committed-api","credentialRef":prepared["connection"]["secretRef"]});
    let conn = f.call("modelConnection/saveCommit", ack.clone());
    f.reject(
        "modelConnection/saveAbort",
        json!({"operationId":"committed-api"}),
        "model_operation_already_committed",
    );
    f.reject(
        "modelConnection/credentialCleanupAck",
        ack,
        "model_credential_cleanup_not_aborted",
    );
    f.call("modelConnection/savePrepare", json!({"operationId":"keep-api","connectionId":conn["connectionId"],"expectedConfigRevision":1,"config":conn["config"],"credentialAction":"keep"}));
    let kept = f.call(
        "modelConnection/saveAbort",
        json!({"operationId":"keep-api"}),
    );
    assert_eq!(kept["credentialCleanup"], "not_required");
    f.reject(
        "modelConnection/credentialCleanupAck",
        json!({"operationId":"keep-api","credentialRef":conn["secretRef"]}),
        "model_credential_cleanup_not_aborted",
    );
    assert_eq!(
        f.call(
            "modelConnection/get",
            json!({"connectionId":conn["connectionId"]})
        ),
        conn
    );
    let ledger = f.db.ai_business_ledger().unwrap();
    let record = ledger["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == entry_id("change", "keep-api"))
        .unwrap();
    let mut forged = record["value"].clone();
    forged["value"]["credentialCleanup"] = json!("confirmed");
    let request = json!({"commandId":"forged-cleanup","records":[{"collection":"entries","id":record["id"],"expectedDigest":record["digest"],"position":0,"value":forged}]});
    assert_eq!(
        f.db.apply_ai_business_batch(&request, 125)
            .unwrap_err()
            .reason(),
        Some("ai_ledger_writer_scope_invalid")
    );
}

#[test]
fn legacy_cancelled_credentials_remain_unconfirmed_until_an_exact_cleanup_ack() {
    let mut f = Fixture::new();
    let prepared = prepare_api(&mut f, "legacy-api");
    f.call(
        "modelConnection/saveAbort",
        json!({"operationId":"legacy-api"}),
    );
    let id = entry_id("change", "legacy-api");
    let raw: String =
        f.db.connection
            .query_row(
                "SELECT record_json FROM ai_runtime_entries WHERE record_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
    let mut legacy: Value = serde_json::from_str(&raw).unwrap();
    legacy["value"]
        .as_object_mut()
        .unwrap()
        .remove("credentialCleanup");
    let legacy_digest = digest(&json!({"position":0,"value":legacy}));
    f.db.connection
        .execute(
            "UPDATE ai_runtime_entries SET record_json=?1,digest=?2 WHERE record_id=?3",
            params![legacy.to_string(), legacy_digest, id],
        )
        .unwrap();
    let mut reopened =
        SqlitePersistence::open(&f.root.join("contexthub.db"), "personal-test").unwrap();
    let status = reopened
        .personal_model_rpc(
            "modelConnection/changeStatus",
            json!({"operationId":"legacy-api"}),
            PROFILE,
            FENCE,
            124,
        )
        .unwrap();
    assert_eq!(status["credentialCleanup"], "unconfirmed");
    assert_eq!(
        f.call(
            "modelConnection/saveAbort",
            json!({"operationId":"legacy-api"})
        ),
        status
    );
    let ack = f.call(
        "modelConnection/credentialCleanupAck",
        json!({"operationId":"legacy-api","credentialRef":prepared["connection"]["secretRef"]}),
    );
    assert_eq!(ack["credentialCleanup"], "confirmed");
    f.reject(
        "modelConnection/saveCommit",
        json!({"operationId":"legacy-api","credentialRef":prepared["connection"]["secretRef"]}),
        "model_operation_aborted",
    );
}

#[test]
fn concurrent_commit_and_abort_have_one_durable_terminal_winner() {
    let mut f = Fixture::new();
    let prepared = prepare_api(&mut f, "race-api");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let spawn = |method: &'static str, value: Value| {
        let barrier = std::sync::Arc::clone(&barrier);
        let path = f.root.join("contexthub.db");
        std::thread::spawn(move || {
            let mut db = SqlitePersistence::open(&path, "personal-test").unwrap();
            barrier.wait();
            db.personal_model_rpc(method, value, PROFILE, FENCE, 124)
                .map_err(|e| e.reason().unwrap_or("unexpected_database_error").to_owned())
        })
    };
    let commit = spawn(
        "modelConnection/saveCommit",
        json!({"operationId":"race-api","credentialRef":prepared["connection"]["secretRef"]}),
    );
    let abort = spawn(
        "modelConnection/saveAbort",
        json!({"operationId":"race-api"}),
    );
    let (commit, abort) = (commit.join().unwrap(), abort.join().unwrap());
    let state = f.call(
        "modelConnection/changeStatus",
        json!({"operationId":"race-api"}),
    );
    match (commit, abort) {
        (Ok(conn), Err(reason)) => {
            assert_eq!(reason, "model_operation_already_committed");
            assert_eq!(state["state"], "committed");
            assert_eq!(state["credentialCleanup"], "not_required");
            assert_eq!(
                f.call(
                    "modelConnection/get",
                    json!({"connectionId":conn["connectionId"]})
                ),
                conn
            );
        }
        (Err(reason), Ok(aborted)) => {
            assert_eq!(reason, "model_operation_aborted");
            assert_eq!(state, aborted);
            assert_eq!(state["state"], "aborted");
            assert_eq!(state["credentialCleanup"], "pending");
            assert_eq!(f.call("modelConnection/list", json!({})), json!([]));
        }
        result => panic!("commit/abort must have exactly one winner: {result:?}"),
    }
}

#[test]
fn pending_and_aborted_saves_never_reuse_a_credential_generation() {
    let mut f = Fixture::new();
    let first=f.call("modelConnection/savePrepare",json!({"operationId":"first","config":{"name":"API","kind":"openai_key","authMode":"api_key","enabled":true},"credentialAction":"replace"}));
    let conn = f.call(
        "modelConnection/saveCommit",
        json!({"operationId":"first","credentialRef":first["connection"]["secretRef"]}),
    );
    let base = json!({"connectionId":conn["connectionId"],"expectedConfigRevision":1,"config":conn["config"],"credentialAction":"replace"});
    let mut a = base.clone();
    a["operationId"] = json!("second");
    let second = f.call("modelConnection/savePrepare", a);
    let mut b = base.clone();
    b["operationId"] = json!("third");
    let third = f.call("modelConnection/savePrepare", b);
    assert_eq!(second["connection"]["authGeneration"], 2);
    assert_eq!(third["connection"]["authGeneration"], 3);
    f.call("modelConnection/saveAbort", json!({"operationId":"third"}));
    let mut c = base;
    c["operationId"] = json!("fourth");
    let fourth = f.call("modelConnection/savePrepare", c);
    assert_eq!(fourth["connection"]["authGeneration"], 4);
    let current = f.call(
        "modelConnection/saveCommit",
        json!({"operationId":"second","credentialRef":second["connection"]["secretRef"]}),
    );
    f.reject(
        "modelConnection/saveCommit",
        json!({"operationId":"fourth","credentialRef":fourth["connection"]["secretRef"]}),
        "model_connection_revision_conflict",
    );
    assert_eq!(
        f.call(
            "modelConnection/get",
            json!({"connectionId":conn["connectionId"]})
        ),
        current
    );
}

#[test]
fn generic_and_typed_ledger_writers_cannot_cross_owner_namespaces() {
    let mut f = Fixture::new();
    let conn = f.local();
    let ledger = f.db.ai_business_ledger().unwrap();
    let record = ledger["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == entry_id("connection", conn["connectionId"].as_str().unwrap()))
        .unwrap();
    for value in [Value::Null, record["value"].clone()] {
        let request = json!({"commandId":uuid::Uuid::new_v4().simple().to_string(),"records":[{"collection":"entries","id":record["id"],"expectedDigest":record["digest"],"position":0,"value":value}]});
        assert_eq!(
            f.db.apply_ai_business_batch(&request, 124)
                .unwrap_err()
                .reason(),
            Some("ai_ledger_writer_scope_invalid")
        );
    }
    let tx = f.db.connection.transaction().unwrap();
    let forged = json!({"commandId":"personalModel:forged","records":[{"collection":"entries","id":"agents:old","expectedDigest":null,"position":0,"value":{"entryId":"agents:old","owner":MODEL_ENTRY_OWNER}}]});
    assert_eq!(
        apply_model_batch(&tx, &forged, 125).unwrap_err().reason(),
        Some("ai_ledger_writer_scope_invalid")
    );
    tx.rollback().unwrap();
    assert_eq!(
        f.call(
            "modelConnection/get",
            json!({"connectionId":conn["connectionId"]})
        ),
        conn
    );
}

#[test]
fn manually_added_models_remain_unknown_and_catalog_does_not_validate_tools() {
    let mut f = Fixture::new();
    let conn = f.local();
    f.descriptor(&conn);
    let descriptors = f.call(
        "modelCatalog/read",
        json!({"connectionId":conn["connectionId"]}),
    );
    assert!(descriptors[0]["contextWindowTokens"].is_null());
    assert_eq!(descriptors[0]["capabilities"], json!({}));
    let pending=f.call("modelValidation/prepare",json!({"connectionId":conn["connectionId"],"layer":"catalog","expectedConfigRevision":1,"expectedAuthGeneration":0}));
    f.call(
        "modelValidation/report",
        json!({"validationId":pending["validationId"],"outcome":"passed"}),
    );
    let after = f.call(
        "modelCatalog/read",
        json!({"connectionId":conn["connectionId"]}),
    );
    assert_eq!(after[0]["capabilities"], json!({}));
}

#[test]
fn stale_validation_is_rejected_after_configuration_changes() {
    let mut f = Fixture::new();
    let conn = f.local();
    let pending=f.call("modelValidation/prepare",json!({"connectionId":conn["connectionId"],"layer":"network","expectedConfigRevision":1,"expectedAuthGeneration":0}));
    let mut config = conn["config"].clone();
    config["baseUrl"] = json!("http://127.0.0.1:22334/v1");
    f.call("modelConnection/savePrepare",json!({"operationId":"edit","connectionId":conn["connectionId"],"expectedConfigRevision":1,"config":config,"credentialAction":"clear"}));
    f.call("modelConnection/saveCommit", json!({"operationId":"edit"}));
    f.reject(
        "modelValidation/report",
        json!({"validationId":pending["validationId"],"outcome":"passed"}),
        "model_validation_stale",
    );
    let records = f.call(
        "modelValidation/list",
        json!({"connectionId":conn["connectionId"]}),
    );
    assert!(records[0]["outcome"].is_null());
}

#[test]
fn deleted_connections_can_be_removed_from_disabled_purpose_bindings() {
    let mut f = Fixture::new();
    let conn = f.local();
    f.descriptor(&conn);
    let mut binding = json!({"bindingId":"profile:foreground","purpose":"foreground","policy":"explicit","selection":{"connectionId":conn["connectionId"],"modelId":"local-model"},"enabled":true,"revision":0});
    f.call("modelPurpose/save", json!({"binding":binding}));
    let check = f.preflight(&conn, "delete");
    assert_eq!(check["references"][0]["kind"], "purpose_binding");
    f.call("modelConnection/deleteCommit",json!({"connectionId":conn["connectionId"],"expectedConfigRevision":1,"impactDigest":check["impactDigest"]}));
    binding["enabled"] = json!(false);
    binding["selection"] = Value::Null;
    let saved = f.call(
        "modelPurpose/save",
        json!({"binding":binding,"expectedRevision":1}),
    );
    assert_eq!(saved["enabled"], false);
    assert!(saved["selection"].is_null());
    f.reject(
        "modelExecution/resolve",
        json!({"purpose":"foreground"}),
        "model_purpose_disabled",
    );
}

#[test]
fn background_history_consent_is_required_and_does_not_follow_another_default() {
    let mut f = Fixture::new();
    let a = f.local();
    let b = f.local();
    f.descriptor(&a);
    f.descriptor(&b);
    let mut default = json!({"bindingId":"profile:memory","purpose":"memory","policy":"explicit","selection":{"connectionId":a["connectionId"],"modelId":"local-model"},"enabled":true,"revision":0});
    f.reject(
        "modelPurpose/save",
        json!({"binding":default}),
        "model_history_consent_required",
    );
    default["consent"] = json!({"serviceName":"本机","historyScope":"profile_history","mayIncurCost":false,"confirmedAt":123});
    f.call("modelPurpose/save", json!({"binding":default}));
    let child = json!({"bindingId":"memory:task","purpose":"memory","scopeId":"task","policy":"inherit_default","enabled":true,"revision":0,"consent":default["consent"]});
    f.call("modelPurpose/save", json!({"binding":child}));
    f.call(
        "modelExecution/resolve",
        json!({"purpose":"memory","bindingId":"memory:task"}),
    );
    default["selection"]["connectionId"] = b["connectionId"].clone();
    f.call(
        "modelPurpose/save",
        json!({"binding":default,"expectedRevision":1}),
    );
    f.reject(
        "modelExecution/resolve",
        json!({"purpose":"memory","bindingId":"memory:task"}),
        "model_history_consent_stale",
    );
}

#[test]
fn execution_lease_requires_process_registration_and_blocks_disabling() {
    let mut f = Fixture::new();
    let conn = f.local();
    f.descriptor(&conn);
    let lease = f.lease(&conn, &Fixture::owner());
    let identity = json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"]});
    f.reject(
        "modelExecution/validateForRequest",
        identity.clone(),
        "model_execution_process_not_registered",
    );
    let check = f.preflight(&conn, "disable");
    assert_eq!(check["activeLeaseIds"], json!([lease["leaseId"]]));
    f.reject("modelConnection/setEnabled",json!({"connectionId":conn["connectionId"],"expectedConfigRevision":1,"enabled":false,"impactDigest":check["impactDigest"]}),"model_connection_in_use");
    f.call("modelExecution/abortUnlaunched", json!({"leaseId":identity["leaseId"],"snapshotDigest":identity["snapshotDigest"],"owner":Fixture::owner()}));
    let check = f.preflight(&conn, "disable");
    f.call("modelConnection/setEnabled",json!({"connectionId":conn["connectionId"],"expectedConfigRevision":1,"enabled":false,"impactDigest":check["impactDigest"]}));
}

struct OwnedProcess(Child);
impl OwnedProcess {
    fn new() -> Self {
        #[cfg(unix)]
        let child = Command::new("/bin/sleep")
            .arg("120")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        #[cfg(windows)]
        let child = Command::new("cmd")
            .args(["/C", "ping -n 120 127.0.0.1 >NUL"])
            .spawn()
            .unwrap();
        Self(child)
    }
    fn identity(&self) -> Value {
        json!({"pid":self.0.id(),"startFingerprint":contexthub_platform_adapter::instance::process_start_fingerprint(self.0.id()).unwrap().unwrap()})
    }
    fn reap(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn restart_recovery_only_releases_proven_dead_owners_and_execution_processes() {
    let mut f = Fixture::new();
    let conn = f.local();
    f.descriptor(&conn);
    let mut owner = OwnedProcess::new();
    let mut process = OwnedProcess::new();
    let mut owner_identity = owner.identity();
    owner_identity["instanceId"] = json!("dead-host-test");
    let lease = f.lease(&conn, &owner_identity);
    f.call("modelExecution/registerProcess",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"process":process.identity()}));
    f.reject(
        "modelExecution/release",
        json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"]}),
        "model_execution_process_alive",
    );
    f.db.begin_personal_profile_upgrade(PROFILE, "srv-restarted")
        .unwrap();
    f.db.finish_personal_profile_upgrade(PROFILE, "srv-restarted", "same-manifest")
        .unwrap();
    let call = |db: &mut SqlitePersistence, method: &str, value: Value| {
        db.personal_model_rpc(method, value, PROFILE, "srv-restarted", 125)
    };
    assert_eq!(
        call(
            &mut f.db,
            "modelExecution/validate",
            json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"]})
        )
        .unwrap_err()
        .reason(),
        Some("model_execution_instance_changed")
    );
    let result = call(
        &mut f.db,
        "modelExecution/recover",
        json!({"connectionId":conn["connectionId"]}),
    )
    .unwrap();
    assert_eq!(result["leases"][0]["state"], "owner_alive");
    owner.reap();
    let result = call(
        &mut f.db,
        "modelExecution/recover",
        json!({"connectionId":conn["connectionId"]}),
    )
    .unwrap();
    assert_eq!(result["leases"][0]["state"], "process_alive");
    process.reap();
    let result = call(
        &mut f.db,
        "modelExecution/recover",
        json!({"connectionId":conn["connectionId"]}),
    )
    .unwrap();
    assert_eq!(result["leases"][0]["state"], "released");
    let check = call(
        &mut f.db,
        "modelConnection/preflight",
        json!({"connectionId":conn["connectionId"],"action":"delete"}),
    )
    .unwrap();
    assert_eq!(check["activeLeaseIds"], json!([]));
}

#[test]
fn unknown_fields_are_rejected_without_storing_secret_text() {
    let mut f = Fixture::new();
    f.reject("modelConnection/savePrepare",json!({"operationId":"secret-field","config":{"name":"API","kind":"openai_key","authMode":"api_key","enabled":true,"apiKey":"not-a-real-secret"},"credentialAction":"replace"}),"model_request_invalid");
    assert_eq!(f.call("modelConnection/changeList", json!({})), json!([]));
    assert!(
        !f.db
            .ai_business_ledger()
            .unwrap()
            .to_string()
            .contains("not-a-real-secret")
    );
}

#[test]
fn stopping_a_background_purpose_blocks_the_next_request_of_an_existing_lease() {
    let mut f = Fixture::new();
    let conn = f.local();
    f.descriptor(&conn);
    let mut process = OwnedProcess::new();
    let mut binding = json!({"bindingId":"profile:memory","purpose":"memory","policy":"explicit","selection":{"connectionId":conn["connectionId"],"modelId":"local-model"},"enabled":true,"revision":0,"consent":{"serviceName":"本机","historyScope":"profile_history","mayIncurCost":false,"confirmedAt":123}});
    f.call("modelPurpose/save", json!({"binding":binding}));
    let lease = f.call(
        "modelExecution/prepare",
        json!({"owner":Fixture::owner(),"launchProtocol":1,"purpose":"memory"}),
    );
    f.call("modelExecution/registerProcess",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"process":process.identity()}));
    let params = json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"]});
    f.call("modelExecution/validateForRequest", params.clone());
    binding["enabled"] = json!(false);
    binding["consent"] = Value::Null;
    f.call(
        "modelPurpose/save",
        json!({"binding":binding,"expectedRevision":1}),
    );
    f.reject(
        "modelExecution/validateForRequest",
        params.clone(),
        "model_purpose_disabled",
    );
    process.reap();
    f.call("modelExecution/release", params);
}

#[test]
fn an_unregistered_launch_remains_unconfirmed_after_its_owner_exits() {
    let mut f = Fixture::new();
    let conn = f.local();
    f.descriptor(&conn);
    let mut owner = OwnedProcess::new();
    let mut identity = owner.identity();
    identity["instanceId"] = json!("unconfirmed-launch-host");
    let lease = f.lease(&conn, &identity);
    owner.reap();
    let result = f.call(
        "modelExecution/recover",
        json!({"connectionId":conn["connectionId"]}),
    );
    assert_eq!(result["leases"][0]["state"], "unconfirmed");
    assert_eq!(
        f.preflight(&conn, "delete")["activeLeaseIds"],
        json!([lease["leaseId"]])
    );
}

#[test]
fn personal_native_journal_requires_the_matching_lease_owner_and_history_consent() {
    let mut f = Fixture::new();
    let a = f.local();
    let b = f.local();
    f.descriptor(&a);
    f.descriptor(&b);
    let mut process = OwnedProcess::new();
    let conversation = "application:personal-scope-test";
    let make_lease = |f: &mut Fixture, conn: &Value| {
        let id = format!("page:{}", conn["connectionId"].as_str().unwrap());
        f.call("modelPurpose/save",json!({"binding":{"bindingId":id,"purpose":"page_ai","scopeId":conn["connectionId"],"policy":"explicit","selection":{"connectionId":conn["connectionId"],"modelId":"local-model"},"enabled":true,"revision":0,"consent":{"serviceName":"本机","historyScope":"selected_page","mayIncurCost":false,"confirmedAt":123}}}));
        f.call("modelExecution/prepare",json!({"owner":Fixture::owner(),"launchProtocol":1,"purpose":"page_ai","bindingId":id,"conversationId":conversation}))
    };
    let a_lease = make_lease(&mut f, &a);
    let b_lease = make_lease(&mut f, &b);
    for lease in [&a_lease, &b_lease] {
        f.call("modelExecution/registerProcess",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"process":process.identity()}));
    }
    let scope = |lease: &Value| json!({"profileId":PROFILE,"connectionId":lease["snapshot"]["connectionId"],"configRevision":lease["snapshot"]["configRevision"],"authGeneration":lease["snapshot"]["authGeneration"],"runtimeHomeScope":lease["snapshot"]["runtimeHomeScope"],"runtimeKind":"application"});
    let mut request = json!({"conversationId":conversation,"operationId":"scope-create","runId":"app_run_scope","ownerInstanceId":"host-test","expectedBindingRevision":0,"protocolVersion":1,"kind":"create","applicationScope":{"applicationId":"personal-demo","sessionKind":"application_page"},"input":{}});
    assert_eq!(
        f.db.prepare_ai_thread_operation(&request, 123)
            .unwrap_err()
            .reason(),
        Some("model_thread_scope_required")
    );
    request["input"] = json!({"runtimeScope":scope(&a_lease),"modelExecutionLeaseId":a_lease["leaseId"],"writerProcess":process.identity()});
    let mut wrong = request.clone();
    wrong["ownerInstanceId"] = json!("another-owner");
    assert_eq!(
        f.db.prepare_ai_thread_operation(&wrong, 123)
            .unwrap_err()
            .reason(),
        Some("model_thread_owner_scope_mismatch")
    );
    f.db.prepare_ai_thread_operation(&request, 123).unwrap();
    let ack = |operation: &str, event: &str, receipt: Value| json!({"conversationId":conversation,"operationId":operation,"runId":"app_run_scope","ownerInstanceId":"host-test","event":event,"receipt":receipt});
    f.db.acknowledge_ai_thread_operation(&ack("scope-create", "dispatch", Value::Null), 124)
        .unwrap();
    f.db.acknowledge_ai_thread_operation(
        &ack(
            "scope-create",
            "acknowledge",
            json!({"nativeThreadId":"native-first"}),
        ),
        125,
    )
    .unwrap();
    let current = f.db.ai_thread_state(conversation).unwrap();
    assert_eq!(current["binding"]["runtimeScope"], scope(&a_lease));
    request["operationId"] = json!("scope-fork");
    request["expectedBindingRevision"] = current["binding"]["bindingRevision"].clone();
    request["kind"] = json!("resume");
    request["input"]["runtimeScope"] = scope(&b_lease);
    request["input"]["modelExecutionLeaseId"] = b_lease["leaseId"].clone();
    assert_eq!(
        f.db.prepare_ai_thread_operation(&request, 126)
            .unwrap_err()
            .reason(),
        Some("model_thread_scope_changed")
    );
    request["kind"] = json!("fork");
    assert_eq!(
        f.db.prepare_ai_thread_operation(&request, 126)
            .unwrap_err()
            .reason(),
        Some("model_thread_history_consent_required")
    );
    request["input"]["historyConsentConfirmed"] = json!(true);
    f.db.prepare_ai_thread_operation(&request, 126).unwrap();
    assert_eq!(
        f.db.ai_thread_state(conversation).unwrap()["binding"]["runtimeScope"],
        scope(&a_lease)
    );
    f.db.acknowledge_ai_thread_operation(&ack("scope-fork", "dispatch", Value::Null), 127)
        .unwrap();
    f.db.acknowledge_ai_thread_operation(
        &ack(
            "scope-fork",
            "acknowledge",
            json!({"nativeThreadId":"native-second","sourceThreadId":"native-first"}),
        ),
        128,
    )
    .unwrap();
    let journal = f.db.ai_thread_state(conversation).unwrap();
    assert_eq!(journal["generations"].as_array().unwrap().len(), 2);
    assert_eq!(journal["generations"][0]["runtimeScope"], scope(&a_lease));
    assert_eq!(journal["generations"][1]["runtimeScope"], scope(&b_lease));
    process.reap();
}

fn chatgpt(f: &mut Fixture) -> Value {
    f.call("modelConnection/savePrepare",json!({"operationId":"create-chatgpt","config":{"name":"个人ChatGPT","kind":"chatgpt","authMode":"runtime_managed_chatgpt","enabled":true},"credentialAction":"keep"}));
    f.call(
        "modelConnection/saveCommit",
        json!({"operationId":"create-chatgpt"}),
    )
}
fn auth_prepare(f: &mut Fixture, conn: &Value, id: &str, kind: &str) -> Value {
    f.call("modelAuth/startPrepare",json!({"operationId":id,"connectionId":conn["connectionId"],"expectedConfigRevision":conn["configRevision"],"runtimeKind":kind,"owner":Fixture::owner(),"launchProtocol":1}))
}
fn auth_confirm_fixture(f: &mut Fixture, op: &Value, state: &str) {
    let step = &op["steps"][0];
    let mut process = OwnedProcess::new();
    f.call("modelAuth/registerProcess",json!({"operationId":op["operationId"],"runtimeKind":step["runtimeKind"],"authGeneration":step["authGeneration"],"process":process.identity()}));
    f.reject("modelAuth/step",json!({"operationId":op["operationId"],"runtimeKind":step["runtimeKind"],"authGeneration":step["authGeneration"],"state":state}),"model_auth_process_alive");
    process.reap();
    f.call("modelAuth/step",json!({"operationId":op["operationId"],"runtimeKind":step["runtimeKind"],"authGeneration":step["authGeneration"],"state":state}));
}
#[test]
fn chatgpt_authorization_scopes_and_observed_capabilities_do_not_cross_runtime_kinds() {
    let mut f = Fixture::new();
    let conn = chatgpt(&mut f);
    f.descriptor(&conn);
    let work = auth_prepare(&mut f, &conn, "work-login", "workbench");
    f.reject(
        "modelConnection/saveCommit",
        json!({"operationId":"work-login"}),
        "model_operation_owner_mismatch",
    );
    f.reject(
        "modelConnection/saveAbort",
        json!({"operationId":"work-login"}),
        "model_operation_owner_mismatch",
    );
    f.reject(
        "modelConnection/credentialCleanupAck",
        json!({"operationId":"work-login","credentialRef":{"service":MODEL_CREDENTIAL_SERVICE,"account":"unused"}}),
        "model_operation_owner_mismatch",
    );
    f.reject("modelAuth/step",json!({"operationId":"work-login","runtimeKind":"workbench","authGeneration":work["targetGeneration"],"state":"confirmed"}),"model_auth_process_not_registered");
    auth_confirm_fixture(&mut f, &work, "confirmed");
    f.call("modelAuth/finish", json!({"operationId":"work-login"}));
    let record=f.call("modelValidation/prepare",json!({"runtimeKind":"workbench","connectionId":conn["connectionId"],"modelId":"local-model","layer":"tools","expectedConfigRevision":1,"expectedAuthGeneration":work["targetGeneration"]}));
    f.call("modelValidation/report",json!({"validationId":record["validationId"],"outcome":"passed","runtimeVersion":"test-fixture-only"}));
    let resolve = |f: &mut Fixture| {
        f.call("modelExecution/resolve",json!({"purpose":"foreground","selection":{"connectionId":conn["connectionId"],"modelId":"local-model"}}))
    };
    assert_eq!(
        resolve(&mut f)["capabilities"]["tools"]["state"],
        "supported"
    );
    let page = auth_prepare(&mut f, &conn, "page-login", "application");
    assert!(page["targetGeneration"].as_u64() > work["targetGeneration"].as_u64());
    assert_eq!(resolve(&mut f)["authGeneration"], work["targetGeneration"]);
    auth_confirm_fixture(&mut f, &page, "confirmed");
    f.call("modelAuth/finish", json!({"operationId":"page-login"}));
    let validation=f.call("modelValidation/prepare",json!({"runtimeKind":"application","connectionId":conn["connectionId"],"modelId":"local-model","layer":"tools","expectedConfigRevision":1,"expectedAuthGeneration":page["targetGeneration"]}));
    let probe=f.call("modelValidation/prepareExecution",json!({"validationId":validation["validationId"],"owner":Fixture::owner(),"launchProtocol":1}));
    assert_eq!(probe["snapshot"]["runtimeKind"], "application");
    assert_eq!(
        probe["snapshot"]["authGeneration"],
        page["targetGeneration"]
    );
    assert!(probe["snapshot"]["capabilities"]["tools"].is_null());
    f.reject(
        "modelValidation/report",
        json!({"validationId":validation["validationId"],"outcome":"passed"}),
        "model_validation_execution_in_use",
    );
    f.call(
        "modelExecution/abortUnlaunched",
        json!({"leaseId":probe["leaseId"],"snapshotDigest":probe["snapshotDigest"],"owner":Fixture::owner()}),
    );
    f.call("modelValidation/report",json!({"validationId":validation["validationId"],"outcome":"failed","reason":"fixture_tools_unsupported"}));
    assert_eq!(
        resolve(&mut f)["capabilities"]["tools"]["state"],
        "supported"
    );
    let check = f.preflight(&conn, "disable");
    f.reject("modelConnection/setEnabled",json!({"connectionId":conn["connectionId"],"expectedConfigRevision":1,"enabled":false,"impactDigest":check["impactDigest"]}),"model_auth_logout_required");
}
#[test]
fn pending_oauth_recovery_requires_owned_processes_or_explicit_unstarted_cleanup() {
    let mut f = Fixture::new();
    let conn = chatgpt(&mut f);
    let mut owner = OwnedProcess::new();
    let mut identity = owner.identity();
    identity["instanceId"] = json!("old-auth-owner");
    let op=f.call("modelAuth/startPrepare",json!({"operationId":"interrupted-login","connectionId":conn["connectionId"],"expectedConfigRevision":1,"runtimeKind":"workbench","owner":identity,"launchProtocol":1}));
    let recover =
        json!({"operationId":"interrupted-login","owner":Fixture::owner(),"launchProtocol":1});
    f.reject(
        "modelAuth/recover",
        recover.clone(),
        "model_auth_owner_alive",
    );
    owner.reap();
    f.reject(
        "modelAuth/recover",
        recover.clone(),
        "model_auth_ownership_unconfirmed",
    );
    let mut explicit = recover;
    explicit["allowUnstartedLoginCleanup"] = json!(true);
    let recovered = f.call("modelAuth/recover", explicit);
    assert_eq!(recovered["state"], "prepared");
    f.reject(
        "modelAuth/cancel",
        json!({"operationId":"interrupted-login"}),
        "model_auth_process_not_registered",
    );
    auth_confirm_fixture(&mut f, &op, "signed_out");
    let cancelled = f.call(
        "modelAuth/cancel",
        json!({"operationId":"interrupted-login"}),
    );
    assert_eq!(cancelled["state"], "cancelled");
    let next = auth_prepare(&mut f, &conn, "new-login", "workbench");
    assert!(next["targetGeneration"].as_u64() > op["targetGeneration"].as_u64());
}
#[test]
fn connection_kind_and_endpoint_changes_cannot_reuse_previous_authorization() {
    let mut f = Fixture::new();
    let conn = f.local();
    let mut config = conn["config"].clone();
    config["baseUrl"] = json!("http://127.0.0.1:1234/v1");
    f.reject("modelConnection/savePrepare",json!({"operationId":"endpoint-keep","connectionId":conn["connectionId"],"expectedConfigRevision":1,"config":config,"credentialAction":"keep"}),"model_endpoint_change_requires_auth_rotation");
    config["localRuntime"] = json!("lmstudio");
    f.reject("modelConnection/savePrepare",json!({"operationId":"kind-change","connectionId":conn["connectionId"],"expectedConfigRevision":1,"config":config,"credentialAction":"clear"}),"model_connection_kind_immutable");
}

#[test]
fn readonly_metadata_ownership_never_allocates_or_activates_an_account_scope() {
    let mut f = Fixture::new();
    let conn = chatgpt(&mut f);
    let mut owner = OwnedProcess::new();
    let mut identity = owner.identity();
    identity["instanceId"] = json!("old-catalog-owner");
    for layer in ["configuration", "network", "catalog"] {
        let record=f.call("modelValidation/prepare",json!({"connectionId":conn["connectionId"],"runtimeKind":"application","layer":layer,"expectedConfigRevision":1,"expectedAuthGeneration":0}));
        assert_eq!(record["authGeneration"], 0);
    }
    f.reject("modelAuth/readPrepare",json!({"operationId":"status-no-account","connectionId":conn["connectionId"],"expectedConfigRevision":1,"runtimeKind":"workbench","owner":Fixture::owner(),"launchProtocol":1}),"model_auth_required");
    let op=f.call("modelAuth/catalogPrepare",json!({"operationId":"catalog-interrupted","connectionId":conn["connectionId"],"expectedConfigRevision":1,"runtimeKind":"workbench","owner":identity,"launchProtocol":1}));
    assert_eq!(op["targetGeneration"], 0);
    owner.reap();
    let recover = json!({"operationId":"catalog-interrupted","owner":Fixture::owner(),"launchProtocol":1,"allowUnstartedLoginCleanup":true});
    f.reject(
        "modelAuth/recover",
        recover.clone(),
        "model_auth_ownership_unconfirmed",
    );
    let mut cancel = recover;
    cancel["allowUnstartedReadonlyCleanup"] = json!(true);
    assert_eq!(f.call("modelAuth/recover", cancel)["state"], "cancelled");
    assert_eq!(
        f.call(
            "modelConnection/get",
            json!({"connectionId":conn["connectionId"]})
        ),
        conn
    );
}
#[test]
fn complete_logout_requires_every_active_and_retired_scope_to_be_confirmed_signed_out() {
    let mut f = Fixture::new();
    let conn = chatgpt(&mut f);
    for (id, kind) in [
        ("login-a", "workbench"),
        ("login-b", "workbench"),
        ("login-c", "application"),
    ] {
        let op = auth_prepare(&mut f, &conn, id, kind);
        auth_confirm_fixture(&mut f, &op, "confirmed");
        f.call("modelAuth/finish", json!({"operationId":id}));
    }
    let original = f.call(
        "modelConnection/get",
        json!({"connectionId":conn["connectionId"]}),
    );
    assert_eq!(original["authScopes"].as_array().unwrap().len(), 3);
    let logout=f.call("modelAuth/logoutPrepare",json!({"operationId":"logout-all","connectionId":conn["connectionId"],"expectedConfigRevision":1,"owner":Fixture::owner(),"launchProtocol":1}));
    assert_eq!(logout["steps"].as_array().unwrap().len(), 3);
    f.reject(
        "modelAuth/cancel",
        json!({"operationId":"logout-all"}),
        "model_auth_process_not_registered",
    );
    for step in logout["steps"].as_array().unwrap() {
        let only = json!({"operationId":"logout-all","steps":[step]});
        auth_confirm_fixture(&mut f, &only, "signed_out");
    }
    let finished = f.call("modelAuth/finish", json!({"operationId":"logout-all"}));
    assert_eq!(finished["state"], "completed");
    let current = f.call(
        "modelConnection/get",
        json!({"connectionId":conn["connectionId"]}),
    );
    assert!(
        current["authScopes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["state"] == "signed_out")
    );
    assert_eq!(current["connectionId"], original["connectionId"]);
    let check = f.preflight(&conn, "disable");
    f.call("modelConnection/setEnabled",json!({"connectionId":conn["connectionId"],"expectedConfigRevision":1,"enabled":false,"impactDigest":check["impactDigest"]}));
}

#[test]
fn an_auth_launch_can_only_be_aborted_by_its_owner_before_any_process_registration() {
    let mut f = Fixture::new();
    let conn = chatgpt(&mut f);
    let old = auth_prepare(&mut f, &conn, "existing-scope", "workbench");
    auth_confirm_fixture(&mut f, &old, "confirmed");
    f.call("modelAuth/finish", json!({"operationId":"existing-scope"}));
    let fresh = auth_prepare(&mut f, &conn, "not-launched", "workbench");
    let mut wrong = Fixture::owner();
    wrong["instanceId"] = json!("different-host");
    f.reject(
        "modelAuth/abortUnlaunched",
        json!({"operationId":"not-launched","owner":wrong,"launchProtocol":1}),
        "model_auth_owner_mismatch",
    );
    let cancelled = f.call(
        "modelAuth/abortUnlaunched",
        json!({"operationId":"not-launched","owner":Fixture::owner(),"launchProtocol":1}),
    );
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["steps"][0]["reason"], "launch_not_attempted");
    let current = f.call(
        "modelConnection/get",
        json!({"connectionId":conn["connectionId"]}),
    );
    assert!(
        current["authScopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["authGeneration"] == old["targetGeneration"] && s["state"] == "active")
    );
    assert!(current["authScopes"].as_array().unwrap().iter().any(|s|s["authGeneration"]==fresh["targetGeneration"] && s["state"]=="signed_out"));
    let launched = auth_prepare(&mut f, &conn, "already-launched", "workbench");
    let mut process = OwnedProcess::new();
    f.call("modelAuth/registerProcess",json!({"operationId":"already-launched","runtimeKind":"workbench","authGeneration":launched["targetGeneration"],"process":process.identity()}));
    f.reject(
        "modelAuth/abortUnlaunched",
        json!({"operationId":"already-launched","owner":Fixture::owner(),"launchProtocol":1}),
        "model_auth_already_launched",
    );
    process.reap();
    f.call("modelAuth/step",json!({"operationId":"already-launched","runtimeKind":"workbench","authGeneration":launched["targetGeneration"],"state":"signed_out"}));
    f.call(
        "modelAuth/cancel",
        json!({"operationId":"already-launched"}),
    );
    f.call("modelAuth/logoutPrepare",json!({"operationId":"old-scope-logout","connectionId":conn["connectionId"],"expectedConfigRevision":1,"owner":Fixture::owner(),"launchProtocol":1}));
    f.reject(
        "modelAuth/abortUnlaunched",
        json!({"operationId":"old-scope-logout","owner":Fixture::owner(),"launchProtocol":1}),
        "model_auth_already_launched",
    );
}
