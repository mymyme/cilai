use super::{PersistenceError, database_error, mark_projection_pending};
use contexthub_application::WorkspacePlatformBindingRecord;
use contexthub_domain::WorkspaceId;
use rusqlite::{Transaction, params};

pub(super) struct HistoricalPlatformSnapshot {
    pub snapshot_id: String,
    pub revision: String,
    pub path: String,
    pub version_id: String,
    pub timestamp: i64,
}

pub(super) fn insert_historical_platform_binding(
    transaction: &Transaction<'_>,
    binding: &WorkspacePlatformBindingRecord,
) -> Result<(), PersistenceError> {
    transaction.execute(
        "INSERT INTO workspace_platform_bindings(workspace_id, account_id, organization_id, platform_workspace_id, project_id, platform_root, binding_state, platform_snapshot_revision, last_platform_check_at, created_at, updated_at, team_space_id, managed_root, device_id, baseline_revision, change_cursor, permission_role, permission_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![binding.workspace_id.as_str(),binding.account_id.as_str(),binding.organization_id,binding.platform_workspace_id,binding.project_id,binding.platform_root,binding.binding_state.as_str(),binding.platform_snapshot_revision,binding.last_platform_check_at,binding.created_at,binding.updated_at,binding.team_space_id,binding.managed_root,binding.device_id,binding.baseline_revision,binding.change_cursor,binding.permission_role,binding.permission_revision],
    ).map_err(database_error("prepare historical platform binding"))?;
    mark_projection_pending(transaction, &binding.workspace_id, binding.updated_at)
}

pub(super) fn remove_historical_platform_binding(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    timestamp: i64,
) -> Result<(), PersistenceError> {
    transaction
        .execute(
            "DELETE FROM workspace_platform_bindings WHERE workspace_id=?1",
            [workspace_id.as_str()],
        )
        .map_err(database_error("prepare historical unbound workspace"))?;
    mark_projection_pending(transaction, workspace_id, timestamp)
}

pub(super) fn seed_historical_platform_snapshot(
    transaction: &Transaction<'_>,
    snapshot: &HistoricalPlatformSnapshot,
) -> Result<(), PersistenceError> {
    transaction.execute(
        "INSERT INTO platform_snapshots(snapshot_id,workspace_id,platform_revision,cursor,snapshot_kind,received_at,platform_time,item_count,request_id) VALUES (?1,'w-snapshot',?2,?3,'full',?4,?4,1,?5)",
        params![snapshot.snapshot_id,snapshot.revision,format!("cursor-{}",snapshot.revision),snapshot.timestamp,format!("request-{}",snapshot.revision)],
    ).map_err(database_error("prepare historical platform snapshot"))?;
    transaction.execute(
        "INSERT INTO documents(document_id,workspace_id,platform_path,normalized_platform_path,node_type,scope,platform_state,metadata_revision,platform_checked_at,capabilities_json) VALUES ('document-1','w-snapshot',?1,?1,'file','project','published',?2,?3,'{}') ON CONFLICT(document_id) DO UPDATE SET platform_path=excluded.platform_path,normalized_platform_path=excluded.normalized_platform_path,metadata_revision=excluded.metadata_revision,platform_checked_at=excluded.platform_checked_at",
        params![snapshot.path,snapshot.revision,snapshot.timestamp],
    ).map_err(database_error("prepare historical document identity"))?;
    let checksum = format!("checksum-{}", snapshot.version_id);
    transaction.execute(
        "INSERT INTO document_versions(version_id,document_id,status,checksum_algorithm,checksum_value,size_bytes,created_at,published_at,cached_at) VALUES (?1,'document-1','published','sha256',?2,10,?3,?3,?3)",
        params![snapshot.version_id,checksum,snapshot.timestamp],
    ).map_err(database_error("prepare immutable historical document version"))?;
    transaction.execute(
        "UPDATE documents SET latest_published_version_id=?1,latest_published_checksum_algorithm='sha256',latest_published_checksum_value=?2,size_bytes=10,platform_updated_at=?3 WHERE document_id='document-1' AND workspace_id='w-snapshot'",
        params![snapshot.version_id,checksum,snapshot.timestamp],
    ).map_err(database_error("prepare historical published baseline"))?;
    transaction.execute(
        "UPDATE workspace_platform_bindings SET platform_snapshot_revision=?1,last_platform_check_at=?2,updated_at=?2 WHERE workspace_id='w-snapshot'",
        params![snapshot.revision,snapshot.timestamp],
    ).map_err(database_error("prepare historical binding revision"))?;
    transaction.execute(
        "INSERT INTO durable_events(event_id,workspace_id,workspace_sequence,event_type,object_type,object_id,payload_json,correlation_id,occurred_at,created_at) VALUES (?1,'w-snapshot',(SELECT COALESCE(MAX(workspace_sequence),0)+1 FROM durable_events WHERE workspace_id='w-snapshot'),'workspace.platform_snapshot_committed','platform_snapshot',?2,'{\"document_count\":1,\"version_count\":1}',?3,?4,?4)",
        params![format!("event-{}",snapshot.snapshot_id),snapshot.snapshot_id,format!("correlation-{}",snapshot.snapshot_id),snapshot.timestamp],
    ).map_err(database_error("prepare historical snapshot event"))?;
    mark_projection_pending(
        transaction,
        &WorkspaceId::new("w-snapshot"),
        snapshot.timestamp,
    )
}
