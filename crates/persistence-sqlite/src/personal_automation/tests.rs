use super::*;
use contexthub_protocol::ModelExecutionPrepareParams;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
};

const PROFILE: &str = "11111111111141118111111111111111";
const FENCE: &str = "automation-test-instance";

struct Fixture {
    root: PathBuf,
    db: SqlitePersistence,
    owner: ModelExecutionOwner,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("automation-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&root).unwrap();
        let mut db =
            SqlitePersistence::open(&root.join("contexthub.db"), "automation-test").unwrap();
        db.begin_personal_profile_upgrade(PROFILE, FENCE).unwrap();
        db.finish_personal_profile_upgrade(PROFILE, FENCE, "test-manifest")
            .unwrap();
        let owner = ModelExecutionOwner {
            instance_id: "automation-host".into(),
            pid: std::process::id(),
            start_fingerprint: contexthub_platform_adapter::instance::process_start_fingerprint(
                std::process::id(),
            )
            .unwrap()
            .unwrap(),
        };
        let mut f = Self { root, db, owner };
        f.write_rule(None, 1, 100, "原配置");
        f
    }
    fn write_rule(&mut self, expected: Option<&str>, version: u32, due: i64, name: &str) -> Value {
        let value = json!({"entryId":"automationRules:rule","field":"automationRules","member":"rule","shape":"array","value":{"automationRuleId":"rule","ruleVersion":version,"enabled":true,"modelId":"test-model","nextRunAt":due,"name":name,"updatedAt":1,"lastError":null,"lastRunId":null}});
        self.db.apply_ai_business_batch(&json!({"commandId":uuid::Uuid::new_v4().simple().to_string(),"records":[{"collection":"entries","id":"automationRules:rule","position":7,"expectedDigest":expected,"value":value}]}),1).unwrap()
    }
    fn current(&self) -> PersonalAutomationRuleRecord {
        let connection = &self.db.connection;
        let (position,raw,digest):(i64,String,String)=connection.query_row("SELECT position,record_json,digest FROM ai_runtime_entries WHERE record_id='automationRules:rule'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        PersonalAutomationRuleRecord {
            id: "automationRules:rule".into(),
            position,
            digest,
            value: serde_json::from_str(&raw).unwrap(),
        }
    }
    fn request(&self, scheduled: bool) -> PersonalAutomationAdmitParams {
        PersonalAutomationAdmitParams {
            trigger_id: uuid::Uuid::new_v4().simple().to_string(),
            rule_id: "rule".into(),
            expected_rule_digest: self.current().digest,
            scheduled_due_at: scheduled.then_some(100),
            next_run_at: Some(300),
            owner: self.owner.clone(),
        }
    }
    fn call<T: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        p: &T,
    ) -> Result<R, PersistenceError> {
        serde_json::from_value(self.db.personal_automation_rpc(
            method,
            encode(p).unwrap(),
            PROFILE,
            200,
        )?)
        .map_err(|_| reject("test_decode_failed"))
    }
    fn admit(&mut self) -> PersonalAutomationAdmitResult {
        let request = self.request(false);
        self.call("personalAutomation/admit", &request).unwrap()
    }
    fn finish(&mut self, claim: &PersonalAutomationClaim) -> PersonalAutomationFinishResult {
        self.call(
            "personalAutomation/finish",
            &PersonalAutomationFinishParams {
                claim_id: claim.claim_id.clone(),
                owner: claim.owner.clone(),
                outcome: PersonalAutomationFinishOutcome::Failed,
                reason_code: Some("automation_preflight_failed".into()),
            },
        )
        .unwrap()
    }
    fn model(&mut self) -> ModelExecutionPrepareParams {
        let prepared=self.db.personal_model_rpc("modelConnection/savePrepare",json!({"operationId":"automation-model","config":{"name":"本地夹具","kind":"local","baseUrl":"http://127.0.0.1:11434/v1","localRuntime":"ollama","authMode":"none","enabled":true},"credentialAction":"clear"}),PROFILE,FENCE,1).unwrap();
        let c = self
            .db
            .personal_model_rpc(
                "modelConnection/saveCommit",
                json!({"operationId":prepared["operationId"]}),
                PROFILE,
                FENCE,
                1,
            )
            .unwrap();
        self.db.personal_model_rpc("modelDescriptor/save",json!({"connectionId":c["connectionId"],"modelId":"test-model","displayName":"本地夹具模型"}),PROFILE,FENCE,1).unwrap();
        self.db.personal_model_rpc("modelPurpose/save",json!({"binding":{"bindingId":"profile:automation","purpose":"automation","policy":"explicit","selection":{"connectionId":c["connectionId"],"modelId":"test-model"},"enabled":true,"revision":0,"consent":{"serviceName":"本机夹具","historyScope":"test","mayIncurCost":false,"confirmedAt":1}}}),PROFILE,FENCE,1).unwrap();
        ModelExecutionPrepareParams {
            owner: Some(self.owner.clone()),
            launch_protocol: 1,
            purpose: ModelPurpose::Automation,
            binding_id: Some("profile:automation".into()),
            selection: None,
            conversation_id: None,
        }
    }
    fn history(&mut self) -> PersonalAutomationTriggerPage {
        self.call(
            "personalAutomation/listTriggers",
            &PersonalAutomationListTriggersParams {
                rule_id: None,
                limit: 100,
                before_sequence: None,
            },
        )
        .unwrap()
    }
    fn seed_ledger_shapes(&mut self) {
        self.model();
        let records = json!([
            {"collection":"entries","id":"appearance","position":13,"expectedDigest":null,"value":{"entryId":"appearance","field":"appearance","shape":"scalar","value":{"theme":"dark"}}},
            {"collection":"entries","id":"agents:agent","position":27,"expectedDigest":null,"value":{"entryId":"agents:agent","field":"agents","member":"agent","shape":"array","value":{"agentId":"agent","enabled":true}}},
            {"collection":"entries","id":"draftAttachments:draft","position":31,"expectedDigest":null,"value":{"entryId":"draftAttachments:draft","field":"draftAttachments","member":"draft","shape":"object","value":[]}},
            {"collection":"conversations","id":"other-conversation","position":11,"expectedDigest":null,"value":{"conversationId":"other-conversation","context":{}}},
            {"collection":"messages","id":"other-message","position":19,"expectedDigest":null,"value":{"messageId":"other-message","conversationId":"other-conversation","contentDigest":"metadata-only"}},
            {"collection":"runs","id":"other-run","position":23,"expectedDigest":null,"value":{"runId":"other-run","conversationId":"other-conversation","status":"failed"}}
        ]);
        self.db
            .apply_ai_business_batch(
                &json!({"commandId":"all-ledger-shapes","records":records}),
                1,
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
#[ignore = "由独立子进程精确启动的拥有者夹具"]
fn owner_probe() {
    let mut byte = [0_u8; 1];
    assert_eq!(
        std::io::Read::read(&mut std::io::stdin(), &mut byte).unwrap(),
        0
    );
}

fn child_owner() -> (Child, ModelExecutionOwner) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "personal_automation::tests::owner_probe",
        "--ignored",
        "--nocapture",
    ]);
    let child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let owner = ModelExecutionOwner {
        instance_id: "other-automation-host".into(),
        pid: child.id(),
        start_fingerprint: contexthub_platform_adapter::instance::process_start_fingerprint(
            child.id(),
        )
        .unwrap()
        .unwrap(),
    };
    (child, owner)
}

fn assert_canonical_ledger(tx: &Transaction<'_>) {
    let mut entries = 0;
    for table in [
        "ai_runtime_entries",
        "ai_conversation_ledger",
        "ai_message_ledger",
        "ai_run_ledger",
    ] {
        let mut query = tx
            .prepare(&format!("SELECT position,record_json,digest FROM {table}"))
            .unwrap();
        let rows = query
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap();
        let mut count = 0;
        for row in rows {
            let (position, raw, actual) = row.unwrap();
            let value: Value = serde_json::from_str(&raw).unwrap();
            assert_eq!(
                actual,
                digest(&json!({"position":position,"value":value})).unwrap(),
                "{table} position={position}"
            );
            count += 1;
        }
        assert!(count > 0);
        if table == "ai_runtime_entries" {
            entries = count;
        }
    }
    assert!(entries >= 6);
}

#[test]
fn same_rule_cross_database_busy_skip_advances_once_and_replay_never_readmits() {
    let mut f = Fixture::new();
    let first = f.admit();
    let claim = first.claim.unwrap();
    let current = f.current();
    f.write_rule(Some(&current.digest), 2, 100, "占用期间新计划");
    let mut peer =
        SqlitePersistence::open(&f.root.join("contexthub.db"), "automation-peer").unwrap();
    let request = f.request(true);
    let result: PersonalAutomationAdmitResult = decode(
        peer.personal_automation_rpc(
            "personalAutomation/admit",
            encode(&request).unwrap(),
            PROFILE,
            200,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        result.trigger.outcome,
        PersonalAutomationTriggerOutcome::Skipped
    );
    assert!(result.claim.is_none());
    assert_eq!(result.rule_record.value["value"]["nextRunAt"], 300);
    assert_eq!(result.rule_record.position, 7);
    let replay: PersonalAutomationAdmitResult = decode(
        peer.personal_automation_rpc(
            "personalAutomation/admit",
            encode(&request).unwrap(),
            PROFILE,
            200,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(replay.replayed);
    assert_eq!(result.trigger, replay.trigger);
    assert_eq!(f.history().items.len(), 2);
    assert_eq!(
        f.finish(&claim).trigger.outcome,
        PersonalAutomationTriggerOutcome::Failed
    );
    assert!(f.admit().claim.is_some());
}

#[test]
fn scheduled_cas_and_failed_transaction_preserve_rule_and_claims() {
    let mut f = Fixture::new();
    let stale = f.request(true);
    let current = f.current();
    f.write_rule(Some(&current.digest), 2, 100, "另一宿主配置");
    assert_eq!(
        f.call::<_, PersonalAutomationAdmitResult>("personalAutomation/admit", &stale)
            .unwrap_err()
            .reason(),
        Some("automation_rule_changed")
    );
    assert!(f.history().items.is_empty());
    f.db.connection.execute_batch("CREATE TEMP TRIGGER deny_automation_receipt BEFORE INSERT ON durable_events WHEN NEW.object_type='personal_automation_trigger' BEGIN SELECT RAISE(ABORT,'deny'); END;").unwrap();
    let request = f.request(true);
    let before = f.current();
    assert!(
        f.call::<_, PersonalAutomationAdmitResult>("personalAutomation/admit", &request)
            .is_err()
    );
    assert_eq!(f.current(), before);
    assert_eq!(
        f.db.connection
            .query_row("SELECT COUNT(*) FROM personal_profile_writers", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}

#[test]
fn model_prepare_and_claim_binding_commit_together_and_replay_keeps_one_lease() {
    let mut f = Fixture::new();
    let execution = f.model();
    let claim = f.admit().claim.unwrap();
    let p = PersonalAutomationPrepareExecutionParams {
        claim_id: claim.claim_id.clone(),
        owner: f.owner.clone(),
        client_operation_id: "prepare-once".into(),
        execution,
    };
    let lease: ModelExecutionLease = f.call("personalAutomation/prepareExecution", &p).unwrap();
    let again: ModelExecutionLease = f.call("personalAutomation/prepareExecution", &p).unwrap();
    assert_eq!(lease, again);
    assert_eq!(f.db.connection.query_row("SELECT COUNT(*) FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:lease:*'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        f.call::<_, PersonalAutomationFinishResult>(
            "personalAutomation/finish",
            &PersonalAutomationFinishParams {
                claim_id: claim.claim_id.clone(),
                owner: f.owner.clone(),
                outcome: PersonalAutomationFinishOutcome::Failed,
                reason_code: None
            }
        )
        .unwrap_err()
        .reason(),
        Some("automation_claim_execution_unconfirmed")
    );
    f.db.personal_model_rpc(
        "modelExecution/abortUnlaunched",
        json!({"leaseId":lease.lease_id,"snapshotDigest":lease.snapshot_digest,"owner":f.owner}),
        PROFILE,
        FENCE,
        201,
    )
    .unwrap();
    assert!(f.finish(&claim).claim.released);
}

#[test]
fn current_rule_edits_survive_bind_and_finish_and_unknown_run_stays_occupied() {
    let mut f = Fixture::new();
    let claim = f.admit().claim.unwrap();
    let current = f.current();
    f.write_rule(Some(&current.digest), 2, 999, "新配置保留");
    let bound: PersonalAutomationBindResult = f
        .call(
            "personalAutomation/bind",
            &PersonalAutomationBindParams {
                claim_id: claim.claim_id.clone(),
                owner: f.owner.clone(),
                run_id: "run-a".into(),
            },
        )
        .unwrap();
    assert_eq!(
        bound.rule_record.as_ref().unwrap().value["value"]["name"],
        "新配置保留"
    );
    let run = json!({"runId":"run-a","automationRuleId":"rule","automationRuleVersion":1,"status":"failed","completedAt":220,"threadWriterActive":false,"errorReason":"mcp_tool_outcome_unconfirmed"});
    f.db.apply_ai_business_batch(&json!({"commandId":"terminal-run","records":[{"collection":"runs","id":"run-a","position":0,"expectedDigest":null,"value":run}]}),220).unwrap();
    let p = PersonalAutomationFinishParams {
        claim_id: claim.claim_id,
        owner: f.owner.clone(),
        outcome: PersonalAutomationFinishOutcome::Failed,
        reason_code: Some("mcp_tool_outcome_unconfirmed".into()),
    };
    assert_eq!(
        f.call::<_, PersonalAutomationFinishResult>("personalAutomation/finish", &p)
            .unwrap_err()
            .reason(),
        Some("automation_run_terminal_unconfirmed")
    );
    assert_eq!(f.current().value["value"]["nextRunAt"], 999);
    assert_eq!(
        f.admit().trigger.outcome,
        PersonalAutomationTriggerOutcome::Skipped
    );
}

#[test]
fn explicit_reconcile_requires_dead_owner_and_keeps_outcome_unknown() {
    let mut f = Fixture::new();
    let (mut child, owner) = child_owner();
    f.owner = owner;
    let claim = f.admit().claim.unwrap();
    let baseline = f.current();
    f.write_rule(Some(&baseline.digest), 2, 999, "退出前另一宿主的新配置");
    let p = PersonalAutomationResolveInterruptedParams {
        claim_id: claim.claim_id,
    };
    assert_eq!(
        f.call::<_, PersonalAutomationFinishResult>("personalAutomation/resolveInterrupted", &p)
            .unwrap_err()
            .reason(),
        Some("automation_claim_owner_alive")
    );
    child.stdin.take();
    child.wait().unwrap();
    let result: PersonalAutomationFinishResult =
        f.call("personalAutomation/resolveInterrupted", &p).unwrap();
    assert_eq!(
        result.trigger.outcome,
        PersonalAutomationTriggerOutcome::OutcomeUnconfirmed
    );
    assert!(result.claim.released);
    let current = f.current();
    assert_eq!(current.value["value"]["name"], "退出前另一宿主的新配置");
    assert_eq!(current.value["value"]["ruleVersion"], 2);
    assert_eq!(current.value["value"]["enabled"], false);
    assert!(current.value["value"]["nextRunAt"].is_null());
    assert_eq!(
        f.call::<_, PersonalAutomationFinishResult>("personalAutomation/resolveInterrupted", &p)
            .unwrap(),
        result
    );
    f.owner = ModelExecutionOwner {
        instance_id: "explicit-new-host".into(),
        pid: std::process::id(),
        start_fingerprint: contexthub_platform_adapter::instance::process_start_fingerprint(
            std::process::id(),
        )
        .unwrap()
        .unwrap(),
    };
    let scheduled = f.request(true);
    assert_eq!(
        f.call::<_, PersonalAutomationAdmitResult>("personalAutomation/admit", &scheduled)
            .unwrap_err()
            .reason(),
        Some("automation_schedule_changed")
    );
    assert_eq!(f.history().items.len(), 1);
}

#[test]
fn concurrent_stale_busy_trigger_is_recorded_without_changing_rule_or_authorization() {
    let mut f = Fixture::new();
    let stale = f.request(true);
    let first = f.admit();
    let current = f.current();
    f.write_rule(Some(&current.digest), 2, 999, "并发新配置");
    let before = f.current();
    let writer_count: i64 =
        f.db.connection
            .query_row("SELECT COUNT(*) FROM personal_profile_writers", [], |row| {
                row.get(0)
            })
            .unwrap();
    let mut peer = SqlitePersistence::open(&f.root.join("contexthub.db"), "stale-peer").unwrap();
    let result: PersonalAutomationAdmitResult = decode(
        peer.personal_automation_rpc(
            "personalAutomation/admit",
            encode(&stale).unwrap(),
            PROFILE,
            200,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        result.trigger.outcome,
        PersonalAutomationTriggerOutcome::Skipped
    );
    assert!(result.claim.is_none());
    assert_eq!(result.rule_record, before);
    assert_eq!(f.current(), before);
    assert_eq!(
        f.db.connection
            .query_row("SELECT COUNT(*) FROM personal_profile_writers", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        writer_count
    );
    assert_eq!(f.db.connection.query_row("SELECT COUNT(*) FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:lease:*'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(f.history().items.len(), 2);
    assert!(!first.claim.unwrap().released);
}

#[test]
fn failed_prepare_receipt_rolls_back_model_lease_and_claim_binding() {
    let mut f = Fixture::new();
    let execution = f.model();
    let claim = f.admit().claim.unwrap();
    f.db.connection.execute_batch("CREATE TEMP TRIGGER deny_preparation BEFORE INSERT ON durable_events WHEN NEW.object_type='personal_automation_trigger' BEGIN SELECT RAISE(ABORT,'deny'); END;").unwrap();
    let p = PersonalAutomationPrepareExecutionParams {
        claim_id: claim.claim_id,
        owner: f.owner.clone(),
        client_operation_id: "prepare-failed".into(),
        execution,
    };
    assert!(
        f.call::<_, ModelExecutionLease>("personalAutomation/prepareExecution", &p)
            .is_err()
    );
    assert_eq!(f.db.connection.query_row("SELECT COUNT(*) FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:lease:*'",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    let tx = f.db.connection.transaction().unwrap();
    let stored = read(&tx, &p.claim_id).unwrap().unwrap();
    assert!(stored.claim.unwrap().model_lease_ids.is_empty());
    assert!(stored.preparations.is_empty());
}

#[test]
fn explicit_reconcile_waits_for_exact_registered_model_process_exit() {
    let mut f = Fixture::new();
    let (mut owner_process, owner) = child_owner();
    f.owner = owner;
    let execution = f.model();
    let claim = f.admit().claim.unwrap();
    let p = PersonalAutomationPrepareExecutionParams {
        claim_id: claim.claim_id.clone(),
        owner: f.owner.clone(),
        client_operation_id: "model-with-process".into(),
        execution,
    };
    let lease: ModelExecutionLease = f.call("personalAutomation/prepareExecution", &p).unwrap();
    let (mut model_process, process) = child_owner();
    f.db.personal_model_rpc("modelExecution/registerProcess",json!({"leaseId":lease.lease_id,"snapshotDigest":lease.snapshot_digest,"process":{"pid":process.pid,"startFingerprint":process.start_fingerprint}}),PROFILE,FENCE,201).unwrap();
    owner_process.stdin.take();
    owner_process.wait().unwrap();
    let resolve = PersonalAutomationResolveInterruptedParams {
        claim_id: claim.claim_id,
    };
    assert_eq!(
        f.call::<_, PersonalAutomationFinishResult>(
            "personalAutomation/resolveInterrupted",
            &resolve
        )
        .unwrap_err()
        .reason(),
        Some("model_execution_process_alive")
    );
    model_process.stdin.take();
    model_process.wait().unwrap();
    let result: PersonalAutomationFinishResult = f
        .call("personalAutomation/resolveInterrupted", &resolve)
        .unwrap();
    assert_eq!(
        result.trigger.outcome,
        PersonalAutomationTriggerOutcome::OutcomeUnconfirmed
    );
    assert!(result.claim.released);
    let tx = f.db.connection.transaction().unwrap();
    assert!(model_lease(&tx, &lease.lease_id).unwrap().released);
}

#[test]
fn missing_registered_process_proof_is_not_released_when_owner_exits() {
    let mut f = Fixture::new();
    let (mut owner_process, owner) = child_owner();
    f.owner = owner;
    let execution = f.model();
    let claim = f.admit().claim.unwrap();
    let p = PersonalAutomationPrepareExecutionParams {
        claim_id: claim.claim_id.clone(),
        owner: f.owner.clone(),
        client_operation_id: "unknown-before-register".into(),
        execution,
    };
    let lease: ModelExecutionLease = f.call("personalAutomation/prepareExecution", &p).unwrap();
    owner_process.stdin.take();
    owner_process.wait().unwrap();
    assert_eq!(
        f.call::<_, PersonalAutomationFinishResult>(
            "personalAutomation/resolveInterrupted",
            &PersonalAutomationResolveInterruptedParams {
                claim_id: claim.claim_id
            }
        )
        .unwrap_err()
        .reason(),
        Some("model_execution_ownership_unconfirmed")
    );
    let tx = f.db.connection.transaction().unwrap();
    assert!(!model_lease(&tx, &lease.lease_id).unwrap().released);
    assert_eq!(
        crate::profile_writers::active_writers(&tx, PROFILE)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn restored_trigger_is_read_only_and_new_rule_keeps_canonical_revision_digest() {
    let mut f = Fixture::new();
    f.seed_ledger_shapes();
    let claim = f.admit().claim.unwrap();
    f.finish(&claim);
    let candidate = f
        .root
        .join("cn.cilai.personal.workbench/business/profiles")
        .join(format!(
            "restore-candidate-{}",
            uuid::Uuid::new_v4().simple()
        ));
    contexthub_platform_adapter::security::ensure_restricted_directory(&candidate).unwrap();
    let file = candidate.join("contexthub.db");
    f.db.snapshot_personal_data_database(&file).unwrap();
    let mut restored = SqlitePersistence::open(&file, "restored-automation").unwrap();
    let raw: String = restored
        .connection
        .query_row(
            "SELECT record_json FROM ai_runtime_entries WHERE record_id='automationRules:rule'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    restored
        .connection
        .execute(
            "UPDATE ai_runtime_entries SET digest=?1 WHERE record_id='automationRules:rule'",
            [format!("{:x}", Sha256::digest(raw.as_bytes()))],
        )
        .unwrap();
    let tx = restored.connection.transaction().unwrap();
    assert_eq!(
        rule(&tx, "rule").unwrap_err().reason(),
        Some("automation_rule_record_invalid")
    );
    tx.commit().unwrap();
    let target = uuid::Uuid::new_v4().simple().to_string();
    restored
        .prepare_restored_personal_profile(PROFILE, &target)
        .unwrap();
    restored
        .begin_personal_profile_upgrade(&target, "restored-instance")
        .unwrap();
    restored
        .finish_personal_profile_upgrade(&target, "restored-instance", "new-manifest")
        .unwrap();
    let page: PersonalAutomationTriggerPage = decode(
        restored
            .personal_automation_rpc(
                "personalAutomation/listTriggers",
                json!({"ruleId":"rule","limit":10,"beforeSequence":null}),
                &target,
                400,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page.items[0].record.profile_id, PROFILE);
    assert!(page.items[0].record.read_only_after_restore);
    assert_eq!(
        page.items[0].record.outcome,
        PersonalAutomationTriggerOutcome::Failed
    );
    assert_eq!(
        restored
            .personal_automation_rpc(
                "personalAutomation/resolveInterrupted",
                json!({"claimId":claim.claim_id}),
                &target,
                400
            )
            .unwrap_err()
            .reason(),
        Some("automation_claim_restored_read_only")
    );
    let tx = restored.connection.transaction().unwrap();
    assert_canonical_ledger(&tx);
    let record = rule(&tx, "rule").unwrap().unwrap();
    assert_eq!(
        record.digest,
        digest(&json!({"position":record.position,"value":record.value})).unwrap()
    );
    assert_eq!(record.value["value"]["enabled"], false);
    assert!(
        crate::profile_writers::active_writers(&tx, &target)
            .unwrap()
            .is_empty()
    );
    tx.commit().unwrap();
    assert!(!f.history().items[0].record.read_only_after_restore);
}

#[test]
fn maintenance_blocks_new_claims_but_allows_known_preflight_failure_settlement() {
    let mut f = Fixture::new();
    let claim = f.admit().claim.unwrap();
    let request = f.request(false);
    let tx = f.db.connection.transaction().unwrap();
    crate::maintenance_operations::register(
        &tx,
        &contexthub_protocol::PersonalMaintenanceOperationStatus {
            operation_id: "automation-maintenance".into(),
            profile_id: PROFILE.into(),
            kind: "backup".into(),
            state: "prepared".into(),
            preview_digest: "preview".into(),
        },
        201,
    )
    .unwrap();
    tx.commit().unwrap();
    f.db.personal_writer_rpc(
        "personalMaintenance/acquire",
        json!({"operationId":"automation-maintenance","previewDigest":"preview","owner":f.owner}),
        PROFILE,
        FENCE,
        201,
    )
    .unwrap();
    assert_eq!(
        f.call::<_, PersonalAutomationAdmitResult>("personalAutomation/admit", &request)
            .unwrap_err()
            .reason(),
        Some("personal_data_maintenance_active")
    );
    assert!(f.finish(&claim).claim.released);
    assert_eq!(f.history().items.len(), 1);
}

#[test]
fn terminal_run_with_unknown_application_write_cannot_release_claim() {
    let mut f = Fixture::new();
    let claim = f.admit().claim.unwrap();
    let _: PersonalAutomationBindResult = f
        .call(
            "personalAutomation/bind",
            &PersonalAutomationBindParams {
                claim_id: claim.claim_id.clone(),
                owner: f.owner.clone(),
                run_id: "unknown-write-run".into(),
            },
        )
        .unwrap();
    f.db.apply_ai_business_batch(&json!({"commandId":"known-run-unknown-write","records":[
        {"collection":"runs","id":"unknown-write-run","position":0,"expectedDigest":null,"value":{"runId":"unknown-write-run","automationRuleId":"rule","automationRuleVersion":1,"status":"failed","completedAt":200,"threadWriterActive":false,"errorReason":"automation_failed"}},
        {"collection":"entries","id":"permissionRequests:unknown-write","position":0,"expectedDigest":null,"value":{"entryId":"permissionRequests:unknown-write","field":"permissionRequests","member":"unknown-write","shape":"array","value":{"permissionRequestId":"unknown-write","runId":"unknown-write-run","status":"approved","applicationWrite":{"executionStatus":"outcome_unknown"}}}}
    ]}),200).unwrap();
    let p = PersonalAutomationFinishParams {
        claim_id: claim.claim_id,
        owner: f.owner.clone(),
        outcome: PersonalAutomationFinishOutcome::Failed,
        reason_code: Some("automation_failed".into()),
    };
    assert_eq!(
        f.call::<_, PersonalAutomationFinishResult>("personalAutomation/finish", &p)
            .unwrap_err()
            .reason(),
        Some("automation_permission_outcome_unconfirmed")
    );
    assert_eq!(
        f.admit().trigger.outcome,
        PersonalAutomationTriggerOutcome::Skipped
    );
}

#[test]
fn unknown_report_is_visible_without_releasing_occupancy_or_allowing_known_overwrite() {
    let mut f = Fixture::new();
    let (mut child, owner) = child_owner();
    f.owner = owner;
    let claim = f.admit().claim.unwrap();
    let p = PersonalAutomationFinishParams {
        claim_id: claim.claim_id.clone(),
        owner: f.owner.clone(),
        outcome: PersonalAutomationFinishOutcome::OutcomeUnconfirmed,
        reason_code: Some("automation_execution_receipt_unconfirmed".into()),
    };
    let result: PersonalAutomationFinishResult = f.call("personalAutomation/finish", &p).unwrap();
    assert!(!result.claim.released);
    assert!(result.trigger.finished_at.is_none());
    assert_eq!(
        result.trigger.outcome,
        PersonalAutomationTriggerOutcome::OutcomeUnconfirmed
    );
    let page = f.history();
    assert_eq!(page.items[0].claim_released, Some(false));
    let known = PersonalAutomationFinishParams {
        outcome: PersonalAutomationFinishOutcome::Failed,
        ..p
    };
    assert_eq!(
        f.call::<_, PersonalAutomationFinishResult>("personalAutomation/finish", &known)
            .unwrap_err()
            .reason(),
        Some("automation_claim_outcome_unconfirmed")
    );
    child.stdin.take();
    child.wait().unwrap();
    let _: PersonalAutomationFinishResult = f
        .call(
            "personalAutomation/resolveInterrupted",
            &PersonalAutomationResolveInterruptedParams {
                claim_id: claim.claim_id,
            },
        )
        .unwrap();
    let page = f.history();
    assert_eq!(page.items[0].claim_released, Some(true));
    assert_eq!(
        page.items[0].record.outcome,
        PersonalAutomationTriggerOutcome::OutcomeUnconfirmed
    );
}

#[test]
fn failed_resolution_receipt_preserves_occupancy_and_the_existing_schedule() {
    let mut f = Fixture::new();
    let (mut child, owner) = child_owner();
    f.owner = owner;
    let claim = f.admit().claim.unwrap();
    let before = f.current();
    f.db.connection.execute_batch("CREATE TEMP TRIGGER deny_resolution BEFORE INSERT ON durable_events WHEN NEW.object_type='personal_automation_trigger' BEGIN SELECT RAISE(ABORT,'deny'); END;").unwrap();
    child.stdin.take();
    child.wait().unwrap();
    let p = PersonalAutomationResolveInterruptedParams {
        claim_id: claim.claim_id.clone(),
    };
    assert!(
        f.call::<_, PersonalAutomationFinishResult>("personalAutomation/resolveInterrupted", &p)
            .is_err()
    );
    assert_eq!(f.current(), before);
    let tx = f.db.connection.transaction().unwrap();
    let stored = read(&tx, &claim.claim_id).unwrap().unwrap();
    assert!(!stored.claim.unwrap().released);
    assert_eq!(
        crate::profile_writers::active_writers(&tx, PROFILE)
            .unwrap()
            .len(),
        1
    );
}
