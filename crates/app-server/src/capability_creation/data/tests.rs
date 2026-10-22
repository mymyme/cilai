use super::super::*;
use super::*;
use contexthub_persistence_sqlite::SqlitePersistence;
use serde_json::json;

const INSTANCE: &str = "application-data-test";
struct Fixture {
    base: PathBuf,
    profile: PathBuf,
    id: String,
    project: String,
    services: AppServerServices,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "cilai-application-data-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let id = uuid::Uuid::new_v4().simple().to_string();
        let profile = base
            .join("cn.cilai.personal.workbench/business/profiles")
            .join(&id);
        fs::create_dir_all(&profile).unwrap();
        let mut db =
            SqlitePersistence::open(&profile.join("contexthub.db"), "application-data-test")
                .unwrap();
        db.begin_personal_profile_upgrade(&id, INSTANCE).unwrap();
        db.finish_personal_profile_upgrade(&id, INSTANCE, "test-manifest")
            .unwrap();
        let mut services = AppServerServices::with_data_dir(Some(db), Some(profile.clone()));
        let mut request: CapabilityCreationCreateParams = serde_json::from_value(json!({
            "title":"个人资料应用", "businessGoal":"保存业务资料并显示内容", "targetType":"web_mcp_application",
            "conversationId":null,"requestedPermissions":["personal_file.read"],
            "acceptanceExamples":[{"input":"打开", "expected":"显示已保存内容"}],
            "sourceFiles":Self::files("export const version=1;"),
            "clientOperationId":"create", "requestFingerprint":""
        })).unwrap();
        request.request_fingerprint = capability_creation_request_fingerprint(&request);
        let project = services
            .capability_creation_create(&request, 1)
            .unwrap()
            .project
            .creation_project_id;
        let mut f = Self {
            base,
            profile,
            id,
            project,
            services,
        };
        f.upgrade("initial");
        fs::write(f.active().join("state.json"), "original-data").unwrap();
        f
    }
    fn files(code: &str) -> Value {
        json!([{"path":"web/index.html","content":"<h1>个人应用</h1>"}, {"path":"mcp/server.mjs","content":code}])
    }
    fn project_record(&self) -> CapabilityCreationProjectRecord {
        self.services
            .persistence()
            .unwrap()
            .capability_creation_project(&self.project)
            .unwrap()
            .unwrap()
    }
    fn build(&mut self, code: &str) {
        let request = serde_json::from_value(json!({
            "creationProjectId":self.project,"expectedRevision":self.project_record().state_revision,
            "clientOperationId":uuid::Uuid::new_v4().to_string(), "title":"新版本", "businessGoal":"升级后继续保存资料",
            "requestedPermissions":["personal_file.read"], "sourceFiles":Self::files(code)
        })).unwrap();
        self.services
            .personal_capability_build(&request, 3)
            .unwrap();
    }
    fn upgrade_request(&mut self, client: &str) -> Value {
        let editor = self
            .services
            .personal_capability_editor(
                &PersonalApplicationOpenParams {
                    creation_project_id: self.project.clone(),
                },
                3,
            )
            .unwrap();
        json!({"creationProjectId":self.project,"expectedRevision":self.project_record().state_revision,
            "clientOperationId":client,"packageDigest":editor["candidateDigest"],"confirmedPermissions":["personal_file.read"]})
    }
    fn upgrade(&mut self, client: &str) -> Operation {
        let request = self.upgrade_request(client);
        test_support::upgrade(&mut self.services, request, INSTANCE, 4).unwrap();
        self.operation(client)
    }
    fn operation(&self, client: &str) -> Operation {
        self.services
            .persistence()
            .unwrap()
            .personal_application_data_operations(&self.id)
            .unwrap()
            .into_iter()
            .find(|r| r.operation.client_operation_id == client)
            .unwrap()
            .operation
    }
    fn active(&self) -> PathBuf {
        self.profile.join(
            self.services
                .persistence()
                .unwrap()
                .personal_application_data_slot(&self.id, &self.project)
                .unwrap()
                .unwrap()
                .relative_path,
        )
    }
    fn prepare(
        &mut self,
        client: &str,
        action: Action,
        digest: &str,
        source: Option<String>,
    ) -> Prepare {
        Prepare {
            creation_project_id: self.project.clone(),
            expected_revision: self.project_record().state_revision,
            package_digest: digest.into(),
            client_operation_id: client.into(),
            action,
            target_operation_id: source,
        }
    }
    fn guarded_candidate(&mut self, client: &str) -> (Value, Execute) {
        let request = self.upgrade_request(client);
        let p = self.prepare(
            client,
            Action::Upgrade,
            request["packageDigest"].as_str().unwrap(),
            None,
        );
        let execute = test_support::guarded(&mut self.services, &p, INSTANCE);
        (request, execute)
    }
    fn execute_upgrade(
        &mut self,
        mut request: Value,
        execute: &Execute,
    ) -> Result<Value, ServiceError> {
        request["dataChange"] = serde_json::to_value(execute).unwrap();
        self.services
            .personal_capability_upgrade(&serde_json::from_value(request).unwrap(), 5)
    }
    fn sql(&self, sql: &str) {
        rusqlite::Connection::open(self.profile.join("contexthub.db"))
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.services.persistence.take();
        fs::remove_dir_all(&self.base).unwrap();
    }
}

#[test]
fn upgrade_forks_data_and_installation_pointer_together_and_replay_does_not_fork_again() {
    let mut f = Fixture::new();
    let original = f.active();
    f.build("export const version=2;");
    let request = f.upgrade_request("upgrade");
    let op = f.upgrade("upgrade");
    let upgraded = f.active();
    assert_ne!(original, upgraded);
    assert!(upgraded.starts_with(f.profile.join("personal-capability-slots").join(&f.project)));
    assert_eq!(
        fs::read_to_string(original.join("state.json")).unwrap(),
        "original-data"
    );
    assert_eq!(
        fs::read_to_string(upgraded.join("state.json")).unwrap(),
        "original-data"
    );
    assert_eq!(op.state, State::Completed);
    assert!(op.snapshot_digest.is_some());
    assert_eq!(
        f.project_record().package_digest.as_ref(),
        Some(&op.package_digest)
    );
    let replay = f
        .services
        .personal_capability_upgrade(&serde_json::from_value(request).unwrap(), 6)
        .unwrap();
    assert_eq!(replay["installedDigest"], op.package_digest);
    assert_eq!(f.active(), upgraded);
    assert_eq!(f.operation("upgrade"), op);
}

#[test]
fn activation_failure_preserves_original_binding_and_same_operation_never_retries_copy() {
    let mut f = Fixture::new();
    let original = f.active();
    let old_package = f.project_record().package_digest;
    f.build("export const version=2;");
    let (request, execute) = f.guarded_candidate("fail-activation");
    f.sql("CREATE TRIGGER fail_data_switch BEFORE UPDATE ON personal_application_data_pointers BEGIN SELECT RAISE(ABORT,'test failure'); END;");
    assert!(f.execute_upgrade(request.clone(), &execute).is_err());
    assert!(
        f.profile
            .join("personal-capabilities")
            .join(&f.project)
            .join(
                request["packageDigest"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("sha256:")
            )
            .is_dir()
    );
    assert_eq!(f.active(), original);
    assert_eq!(f.project_record().package_digest, old_package);
    assert_eq!(f.operation("fail-activation").state, State::Incomplete);
    assert_eq!(
        fs::read_to_string(original.join("state.json")).unwrap(),
        "original-data"
    );
    f.sql("DROP TRIGGER fail_data_switch;");
    assert_eq!(
        f.execute_upgrade(request, &execute).unwrap_err().reason,
        "personal_application_data_operation_unconfirmed"
    );
    test_support::release(&mut f.services, &execute, INSTANCE);
}

#[test]
fn missing_quiescence_and_changed_snapshot_do_not_replace_original_data() {
    let mut f = Fixture::new();
    let original = f.active();
    f.build("export const version=2;");
    let (request, mut execute) = f.guarded_candidate("missing-proof");
    let held = execute.clone();
    execute.quiescence = None;
    assert_eq!(
        f.execute_upgrade(request, &execute).unwrap_err().reason,
        "personal_data_quiescence_unconfirmed"
    );
    assert_eq!(f.operation("missing-proof").state, State::Incomplete);
    assert_eq!(f.active(), original);
    test_support::release(&mut f.services, &held, INSTANCE);
    let op = f.upgrade("second-attempt-new-operation");
    fs::write(original.join("state.json"), "changed-after-upgrade").unwrap();
    let p = f.prepare(
        "rollback-changed",
        Action::Rollback,
        op.source_package_digest.as_ref().unwrap(),
        Some(op.operation_id),
    );
    let execute = test_support::guarded(&mut f.services, &p, INSTANCE);
    assert_eq!(
        f.services
            .application_data_rollback(&execute, 8)
            .unwrap_err()
            .reason,
        "personal_application_data_retained_changed"
    );
    assert_eq!(f.operation("rollback-changed").state, State::Incomplete);
    assert_ne!(f.active(), original);
    test_support::release(&mut f.services, &execute, INSTANCE);
}

#[test]
fn restored_operation_remains_readable_and_cannot_execute_or_become_rollback_source() {
    let mut f = Fixture::new();
    f.build("export const version=2;");
    let op = f.upgrade("restored-upgrade");
    f.sql(&format!("UPDATE personal_application_data_operations SET record_json=json_set(record_json,'$.operation.readOnlyAfterRestore',json('true'),'$.operation.sourceProfileId','old-source') WHERE operation_id='{}';",op.operation_id));
    assert!(f.operation("restored-upgrade").read_only_after_restore);
    let p = f.prepare(
        "rollback-restored",
        Action::Rollback,
        op.source_package_digest.as_ref().unwrap(),
        Some(op.operation_id.clone()),
    );
    assert_eq!(
        f.services
            .personal_application_data_rpc(
                "personalApplicationData/prepare",
                serde_json::to_value(p).unwrap(),
                9
            )
            .unwrap_err()
            .reason,
        "personal_application_data_rollback_source_invalid"
    );
    for method in [
        "personalApplicationData/cancel",
        "personalApplicationData/resolveInterrupted",
    ] {
        assert_eq!(
            f.services
                .personal_application_data_rpc(
                    method,
                    json!({"operationId":op.operation_id,"previewDigest":op.preview_digest}),
                    10
                )
                .unwrap_err()
                .reason,
            "personal_application_data_restored_read_only"
        );
    }
}

#[test]
#[ignore = "requires explicitly supplied personal Node 24 runtime and real platform sandbox"]
fn real_node_failed_upgrade_keeps_original_data_and_explicit_rollback_restores_both_pointers() {
    let node = PathBuf::from(
        std::env::var_os("CILAI_PERSONAL_TEST_NODE").expect("explicit personal Node24 runtime"),
    );
    let mut f = Fixture::new();
    let old_project = f.project_record();
    let original = f.active();
    f.build(&format!("import fs from 'node:fs'; fs.writeFileSync('state.json','broken-new-version'); try {{ fs.writeFileSync({},'corrupt-old-data'); process.exit(91); }} catch (error) {{ if (error.code!=='ERR_ACCESS_DENIED' && error.code!=='EACCES' && error.code!=='EPERM') throw error; }} process.exit(9);",serde_json::to_string(&original.join("state.json").to_str().unwrap()).unwrap()));
    let op = f.upgrade("node-failing-upgrade");
    let candidate = f.active();
    let package = f.profile.join(
        f.services
            .persistence()
            .unwrap()
            .personal_capability_installation(&f.project)
            .unwrap()
            .unwrap()
            .installed_relative_path,
    );
    let mut launch = contexthub_platform_adapter::managed_component::command(
        &node,
        &package,
        &candidate,
        &std::env::current_exe().unwrap(),
    )
    .unwrap();
    let output = launch
        .command
        .current_dir(&candidate)
        .env_clear()
        .env("HOME", &candidate)
        .env("TMPDIR", &candidate)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(9),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(candidate.join("state.json")).unwrap(),
        "broken-new-version"
    );
    assert_eq!(
        fs::read_to_string(original.join("state.json")).unwrap(),
        "original-data"
    );
    let p = f.prepare(
        "explicit-rollback",
        Action::Rollback,
        old_project.package_digest.as_ref().unwrap(),
        Some(op.operation_id),
    );
    let execute = test_support::guarded(&mut f.services, &p, INSTANCE);
    let result = f.services.application_data_rollback(&execute, 9).unwrap();
    assert_eq!(result["state"], "completed");
    test_support::release(&mut f.services, &execute, INSTANCE);
    assert_eq!(f.active(), original);
    assert_eq!(
        f.project_record().package_digest,
        old_project.package_digest
    );
    assert_eq!(f.project_record().business_goal, old_project.business_goal);
    assert_eq!(
        f.project_record().acceptance_examples_json,
        old_project.acceptance_examples_json
    );
    assert!(candidate.exists());
    assert_eq!(
        fs::read_to_string(f.active().join("state.json")).unwrap(),
        "original-data"
    );
}

fn owner_probe() -> (
    std::process::Child,
    contexthub_protocol::ModelExecutionOwner,
) {
    use contexthub_protocol::ModelExecutionOwner;
    use std::process::{Command, Stdio};
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "capability_creation::data::tests::application_data_owner_probe",
            "--ignored",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let owner = ModelExecutionOwner {
        instance_id: "actual-child-owner".into(),
        pid: child.id(),
        start_fingerprint: contexthub_platform_adapter::instance::process_start_fingerprint(
            child.id(),
        )
        .unwrap()
        .unwrap(),
    };
    (child, owner)
}

#[test]
fn interrupted_copy_requires_the_real_owner_to_exit_and_does_not_reapply_or_release_fence() {
    let mut f = Fixture::new();
    let original = f.active();
    f.build("export const version=2;");
    let request = f.upgrade_request("interrupted-copy");
    let p = f.prepare(
        "interrupted-copy",
        Action::Upgrade,
        request["packageDigest"].as_str().unwrap(),
        None,
    );
    let op: Operation = serde_json::from_value(
        f.services
            .personal_application_data_rpc(
                "personalApplicationData/prepare",
                serde_json::to_value(&p).unwrap(),
                3,
            )
            .unwrap(),
    )
    .unwrap();
    let (mut child, owner) = owner_probe();
    let execute = test_support::fence(&mut f.services, op.clone(), INSTANCE, owner);
    f.services
        .prepare_application_data_copy(
            &execute,
            &Activation {
                client: &p.client_operation_id,
                project: &p.creation_project_id,
                digest: &p.package_digest,
                revision: p.expected_revision,
                action: p.action,
            },
            4,
        )
        .unwrap();
    let confirm = json!({"operationId":op.operation_id,"previewDigest":op.preview_digest});
    assert_eq!(
        f.services
            .personal_application_data_rpc(
                "personalApplicationData/resolveInterrupted",
                confirm.clone(),
                5
            )
            .unwrap_err()
            .reason,
        "personal_maintenance_owner_alive"
    );
    drop(child.stdin.take());
    assert!(child.wait().unwrap().success());
    let resolved = f
        .services
        .personal_application_data_rpc("personalApplicationData/resolveInterrupted", confirm, 6)
        .unwrap();
    assert_eq!(resolved["state"], "incomplete");
    assert_eq!(f.active(), original);
    assert_eq!(
        fs::read_to_string(original.join("state.json")).unwrap(),
        "original-data"
    );
    let db = rusqlite::Connection::open(f.profile.join("contexthub.db")).unwrap();
    let state: String = db
        .query_row(
            "SELECT state FROM personal_maintenance_fences WHERE profile_id=?1 AND operation_id=?2",
            rusqlite::params![&f.id, &op.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "draining");
    let record = f
        .services
        .persistence()
        .unwrap()
        .personal_application_data_operation(&f.id, &op.operation_id)
        .unwrap();
    assert_eq!(
        fs::read_to_string(
            f.profile
                .join(record.target_slot.relative_path)
                .join("state.json")
        )
        .unwrap(),
        "original-data"
    );
    assert_eq!(
        f.execute_upgrade(request, &execute).unwrap_err().reason,
        "personal_application_data_operation_unconfirmed"
    );
}

#[test]
#[ignore = "only spawned by the interrupted owner test; reads no user files"]
fn application_data_owner_probe() {
    use std::io::Read as _;
    let _ = std::io::stdin().read_exact(&mut [0u8; 1]);
}

#[test]
fn invalid_record_cannot_redirect_data_copy_to_other_profile_storage() {
    let mut f = Fixture::new();
    let original = f.active();
    f.build("export const version=2;");
    let (request, execute) = f.guarded_candidate("corrupt-target");
    f.sql(&format!("UPDATE personal_application_data_operations SET record_json=json_set(record_json,'$.targetSlot.relativePath','ai-runtime') WHERE operation_id='{}';",execute.operation_id));
    assert_eq!(
        f.execute_upgrade(request, &execute).unwrap_err().reason,
        "personal_application_data_record_invalid"
    );
    assert_eq!(f.active(), original);
    assert!(!f.profile.join("ai-runtime").exists());
}

#[test]
fn legacy_install_rpc_is_explicitly_refused_for_desktop_and_internal_clients() {
    let mut f = Fixture::new();
    let before = f.project_record();
    let original = f.active();
    let services = std::sync::Arc::new(std::sync::Mutex::new(std::mem::replace(
        &mut f.services,
        AppServerServices::new(None),
    )));
    for client in ["desktop", "runtime"] {
        let mut router = crate::ConnectionRouter::with_services(
            "test-server".into(),
            "test-connection".into(),
            "test-session".into(),
            true,
            services.clone(),
        );
        router.phase = crate::Phase::Ready;
        router.source_client = Some(client);
        let frame = contexthub_protocol::Frame::Message(serde_json::to_vec(&json!({
            "jsonrpc":"2.0","id":"legacy-install","method":"capabilityCreation/install",
            "params":{"creationProjectId":f.project,"expectedRevision":before.state_revision,"packageDigest":before.package_digest,"clientOperationId":"legacy-bypass","requestFingerprint":"unused"}
        })).unwrap());
        let response: Value =
            serde_json::from_str(&router.handle_frame(frame).response.unwrap()).unwrap();
        assert_eq!(
            response["error"]["data"]["reason"],
            "personal_application_data_legacy_install_disabled"
        );
    }
    f.services = std::sync::Arc::try_unwrap(services)
        .ok()
        .unwrap()
        .into_inner()
        .unwrap();
    assert_eq!(f.project_record(), before);
    assert_eq!(f.active(), original);
}
