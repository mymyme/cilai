use super::*;
use contexthub_protocol::{
    ModelExecutionOwner, PersonalWriterLease, PersonalWriterMode, PersonalWriterState,
};
use rusqlite::params;
fn fixture() -> (std::path::PathBuf, SqlitePersistence) {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "cilai-shutdown-writers-{}",
        uuid::Uuid::new_v4().simple()
    ));
    contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
    let db = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
    (root, db)
}
#[test]
fn unresolved_writer_blocks_shutdown_without_inventing_exit_or_mutating_its_lease() {
    let (root, db) = fixture();
    let lease = PersonalWriterLease {
        lease_id: "writer-test-unconfirmed".into(),
        profile_id: "test-profile".into(),
        instance_fence: "test-server".into(),
        kind: "user_mcp".into(),
        resource_id: "test-resource".into(),
        resource_revision: "test-revision".into(),
        owner: ModelExecutionOwner {
            instance_id: "synthetic-owner".into(),
            pid: u32::MAX,
            start_fingerprint: "synthetic-unconfirmed-fingerprint".into(),
        },
        launch_protocol: 1,
        process_mode: PersonalWriterMode::LocalProcess,
        processes: vec![],
        state: PersonalWriterState::Prepared,
        local_transport_finished: false,
        remote_outcome_unconfirmed: false,
        created_at: 1,
        updated_at: 1,
    };
    let raw = serde_json::to_string(&lease).unwrap();
    db.connection.execute("INSERT INTO personal_profile_writers(lease_id,profile_id,client_operation_id,request_fingerprint,record_json,state,created_at,updated_at) VALUES(?1,?2,'test-client','test',?3,'prepared',1,1)",params![lease.lease_id,lease.profile_id,raw]).unwrap();
    assert_eq!(
        db.personal_shutdown_blocking_reasons().unwrap(),
        vec!["profile_writer_active"]
    );
    assert_eq!(
        db.connection
            .query_row(
                "SELECT record_json FROM personal_profile_writers WHERE lease_id=?1",
                [&lease.lease_id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        raw
    );
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn malformed_native_execution_lease_is_not_treated_as_an_empty_writer_list() {
    let (root, db) = fixture();
    db.connection.execute("INSERT INTO ai_runtime_entries(record_id,position,record_json,digest,updated_at) VALUES('personalModel:lease:invalid',0,'{}','test',1)",[]).unwrap();
    assert_eq!(
        db.personal_shutdown_blocking_reasons()
            .unwrap_err()
            .reason(),
        Some("app_server_writers_unconfirmed")
    );
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn an_applying_maintenance_receipt_keeps_service_alive_without_writers() {
    let (root, db) = fixture();
    db.connection.execute("INSERT INTO personal_maintenance_operation_registry(operation_id,profile_id,operation_kind,preview_digest,state,updated_at) VALUES('test-operation','test-profile','storage_cleanup','test-preview','applying',1)",[]).unwrap();
    assert_eq!(
        db.personal_shutdown_blocking_reasons().unwrap(),
        vec!["personal_maintenance_operation_unconfirmed"]
    );
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}
