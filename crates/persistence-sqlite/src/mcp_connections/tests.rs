use super::*;
use contexthub_protocol::{McpLocalPermissions, McpNetworkConfig};
const PROFILE: &str = "11111111111141118111111111111111";
const FENCE: &str = "mcp-fixture-server";
struct Fixture {
    root: std::path::PathBuf,
    db: SqlitePersistence,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "cilai-mcp-fixture-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("contexthub.db"), "mcp-fixture").unwrap();
        db.begin_personal_profile_upgrade(PROFILE, FENCE).unwrap();
        db.finish_personal_profile_upgrade(PROFILE, FENCE, "isolated_fixture")
            .unwrap();
        Self { root, db }
    }
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.db
            .personal_mcp_rpc(method, params, PROFILE, FENCE, 123)
            .unwrap()
    }
    fn reject(&mut self, method: &str, params: Value, reason: &str) {
        assert_eq!(
            self.db
                .personal_mcp_rpc(method, params, PROFILE, FENCE, 123)
                .unwrap_err()
                .reason(),
            Some(reason)
        );
    }
    fn config(auth: &str) -> Value {
        json!({"name":"本机隔离夹具","source":{"kind":"remote_http","url":"http://127.0.0.1:9/mcp"},"authMode":auth,"environmentVariables":[],"network":{"mode":"direct","proxyUrl":null}})
    }
    fn save(&mut self, auth: &str) -> Value {
        let p=self.call("mcpConnection/savePrepare",json!({"operationId":uuid::Uuid::new_v4().simple().to_string(),"connectionId":null,"expectedRevision":null,"config":Self::config(auth),"credentialAction":if auth=="none" {"clear"} else {"replace"}}));
        self.call("mcpConnection/saveCommit",json!({"operationId":p["operationId"],"credentialRef":p["connection"]["credentialRef"]}))
    }
    fn owner() -> Value {
        json!({"instanceId":"isolated_fixture","pid":std::process::id(),"startFingerprint":contexthub_platform_adapter::instance::process_start_fingerprint(std::process::id()).unwrap().unwrap()})
    }
    fn prepare(&mut self, conn: &Value, intent: &str, tool: Option<&str>) -> Value {
        self.call("mcpExecution/prepare",json!({"connectionId":conn["connectionId"],"expectedRevision":conn["revision"],"intent":intent,"purpose":if intent=="tool_call" {json!("foreground")}else{Value::Null},"toolName":tool,"catalogDigest":if intent=="tool_call" {conn["approvedCatalogDigest"].clone()}else{Value::Null},"sourceConsent":true,"owner":Self::owner(),"launchProtocol":1}))
    }
    fn simulated_validated(&mut self, conn: &Value) -> Value {
        let lease = self.prepare(conn, "validation", None);
        let record=self.call("mcpExecution/reportValidation",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"state":"passed","reasonCode":"isolated_fixture","protocolVersion":"2025-06-18","serverName":"isolated_fixture","serverVersion":"1","sourceFingerprint":"isolated_fixture","tools":[{"name":"echo","description":"Fixture only","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}));
        self.call("mcpExecution/finish",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"outcome":"success","requestSent":false}));
        record
    }
    fn grant(&mut self, conn: &Value) -> Value {
        let validated = self.simulated_validated(conn);
        self.call("mcpConnection/grant",json!({"connectionId":conn["connectionId"],"expectedRevision":conn["revision"],"validationId":validated["validationId"],"approvedToolNames":["echo"],"purposes":["foreground"],"sourceConsent":true}))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn staged_save_is_durable_disabled_and_cannot_commit_another_secret_namespace() {
    let mut f = Fixture::new();
    let request = json!({"operationId":"save-once","connectionId":null,"expectedRevision":null,"config":Fixture::config("bearer"),"credentialAction":"replace"});
    let prepared = f.call("mcpConnection/savePrepare", request.clone());
    assert_eq!(f.call("mcpConnection/list", json!({})), json!([]));
    assert_eq!(f.call("mcpConnection/savePrepare", request), prepared);
    let mut other = prepared["connection"]["credentialRef"].clone();
    other["service"] = json!("cn.cilai.personal.workbench.model");
    f.reject(
        "mcpConnection/saveCommit",
        json!({"operationId":"save-once","credentialRef":other}),
        "mcp_credentials_reference_mismatch",
    );
    let mut reopened =
        SqlitePersistence::open(&f.root.join("contexthub.db"), "mcp-fixture").unwrap();
    let status = reopened
        .personal_mcp_rpc(
            "mcpConnection/changeStatus",
            json!({"operationId":"save-once"}),
            PROFILE,
            FENCE,
            124,
        )
        .unwrap();
    assert_eq!(status["state"], "prepared");
    let saved = f.call(
        "mcpConnection/saveCommit",
        json!({"operationId":"save-once","credentialRef":prepared["connection"]["credentialRef"]}),
    );
    assert_eq!(saved["enabled"], false);
    assert_eq!(saved["approvedToolNames"], json!([]));
}

#[test]
fn cancelled_secret_generation_never_reuses_an_account_and_cleanup_is_reserved_and_idempotent() {
    let mut f = Fixture::new();
    let original = f.save("bearer");
    let p=f.call("mcpConnection/savePrepare",json!({"operationId":"rotate-cancel","connectionId":original["connectionId"],"expectedRevision":original["revision"],"config":original["config"],"credentialAction":"replace"}));
    f.call(
        "mcpConnection/saveAbort",
        json!({"operationId":"rotate-cancel"}),
    );
    let plan = f.call(
        "mcpCredentialCleanup/prepare",
        json!({"operationId":"rotate-cancel","credentialRef":p["connection"]["credentialRef"]}),
    );
    assert_eq!(plan["state"], "prepared");
    let next=f.call("mcpConnection/savePrepare",json!({"operationId":"rotate-next","connectionId":original["connectionId"],"expectedRevision":original["revision"],"config":original["config"],"credentialAction":"replace"}));
    assert_ne!(
        next["connection"]["credentialRef"],
        p["connection"]["credentialRef"]
    );
    assert!(
        next["connection"]["authGeneration"].as_u64().unwrap()
            > p["connection"]["authGeneration"].as_u64().unwrap()
    );
    let ack =
        json!({"cleanupId":plan["cleanupId"],"expectedReferenceDigest":plan["referenceDigest"]});
    let done = f.call("mcpCredentialCleanup/confirm", ack.clone());
    assert_eq!(done["state"], "confirmed");
    assert_eq!(f.call("mcpCredentialCleanup/confirm", ack), done);
    assert_eq!(
        f.call(
            "mcpConnection/changeStatus",
            json!({"operationId":"rotate-cancel"})
        )["cleanedReferences"],
        json!([p["connection"]["credentialRef"]])
    );
}

#[test]
fn changed_service_needs_its_own_new_secret_and_unconfirmed_tools_never_enable() {
    let mut f = Fixture::new();
    let conn = f.save("bearer");
    let mut config = conn["config"].clone();
    config["source"]["url"] = json!("https://example.invalid/mcp");
    f.reject("mcpConnection/savePrepare",json!({"operationId":"changed-source","connectionId":conn["connectionId"],"expectedRevision":conn["revision"],"config":config,"credentialAction":"keep"}),"mcp_credentials_reentry_required");
    f.reject("mcpConnection/grant",json!({"connectionId":conn["connectionId"],"expectedRevision":conn["revision"],"validationId":"unknown","approvedToolNames":["echo"],"purposes":["foreground"],"sourceConsent":true}),"mcp_validation_required");
    let validation = f.simulated_validated(&conn);
    f.reject("mcpConnection/grant",json!({"connectionId":conn["connectionId"],"expectedRevision":conn["revision"],"validationId":validation["validationId"],"approvedToolNames":["delete_everything"],"purposes":["foreground"],"sourceConsent":true}),"mcp_tool_not_validated");
    assert_eq!(
        f.call(
            "mcpConnection/get",
            json!({"connectionId":conn["connectionId"]})
        )["enabled"],
        false
    );
}

#[test]
fn disable_revokes_old_snapshot_but_keeps_actual_remote_outcome_and_writer_lifetime_separate() {
    let mut f = Fixture::new();
    let initial = f.save("none");
    let conn = f.grant(&initial);
    let lease = f.prepare(&conn, "tool_call", Some("echo"));
    let disabled = f.call(
        "mcpConnection/disable",
        json!({"connectionId":conn["connectionId"],"expectedRevision":conn["revision"]}),
    );
    f.reject(
        "mcpExecution/validate",
        json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"]}),
        "mcp_revision_changed",
    );
    assert_eq!(disabled["enabled"], false);
    let done=f.call("mcpExecution/finish",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"outcome":"outcome_unconfirmed","requestSent":true}));
    assert_eq!(done["state"], "finished");
    assert_eq!(done["outcome"], "outcome_unconfirmed");
    let writer =
        f.db.personal_writer_rpc(
            "profileWriter/read",
            json!({"leaseId":lease["writerLeaseId"]}),
            PROFILE,
            FENCE,
            124,
        )
        .unwrap();
    assert_eq!(writer["state"], "released");
    assert_eq!(writer["remoteOutcomeUnconfirmed"], true);
    f.reject(
        "mcpTool/snapshot",
        json!({"purpose":"foreground","connectionIds":[conn["connectionId"]]}),
        "mcp_connection_not_authorized",
    );
}

#[test]
fn credentials_and_reserved_environment_cannot_be_imported_as_ordinary_source_configuration() {
    for url in [
        "http://example.invalid/mcp",
        "https://user:secret@example.invalid/mcp",
        "https://example.invalid/mcp?token=secret",
    ] {
        let mut config: McpConnectionConfig = decode(Fixture::config("none")).unwrap();
        config.source = McpSource::RemoteHttp { url: url.into() };
        assert!(config_valid(&config).is_err());
    }
    let config = McpConnectionConfig {
        name: "local".into(),
        source: McpSource::LocalCommand {
            program: "/usr/bin/env".into(),
            args: Vec::new(),
            cwd: "/private/tmp".into(),
            permissions: McpLocalPermissions::default(),
        },
        auth_mode: McpAuthMode::Environment,
        environment_variables: vec!["CODEX_HOME".into()],
        network: McpNetworkConfig::default(),
    };
    assert!(config_valid(&config).is_err());
    for argument in [
        "--api-key=secret",
        "--token",
        "--access_token=secret",
        "--header=Authorization: Bearer secret",
        "https://user:secret@example.invalid/path",
    ] {
        let mut candidate = config.clone();
        candidate.auth_mode = McpAuthMode::None;
        candidate.environment_variables.clear();
        if let McpSource::LocalCommand { args, .. } = &mut candidate.source {
            args.push(argument.into());
        }
        assert!(config_valid(&candidate).is_err(), "{argument}");
    }
    let mut candidate = config;
    candidate.auth_mode = McpAuthMode::None;
    candidate.environment_variables.clear();
    assert!(config_valid(&candidate).is_ok());
    candidate.network.mode = ModelNetworkMode::SystemProxy;
    assert_eq!(
        config_valid(&candidate).unwrap_err().reason(),
        Some("mcp_local_proxy_not_supported")
    );
    candidate.network.mode = ModelNetworkMode::CustomProxy;
    candidate.network.proxy_url = Some("http://127.0.0.1:8080".into());
    assert_eq!(
        config_valid(&candidate).unwrap_err().reason(),
        Some("mcp_local_proxy_not_supported")
    );
}

#[test]
fn current_disabled_credentials_cannot_be_deleted_and_aborted_keep_does_not_clean_current() {
    let mut f = Fixture::new();
    let conn = f.save("bearer");
    let kept=f.call("mcpConnection/savePrepare",json!({"operationId":"keep-abort","connectionId":conn["connectionId"],"expectedRevision":conn["revision"],"config":conn["config"],"credentialAction":"keep"}));
    assert_eq!(kept["connection"]["credentialRef"], conn["credentialRef"]);
    let aborted = f.call(
        "mcpConnection/saveAbort",
        json!({"operationId":"keep-abort"}),
    );
    assert_eq!(aborted["cleanupReferences"], json!([]));
    f.reject(
        "mcpCredentialCleanup/prepare",
        json!({"operationId":"keep-abort","credentialRef":conn["credentialRef"]}),
        "mcp_credential_cleanup_not_requested",
    );
    assert_eq!(
        f.call(
            "mcpConnection/get",
            json!({"connectionId":conn["connectionId"]})
        )["credentialRef"],
        conn["credentialRef"]
    );
}

#[test]
fn stale_save_commit_cannot_restore_a_disabled_connection_or_overwrite_its_revision() {
    let mut f = Fixture::new();
    let conn = f.save("bearer");
    let prepared=f.call("mcpConnection/savePrepare",json!({"operationId":"stale-rotation","connectionId":conn["connectionId"],"expectedRevision":conn["revision"],"config":conn["config"],"credentialAction":"replace"}));
    let disabled = f.call(
        "mcpConnection/disable",
        json!({"connectionId":conn["connectionId"],"expectedRevision":conn["revision"]}),
    );
    f.reject("mcpConnection/saveCommit",json!({"operationId":"stale-rotation","credentialRef":prepared["connection"]["credentialRef"]}),"mcp_revision_changed");
    assert_eq!(
        f.call(
            "mcpConnection/get",
            json!({"connectionId":conn["connectionId"]})
        ),
        disabled
    );
}

#[test]
fn revalidation_that_revokes_a_changed_source_records_the_new_revision_for_explicit_regrant() {
    let mut f = Fixture::new();
    let original = f.save("none");
    let granted = f.grant(&original);
    let lease = f.prepare(&granted, "validation", None);
    let report=f.call("mcpExecution/reportValidation",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"state":"passed","reasonCode":null,"protocolVersion":"2025-06-18","serverName":"fixture","serverVersion":"2","sourceFingerprint":"changed_source_fixture","tools":[{"name":"echo","description":"changed","inputSchema":{"type":"object"},"annotations":null}]}));
    f.call("mcpExecution/finish",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"outcome":"success","requestSent":false}));
    let current = f.call(
        "mcpConnection/get",
        json!({"connectionId":granted["connectionId"]}),
    );
    assert_eq!(current["enabled"], false);
    assert_eq!(report["revision"], current["revision"]);
    let renewed=f.call("mcpConnection/grant",json!({"connectionId":current["connectionId"],"expectedRevision":current["revision"],"validationId":report["validationId"],"approvedToolNames":["echo"],"purposes":["foreground"],"sourceConsent":true}));
    assert_eq!(renewed["enabled"], true);
}

#[test]
fn unknown_remote_tool_outcome_releases_transport_but_revokes_future_automatic_calls_atomically() {
    let mut f = Fixture::new();
    let conn = f.save("none");
    let granted = f.grant(&conn);
    let lease = f.prepare(&granted, "tool_call", Some("echo"));
    let finished=f.call("mcpExecution/finish",json!({"leaseId":lease["leaseId"],"snapshotDigest":lease["snapshotDigest"],"outcome":"outcome_unconfirmed","requestSent":true,"reasonCode":"mcp_tool_response_invalid"}));
    assert_eq!(finished["state"], "finished");
    assert_eq!(finished["reasonCode"], "mcp_tool_response_invalid");
    let current = f.call(
        "mcpConnection/get",
        json!({"connectionId":granted["connectionId"]}),
    );
    assert_eq!(current["enabled"], false);
    assert_eq!(current["approvedToolNames"], json!([]));
    f.reject("mcpExecution/prepare",json!({"connectionId":current["connectionId"],"expectedRevision":current["revision"],"intent":"tool_call","purpose":"foreground","toolName":"echo","catalogDigest":granted["approvedCatalogDigest"],"sourceConsent":false,"owner":Fixture::owner(),"launchProtocol":1}),"mcp_tool_not_authorized");
}

fn replace_stored_connection(fixture: &mut Fixture, conn: &Value) {
    fixture.db.connection.execute("UPDATE personal_mcp_records SET record_json=?1 WHERE profile_id=?2 AND kind='connection' AND record_id=?3", rusqlite::params![conn.to_string(), PROFILE, conn["connectionId"].as_str().unwrap()]).unwrap();
}
#[test]
fn legacy_reference_needs_explicit_reentry_and_old_cleanup_stays_unconfirmed() {
    let mut f = Fixture::new();
    let mut old = f.save("bearer");
    assert_eq!(old["credentialRef"]["service"], MCP_CREDENTIAL_SERVICE);
    old["credentialRef"]["service"] = json!(LEGACY_MCP_CREDENTIAL_SERVICE);
    replace_stored_connection(&mut f, &old);
    f.reject("mcpConnection/savePrepare",json!({"operationId":"legacy-keep","connectionId":old["connectionId"],"expectedRevision":old["revision"],"config":old["config"],"credentialAction":"keep"}),"mcp_credential_reentry_required");
    f.reject("mcpExecution/prepare",json!({"connectionId":old["connectionId"],"expectedRevision":old["revision"],"intent":"validation","purpose":null,"toolName":null,"catalogDigest":null,"sourceConsent":true,"owner":Fixture::owner(),"launchProtocol":1}),"mcp_credential_reentry_required");
    let replacement = f.call("mcpConnection/savePrepare",json!({"operationId":"legacy-replace","connectionId":old["connectionId"],"expectedRevision":old["revision"],"config":old["config"],"credentialAction":"replace"}));
    assert_eq!(
        replacement["connection"]["credentialRef"]["service"],
        MCP_CREDENTIAL_SERVICE
    );
    assert_eq!(replacement["connection"]["authGeneration"], 2);
    assert_eq!(
        replacement["cleanupReferences"],
        json!([old["credentialRef"]])
    );
    f.call("mcpConnection/saveCommit",json!({"operationId":"legacy-replace","credentialRef":replacement["connection"]["credentialRef"]}));
    f.reject(
        "mcpCredentialCleanup/prepare",
        json!({"operationId":"legacy-replace","credentialRef":old["credentialRef"]}),
        "mcp_credential_legacy_cleanup_required",
    );
    assert_eq!(f.call("mcpCredentialCleanup/list", json!({})), json!([]));
    assert_eq!(
        f.call(
            "mcpConnection/changeStatus",
            json!({"operationId":"legacy-replace"})
        )["cleanedReferences"],
        json!([])
    );
}
#[test]
fn old_pending_save_and_cleanup_confirmation_cannot_bypass_legacy_reentry() {
    let mut f = Fixture::new();
    let mut prepared=f.call("mcpConnection/savePrepare",json!({"operationId":"old-pending","connectionId":null,"expectedRevision":null,"config":Fixture::config("bearer"),"credentialAction":"replace"}));
    prepared["connection"]["credentialRef"]["service"] = json!(LEGACY_MCP_CREDENTIAL_SERVICE);
    f.db.connection.execute("UPDATE personal_mcp_records SET record_json=?1 WHERE profile_id=?2 AND kind='change' AND record_id='old-pending'",rusqlite::params![prepared.to_string(),PROFILE]).unwrap();
    f.reject("mcpConnection/saveCommit",json!({"operationId":"old-pending","credentialRef":prepared["connection"]["credentialRef"]}),"mcp_credential_reentry_required");
    let plan = McpCredentialCleanupPlan {
        cleanup_id: "legacy-plan".into(),
        profile_id: PROFILE.into(),
        credential_ref: serde_json::from_value(prepared["connection"]["credentialRef"].clone())
            .unwrap(),
        state: McpCredentialCleanupState::Prepared,
        source_operation_ids: vec!["old-pending".into()],
        reference_digest: "fixed".into(),
    };
    let tx = f.db.connection.transaction().unwrap();
    put(&tx, PROFILE, "cleanup", &plan.cleanup_id, &plan, 123).unwrap();
    tx.commit().unwrap();
    f.reject(
        "mcpCredentialCleanup/confirm",
        json!({"cleanupId":plan.cleanup_id,"expectedReferenceDigest":plan.reference_digest}),
        "mcp_credential_legacy_cleanup_required",
    );
    assert_eq!(
        f.call("mcpCredentialCleanup/list", json!({}))[0]["state"],
        "prepared"
    );
}
