use super::{PersistenceError, SqlitePersistence, database_error, encode, reject};
use contexthub_protocol::{PersonalWorkspaceContentEntry, PersonalWorkspaceContentSnapshot};
use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

fn snapshot_digest(
    snapshot: &PersonalWorkspaceContentSnapshot,
) -> Result<String, PersistenceError> {
    let mut content = snapshot.clone();
    content.digest.clear();
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(encode(&content)?.as_bytes())
    ))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn read_entries(
    db: &Connection,
    workspace: &str,
) -> Result<Vec<PersonalWorkspaceContentEntry>, PersistenceError> {
    let mut statement = db.prepare(
        "SELECT lf.local_file_id,lf.relative_path,lf.normalized_relative_path,lf.file_type,lf.file_identity,lf.state_revision,lf.size_bytes,lf.modified_at_fs,lf.local_checksum_algorithm,lf.local_checksum_value,lf.materialization_state,lf.scan_state FROM local_files lf WHERE lf.workspace_id=?1 AND lf.materialization_state<>'missing' AND lf.local_file_id=(SELECT candidate.local_file_id FROM local_files candidate WHERE candidate.workspace_id=lf.workspace_id AND candidate.normalized_relative_path=lf.normalized_relative_path AND candidate.materialization_state<>'missing' ORDER BY candidate.updated_at DESC,candidate.local_file_id DESC LIMIT 1) ORDER BY lf.normalized_relative_path,lf.local_file_id",
    ).map_err(database_error("read original file snapshot members"))?;
    let rows = statement
        .query_map([workspace], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
            ))
        })
        .map_err(database_error("query original file snapshot members"))?;
    let mut entries = Vec::new();
    for row in rows {
        let (
            id,
            path,
            normalized,
            kind,
            identity,
            revision,
            bytes,
            modified,
            algorithm,
            checksum,
            materialized,
            scan,
        ) = row.map_err(database_error("read original file snapshot member"))?;
        if path != normalized
            || materialized != "materialized"
            || scan != "current"
            || !matches!(kind.as_str(), "file" | "directory")
            || revision < 0
        {
            return Err(reject("personal_data_workspace_index_unready"));
        }
        let identity = identity
            .filter(|value| !value.is_empty())
            .ok_or_else(|| reject("personal_data_workspace_identity_missing"))?;
        let bytes = bytes
            .map(u64::try_from)
            .transpose()
            .map_err(|_| reject("personal_data_workspace_index_invalid"))?;
        if kind == "file"
            && (bytes.is_none()
                || algorithm.as_deref() != Some("sha256")
                || !checksum.as_deref().is_some_and(valid_sha256))
        {
            return Err(reject("personal_data_workspace_hash_missing"));
        }
        entries.push(PersonalWorkspaceContentEntry {
            local_file_id: id,
            relative_path: path,
            file_type: kind,
            file_identity: identity,
            state_revision: revision,
            bytes,
            modified_at: modified,
            sha256: checksum,
        });
    }
    Ok(entries)
}

type WorkspaceBasisRow = (String, Option<String>, String, i64, String, Option<i64>);

fn read_snapshot(
    db: &Connection,
    workspace: &str,
) -> Result<PersonalWorkspaceContentSnapshot, PersistenceError> {
    let row: Option<WorkspaceBasisRow> = db.query_row(
        "SELECT local_root,local_root_identity,local_status,state_revision,workspace_kind,last_local_scan_at FROM workspaces WHERE workspace_id=?1",
        [workspace], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
    ).optional().map_err(database_error("read original file workspace basis"))?;
    let (root_path, root_identity, status, revision, kind, last_full_scan) =
        row.ok_or_else(|| reject("personal_data_workspace_unknown"))?;
    if status != "ready" || kind != "local" || revision < 0 {
        return Err(reject("personal_data_workspace_index_unready"));
    }
    let full: Option<(String, i64, i64)> = db.query_row(
        "SELECT snapshot_id,generation,issue_count FROM local_scan_snapshots WHERE workspace_id=?1 AND snapshot_kind='full' AND completed_at=?2 ORDER BY generation DESC LIMIT 1",
        rusqlite::params![workspace,last_full_scan], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional().map_err(database_error("read complete original file generation"))?;
    let (full_snapshot_id, full_generation, issues) =
        full.ok_or_else(|| reject("personal_data_workspace_full_index_required"))?;
    let latest_generation: i64 = db
        .query_row(
            "SELECT MAX(generation) FROM local_scan_snapshots WHERE workspace_id=?1",
            [workspace],
            |row| row.get(0),
        )
        .map_err(database_error("read latest original file generation"))?;
    if issues != 0 || full_generation <= 0 || latest_generation < full_generation {
        return Err(reject("personal_data_workspace_index_unready"));
    }
    let mut snapshot = PersonalWorkspaceContentSnapshot {
        protocol_version: 1,
        workspace_id: workspace.into(),
        root_identity: root_identity
            .filter(|value| !value.is_empty())
            .ok_or_else(|| reject("personal_data_workspace_identity_missing"))?,
        root_path,
        workspace_revision: revision,
        latest_generation,
        full_snapshot_id,
        full_generation,
        digest: String::new(),
        entries: read_entries(db, workspace)?,
    };
    snapshot.digest = snapshot_digest(&snapshot)?;
    Ok(snapshot)
}

fn read_snapshots(
    db: &Connection,
    workspaces: &[String],
) -> Result<BTreeMap<String, PersonalWorkspaceContentSnapshot>, PersistenceError> {
    let mut snapshots = BTreeMap::new();
    for workspace in workspaces {
        if snapshots.contains_key(workspace) {
            return Err(reject("personal_data_request_invalid"));
        }
        snapshots.insert(workspace.clone(), read_snapshot(db, workspace)?);
    }
    Ok(snapshots)
}

impl SqlitePersistence {
    /// # Errors
    /// 选择目录没有完整且可核验的索引时拒绝；本方法不扫描或更新索引。
    pub fn personal_data_workspace_snapshots(
        &self,
        workspaces: &[String],
    ) -> Result<BTreeMap<String, PersonalWorkspaceContentSnapshot>, PersistenceError> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(database_error("begin original file basis read"))?;
        let snapshots = read_snapshots(&transaction, workspaces)?;
        transaction
            .commit()
            .map_err(database_error("finish original file basis read"))?;
        Ok(snapshots)
    }

    /// # Errors
    /// 归档业务库中的索引与用户确认的原文修订不同时拒绝。
    pub fn verify_personal_data_workspace_snapshot(
        &self,
        database: &Path,
        expected: &BTreeMap<String, PersonalWorkspaceContentSnapshot>,
    ) -> Result<(), PersistenceError> {
        let db = Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(database_error("open captured original file basis"))?;
        let ids = expected.keys().cloned().collect::<Vec<_>>();
        if read_snapshots(&db, &ids)? != *expected {
            return Err(reject("personal_data_workspace_index_changed"));
        }
        Ok(())
    }
}

type RestoredLocations =
    BTreeMap<String, (String, String, BTreeMap<String, (String, Option<i64>)>)>;
impl SqlitePersistence {
    /// # Errors
    /// 仅恢复候选允许重绑；归档索引或新目录逐项身份缺失时拒绝，保留旧历史观察。
    pub fn restore_personal_workspace_locations(
        &mut self,
        expected: &BTreeMap<String, PersonalWorkspaceContentSnapshot>,
        locations: &RestoredLocations,
        now: i64,
    ) -> Result<(), PersistenceError> {
        super::verify_restore_candidate(&self.database_path)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin restored workspace relocation"))?;
        let ids = expected.keys().cloned().collect::<Vec<_>>();
        if read_snapshots(&tx, &ids)? != *expected || expected.len() != locations.len() {
            return Err(reject("personal_data_workspace_index_changed"));
        }
        tx.execute("UPDATE workspaces SET local_status='unavailable',state_revision=state_revision+1 WHERE workspace_kind='local'", [])
            .map_err(database_error("mark unrelocated workspace references unavailable"))?;
        for (workspace, snapshot) in expected {
            let (root, root_identity, entries) = locations
                .get(workspace)
                .ok_or_else(|| reject("personal_data_workspace_destination_invalid"))?;
            if root.is_empty()
                || root_identity.is_empty()
                || entries.len() != snapshot.entries.len()
            {
                return Err(reject("personal_data_workspace_destination_invalid"));
            }
            let generation = snapshot
                .latest_generation
                .checked_add(1)
                .ok_or_else(|| reject("personal_data_workspace_index_invalid"))?;
            let scan_id = format!("restore_{}", uuid::Uuid::new_v4().simple());
            tx.execute("INSERT INTO local_scan_snapshots(snapshot_id,workspace_id,snapshot_kind,generation,observed_at,completed_at,entry_count,issue_count) VALUES(?1,?2,'full',?3,?4,?4,?5,0)",
                rusqlite::params![scan_id, workspace, generation, now, i64::try_from(entries.len()).map_err(|_| reject("personal_data_workspace_index_invalid"))?])
                .map_err(database_error("record restored complete content generation"))?;
            for entry in &snapshot.entries {
                let (identity, modified) = entries
                    .get(&entry.relative_path)
                    .ok_or_else(|| reject("personal_data_workspace_destination_invalid"))?;
                if identity.is_empty() {
                    return Err(reject("personal_data_workspace_identity_missing"));
                }
                let updated = tx.execute("UPDATE local_files SET file_identity=?1,modified_at_fs=?2,last_seen_snapshot_id=?3,state_revision=state_revision+1,updated_at=?4 WHERE workspace_id=?5 AND local_file_id=?6 AND relative_path=?7",
                    rusqlite::params![identity, modified, scan_id, now, workspace, entry.local_file_id, entry.relative_path])
                    .map_err(database_error("rebind restored current file identity"))?;
                if updated != 1 {
                    return Err(reject("personal_data_workspace_index_changed"));
                }
                tx.execute("INSERT INTO local_file_observations(snapshot_id,local_file_id,relative_path,normalized_relative_path,file_identity,file_type,size_bytes,modified_at_fs,checksum_algorithm,checksum_value,observed_at) VALUES(?1,?2,?3,?3,?4,?5,?6,?7,?8,?9,?10)",
                    rusqlite::params![scan_id, entry.local_file_id, entry.relative_path, identity, entry.file_type, entry.bytes.map(i64::try_from).transpose().map_err(|_| reject("personal_data_workspace_index_invalid"))?, modified, entry.sha256.as_ref().map(|_| "sha256"), entry.sha256, now])
                    .map_err(database_error("record restored local file observation"))?;
            }
            tx.execute("UPDATE workspaces SET local_root=?1,local_root_identity=?2,local_status='ready',last_local_scan_at=?3 WHERE workspace_id=?4", rusqlite::params![root,root_identity,now,workspace])
                .map_err(database_error("rebind restored workspace directory"))?;
            tx.execute("UPDATE workspace_exports SET desired_revision=desired_revision+1,export_state='pending',last_error_reason=NULL,updated_at=?1 WHERE workspace_id=?2", rusqlite::params![now,workspace]).map_err(database_error("invalidate restored workspace projection"))?;
        }
        tx.commit()
            .map_err(database_error("commit restored workspace relocation"))
    }
}

impl SqlitePersistence {
    /// # Errors
    /// 仅尚未执行且未获维护许可的恢复可一次确定新目录；并发预览变化时拒绝。
    pub fn plan_personal_workspace_restore(
        &mut self,
        operation: &contexthub_protocol::PersonalDataOperation,
        previous_digest: &str,
    ) -> Result<contexthub_protocol::PersonalDataOperation, PersistenceError> {
        use contexthub_protocol::{PersonalDataOperationKind, PersonalDataOperationState};
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin restored workspace plan"))?;
        let old = super::read(&tx, &operation.profile_id, &operation.operation_id)?
            .ok_or_else(|| reject("personal_data_operation_not_found"))?;
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_maintenance_fences WHERE profile_id=?1 AND state='draining')", [&operation.profile_id], |row| row.get(0))
            .map_err(database_error("check restored workspace plan maintenance"))?;
        if active
            || old.preview_digest != previous_digest
            || old.state != PersonalDataOperationState::Prepared
            || old.kind != PersonalDataOperationKind::Restore
        {
            return Err(reject("personal_data_preview_changed"));
        }
        let destination = operation
            .restore_preview
            .as_ref()
            .and_then(|preview| preview.workspace_destination.clone())
            .ok_or_else(|| reject("personal_data_workspace_destination_required"))?;
        let mut expected = old.clone();
        let preview = expected
            .restore_preview
            .as_mut()
            .ok_or_else(|| reject("personal_data_record_invalid"))?;
        if preview.workspace_destination.is_some() {
            return Err(reject("personal_data_preview_changed"));
        }
        preview.workspace_destination = Some(destination);
        expected
            .preview_digest
            .clone_from(&operation.preview_digest);
        expected.updated_at = operation.updated_at;
        if expected != *operation {
            return Err(reject("personal_data_preview_changed"));
        }
        tx.execute("UPDATE personal_data_operations SET record_json=?1,preview_digest=?2,updated_at=?3 WHERE profile_id=?4 AND operation_id=?5", rusqlite::params![encode(operation)?,operation.preview_digest,operation.updated_at,operation.profile_id,operation.operation_id])
            .map_err(database_error("save restored workspace plan"))?;
        crate::maintenance_operations::register(
            &tx,
            &contexthub_protocol::PersonalMaintenanceOperationStatus {
                operation_id: operation.operation_id.clone(),
                profile_id: operation.profile_id.clone(),
                kind: "restore".into(),
                state: "prepared".into(),
                preview_digest: operation.preview_digest.clone(),
            },
            operation.updated_at,
        )?;
        tx.commit()
            .map_err(database_error("commit restored workspace plan"))?;
        Ok(operation.clone())
    }
}
