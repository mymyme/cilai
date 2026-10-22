use crate::{PersonalApplicationDataRecord, PersonalApplicationDataSlot};
use contexthub_protocol::{
    PersonalApplicationDataAction as AppAction, PersonalApplicationDataOperation as AppOperation,
    PersonalApplicationDataPrepareParams as AppPrepare, PersonalApplicationDataState as AppState,
    PersonalMaintenanceOperationStatus,
};
use std::path::PathBuf;

const APP_SOURCE: &str = "11111111111111111111111111111111";
const APP_TARGET: &str = "22222222222222222222222222222222";

struct ApplicationRestoreFixture {
    root: PathBuf,
    profiles: PathBuf,
    source: SqlitePersistence,
    record: PersonalApplicationDataRecord,
}
impl ApplicationRestoreFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "cilai-application-data-restore-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let profiles = root.join("cn.cilai.personal.workbench/business/profiles");
        let source_dir = profiles.join(APP_SOURCE);
        contexthub_platform_adapter::security::ensure_restricted_directory(&source_dir).unwrap();
        let mut source =
            SqlitePersistence::open(&source_dir.join("contexthub.db"), "test").unwrap();
        source
            .begin_personal_profile_upgrade(APP_SOURCE, "test-instance")
            .unwrap();
        source
            .finish_personal_profile_upgrade(APP_SOURCE, "test-instance", "manifest")
            .unwrap();
        source.connection.execute("INSERT INTO app_metadata(key,value,updated_at) VALUES('retained-original','unchanged',1)", []).unwrap();
        let record = application_history_record();
        for slot in [record.source_slot.as_ref().unwrap(), &record.target_slot] {
            source.connection.execute("INSERT INTO personal_application_data_slots(profile_id,slot_id,creation_project_id,record_json,created_at) VALUES(?1,?2,?3,?4,1)", params![slot.profile_id,slot.slot_id,slot.creation_project_id,encode(slot).unwrap()]).unwrap();
        }
        source.connection.execute("INSERT INTO personal_application_data_pointers(profile_id,creation_project_id,slot_id) VALUES(?1,?2,?3)", params![APP_SOURCE,record.operation.creation_project_id,record.target_slot.slot_id]).unwrap();
        crate::maintenance_operations::register(
            &source.connection,
            &PersonalMaintenanceOperationStatus {
                operation_id: record.operation.operation_id.clone(),
                profile_id: APP_SOURCE.into(),
                kind: "personal_capability_data_change".into(),
                state: "completed".into(),
                preview_digest: record.operation.preview_digest.clone(),
            },
            2,
        )
        .unwrap();
        source.connection.execute("INSERT INTO personal_application_data_operations(operation_id,profile_id,creation_project_id,client_operation_id,fingerprint,record_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,1,2)",params![record.operation.operation_id,APP_SOURCE,record.operation.creation_project_id,record.operation.client_operation_id,record.fingerprint,encode(&record).unwrap()]).unwrap();
        Self {
            root,
            profiles,
            source,
            record,
        }
    }
    fn candidate(&self) -> SqlitePersistence {
        let path = self.profiles.join(format!(
            "restore-candidate-{}",
            uuid::Uuid::new_v4().simple()
        ));
        contexthub_platform_adapter::security::ensure_restricted_directory(&path).unwrap();
        self.source
            .snapshot_personal_data_database(&path.join("contexthub.db"))
            .unwrap();
        SqlitePersistence::open(&path.join("contexthub.db"), "test").unwrap()
    }
    fn assert_source_retained(&self) {
        let old = self
            .source
            .personal_application_data_operation(APP_SOURCE, &self.record.operation.operation_id)
            .unwrap();
        assert!(!old.operation.read_only_after_restore);
        assert_eq!(old.operation.state, AppState::Completed);
        assert_eq!(old.operation.source_profile_id, None);
        assert_eq!(
            self.source
                .connection
                .query_row(
                    "SELECT value FROM app_metadata WHERE key='retained-original'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "unchanged"
        );
    }
}
impl Drop for ApplicationRestoreFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}
fn application_history_record() -> PersonalApplicationDataRecord {
    let project = format!("capability_{}", uuid::Uuid::new_v4().simple());
    let source_slot = PersonalApplicationDataSlot {
        profile_id: APP_SOURCE.into(),
        creation_project_id: project.clone(),
        slot_id: uuid::Uuid::new_v4().simple().to_string(),
        package_digest: "sha256:old".into(),
        relative_path: format!("personal-capability-data/{project}"),
        created_at: 1,
    };
    let target_id = uuid::Uuid::new_v4().simple().to_string();
    let target_slot = PersonalApplicationDataSlot {
        profile_id: APP_SOURCE.into(),
        creation_project_id: project.clone(),
        slot_id: target_id.clone(),
        package_digest: "sha256:new".into(),
        relative_path: format!("personal-capability-slots/{project}/{target_id}"),
        created_at: 2,
    };
    let request = AppPrepare {
        creation_project_id: project.clone(),
        expected_revision: 1,
        package_digest: "sha256:new".into(),
        client_operation_id: "restored-history-client".into(),
        action: AppAction::Upgrade,
        target_operation_id: None,
    };
    let operation = AppOperation {
        read_only_after_restore: false,
        source_profile_id: None,
        operation_id: format!("application_data_{}", uuid::Uuid::new_v4().simple()),
        profile_id: APP_SOURCE.into(),
        creation_project_id: project,
        client_operation_id: request.client_operation_id.clone(),
        expected_revision: 1,
        package_digest: "sha256:new".into(),
        source_package_digest: Some("sha256:old".into()),
        source_slot_id: Some(source_slot.slot_id.clone()),
        target_slot_id: target_id,
        action: AppAction::Upgrade,
        state: AppState::Completed,
        preview_digest: "sha256:historical-preview".into(),
        snapshot_digest: Some("sha256:retained-snapshot".into()),
        reason: None,
        activated_revision: Some(2),
        created_at: 1,
        updated_at: 2,
    };
    PersonalApplicationDataRecord {
        operation,
        request,
        fingerprint: "historical-request".into(),
        source_slot: Some(source_slot),
        target_slot,
        previous_project: None,
        previous_installation: None,
        rollback_project: None,
        rollback_installation: None,
        expected_retained_digest: Some("sha256:retained-snapshot".into()),
    }
}
#[test]
fn restored_application_data_preserves_history_and_rebinds_slots_without_reusing_authority() {
    let f = ApplicationRestoreFixture::new();
    let mut candidate = f.candidate();
    let before = candidate
        .personal_application_data_operation(APP_SOURCE, &f.record.operation.operation_id)
        .unwrap();
    assert_eq!(before.operation.state, AppState::Completed);
    assert!(before.operation.read_only_after_restore);
    assert_eq!(
        before.operation.source_profile_id.as_deref(),
        Some(APP_SOURCE)
    );
    assert_eq!(
        candidate
            .connection
            .query_row(
                "SELECT state FROM personal_maintenance_operation_registry WHERE operation_id=?1",
                [&before.operation.operation_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "cancelled"
    );
    candidate
        .prepare_restored_personal_profile(APP_SOURCE, APP_TARGET)
        .unwrap();
    let restored = candidate
        .personal_application_data_operation(APP_TARGET, &f.record.operation.operation_id)
        .unwrap();
    assert_eq!(restored.operation.state, AppState::Completed);
    assert!(restored.operation.read_only_after_restore);
    assert_eq!(
        restored.operation.source_profile_id.as_deref(),
        Some(APP_SOURCE)
    );
    assert_eq!(restored.operation.profile_id, APP_TARGET);
    assert_eq!(
        restored.source_slot.as_ref().unwrap().profile_id,
        APP_TARGET
    );
    assert_eq!(restored.target_slot.profile_id, APP_TARGET);
    assert_eq!(
        restored.operation.snapshot_digest,
        f.record.operation.snapshot_digest
    );
    let active = candidate
        .personal_application_data_slot(APP_TARGET, &restored.operation.creation_project_id)
        .unwrap()
        .unwrap();
    assert_eq!(active, restored.target_slot);
    assert_eq!(
        candidate
            .connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, u32>(
                0
            ))
            .unwrap(),
        0
    );
    assert!(
        candidate
            .transition_personal_application_data(&restored, &AppState::Completed)
            .is_err()
    );
    assert_eq!(
        candidate
            .resolve_interrupted_application_data(
                APP_TARGET,
                &restored.operation.operation_id,
                &restored.operation.preview_digest,
                3,
            )
            .unwrap_err()
            .reason(),
        Some("personal_application_data_restored_read_only")
    );
    f.assert_source_retained();
    drop(candidate);
}
#[test]
fn restored_application_data_refuses_mixed_profile_ownership_and_preserves_source() {
    let f = ApplicationRestoreFixture::new();
    let mut candidate = f.candidate();
    candidate.connection.execute("INSERT INTO personal_application_data_slots(profile_id,slot_id,creation_project_id,record_json,created_at) SELECT 'unknown-profile',slot_id,creation_project_id,record_json,created_at FROM personal_application_data_slots LIMIT 1",[]).unwrap();
    assert_eq!(
        candidate
            .prepare_restored_personal_profile(APP_SOURCE, APP_TARGET)
            .unwrap_err()
            .reason(),
        Some("personal_data_restore_profile_reference_unknown")
    );
    assert_eq!(
        candidate
            .connection
            .query_row(
                "SELECT profile_id FROM personal_profile_compatibility WHERE singleton=1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        APP_SOURCE
    );
    assert_eq!(
        candidate
            .connection
            .query_row(
                "SELECT COUNT(*) FROM personal_application_data_slots WHERE profile_id=?1",
                [APP_TARGET],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    f.assert_source_retained();
    drop(candidate);
}

#[test]
fn restored_application_data_rejects_dangling_pointers_before_any_profile_switch() {
    let f = ApplicationRestoreFixture::new();
    let mut candidate = f.candidate();
    candidate
        .connection
        .execute_batch("PRAGMA foreign_keys=OFF;")
        .unwrap();
    candidate
        .connection
        .execute(
            "UPDATE personal_application_data_pointers SET slot_id='unclassified-missing-slot'",
            [],
        )
        .unwrap();
    assert_eq!(
        candidate
            .prepare_restored_personal_profile(APP_SOURCE, APP_TARGET)
            .unwrap_err()
            .reason(),
        Some("personal_data_database_invalid")
    );
    assert_eq!(
        candidate
            .connection
            .query_row(
                "SELECT profile_id FROM personal_profile_compatibility WHERE singleton=1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        APP_SOURCE
    );
    assert_eq!(
        candidate
            .connection
            .query_row("PRAGMA foreign_keys", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        1
    );
    f.assert_source_retained();
    drop(candidate);
}

#[test]
fn restored_application_slots_reject_non_managed_paths_without_rebinding_any_data() {
    let f = ApplicationRestoreFixture::new();
    let mut candidate = f.candidate();
    let mut invalid = f.record.target_slot.clone();
    invalid.relative_path = "../outside-personal-profile".into();
    candidate
        .connection
        .execute(
            "UPDATE personal_application_data_slots SET record_json=?1 WHERE slot_id=?2",
            params![encode(&invalid).unwrap(), invalid.slot_id],
        )
        .unwrap();
    assert_eq!(
        candidate
            .prepare_restored_personal_profile(APP_SOURCE, APP_TARGET)
            .unwrap_err()
            .reason(),
        Some("personal_data_restore_profile_reference_unknown")
    );
    assert_eq!(
        candidate
            .connection
            .query_row(
                "SELECT COUNT(*) FROM personal_application_data_slots WHERE profile_id=?1",
                [APP_TARGET],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    f.assert_source_retained();
    drop(candidate);
}

#[test]
fn restored_application_history_reuses_business_validation_for_nested_slot_paths() {
    let f = ApplicationRestoreFixture::new();
    let mut candidate = f.candidate();
    let mut invalid = f.record.clone();
    invalid.target_slot.relative_path = "../outside-personal-profile".into();
    candidate
        .connection
        .execute(
            "UPDATE personal_application_data_operations SET record_json=?1 WHERE operation_id=?2",
            params![encode(&invalid).unwrap(), invalid.operation.operation_id],
        )
        .unwrap();
    assert_eq!(
        candidate
            .prepare_restored_personal_profile(APP_SOURCE, APP_TARGET)
            .unwrap_err()
            .reason(),
        Some("personal_data_record_invalid")
    );
    assert_eq!(
        candidate
            .connection
            .query_row(
                "SELECT profile_id FROM personal_profile_compatibility WHERE singleton=1",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        APP_SOURCE
    );
    f.assert_source_retained();
    drop(candidate);
}

#[test]
fn restored_notification_authority_and_old_claims_are_removed_only_from_candidate() {
    let fixture = ApplicationRestoreFixture::new();
    let value = serde_json::json!({"entryId":"personalNotifications","field":"personalNotifications","shape":"scalar","value":{"preferences":{"enabled":true},"deliveries":[{"profileId":APP_SOURCE,"state":"shown","eventId":"source-event"}]}});
    fixture.source.connection.execute("INSERT INTO ai_runtime_entries(record_id,position,record_json,digest,updated_at) VALUES('personalNotifications',0,?1,'test',1)", [value.to_string()]).unwrap();
    let mut candidate = fixture.candidate();
    assert_eq!(
        candidate
            .connection
            .query_row(
                "SELECT COUNT(*) FROM ai_runtime_entries WHERE record_id='personalNotifications'",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        1
    );
    candidate
        .prepare_restored_personal_profile(APP_SOURCE, APP_TARGET)
        .unwrap();
    assert_eq!(
        candidate
            .connection
            .query_row(
                "SELECT COUNT(*) FROM ai_runtime_entries WHERE record_id='personalNotifications'",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(fixture.source.connection.query_row("SELECT record_json FROM ai_runtime_entries WHERE record_id='personalNotifications'", [], |row| row.get::<_, String>(0)).unwrap(), value.to_string());
}
