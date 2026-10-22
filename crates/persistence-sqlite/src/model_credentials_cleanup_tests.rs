use super::*;

fn create_api(f: &mut Fixture, operation: &str) -> Value {
    let prepared = prepare_api(f, operation);
    f.call(
        "modelConnection/saveCommit",
        json!({"operationId":operation,"credentialRef":prepared["connection"]["secretRef"]}),
    )
}

fn rotate(f: &mut Fixture, connection: &Value, operation: &str, action: &str) -> Value {
    let prepared = f.call("modelConnection/savePrepare",json!({"operationId":operation,"connectionId":connection["connectionId"],"expectedConfigRevision":connection["configRevision"],"config":connection["config"],"credentialAction":action}));
    f.call(
        "modelConnection/saveCommit",
        json!({"operationId":operation,"credentialRef":prepared["connection"]["secretRef"]}),
    )
}

fn candidate(f: &mut Fixture, reference: &Value) -> Value {
    f.call("modelCredentialCleanup/list", json!({}))
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["credentialRef"] == *reference)
        .unwrap()
        .clone()
}

fn prepare_cleanup(f: &mut Fixture, item: &Value) -> Value {
    f.call(
        "modelCredentialCleanup/prepare",
        json!({"cleanupId":item["cleanupId"],"expectedReferenceDigest":item["referenceDigest"]}),
    )
}

fn ack(plan: &Value) -> Value {
    json!({"cleanupId":plan["cleanupId"],"expectedReferenceDigest":plan["referenceDigest"],"credentialRef":plan["credentialRef"]})
}

#[test]
fn rotated_cleared_and_deleted_references_remain_discoverable_and_cleanup_is_durable() {
    let mut f = Fixture::new();
    let first = create_api(&mut f, "initial-key");
    let second = rotate(&mut f, &first, "rotated-key", "replace");
    let old = candidate(&mut f, &first["secretRef"]);
    assert_eq!(old["canPrepare"], true);
    assert_eq!(old["sourceOperationIds"], json!(["initial-key"]));
    f.reject(
        "modelCredentialCleanup/ack",
        ack(&old),
        "model_credential_cleanup_not_prepared",
    );
    let plan = prepare_cleanup(&mut f, &old);
    assert_eq!(plan["state"], "pending");
    let mut reopened =
        SqlitePersistence::open(&f.root.join("contexthub.db"), "personal-test").unwrap();
    let all = reopened
        .personal_model_rpc(
            "modelCredentialCleanup/list",
            json!({}),
            PROFILE,
            FENCE,
            124,
        )
        .unwrap();
    assert_eq!(
        all.as_array()
            .unwrap()
            .iter()
            .find(|c| c["cleanupId"] == plan["cleanupId"])
            .unwrap()["plan"],
        plan
    );
    for field in ["credentialRef", "expectedReferenceDigest"] {
        let mut wrong = ack(&plan);
        wrong[field] = if field == "credentialRef" {
            json!({"service":"Codex Auth","account":"unrelated"})
        } else {
            json!("stale")
        };
        f.reject(
            "modelCredentialCleanup/ack",
            wrong,
            "model_credential_cleanup_receipt_mismatch",
        );
    }
    let confirmed = f.call("modelCredentialCleanup/ack", ack(&plan));
    assert_eq!(confirmed["state"], "confirmed");
    assert_eq!(f.call("modelCredentialCleanup/ack", ack(&plan)), confirmed);
    assert_eq!(prepare_cleanup(&mut f, &old), confirmed);
    assert_eq!(
        f.call(
            "modelConnection/get",
            json!({"connectionId":second["connectionId"]})
        ),
        second
    );
    let cleared = rotate(&mut f, &second, "cleared-key", "clear");
    assert!(cleared["secretRef"].is_null());
    assert_eq!(candidate(&mut f, &second["secretRef"])["canPrepare"], true);
    let third = rotate(&mut f, &cleared, "third-key", "replace");
    let impact = f.preflight(&third, "delete");
    f.call("modelConnection/deleteCommit",json!({"connectionId":third["connectionId"],"expectedConfigRevision":third["configRevision"],"impactDigest":impact["impactDigest"]}));
    assert_eq!(f.call("modelConnection/list", json!({})), json!([]));
    let deleted = candidate(&mut f, &third["secretRef"]);
    assert_eq!(deleted["deletedConnection"], true);
    assert_eq!(deleted["canPrepare"], true);
    let plan = prepare_cleanup(&mut f, &deleted);
    f.call("modelCredentialCleanup/ack", ack(&plan));
    assert_eq!(candidate(&mut f, &third["secretRef"])["state"], "confirmed");
}

#[test]
fn current_disabled_and_prepared_references_are_preserved_and_stale_impacts_are_rejected() {
    let mut f = Fixture::new();
    let first = create_api(&mut f, "initial-key");
    let active = candidate(&mut f, &first["secretRef"]);
    assert_eq!(active["canPrepare"], false);
    f.reject("modelCredentialCleanup/prepare",json!({"cleanupId":active["cleanupId"],"expectedReferenceDigest":active["referenceDigest"]}),"model_credential_cleanup_in_use");
    let impact = f.preflight(&first, "disable");
    let disabled = f.call("modelConnection/setEnabled",json!({"connectionId":first["connectionId"],"expectedConfigRevision":first["configRevision"],"enabled":false,"impactDigest":impact["impactDigest"]}));
    assert_eq!(candidate(&mut f, &first["secretRef"])["canPrepare"], false);
    f.call("modelConnection/savePrepare",json!({"operationId":"keep-pending","connectionId":disabled["connectionId"],"expectedConfigRevision":disabled["configRevision"],"config":disabled["config"],"credentialAction":"keep"}));
    rotate(&mut f, &disabled, "rotated-key", "replace");
    let pending = candidate(&mut f, &first["secretRef"]);
    assert!(
        pending["blockedReferences"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "prepared_change")
    );
    f.reject("modelCredentialCleanup/prepare",json!({"cleanupId":pending["cleanupId"],"expectedReferenceDigest":pending["referenceDigest"]}),"model_credential_cleanup_in_use");
    f.call(
        "modelConnection/saveAbort",
        json!({"operationId":"keep-pending"}),
    );
    f.reject("modelCredentialCleanup/prepare",json!({"cleanupId":pending["cleanupId"],"expectedReferenceDigest":pending["referenceDigest"]}),"model_credential_cleanup_impact_changed");
    let available = candidate(&mut f, &first["secretRef"]);
    assert_eq!(available["canPrepare"], true);
    prepare_cleanup(&mut f, &available);
}

#[test]
fn live_and_unconfirmed_leases_block_retired_references_until_explicit_release() {
    let mut f = Fixture::new();
    let first = create_api(&mut f, "initial-key");
    f.descriptor(&first);
    let owner = Fixture::owner();
    let lease = f.lease(&first, &owner);
    let unlaunched = f.lease(&first, &owner);
    let mut process = OwnedProcess::new();
    f.call("modelExecution/registerProcess",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"process":process.identity()}));
    rotate(&mut f, &first, "rotated-key", "replace");
    let blocked = candidate(&mut f, &first["secretRef"]);
    assert_eq!(blocked["activeLeaseIds"].as_array().unwrap().len(), 2);
    f.reject("modelCredentialCleanup/prepare",json!({"cleanupId":blocked["cleanupId"],"expectedReferenceDigest":blocked["referenceDigest"]}),"model_credential_cleanup_in_use");
    f.reject(
        "modelExecution/release",
        json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"]}),
        "model_execution_process_alive",
    );
    process.reap();
    f.call(
        "modelExecution/release",
        json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"]}),
    );
    assert_eq!(candidate(&mut f, &first["secretRef"])["canPrepare"], false);
    f.call("modelExecution/abortUnlaunched",json!({"leaseId":unlaunched["leaseId"],"snapshotDigest":unlaunched["snapshotDigest"],"owner":owner}));
    let available = candidate(&mut f, &first["secretRef"]);
    assert_eq!(available["canPrepare"], true);
    prepare_cleanup(&mut f, &available);
}

#[test]
fn unknown_native_owners_and_reservations_fail_closed_without_deleting_history() {
    let mut f = Fixture::new();
    let prepared = prepare_api(&mut f, "aborted-key");
    let aborted = f.call(
        "modelConnection/saveAbort",
        json!({"operationId":"aborted-key"}),
    );
    f.db.connection.execute("INSERT INTO ai_conversation_thread_bindings(conversation_id,binding_revision,generation,protocol_version,status,owner_instance_id,active_run_id,updated_at) VALUES('unknown-writer',1,1,1,'blocked','unknown-host','unknown-run',123)",[]).unwrap();
    let blocked = candidate(&mut f, &prepared["connection"]["secretRef"]);
    assert_eq!(
        blocked["blockedReferences"][0]["kind"],
        "unconfirmed_native_writer"
    );
    f.reject("modelCredentialCleanup/prepare",json!({"cleanupId":blocked["cleanupId"],"expectedReferenceDigest":blocked["referenceDigest"]}),"model_credential_cleanup_in_use");
    f.db.connection.execute("UPDATE ai_conversation_thread_bindings SET owner_instance_id=NULL,active_run_id=NULL WHERE conversation_id='unknown-writer'",[]).unwrap();
    let native_scope = json!({"profileId":PROFILE,"connectionId":prepared["connection"]["connectionId"],"configRevision":1,"authGeneration":prepared["connection"]["authGeneration"],"runtimeHomeScope":model_runtime_home_scope(PROFILE,prepared["connection"]["connectionId"].as_str().unwrap(),1),"runtimeKind":"workbench"});
    f.db.connection.execute("UPDATE ai_conversation_thread_bindings SET owner_instance_id='known-host',active_run_id='known-run',runtime_scope_json=?1 WHERE conversation_id='unknown-writer'",[native_scope.to_string()]).unwrap();
    assert_eq!(
        candidate(&mut f, &prepared["connection"]["secretRef"])["blockedReferences"][0]["kind"],
        "active_native_thread"
    );
    f.db.connection.execute("UPDATE ai_conversation_thread_bindings SET owner_instance_id=NULL,active_run_id=NULL WHERE conversation_id='unknown-writer'",[]).unwrap();
    let available = candidate(&mut f, &prepared["connection"]["secretRef"]);
    let plan = prepare_cleanup(&mut f, &available);
    f.call("modelCredentialCleanup/ack", ack(&plan));
    let confirmed = f.call(
        "modelConnection/changeStatus",
        json!({"operationId":"aborted-key"}),
    );
    assert_eq!(confirmed["credentialCleanup"], "confirmed");
    assert_eq!(confirmed["requestDigest"], aborted["requestDigest"]);
    assert_eq!(confirmed["connection"], aborted["connection"]);
    let count: i64 = f.db.connection.query_row("SELECT COUNT(*) FROM ai_conversation_thread_bindings WHERE conversation_id='unknown-writer'",[],|r| r.get(0)).unwrap();
    assert_eq!(count, 1);
    let preserved: String = f.db.connection.query_row("SELECT runtime_scope_json FROM ai_conversation_thread_bindings WHERE conversation_id='unknown-writer'",[],|r|r.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&preserved).unwrap(),
        native_scope
    );
    let tx = f.db.connection.transaction().unwrap();
    assert_eq!(
        credentials_cleanup::require_available(
            &tx,
            Some(&serde_json::from_value(plan["credentialRef"].clone()).unwrap())
        )
        .unwrap_err()
        .reason(),
        Some("model_credential_reference_cleared")
    );
    tx.rollback().unwrap();
}

#[test]
fn cleanup_and_rotation_compete_using_the_current_sqlite_reference_digest() {
    let mut f = Fixture::new();
    let first = create_api(&mut f, "initial-key");
    f.call("modelConnection/savePrepare",json!({"operationId":"rotated-key","connectionId":first["connectionId"],"expectedConfigRevision":1,"config":first["config"],"credentialAction":"replace"}));
    let staged = f.call(
        "modelConnection/changeStatus",
        json!({"operationId":"rotated-key"}),
    );
    let before = candidate(&mut f, &first["secretRef"]);
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
    let cleanup = spawn(
        "modelCredentialCleanup/prepare",
        json!({"cleanupId":before["cleanupId"],"expectedReferenceDigest":before["referenceDigest"]}),
    );
    let rotation = spawn(
        "modelConnection/saveCommit",
        json!({"operationId":"rotated-key","credentialRef":staged["connection"]["secretRef"]}),
    );
    let reason = cleanup.join().unwrap().unwrap_err();
    assert!(matches!(
        reason.as_str(),
        "model_credential_cleanup_in_use" | "model_credential_cleanup_impact_changed"
    ));
    let second = rotation.join().unwrap().unwrap();
    let available = candidate(&mut f, &first["secretRef"]);
    let plan = prepare_cleanup(&mut f, &available);
    let tx = f.db.connection.transaction().unwrap();
    assert_eq!(
        credentials_cleanup::require_available(
            &tx,
            Some(&serde_json::from_value(plan["credentialRef"].clone()).unwrap())
        )
        .unwrap_err()
        .reason(),
        Some("model_credential_cleanup_pending")
    );
    tx.rollback().unwrap();
    f.descriptor(&second);
    let lease = f.lease(&second, &Fixture::owner());
    assert_eq!(lease["snapshot"]["secretRef"], second["secretRef"]);
    f.call("modelExecution/abortUnlaunched",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"owner":lease["owner"]}));
    f.call("modelCredentialCleanup/ack", ack(&plan));
}

#[test]
fn cleanup_plans_cannot_be_forged_by_the_generic_ledger_writer() {
    let mut f = Fixture::new();
    let prepared = prepare_api(&mut f, "aborted-key");
    f.call(
        "modelConnection/saveAbort",
        json!({"operationId":"aborted-key"}),
    );
    let available = candidate(&mut f, &prepared["connection"]["secretRef"]);
    let plan = prepare_cleanup(&mut f, &available);
    let ledger = f.db.ai_business_ledger().unwrap();
    let id = entry_id("credentialCleanup", plan["cleanupId"].as_str().unwrap());
    let record = ledger["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap();
    let mut forged = record["value"].clone();
    forged["value"]["state"] = json!("confirmed");
    let request = json!({"commandId":"forged-global-cleanup","records":[{"collection":"entries","id":id,"expectedDigest":record["digest"],"position":0,"value":forged}]});
    assert_eq!(
        f.db.apply_ai_business_batch(&request, 125)
            .unwrap_err()
            .reason(),
        Some("ai_ledger_writer_scope_invalid")
    );
    assert_eq!(
        candidate(&mut f, &prepared["connection"]["secretRef"])["plan"],
        plan
    );
}

#[test]
fn foreign_or_malformed_credential_sources_are_rejected_before_becoming_cleanup_candidates() {
    let mut f = Fixture::new();
    let prepared = prepare_api(&mut f, "malformed-source");
    let id = entry_id("change", "malformed-source");
    let raw: String =
        f.db.connection
            .query_row(
                "SELECT record_json FROM ai_runtime_entries WHERE record_id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
    let original: Value = serde_json::from_str(&raw).unwrap();
    let connection = prepared["connection"]["connectionId"].as_str().unwrap();
    for reference in [
        json!({"service":"Codex Auth","account":prepared["connection"]["secretRef"]["account"]}),
        json!({"service":MODEL_CREDENTIAL_SERVICE,"account":format!("22222222222242228222222222222222:{connection}:1")}),
        json!({"service":MODEL_CREDENTIAL_SERVICE,"account":format!("{PROFILE}:{connection}:01")}),
        json!({"service":MODEL_CREDENTIAL_SERVICE,"account":format!("{PROFILE}:{connection}:2")}),
        json!({"service":MODEL_CREDENTIAL_SERVICE,"account":format!("{PROFILE}:not-a-connection:1")}),
    ] {
        let mut malformed = original.clone();
        malformed["value"]["connection"]["secretRef"] = reference;
        let hash = digest(&json!({"position":0,"value":malformed}));
        f.db.connection
            .execute(
                "UPDATE ai_runtime_entries SET record_json=?1,digest=?2 WHERE record_id=?3",
                params![malformed.to_string(), hash, id],
            )
            .unwrap();
        f.reject(
            "modelCredentialCleanup/list",
            json!({}),
            "model_credential_scope_invalid",
        );
    }
}

fn make_model_references_legacy(f: &mut Fixture) {
    let rows: Vec<(String, String)> = {
        let mut statement = f.db.connection.prepare("SELECT record_id,record_json FROM ai_runtime_entries WHERE record_id LIKE 'personalModel:%'").unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for (id, raw) in rows {
        let next = raw.replace(
            contexthub_protocol::MODEL_CREDENTIAL_SERVICE,
            contexthub_protocol::LEGACY_MODEL_CREDENTIAL_SERVICE,
        );
        let record: Value = serde_json::from_str(&next).unwrap();
        let value_digest = digest(&json!({"position":0,"value":record}));
        f.db.connection
            .execute(
                "UPDATE ai_runtime_entries SET record_json=?1,digest=?2 WHERE record_id=?3",
                params![next, value_digest, id],
            )
            .unwrap();
    }
}

#[test]
fn legacy_system_references_can_be_cancelled_and_replaced_but_never_locally_cleared() {
    let mut f = Fixture::new();
    let pending = prepare_api(&mut f, "old-pending");
    make_model_references_legacy(&mut f);
    let old = f.call(
        "modelConnection/changeStatus",
        json!({"operationId":"old-pending"}),
    );
    f.reject(
        "modelConnection/saveCommit",
        json!({"operationId":"old-pending","credentialRef":old["connection"]["secretRef"]}),
        "model_credential_reentry_required",
    );
    let cancelled = f.call(
        "modelConnection/saveAbort",
        json!({"operationId":"old-pending"}),
    );
    assert_eq!(cancelled["state"], "aborted");
    assert_ne!(cancelled["credentialCleanup"], "confirmed");
    let cleanup = candidate(&mut f, &cancelled["connection"]["secretRef"]);
    assert_eq!(cleanup["canPrepare"], false);
    f.reject("modelCredentialCleanup/prepare", json!({"cleanupId":cleanup["cleanupId"],"expectedReferenceDigest":cleanup["referenceDigest"]}), "model_credential_legacy_cleanup_required");
    let next = f.call("modelConnection/savePrepare", json!({"operationId":"local-reentry","connectionId":pending["connection"]["connectionId"],"config":pending["connection"]["config"],"credentialAction":"replace"}));
    assert_eq!(next["connection"]["authGeneration"], 2);
    assert_eq!(
        next["connection"]["secretRef"]["service"],
        MODEL_CREDENTIAL_SERVICE
    );
    let saved = f.call(
        "modelConnection/saveCommit",
        json!({"operationId":"local-reentry","credentialRef":next["connection"]["secretRef"]}),
    );
    make_model_references_legacy(&mut f);
    f.reject("modelConnection/savePrepare", json!({"operationId":"cannot-keep-old","connectionId":saved["connectionId"],"expectedConfigRevision":saved["configRevision"],"config":saved["config"],"credentialAction":"keep"}), "model_credential_reentry_required");
    let replacement = f.call("modelConnection/savePrepare", json!({"operationId":"replace-saved-old","connectionId":saved["connectionId"],"expectedConfigRevision":saved["configRevision"],"config":saved["config"],"credentialAction":"replace"}));
    assert_eq!(replacement["connection"]["authGeneration"], 3);
    assert_eq!(
        replacement["connection"]["secretRef"]["service"],
        MODEL_CREDENTIAL_SERVICE
    );
}
