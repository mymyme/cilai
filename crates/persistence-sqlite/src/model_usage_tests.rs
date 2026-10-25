use super::*;
use contexthub_protocol::{ModelUsageTokens, PersonalMaintenanceOperationStatus};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
};

const PROFILE: &str = "11111111111141118111111111111111";
const INSTANCE: &str = "usage-test-instance";
struct Fixture {
    root: PathBuf,
    db: SqlitePersistence,
    child: Child,
    lease: Value,
}
impl Fixture {
    fn call(db: &mut SqlitePersistence, method: &str, p: Value) -> Value {
        db.personal_model_rpc(method, p, PROFILE, INSTANCE, 1)
            .unwrap()
    }
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("model-usage-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("contexthub.db"), "usage").unwrap();
        db.begin_personal_profile_upgrade(PROFILE, INSTANCE)
            .unwrap();
        db.finish_personal_profile_upgrade(PROFILE, INSTANCE, "test-manifest")
            .unwrap();
        let prepared = Self::call(
            &mut db,
            "modelConnection/savePrepare",
            json!({"operationId":"usage-connection","config":{"name":"本机用量夹具","kind":"local","baseUrl":"http://127.0.0.1:11434/v1","localRuntime":"ollama","authMode":"none","enabled":true},"credentialAction":"clear"}),
        );
        let connection = Self::call(
            &mut db,
            "modelConnection/saveCommit",
            json!({"operationId":prepared["operationId"]}),
        );
        Self::call(
            &mut db,
            "modelDescriptor/save",
            json!({"connectionId":connection["connectionId"],"modelId":"usage-model","displayName":"本机用量模型"}),
        );
        Self::call(
            &mut db,
            "modelPurpose/save",
            json!({"binding":{"bindingId":"profile:memory","purpose":"memory","policy":"explicit","selection":{"connectionId":connection["connectionId"],"modelId":"usage-model"},"enabled":true,"revision":0,"consent":{"serviceName":"本机组件","historyScope":"test","mayIncurCost":false,"confirmedAt":1}}}),
        );
        let pid = std::process::id();
        let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
            .unwrap()
            .unwrap();
        let lease = Self::call(
            &mut db,
            "modelExecution/prepare",
            json!({"owner":{"instanceId":"usage-owner","pid":pid,"startFingerprint":fingerprint},"launchProtocol":1,"purpose":"memory"}),
        );
        #[cfg(unix)]
        let mut command = {
            let mut value = Command::new("/bin/sh");
            value.args(["-c", "read line"]);
            value
        };
        #[cfg(windows)]
        let mut command = {
            let mut value = Command::new("cmd");
            value.args(["/C", "set /p usage_input="]);
            value
        };
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = contexthub_platform_adapter::instance::process_start_fingerprint(child.id())
            .unwrap()
            .unwrap();
        Self::call(
            &mut db,
            "modelExecution/registerProcess",
            json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"process":{"pid":child.id(),"startFingerprint":start}}),
        );
        Self {
            root,
            db,
            child,
            lease,
        }
    }
    fn begin(&mut self) -> ModelUsageRecord {
        self.db
            .begin_model_usage(
                PROFILE,
                &ModelUsageBeginParams {
                    request_id: uuid::Uuid::new_v4().simple().to_string(),
                    lease_id: self.lease["leaseId"].as_str().unwrap().into(),
                    snapshot_digest: self.lease["snapshotDigest"].as_str().unwrap().into(),
                    scope_id: "native-memory-fixture".into(),
                },
                2,
            )
            .unwrap()
    }
    fn finish(r: &ModelUsageRecord) -> ModelUsageFinishParams {
        ModelUsageFinishParams {
            request_id: r.request_id.clone(),
            lease_id: r.lease_id.clone(),
            snapshot_digest: r.snapshot_digest.clone(),
            state: ModelUsageState::Completed,
            response_id_sha256: Some("a".repeat(64)),
            tokens: Some(ModelUsageTokens {
                input_tokens: Some(2),
                output_tokens: Some(3),
                ..ModelUsageTokens::default()
            }),
            cost: None,
            reason_code: None,
        }
    }
    fn release(&mut self) {
        let _ = self.child.kill();
        self.child.wait().unwrap();
        Self::call(
            &mut self.db,
            "modelExecution/release",
            json!({"leaseId":self.lease["leaseId"],"snapshotDigest":self.lease["snapshotDigest"]}),
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn model_usage_requires_a_real_registered_scope_and_cannot_reuse_a_request() {
    let mut f = Fixture::new();
    let r = f.begin();
    assert_eq!(r.purpose, ModelPurpose::Memory);
    assert_eq!(r.model_id, "usage-model");
    let mut p = ModelUsageBeginParams {
        request_id: r.request_id.clone(),
        lease_id: r.lease_id.clone(),
        snapshot_digest: r.snapshot_digest.clone(),
        scope_id: r.scope_id.clone(),
    };
    assert_eq!(
        f.db.begin_model_usage(PROFILE, &p, 3).unwrap_err().reason(),
        Some("model_usage_request_reused")
    );
    p.request_id = uuid::Uuid::new_v4().simple().to_string();
    p.snapshot_digest = "changed".into();
    assert_eq!(
        f.db.begin_model_usage(PROFILE, &p, 3).unwrap_err().reason(),
        Some("model_execution_snapshot_mismatch")
    );
    assert!(f.db.model_usage(PROFILE, &p.request_id).is_err());
}

#[test]
fn model_usage_accepts_a_prior_receipt_after_revocation_but_never_rewrites_it() {
    let mut f = Fixture::new();
    let r = f.begin();
    let connection = &f.lease["snapshot"]["connectionId"];
    Fixture::call(
        &mut f.db,
        "modelPurpose/save",
        json!({"binding":{"bindingId":"profile:memory","purpose":"memory","policy":"explicit","selection":{"connectionId":connection,"modelId":"usage-model"},"enabled":false,"revision":1,"consent":null},"expectedRevision":1}),
    );
    let p = Fixture::finish(&r);
    let done = f.db.finish_model_usage(PROFILE, &p, 3).unwrap();
    assert_eq!(done.tokens.as_ref().unwrap().cached_input_tokens, None);
    assert_eq!(done.cost, None);
    assert_eq!(f.db.finish_model_usage(PROFILE, &p, 4).unwrap(), done);
    let mut changed = p;
    changed.tokens.as_mut().unwrap().input_tokens = Some(99);
    assert_eq!(
        f.db.finish_model_usage(PROFILE, &changed, 5)
            .unwrap_err()
            .reason(),
        Some("model_usage_terminal_conflict")
    );
}

#[test]
fn model_usage_maintenance_blocks_new_requests_without_discharging_existing_evidence() {
    let mut f = Fixture::new();
    let r = f.begin();
    let tx = f.db.connection.transaction().unwrap();
    crate::maintenance_operations::register(
        &tx,
        &PersonalMaintenanceOperationStatus {
            operation_id: "usage-maintenance".into(),
            profile_id: PROFILE.into(),
            kind: "backup".into(),
            state: "prepared".into(),
            preview_digest: "preview".into(),
        },
        3,
    )
    .unwrap();
    tx.commit().unwrap();
    let pid = std::process::id();
    let fp = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
        .unwrap()
        .unwrap();
    f.db.personal_writer_rpc("personalMaintenance/acquire",json!({"operationId":"usage-maintenance","previewDigest":"preview","owner":{"instanceId":"usage-maintenance-owner","pid":pid,"startFingerprint":fp}}),PROFILE,INSTANCE,3).unwrap();
    let p = ModelUsageBeginParams {
        request_id: uuid::Uuid::new_v4().simple().to_string(),
        lease_id: r.lease_id.clone(),
        snapshot_digest: r.snapshot_digest.clone(),
        scope_id: r.scope_id.clone(),
    };
    assert!(f.db.begin_model_usage(PROFILE, &p, 4).is_err());
    assert_eq!(
        f.db.finish_model_usage(PROFILE, &Fixture::finish(&r), 5)
            .unwrap()
            .state,
        ModelUsageState::Completed
    );
    assert!(
        !Fixture::call(&mut f.db, "modelExecution/list", json!({}))[0]["released"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn model_usage_pages_hold_one_event_bound_while_new_receipts_arrive() {
    let mut f = Fixture::new();
    let first = f.begin();
    let second = f.begin();
    let page =
        f.db.list_model_usage(
            PROFILE,
            &ModelUsageListParams {
                before_sequence: None,
                through_sequence: None,
                limit: 1,
            },
        )
        .unwrap();
    assert_eq!(page.records[0].request_id, second.request_id);
    f.db.finish_model_usage(PROFILE, &Fixture::finish(&first), 4)
        .unwrap();
    let rest =
        f.db.list_model_usage(
            PROFILE,
            &ModelUsageListParams {
                before_sequence: page.next_before_sequence,
                through_sequence: Some(page.snapshot_sequence),
                limit: 1,
            },
        )
        .unwrap();
    assert_eq!(rest.records[0].request_id, first.request_id);
    assert_eq!(rest.records[0].state, ModelUsageState::Started);
    assert!(rest.next_before_sequence.is_none());
    assert_eq!(
        f.db.model_usage(PROFILE, &first.request_id).unwrap().state,
        ModelUsageState::Completed
    );
}

#[test]
fn model_usage_backup_and_restore_keep_history_without_execution_authority() {
    let mut f = Fixture::new();
    let r = f.begin();
    let p = Fixture::finish(&r);
    f.db.finish_model_usage(PROFILE, &p, 3).unwrap();
    f.release();
    let profiles = f.root.join("cn.cilai.personal.workbench/business/profiles");
    let candidate = profiles.join(format!(
        "restore-candidate-{}",
        uuid::Uuid::new_v4().simple()
    ));
    contexthub_platform_adapter::security::ensure_restricted_directory(&candidate).unwrap();
    let file = candidate.join("contexthub.db");
    f.db.snapshot_personal_data_database(&file).unwrap();
    let mut restored = SqlitePersistence::open(&file, "restore-usage").unwrap();
    let target = uuid::Uuid::new_v4().simple().to_string();
    restored
        .prepare_restored_personal_profile(PROFILE, &target)
        .unwrap();
    let history = restored.model_usage(&target, &r.request_id).unwrap();
    assert_eq!(history.source_profile_id, PROFILE);
    assert_eq!(history.state, ModelUsageState::Completed);
    assert!(history.read_only_after_restore);
    assert_eq!(history.tokens, p.tokens);
    assert_eq!(
        restored
            .finish_model_usage(&target, &p, 5)
            .unwrap_err()
            .reason(),
        Some("model_usage_restored_read_only")
    );
    assert!(
        !f.db
            .model_usage(PROFILE, &r.request_id)
            .unwrap()
            .read_only_after_restore
    );
    assert_eq!(restored.connection.query_row("SELECT COUNT(*) FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:lease:*'",[],|row|row.get::<_,u32>(0)).unwrap(),0);
}

#[test]
fn model_usage_missing_finish_and_invalid_cost_never_become_zero_or_completed() {
    let mut f = Fixture::new();
    let r = f.begin();
    let mut p = Fixture::finish(&r);
    p.cost = Some(contexthub_protocol::ModelUsageCost {
        amount: "0.125".into(),
        currency: String::new(),
        source: "upstream.responses.usage.cost".into(),
    });
    assert_eq!(
        f.db.finish_model_usage(PROFILE, &p, 3)
            .unwrap_err()
            .reason(),
        Some("model_usage_request_invalid")
    );
    let reopened = SqlitePersistence::open(&f.root.join("contexthub.db"), "usage-reopen").unwrap();
    let pending = reopened.model_usage(PROFILE, &r.request_id).unwrap();
    assert_eq!(pending.state, ModelUsageState::Started);
    assert_eq!(pending.tokens, None);
    assert_eq!(pending.cost, None);
}

#[test]
fn model_usage_failed_finish_commit_keeps_the_durable_unknown_start() {
    let mut f = Fixture::new();
    let r = f.begin();
    f.db.connection.execute_batch("CREATE TEMP TRIGGER usage_finish_failure BEFORE INSERT ON durable_events WHEN NEW.event_type='personal_model.usage_finished' BEGIN SELECT RAISE(FAIL,'injected write failure'); END;").unwrap();
    assert!(
        f.db.finish_model_usage(PROFILE, &Fixture::finish(&r), 3)
            .is_err()
    );
    let record = f.db.model_usage(PROFILE, &r.request_id).unwrap();
    assert_eq!(record.state, ModelUsageState::Started);
    assert_eq!(record.tokens, None);
    assert!(
        !Fixture::call(&mut f.db, "modelExecution/list", json!({}))[0]["released"]
            .as_bool()
            .unwrap()
    );
}
