use super::*;
use contexthub_protocol::{ModelExecutionOwner, PersonalDataQuiescence};
use serde_json::json;
use sha2::{Digest as _, Sha256};
const PROFILE: &str = "11111111111111111111111111111111";
const INSTANCE: &str = "restore-abort-test-instance";
struct Fixture {
    root: std::path::PathBuf,
    db: SqlitePersistence,
    params: PersonalRestoreAbortBeforeHelperParams,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "cilai-restore-before-helper-{}",
            uuid::Uuid::new_v4().simple()
        ));
        contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
        std::fs::write(root.join("original-data"), b"unchanged-original").unwrap();
        let mut db = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        db.begin_personal_profile_upgrade(PROFILE, INSTANCE)
            .unwrap();
        db.finish_personal_profile_upgrade(PROFILE, INSTANCE, "test-manifest")
            .unwrap();
        let op = PersonalDataOperation {
            operation_id: format!("data_{}", uuid::Uuid::new_v4().simple()),
            profile_id: PROFILE.into(),
            kind: PersonalDataOperationKind::Restore,
            state: PersonalDataOperationState::ReadyToApply,
            preview_digest: "sha256:test-preview".into(),
            created_at: 1,
            updated_at: 1,
            selection: None,
            backup_manifest: None,
            restore_preview: None,
            blockers: vec![],
            result: None,
        };
        db.prepare_personal_data(&op, "test-client", "test-fingerprint")
            .unwrap();
        let owner = ModelExecutionOwner {
            instance_id: "test-host".into(),
            pid: std::process::id(),
            start_fingerprint: contexthub_platform_adapter::instance::process_start_fingerprint(
                std::process::id(),
            )
            .unwrap()
            .unwrap(),
        };
        let fence=db.personal_writer_rpc("personalMaintenance/acquire",json!({"operationId":op.operation_id,"previewDigest":op.preview_digest,"owner":owner}),PROFILE,INSTANCE,2).unwrap();
        let proof = PersonalDataQuiescence {
            protocol_version: 1,
            profile_id: PROFILE.into(),
            token: fence["fenceId"].as_str().unwrap().into(),
            owner,
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
        Self {
            root,
            db,
            params: PersonalRestoreAbortBeforeHelperParams {
                operation_id: op.operation_id,
                preview_digest: op.preview_digest,
                quiescence: proof,
            },
        }
    }
    fn operation(&self) -> PersonalDataOperation {
        self.db
            .personal_data_operation(PROFILE, &self.params.operation_id)
            .unwrap()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn abort_before_helper_records_incomplete_and_retains_fence_until_explicit_receipt_release() {
    let mut f = Fixture::new();
    let op =
        f.db.abort_personal_restore_before_helper(PROFILE, INSTANCE, &f.params, 3)
            .unwrap();
    assert_eq!(op.state, PersonalDataOperationState::Incomplete);
    assert_eq!(
        op.blockers[0].code,
        "personal_data_restore_exclusive_shutdown_refused"
    );
    assert!(f.db.personal_maintenance_active(PROFILE).unwrap());
    assert_eq!(
        f.db.abort_personal_restore_before_helper(PROFILE, INSTANCE, &f.params, 4)
            .unwrap(),
        op
    );
    assert_eq!(
        std::fs::read(f.root.join("original-data")).unwrap(),
        b"unchanged-original"
    );
    assert!(!f.root.join("local-profile.json").exists());
    f.db.personal_writer_rpc(
        "personalMaintenance/release",
        json!({"fenceId":f.params.quiescence.token,"owner":f.params.quiescence.owner}),
        PROFILE,
        INSTANCE,
        5,
    )
    .unwrap();
    assert!(!f.db.personal_maintenance_active(PROFILE).unwrap());
}
#[test]
fn wrong_confirmation_or_an_applying_operation_cannot_be_aborted_or_release_the_fence() {
    let mut f = Fixture::new();
    let mut wrong = f.params.clone();
    wrong.preview_digest = "sha256:changed".into();
    assert_eq!(
        f.db.abort_personal_restore_before_helper(PROFILE, INSTANCE, &wrong, 3)
            .unwrap_err()
            .reason(),
        Some("personal_data_preview_changed")
    );
    wrong = f.params.clone();
    wrong.quiescence.owner.start_fingerprint = "wrong-owner".into();
    assert!(
        f.db.abort_personal_restore_before_helper(PROFILE, INSTANCE, &wrong, 3)
            .is_err()
    );
    assert!(
        f.db.abort_personal_restore_before_helper(PROFILE, "other-instance", &f.params, 3)
            .is_err()
    );
    assert_eq!(
        f.operation().state,
        PersonalDataOperationState::ReadyToApply
    );
    let mut applying = f.operation();
    applying.state = PersonalDataOperationState::Applying;
    f.db.transition_personal_data(&applying, &PersonalDataOperationState::ReadyToApply)
        .unwrap();
    assert_eq!(
        f.db.abort_personal_restore_before_helper(PROFILE, INSTANCE, &f.params, 4)
            .unwrap_err()
            .reason(),
        Some("personal_data_operation_unconfirmed")
    );
    assert_eq!(f.operation().state, PersonalDataOperationState::Applying);
    assert!(f.db.personal_maintenance_active(PROFILE).unwrap());
    assert_eq!(
        std::fs::read(f.root.join("original-data")).unwrap(),
        b"unchanged-original"
    );
}
