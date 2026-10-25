use super::DurableEventRepository;
use super::{
    AccountId, BaselineEntry, BaselineManifest, Checksum, ConfirmationEntry,
    ConfirmationInvalidationRecord, ConfirmationManifest, ConfirmationState, ConflictState,
    EventId, FileOperation, FileSnapshotRepository, FileStateOutput, FileStateRepository, HashMap,
    HashSet, LocalScanSnapshot, LocalSnapshotCommitResult, LocalWorkspaceStatus,
    MANIFEST_SCHEMA_VERSION, ManifestSet, NewDurableEvent, PersistedFileState, PersistenceError,
    PersistenceErrorKind, ProjectionRepository, ProjectionWriteExpectation,
    RecomputeFileStateCommand, Row, SnapshotKind, SqliteRepositoryTransaction, Transaction,
    WorkspaceExportRecord, WorkspaceFilePage, WorkspaceFileRecord, WorkspaceId, WorkspaceManifest,
    WorkspacePlatformBindingRecord, WorkspacePlatformBindingState, WorkspaceProjectionSnapshot,
    WorkspaceRecord, WorkspaceRepository, configuration_error, conflict_error, database_error,
    params,
};
use rusqlite::OptionalExtension;
use std::fmt::Write as _;

fn workspace_files_filtered(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    scope: &WorkspaceFileScope<'_>,
) -> Result<Vec<WorkspaceFileRecord>, PersistenceError> {
    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
            [workspace_id.as_str()],
            |row| row.get(0),
        )
        .map_err(database_error("validate workspace file list"))?;
    if !exists {
        return Err(conflict_error("workspace_not_found"));
    }
    // 按快照取时条件要单独写成等值：写成"参数为空或相等"，查询规划就用不上
    // first_seen_snapshot_id 的索引，只能把整个工作区的记录过一遍。
    let (filter, argument) = match scope {
        WorkspaceFileScope::All => ("", None),
        WorkspaceFileScope::PresentOnly => (" AND lf.materialization_state='materialized'", None),
        WorkspaceFileScope::FirstSeenIn(snapshot_id) => {
            (" AND lf.first_seen_snapshot_id=?2", Some(*snapshot_id))
        }
    };
    let mut statement = transaction.prepare(&format!(
        "SELECT lf.local_file_id, lf.relative_path, lf.normalized_relative_path, lf.file_identity, lf.file_type, lf.size_bytes, lf.modified_at_fs, CASE lf.materialization_state WHEN 'materialized' THEN 'present' WHEN 'missing' THEN 'missing' ELSE lf.materialization_state END, fs.display_state, fs.baseline_relation, fs.local_change_state, fs.confirmation_state, fs.conflict_state, fs.state_revision, lf.local_checksum_algorithm, lf.local_checksum_value, fs.baseline_version_id FROM local_files lf LEFT JOIN file_states fs ON fs.local_file_id=lf.local_file_id WHERE lf.workspace_id=?1{filter} ORDER BY lf.normalized_relative_path, lf.local_file_id",
    )).map_err(database_error("prepare workspace files"))?;
    let bind = |row: &rusqlite::Row<'_>| -> rusqlite::Result<WorkspaceFileRecord> {
        Ok(WorkspaceFileRecord {
            local_file_id: row.get(0)?,
            relative_path: row.get(1)?,
            normalized_relative_path: row.get(2)?,
            file_identity: row.get(3)?,
            file_type: row.get(4)?,
            size_bytes: row.get(5)?,
            modified_at_fs: row.get(6)?,
            presence_state: row.get(7)?,
            display_state: row.get(8)?,
            baseline_relation: row.get(9)?,
            local_change_state: row.get(10)?,
            confirmation_state: row.get(11)?,
            conflict_state: row.get(12)?,
            state_revision: row.get(13)?,
            checksum_algorithm: row.get(14)?,
            checksum_value: row.get(15)?,
            baseline_version_id: row.get(16)?,
        })
    };
    let rows = match argument {
        Some(snapshot_id) => statement.query_map(params![workspace_id.as_str(), snapshot_id], bind),
        None => statement.query_map(params![workspace_id.as_str()], bind),
    };
    rows.map_err(database_error("query workspace files"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read workspace file"))
}

impl WorkspaceRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn insert_workspace(&mut self, workspace: &WorkspaceRecord) -> Result<(), Self::Error> {
        self.transaction
            .execute(
                "INSERT INTO workspaces(workspace_id, display_name, local_root, local_root_identity, local_status, state_revision, last_local_scan_at, created_at, updated_at, workspace_kind) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    workspace.workspace_id.as_str(),
                    workspace.display_name,
                    workspace.local_root,
                    workspace.local_root_identity,
                    workspace.local_status.as_str(),
                    workspace.state_revision,
                    workspace.last_local_scan_at,
                    workspace.created_at,
                    workspace.updated_at,
                    workspace.workspace_kind.as_str(),
                ],
            )
            .map_err(database_error("insert workspace"))?;
        Ok(())
    }

    fn initialize_workspace_export(
        &mut self,
        export: &WorkspaceExportRecord,
    ) -> Result<(), Self::Error> {
        let state = if export.desired_revision == export.exported_revision {
            "current"
        } else {
            "pending"
        };
        self.transaction
            .execute(
                "INSERT INTO workspace_exports(workspace_id, desired_revision, exported_revision, export_state, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![export.workspace_id.as_str(), export.desired_revision, export.exported_revision, state, export.updated_at],
            )
            .map_err(database_error("initialize workspace export"))?;
        Ok(())
    }

    fn workspace(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<WorkspaceRecord>, Self::Error> {
        let record = self
            .transaction
            .query_row(
                "SELECT workspace_id, display_name, local_root, local_root_identity, local_status, state_revision, last_local_scan_at, created_at, updated_at, workspace_kind FROM workspaces WHERE workspace_id=?1",
                [workspace_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error("read workspace"))?;
        record
            .map(|record| {
                let local_status =
                    LocalWorkspaceStatus::try_from(record.4.as_str()).map_err(|()| {
                        PersistenceError::new(
                            PersistenceErrorKind::Database,
                            "database contains an unknown workspace status",
                        )
                    })?;
                let workspace_kind = contexthub_domain::WorkspaceKind::try_from(record.9.as_str())
                    .map_err(|()| {
                        PersistenceError::new(
                            PersistenceErrorKind::Database,
                            "database contains an unknown workspace kind",
                        )
                    })?;
                Ok(WorkspaceRecord {
                    workspace_id: WorkspaceId::new(record.0),
                    workspace_kind,
                    display_name: record.1,
                    local_root: record.2,
                    local_root_identity: record.3,
                    local_status,
                    state_revision: record.5,
                    last_local_scan_at: record.6,
                    created_at: record.7,
                    updated_at: record.8,
                })
            })
            .transpose()
    }

    fn workspaces(&mut self) -> Result<Vec<WorkspaceRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare("SELECT workspace_id FROM workspaces ORDER BY created_at, workspace_id")
            .map_err(database_error("prepare workspace list"))?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error("read workspace list"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("collect workspace list"))?;
        drop(statement);
        ids.into_iter()
            .map(|id| {
                self.workspace(&WorkspaceId::new(id))?.ok_or_else(|| {
                    PersistenceError::new(
                        PersistenceErrorKind::Database,
                        "workspace disappeared while listing transaction",
                    )
                })
            })
            .collect()
    }

    fn workspace_files(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<WorkspaceFileRecord>, Self::Error> {
        workspace_files_filtered(&self.transaction, workspace_id, &WorkspaceFileScope::All)
    }

    fn present_workspace_files(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<WorkspaceFileRecord>, Self::Error> {
        workspace_files_filtered(
            &self.transaction,
            workspace_id,
            &WorkspaceFileScope::PresentOnly,
        )
    }

    fn workspace_files_first_seen_in(
        &mut self,
        workspace_id: &WorkspaceId,
        snapshot_id: &str,
    ) -> Result<Vec<WorkspaceFileRecord>, Self::Error> {
        workspace_files_filtered(
            &self.transaction,
            workspace_id,
            &WorkspaceFileScope::FirstSeenIn(snapshot_id),
        )
    }

    fn workspace_files_page(
        &mut self,
        workspace_id: &WorkspaceId,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<WorkspaceFilePage, Self::Error> {
        if limit == 0 {
            return Err(conflict_error("workspace_file_page_limit_invalid"));
        }
        let exists: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("validate workspace file page"))?;
        if !exists {
            return Err(conflict_error("workspace_not_found"));
        }
        let canonical_path = "lf.local_file_id=(SELECT candidate.local_file_id FROM local_files candidate WHERE candidate.workspace_id=lf.workspace_id AND candidate.normalized_relative_path=lf.normalized_relative_path AND candidate.materialization_state<>'missing' ORDER BY candidate.updated_at DESC, candidate.local_file_id DESC LIMIT 1)";
        let cursor_path = cursor
            .map(|local_file_id| {
                self.transaction
                    .query_row(
                        &format!("SELECT lf.normalized_relative_path FROM local_files lf WHERE lf.workspace_id=?1 AND lf.local_file_id=?2 AND {canonical_path}"),
                        params![workspace_id.as_str(), local_file_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(database_error("resolve workspace file page cursor"))?
                    .ok_or_else(|| conflict_error("workspace_file_page_cursor_invalid"))
            })
            .transpose()?;
        let query_limit = i64::try_from(limit.saturating_add(1))
            .map_err(|_| conflict_error("workspace_file_page_limit_invalid"))?;
        let select = "SELECT lf.local_file_id, lf.relative_path, lf.normalized_relative_path, lf.file_identity, lf.file_type, lf.size_bytes, lf.modified_at_fs, CASE lf.materialization_state WHEN 'materialized' THEN 'present' WHEN 'missing' THEN 'missing' ELSE lf.materialization_state END, fs.display_state, fs.baseline_relation, fs.local_change_state, fs.confirmation_state, fs.conflict_state, fs.state_revision, lf.local_checksum_algorithm, lf.local_checksum_value, fs.baseline_version_id FROM local_files lf LEFT JOIN file_states fs ON fs.local_file_id=lf.local_file_id";
        // The database intentionally retains earlier and missing file identities for history.
        // The current tree exposes only the newest materialized identity for each normalized path.
        let row_to_file = |row: &Row<'_>| {
            Ok(WorkspaceFileRecord {
                local_file_id: row.get(0)?,
                relative_path: row.get(1)?,
                normalized_relative_path: row.get(2)?,
                file_identity: row.get(3)?,
                file_type: row.get(4)?,
                size_bytes: row.get(5)?,
                modified_at_fs: row.get(6)?,
                presence_state: row.get(7)?,
                display_state: row.get(8)?,
                baseline_relation: row.get(9)?,
                local_change_state: row.get(10)?,
                confirmation_state: row.get(11)?,
                conflict_state: row.get(12)?,
                state_revision: row.get(13)?,
                checksum_algorithm: row.get(14)?,
                checksum_value: row.get(15)?,
                baseline_version_id: row.get(16)?,
            })
        };
        let mut files = if let (Some(cursor_path), Some(cursor)) = (cursor_path, cursor) {
            let mut statement = self
                .transaction
                .prepare(&format!("{select} WHERE lf.workspace_id=?1 AND {canonical_path} AND (lf.normalized_relative_path, lf.local_file_id) > (?2, ?3) ORDER BY lf.normalized_relative_path, lf.local_file_id LIMIT ?4"))
                .map_err(database_error("prepare workspace file page"))?;
            statement
                .query_map(
                    params![workspace_id.as_str(), cursor_path, cursor, query_limit],
                    row_to_file,
                )
                .map_err(database_error("query workspace file page"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error("read workspace file page"))?
        } else {
            let mut statement = self
                .transaction
                .prepare(&format!("{select} WHERE lf.workspace_id=?1 AND {canonical_path} ORDER BY lf.normalized_relative_path, lf.local_file_id LIMIT ?2"))
                .map_err(database_error("prepare workspace file page"))?;
            statement
                .query_map(params![workspace_id.as_str(), query_limit], row_to_file)
                .map_err(database_error("query workspace file page"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error("read workspace file page"))?
        };
        let has_more = files.len() > limit;
        files.truncate(limit);
        let next_cursor = has_more
            .then(|| files.last().map(|file| file.local_file_id.clone()))
            .flatten();
        Ok(WorkspaceFilePage {
            files,
            next_cursor,
            has_more,
        })
    }

    fn platform_binding(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<WorkspacePlatformBindingRecord>, Self::Error> {
        let record = self
            .transaction
            .query_row(
                "SELECT account_id, organization_id, platform_workspace_id, project_id, platform_root, binding_state, platform_snapshot_revision, last_platform_check_at, created_at, updated_at, team_space_id, managed_root, device_id, baseline_revision, change_cursor, permission_role, permission_revision FROM workspace_platform_bindings WHERE workspace_id=?1",
                [workspace_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, Option<String>>(6)?, row.get::<_, Option<i64>>(7)?, row.get::<_, i64>(8)?, row.get::<_, i64>(9)?, row.get::<_, Option<String>>(10)?, row.get::<_, Option<String>>(11)?, row.get::<_, Option<String>>(12)?, row.get::<_, Option<String>>(13)?, row.get::<_, Option<String>>(14)?, row.get::<_, Option<String>>(15)?, row.get::<_, Option<String>>(16)?)),
            )
            .optional()
            .map_err(database_error("read workspace platform binding"))?;
        record
            .map(|record| {
                let binding_state = WorkspacePlatformBindingState::try_from(record.5.as_str())
                    .map_err(|()| {
                        PersistenceError::new(
                            PersistenceErrorKind::Database,
                            "database contains an unknown workspace platform binding state",
                        )
                    })?;
                Ok(WorkspacePlatformBindingRecord {
                    workspace_id: workspace_id.clone(),
                    account_id: AccountId::new(record.0),
                    organization_id: record.1,
                    platform_workspace_id: record.2,
                    project_id: record.3,
                    platform_root: record.4,
                    binding_state,
                    platform_snapshot_revision: record.6,
                    last_platform_check_at: record.7,
                    team_space_id: record.10,
                    managed_root: record.11,
                    device_id: record.12,
                    baseline_revision: record.13,
                    change_cursor: record.14,
                    permission_role: record.15,
                    permission_revision: record.16,
                    created_at: record.8,
                    updated_at: record.9,
                })
            })
            .transpose()
    }

    fn update_workspace_status(
        &mut self,
        workspace_id: &WorkspaceId,
        expected_revision: i64,
        status: LocalWorkspaceStatus,
        updated_at: i64,
    ) -> Result<WorkspaceRecord, Self::Error> {
        let changed = self
            .transaction
            .execute(
                "UPDATE workspaces SET local_status=?1, state_revision=state_revision+1, updated_at=?2 WHERE workspace_id=?3 AND state_revision=?4",
                params![status.as_str(), updated_at, workspace_id.as_str(), expected_revision],
            )
            .map_err(database_error("update workspace status"))?;
        if changed != 1 {
            return Err(PersistenceError::new(
                PersistenceErrorKind::Conflict,
                "workspace state revision does not match expected revision",
            ));
        }
        self.workspace(workspace_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "updated workspace disappeared inside transaction",
            )
        })
    }
}

#[derive(Clone)]
struct CurrentLocalFile {
    local_file_id: String,
    normalized_relative_path: String,
    file_identity: Option<String>,
}

impl FileSnapshotRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn next_local_snapshot_generation(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<i64, Self::Error> {
        let exists: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("check local snapshot workspace"))?;
        if !exists {
            return Err(conflict_error("local snapshot workspace does not exist"));
        }
        self.transaction
            .query_row(
                "SELECT COALESCE(MAX(generation), 0) + 1 FROM local_scan_snapshots WHERE workspace_id=?1",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("read next local snapshot generation"))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "identity resolution, path staging, observations and the durable event form one auditable transaction boundary"
    )]
    fn commit_local_snapshot(
        &mut self,
        snapshot: &LocalScanSnapshot,
    ) -> Result<LocalSnapshotCommitResult, Self::Error> {
        validate_local_snapshot(snapshot)?;
        let workspace_exists: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [snapshot.workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("check local snapshot workspace"))?;
        if !workspace_exists {
            return Err(conflict_error("local snapshot workspace does not exist"));
        }
        let expected_generation: i64 = self
            .transaction
            .query_row(
                "SELECT COALESCE(MAX(generation), 0) + 1 FROM local_scan_snapshots WHERE workspace_id=?1",
                [snapshot.workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("allocate local snapshot generation"))?;
        if snapshot.generation != expected_generation {
            return Err(conflict_error(
                "local snapshot generation is not the next generation",
            ));
        }

        let current_files = read_current_local_files(&self.transaction, &snapshot.workspace_id)?;
        let path_to_id = current_files
            .iter()
            .map(|file| {
                (
                    file.normalized_relative_path.clone(),
                    file.local_file_id.clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        let identity_to_id = unique_identity_map(&current_files);
        let mut used_existing = HashSet::new();
        let mut assignments = Vec::with_capacity(snapshot.observations.len());
        for observation in &snapshot.observations {
            let identity_match = observation
                .file_identity
                .as_ref()
                .and_then(|identity| identity_to_id.get(identity))
                .filter(|local_file_id| !used_existing.contains(*local_file_id))
                .cloned();
            let path_match = path_to_id
                .get(&observation.normalized_relative_path)
                .filter(|local_file_id| !used_existing.contains(*local_file_id))
                .and_then(|local_file_id| {
                    let current = current_files
                        .iter()
                        .find(|file| file.local_file_id == *local_file_id)?;
                    let identity_changed = current.file_identity.is_some()
                        && observation.file_identity.is_some()
                        && current.file_identity != observation.file_identity;
                    (!identity_changed).then(|| local_file_id.clone())
                });
            let retained_id = identity_match.or(path_match);
            if let Some(local_file_id) = &retained_id {
                used_existing.insert(local_file_id.clone());
            }
            assignments.push((
                retained_id
                    .clone()
                    .unwrap_or_else(|| observation.proposed_local_file_id.clone()),
                retained_id.is_some(),
            ));
        }

        self.transaction
            .execute(
                "INSERT INTO local_scan_snapshots(snapshot_id, workspace_id, snapshot_kind, generation, observed_at, completed_at, entry_count, issue_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![snapshot.snapshot_id, snapshot.workspace_id.as_str(), snapshot.snapshot_kind.as_str(), snapshot.generation, snapshot.observed_at, snapshot.completed_at, i64::try_from(snapshot.observations.len()).map_err(|_| configuration_error("local snapshot entry count exceeds database range"))?, snapshot.issue_count],
            )
            .map_err(database_error("insert local scan snapshot"))?;

        let mut missing_ids = HashSet::new();
        if snapshot.snapshot_kind == SnapshotKind::Full {
            for current in &current_files {
                missing_ids.insert(current.local_file_id.clone());
            }
        } else {
            for removed_path in &snapshot.removed_normalized_paths {
                if let Some(local_file_id) = path_to_id.get(removed_path) {
                    missing_ids.insert(local_file_id.clone());
                }
            }
            for (observation, (assigned_id, retained)) in
                snapshot.observations.iter().zip(&assignments)
            {
                if *retained {
                    missing_ids.insert(assigned_id.clone());
                }
                if let Some(displaced_id) = path_to_id.get(&observation.normalized_relative_path)
                    && displaced_id != assigned_id
                {
                    missing_ids.insert(displaced_id.clone());
                }
            }
        }
        for local_file_id in &missing_ids {
            self.transaction
                .execute(
                    "UPDATE local_files SET materialization_state='missing', missing_since_snapshot_id=?1, observed_at=?2, updated_at=?3, state_revision=state_revision+1 WHERE local_file_id=?4 AND workspace_id=?5 AND materialization_state<>'missing'",
                    params![snapshot.snapshot_id, snapshot.observed_at, snapshot.completed_at, local_file_id, snapshot.workspace_id.as_str()],
                )
                .map_err(database_error("stage missing local file"))?;
        }

        let mut retained_ids = Vec::new();
        let mut created_ids = Vec::new();
        let mut observed_path_to_id = HashMap::new();
        for (observation, (local_file_id, retained)) in
            snapshot.observations.iter().zip(&assignments)
        {
            if *retained {
                let changed = self
                    .transaction
                    .execute(
                        "UPDATE local_files SET relative_path=?1, normalized_relative_path=?2, file_identity=?3, materialization_state='materialized', file_type=?4, size_bytes=?5, modified_at_fs=?6, observed_at=?7, local_checksum_algorithm=?8, local_checksum_value=?9, scan_state='current', last_seen_snapshot_id=?10, missing_since_snapshot_id=NULL, state_revision=state_revision+1, updated_at=?11 WHERE local_file_id=?12 AND workspace_id=?13",
                        params![observation.relative_path, observation.normalized_relative_path, observation.file_identity, observation.file_type, observation.size_bytes, observation.modified_at_fs, snapshot.observed_at, observation.checksum_algorithm, observation.checksum_value, snapshot.snapshot_id, snapshot.completed_at, local_file_id, snapshot.workspace_id.as_str()],
                    )
                    .map_err(database_error("update retained local file"))?;
                if changed != 1 {
                    return Err(conflict_error("retained local file no longer exists"));
                }
                retained_ids.push(local_file_id.clone());
                missing_ids.remove(local_file_id);
            } else {
                self.transaction
                    .execute(
                        "INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, file_identity, materialization_state, file_type, size_bytes, modified_at_fs, observed_at, local_checksum_algorithm, local_checksum_value, scan_state, first_seen_snapshot_id, last_seen_snapshot_id, state_revision, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 'materialized', ?6, ?7, ?8, ?9, ?10, ?11, 'current', ?12, ?12, 0, ?9, ?13)",
                        params![local_file_id, snapshot.workspace_id.as_str(), observation.relative_path, observation.normalized_relative_path, observation.file_identity, observation.file_type, observation.size_bytes, observation.modified_at_fs, snapshot.observed_at, observation.checksum_algorithm, observation.checksum_value, snapshot.snapshot_id, snapshot.completed_at],
                    )
                    .map_err(database_error("insert observed local file"))?;
                created_ids.push(local_file_id.clone());
            }
            self.transaction
                .execute(
                    "INSERT INTO local_file_observations(snapshot_id, local_file_id, relative_path, normalized_relative_path, file_identity, file_type, size_bytes, modified_at_fs, checksum_algorithm, checksum_value, observed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![snapshot.snapshot_id, local_file_id, observation.relative_path, observation.normalized_relative_path, observation.file_identity, observation.file_type, observation.size_bytes, observation.modified_at_fs, observation.checksum_algorithm, observation.checksum_value, snapshot.observed_at],
                )
                .map_err(database_error("insert local file observation"))?;
            observed_path_to_id.insert(
                observation.normalized_relative_path.clone(),
                local_file_id.clone(),
            );
        }
        for (observation, (local_file_id, _)) in snapshot.observations.iter().zip(&assignments) {
            let parent_id = observation
                .normalized_relative_path
                .rsplit_once('/')
                .and_then(|(parent_path, _)| observed_path_to_id.get(parent_path));
            self.transaction
                .execute(
                    "UPDATE local_files SET parent_local_file_id=?1 WHERE local_file_id=?2",
                    params![parent_id, local_file_id],
                )
                .map_err(database_error("update local file parent"))?;
        }
        self.transaction
            .execute(
                "UPDATE workspaces SET local_status=CASE WHEN ?3 THEN 'ready' ELSE local_status END, last_local_scan_at=CASE WHEN ?3 THEN ?1 ELSE last_local_scan_at END, state_revision=state_revision+1, updated_at=?1 WHERE workspace_id=?2",
                params![snapshot.completed_at, snapshot.workspace_id.as_str(), snapshot.covers_whole_tree],
            )
            .map_err(database_error("update workspace after local snapshot"))?;
        let invalidations = invalidate_changed_local_confirmations(
            &self.transaction,
            &snapshot.workspace_id,
            &snapshot.snapshot_id,
            &snapshot.correlation_id,
            snapshot.completed_at,
        )?;
        for invalidation in invalidations {
            self.append_event(&NewDurableEvent {
                event_id: EventId::new(format!(
                    "{}:confirmation:{}",
                    snapshot.event_id.as_str(),
                    invalidation.confirmation_id
                )),
                workspace_id: Some(snapshot.workspace_id.clone()),
                event_type: "file.confirmation_invalidated".to_owned(),
                object_type: Some("file_confirmation".to_owned()),
                object_id: Some(invalidation.confirmation_id.clone()),
                payload_json: format!(
                    "{{\"local_file_id\":\"{}\",\"invalidation_id\":\"{}\",\"reason\":\"local_content_changed\"}}",
                    invalidation.local_file_id, invalidation.invalidation_id
                ),
                correlation_id: Some(snapshot.correlation_id.clone()),
                occurred_at: snapshot.completed_at,
                created_at: snapshot.completed_at,
            })?;
        }
        self.append_event(&NewDurableEvent {
            event_id: snapshot.event_id.clone(),
            workspace_id: Some(snapshot.workspace_id.clone()),
            event_type: "workspace.local_snapshot_committed".to_owned(),
            object_type: Some("local_scan_snapshot".to_owned()),
            object_id: Some(snapshot.snapshot_id.clone()),
            payload_json: format!(
                "{{\"generation\":{},\"entry_count\":{},\"missing_count\":{}}}",
                snapshot.generation,
                snapshot.observations.len(),
                missing_ids.len()
            ),
            correlation_id: Some(snapshot.correlation_id.clone()),
            occurred_at: snapshot.completed_at,
            created_at: snapshot.completed_at,
        })?;
        mark_projection_pending(
            &self.transaction,
            &snapshot.workspace_id,
            snapshot.completed_at,
        )?;
        prune_superseded_observations(
            &self.transaction,
            &snapshot.workspace_id,
            &snapshot.snapshot_id,
        )?;
        retained_ids.sort();
        created_ids.sort();
        let mut missing_local_file_ids = missing_ids.into_iter().collect::<Vec<_>>();
        missing_local_file_ids.sort();
        Ok(LocalSnapshotCommitResult {
            snapshot_id: snapshot.snapshot_id.clone(),
            retained_local_file_ids: retained_ids,
            created_local_file_ids: created_ids,
            missing_local_file_ids,
        })
    }
}

/// 每次扫描都会写一份完整的观测快照。当前的文件事实存在 `local_files` 里，快照只是
/// 提交过程中的中间产物——产品代码从来没有读过 `local_file_observations`，唯一的
/// SELECT 在测试里。
///
/// 手动扫描年代这只是慢性膨胀；接上文件监听之后它变成了急性的：实测两万多文件的工作区
/// 每次扫描写下 22,347 行，库在一小时里从 166 MB 涨到 3.3 GB。
///
/// 保留最近十份。数据模型文档的"可以清理"和"不能自动清理"两份清单里都没有这张表，
/// 所以这里不做激进删除：十份足够回看最近的扫描轨迹，同时把增长封住——真正的修复是
/// 让监听不再自我触发，这里只是给最坏情况兜底。
fn prune_superseded_observations(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    current_snapshot_id: &str,
) -> Result<(), PersistenceError> {
    transaction
        .execute(
            "DELETE FROM local_file_observations WHERE snapshot_id IN (
                 SELECT snapshot_id FROM local_scan_snapshots
                 WHERE workspace_id=?1 AND snapshot_id<>?2
                 ORDER BY generation DESC LIMIT -1 OFFSET 9
             )",
            params![workspace_id.as_str(), current_snapshot_id],
        )
        .map_err(database_error("prune superseded local file observations"))?;
    Ok(())
}

fn invalidate_changed_local_confirmations(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    snapshot_id: &str,
    correlation_id: &str,
    occurred_at: i64,
) -> Result<Vec<ConfirmationInvalidationRecord>, PersistenceError> {
    let mut statement = transaction
        .prepare(
            "SELECT fc.confirmation_id, fc.local_file_id, fc.confirmed_checksum_value, lf.local_checksum_value FROM file_confirmations fc JOIN local_files lf ON lf.local_file_id=fc.local_file_id WHERE fc.workspace_id=?1 AND fc.result='confirmed' AND fc.invalidated_at IS NULL AND (lf.materialization_state<>'materialized' OR lf.local_checksum_value IS NULL OR lf.local_checksum_value<>fc.confirmed_checksum_value) ORDER BY fc.confirmation_id",
        )
        .map_err(database_error("prepare changed local confirmations"))?;
    let rows = statement
        .query_map([workspace_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(database_error("query changed local confirmations"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read changed local confirmation row"))?;
    drop(statement);
    let mut result = Vec::with_capacity(rows.len());
    for (confirmation_id, local_file_id, previous_checksum, current_checksum) in rows {
        let invalidation_id = format!("invalidation:{snapshot_id}:{confirmation_id}");
        transaction
            .execute(
                "UPDATE file_confirmations SET invalidated_at=?1, invalidation_reason='local_content_changed' WHERE confirmation_id=?2 AND invalidated_at IS NULL",
                params![occurred_at, confirmation_id],
            )
            .map_err(database_error("invalidate changed local confirmation"))?;
        transaction
            .execute(
                "INSERT INTO confirmation_invalidations(invalidation_id, confirmation_id, reason, previous_checksum, current_checksum, occurred_at, correlation_id) VALUES (?1, ?2, 'local_content_changed', ?3, ?4, ?5, ?6)",
                params![invalidation_id, confirmation_id, previous_checksum, current_checksum, occurred_at, correlation_id],
            )
            .map_err(database_error("record changed local confirmation"))?;
        let confirm_bit = i64::from(FileOperation::Confirm.bit());
        let mutation_bits =
            i64::from(FileOperation::PrepareSubmission.bit() | FileOperation::Submit.bit());
        transaction
            .execute(
                "UPDATE file_states SET confirmation_state='invalidated_content_changed', display_state=CASE WHEN local_change_state='deleted_candidate' THEN 'local_missing' ELSE 'confirmation_required' END, capability_bits=(capability_bits | ?1) & ~?2, computed_at=?3, state_revision=state_revision+1 WHERE local_file_id=?4",
                params![confirm_bit, mutation_bits, occurred_at, local_file_id],
            )
            .map_err(database_error("update state for changed local confirmation"))?;
        result.push(ConfirmationInvalidationRecord {
            invalidation_id,
            confirmation_id,
            workspace_id: workspace_id.clone(),
            local_file_id,
            reason: "local_content_changed".to_owned(),
        });
    }
    Ok(result)
}

fn read_current_local_files(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<CurrentLocalFile>, PersistenceError> {
    let mut statement = transaction
        .prepare(
            "SELECT local_file_id, normalized_relative_path, file_identity FROM local_files WHERE workspace_id=?1 AND materialization_state<>'missing' ORDER BY local_file_id",
        )
        .map_err(database_error("prepare current local file query"))?;
    statement
        .query_map([workspace_id.as_str()], |row| {
            Ok(CurrentLocalFile {
                local_file_id: row.get(0)?,
                normalized_relative_path: row.get(1)?,
                file_identity: row.get(2)?,
            })
        })
        .map_err(database_error("query current local files"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read current local files"))
}

fn unique_identity_map(current_files: &[CurrentLocalFile]) -> HashMap<String, String> {
    let mut identities = HashMap::new();
    let mut ambiguous = HashSet::new();
    for file in current_files {
        if let Some(identity) = &file.file_identity
            && identities
                .insert(identity.clone(), file.local_file_id.clone())
                .is_some()
        {
            ambiguous.insert(identity.clone());
        }
    }
    identities.retain(|identity, _| !ambiguous.contains(identity));
    identities
}

fn validate_local_snapshot(snapshot: &LocalScanSnapshot) -> Result<(), PersistenceError> {
    if snapshot.snapshot_id.is_empty()
        || snapshot.workspace_id.as_str().is_empty()
        || snapshot.generation <= 0
        || snapshot.observed_at < 0
        || snapshot.completed_at < snapshot.observed_at
        || snapshot.issue_count < 0
        || snapshot.event_id.as_str().is_empty()
        || snapshot.correlation_id.is_empty()
        || (snapshot.snapshot_kind == SnapshotKind::Full && snapshot.issue_count != 0)
        || (snapshot.snapshot_kind == SnapshotKind::Full
            && !snapshot.removed_normalized_paths.is_empty())
        || (snapshot.snapshot_kind == SnapshotKind::Full && !snapshot.covers_whole_tree)
        || (snapshot.issue_count != 0 && !snapshot.removed_normalized_paths.is_empty())
    {
        return Err(configuration_error("local snapshot fields are invalid"));
    }
    let mut paths = HashSet::new();
    let mut proposed_ids = HashSet::new();
    for observation in &snapshot.observations {
        if observation.proposed_local_file_id.is_empty()
            || observation.relative_path.is_empty()
            || observation.normalized_relative_path.is_empty()
            || !paths.insert(observation.normalized_relative_path.as_str())
            || !proposed_ids.insert(observation.proposed_local_file_id.as_str())
            || !matches!(
                observation.file_type.as_str(),
                "file" | "directory" | "symlink"
            )
            || observation.size_bytes.is_some_and(|value| value < 0)
            || observation.modified_at_fs.is_some_and(|value| value < 0)
            || observation.checksum_algorithm.is_some() != observation.checksum_value.is_some()
        {
            return Err(configuration_error("local file observation is invalid"));
        }
    }
    let mut removed_paths = HashSet::new();
    for removed_path in &snapshot.removed_normalized_paths {
        if removed_path.is_empty()
            || paths.contains(removed_path.as_str())
            || !removed_paths.insert(removed_path.as_str())
        {
            return Err(configuration_error(
                "local snapshot removed path is invalid",
            ));
        }
    }
    Ok(())
}

impl FileStateRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn persist_computed_file_state(
        &mut self,
        command: &RecomputeFileStateCommand,
        output: &FileStateOutput,
    ) -> Result<PersistedFileState, Self::Error> {
        validate_file_state_command(command)?;
        let local_file_workspace = self
            .transaction
            .query_row(
                "SELECT workspace_id FROM local_files WHERE local_file_id=?1",
                [command.local_file_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read computed file workspace"))?;
        if local_file_workspace.as_deref() != Some(command.workspace_id.as_str()) {
            return Err(conflict_error(
                "computed file does not belong to the requested workspace",
            ));
        }
        let current_revision = self
            .transaction
            .query_row(
                "SELECT state_revision FROM file_states WHERE local_file_id=?1",
                [command.local_file_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(database_error("read current file state revision"))?;
        if current_revision != command.expected_state_revision {
            return Err(conflict_error(
                "file state revision does not match expected revision",
            ));
        }

        let invalidated_confirmation =
            reconcile_confirmation(&self.transaction, command, output.confirmation_state)?;
        reconcile_conflict(&self.transaction, command, output.conflict_state)?;
        let next_revision = current_revision.map_or(0, |revision| revision + 1);
        let capability_reasons_json = capability_reasons_json(output);
        match current_revision {
            None => {
                self.transaction
                    .execute(
                        "INSERT INTO file_states(local_file_id, baseline_version_id, baseline_checksum_algorithm, baseline_checksum_value, baseline_relation, local_change_state, confirmation_state, submission_state, conflict_state, display_state, capability_bits, capability_reasons_json, computed_at, state_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0)",
                        params![command.local_file_id, command.input.baseline_version_id, command.input.baseline_checksum_algorithm, command.input.baseline_checksum_value, output.baseline_relation.as_str(), output.local_change_state.as_str(), output.confirmation_state.as_str(), output.submission_state.as_str(), output.conflict_state.as_str(), output.display_state.as_str(), i64::from(output.capabilities.bits), capability_reasons_json, command.computed_at],
                    )
                    .map_err(database_error("insert computed file state"))?;
            }
            Some(expected_revision) => {
                let changed = self
                    .transaction
                    .execute(
                        "UPDATE file_states SET baseline_version_id=?1, baseline_checksum_algorithm=?2, baseline_checksum_value=?3, baseline_relation=?4, local_change_state=?5, confirmation_state=?6, submission_state=?7, conflict_state=?8, display_state=?9, capability_bits=?10, capability_reasons_json=?11, computed_at=?12, state_revision=state_revision+1 WHERE local_file_id=?13 AND state_revision=?14",
                        params![command.input.baseline_version_id, command.input.baseline_checksum_algorithm, command.input.baseline_checksum_value, output.baseline_relation.as_str(), output.local_change_state.as_str(), output.confirmation_state.as_str(), output.submission_state.as_str(), output.conflict_state.as_str(), output.display_state.as_str(), i64::from(output.capabilities.bits), capability_reasons_json, command.computed_at, command.local_file_id, expected_revision],
                    )
                    .map_err(database_error("update computed file state"))?;
                if changed != 1 {
                    return Err(conflict_error(
                        "file state changed before computed state update",
                    ));
                }
            }
        }
        mark_projection_pending(
            &self.transaction,
            &command.workspace_id,
            command.computed_at,
        )?;
        Ok(PersistedFileState {
            local_file_id: command.local_file_id.clone(),
            output: output.clone(),
            state_revision: next_revision,
            invalidated_confirmation,
        })
    }
}

impl ProjectionRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn projection_snapshot(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceProjectionSnapshot, Self::Error> {
        let header = self
            .transaction
            .query_row(
                "SELECT w.local_root, w.created_at, e.desired_revision, e.exported_revision, e.manifest_checksum, e.export_state, b.project_id, b.platform_root FROM workspaces w JOIN workspace_exports e ON e.workspace_id=w.workspace_id LEFT JOIN workspace_platform_bindings b ON b.workspace_id=w.workspace_id AND b.binding_state <> 'disconnected' WHERE w.workspace_id=?1",
                [workspace_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error("read projection header"))?
            .ok_or_else(|| conflict_error("workspace projection state does not exist"))?;
        let (
            local_root,
            created_at,
            desired,
            exported,
            checksum,
            export_state,
            project_id,
            platform_root,
        ) = header;
        let environment = if platform_root.is_some() {
            "platform".to_owned()
        } else {
            "local".to_owned()
        };
        if desired < exported {
            return Err(configuration_error(
                "workspace projection revision is inconsistent",
            ));
        }
        let baseline = read_baseline_manifest(&self.transaction, workspace_id, desired)?;
        let confirmation = read_confirmation_manifest(&self.transaction, workspace_id, desired)?;
        Ok(WorkspaceProjectionSnapshot {
            workspace_id: workspace_id.clone(),
            workspace_root: local_root,
            desired_revision: desired,
            expectation: ProjectionWriteExpectation {
                exported_revision: exported,
                manifest_checksum: checksum,
                allow_incomplete_recovery: export_state == "failed",
            },
            manifests: ManifestSet {
                workspace: WorkspaceManifest {
                    schema_version: MANIFEST_SCHEMA_VERSION,
                    database_export_revision: desired,
                    workspace_id: workspace_id.as_str().to_owned(),
                    project_id,
                    platform_root,
                    local_binding_id: workspace_id.as_str().to_owned(),
                    environment,
                    created_at,
                },
                baseline,
                confirmation,
            },
        })
    }

    fn mark_projection_succeeded(
        &mut self,
        workspace_id: &WorkspaceId,
        expected_desired_revision: i64,
        manifest_checksum: &str,
        updated_at: i64,
    ) -> Result<(), Self::Error> {
        if manifest_checksum.is_empty() || expected_desired_revision < 0 || updated_at < 0 {
            return Err(configuration_error("projection success result is invalid"));
        }
        let changed = self
            .transaction
            .execute(
                "UPDATE workspace_exports SET exported_revision=desired_revision, manifest_checksum=?1, export_state='current', last_error_reason=NULL, updated_at=?2 WHERE workspace_id=?3 AND desired_revision=?4",
                params![manifest_checksum, updated_at, workspace_id.as_str(), expected_desired_revision],
            )
            .map_err(database_error("mark projection succeeded"))?;
        if changed != 1 {
            return Err(conflict_error(
                "projection desired revision changed before success was recorded",
            ));
        }
        Ok(())
    }

    fn mark_projection_failed(
        &mut self,
        workspace_id: &WorkspaceId,
        expected_desired_revision: i64,
        reason: &str,
        conflict: bool,
        updated_at: i64,
    ) -> Result<(), Self::Error> {
        if reason.is_empty() || expected_desired_revision < 0 || updated_at < 0 {
            return Err(configuration_error("projection failure result is invalid"));
        }
        let changed = self
            .transaction
            .execute(
                "UPDATE workspace_exports SET export_state=?1, last_error_reason=?2, updated_at=?3 WHERE workspace_id=?4 AND desired_revision=?5",
                params![if conflict { "conflict" } else { "failed" }, reason, updated_at, workspace_id.as_str(), expected_desired_revision],
            )
            .map_err(database_error("mark projection failed"))?;
        if changed != 1 {
            return Err(conflict_error(
                "projection desired revision changed before failure was recorded",
            ));
        }
        Ok(())
    }
}

fn read_baseline_manifest(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    revision: i64,
) -> Result<BaselineManifest, PersistenceError> {
    let mut statement = transaction
        .prepare(
            "SELECT lf.document_id, lf.relative_path, fs.baseline_version_id, fs.baseline_checksum_algorithm, fs.baseline_checksum_value, lf.updated_at FROM local_files lf JOIN file_states fs ON fs.local_file_id=lf.local_file_id WHERE lf.workspace_id=?1 AND lf.materialization_state='materialized' AND fs.baseline_version_id IS NOT NULL ORDER BY lf.normalized_relative_path, lf.local_file_id",
        )
        .map_err(database_error("prepare baseline projection"))?;
    let rows = statement
        .query_map([workspace_id.as_str()], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(database_error("query baseline projection"))?;
    let mut files = Vec::new();
    for row in rows {
        let (document_id, relative_path, version_id, algorithm, value, materialized_at) =
            row.map_err(database_error("read baseline projection row"))?;
        let (Some(document_id), Some(algorithm), Some(value)) = (document_id, algorithm, value)
        else {
            return Err(configuration_error(
                "baseline projection contains incomplete identity or checksum",
            ));
        };
        files.push(BaselineEntry {
            document_id,
            relative_path,
            baseline_version_id: version_id,
            checksum: Checksum { algorithm, value },
            materialized_at: Some(materialized_at),
        });
    }
    Ok(BaselineManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        database_export_revision: revision,
        files,
    })
}

fn read_confirmation_manifest(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    revision: i64,
) -> Result<ConfirmationManifest, PersistenceError> {
    let mut statement = transaction
        .prepare(
            "SELECT lf.relative_path, fc.document_id, fc.confirmation_id, fc.confirmed_checksum_algorithm, fc.confirmed_checksum_value, fc.baseline_version_id, fc.confirmed_by_account_id, fc.created_at FROM file_confirmations fc JOIN local_files lf ON lf.local_file_id=fc.local_file_id WHERE fc.workspace_id=?1 AND fc.result='confirmed' AND fc.invalidated_at IS NULL AND fc.workspace_kind_at_confirmation=(SELECT workspace_kind FROM workspaces WHERE workspace_id=?1) ORDER BY lf.normalized_relative_path, fc.confirmation_id",
        )
        .map_err(database_error("prepare confirmation projection"))?;
    let rows = statement
        .query_map([workspace_id.as_str()], |row| {
            Ok(ConfirmationEntry {
                relative_path: row.get(0)?,
                document_id: row.get(1)?,
                confirmation_id: row.get(2)?,
                confirmed_checksum: Checksum {
                    algorithm: row.get(3)?,
                    value: row.get(4)?,
                },
                baseline_version_id: row.get(5)?,
                confirmed_by: row.get(6)?,
                confirmed_at: row.get(7)?,
            })
        })
        .map_err(database_error("query confirmation projection"))?;
    let confirmations = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read confirmation projection row"))?;
    Ok(ConfirmationManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        database_export_revision: revision,
        confirmations,
    })
}

pub(super) fn mark_projection_pending(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    updated_at: i64,
) -> Result<(), PersistenceError> {
    if updated_at < 0 {
        return Err(configuration_error("projection pending time is invalid"));
    }
    let changed = transaction
        .execute(
            "UPDATE workspace_exports SET desired_revision=desired_revision+1, export_state=CASE WHEN export_state='conflict' THEN 'conflict' ELSE 'pending' END, last_error_reason=CASE WHEN export_state='conflict' THEN last_error_reason ELSE NULL END, updated_at=?1 WHERE workspace_id=?2",
            params![updated_at, workspace_id.as_str()],
        )
        .map_err(database_error("mark workspace projection pending"))?;
    if changed != 1 {
        return Err(conflict_error(
            "workspace projection state does not exist for changed business facts",
        ));
    }
    Ok(())
}

fn validate_file_state_command(
    command: &RecomputeFileStateCommand,
) -> Result<(), PersistenceError> {
    if command.workspace_id.as_str().is_empty()
        || command.local_file_id.is_empty()
        || command
            .expected_state_revision
            .is_some_and(|revision| revision < 0)
        || command.event_id.as_str().is_empty()
        || command
            .confirmation_invalidation_event_id
            .as_str()
            .is_empty()
        || command.correlation_id.is_empty()
        || command.computed_at < 0
    {
        return Err(configuration_error(
            "computed file state command is invalid",
        ));
    }
    Ok(())
}

fn reconcile_confirmation(
    transaction: &Transaction<'_>,
    command: &RecomputeFileStateCommand,
    confirmation_state: ConfirmationState,
) -> Result<Option<ConfirmationInvalidationRecord>, PersistenceError> {
    let reason = match confirmation_state {
        ConfirmationState::InvalidatedContentChanged => Some("local_content_changed"),
        ConfirmationState::InvalidatedBaselineChanged => Some("baseline_changed"),
        ConfirmationState::BlockedByConflict => Some("conflict_detected"),
        ConfirmationState::BlockedByPermission => Some("permission_changed"),
        _ => None,
    };
    let Some(reason) = reason else {
        return Ok(None);
    };
    let active = transaction
        .query_row(
            "SELECT confirmation_id, confirmed_checksum_value FROM file_confirmations WHERE local_file_id=?1 AND invalidated_at IS NULL AND result='confirmed'",
            [command.local_file_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error("read active confirmation for reconciliation"))?;
    let Some((confirmation_id, previous_checksum)) = active else {
        return Ok(None);
    };
    if command.confirmation_invalidation_id.is_empty() {
        return Err(configuration_error(
            "confirmation invalidation requires an identity",
        ));
    }
    transaction
        .execute(
            "UPDATE file_confirmations SET invalidated_at=?1, invalidation_reason=?2 WHERE confirmation_id=?3 AND invalidated_at IS NULL",
            params![command.computed_at, reason, confirmation_id],
        )
        .map_err(database_error("invalidate active confirmation"))?;
    transaction
        .execute(
            "INSERT INTO confirmation_invalidations(invalidation_id, confirmation_id, reason, previous_checksum, current_checksum, occurred_at, correlation_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![command.confirmation_invalidation_id, confirmation_id, reason, previous_checksum, command.input.local_checksum_value, command.computed_at, command.correlation_id],
        )
        .map_err(database_error("record confirmation invalidation"))?;
    Ok(Some(ConfirmationInvalidationRecord {
        invalidation_id: command.confirmation_invalidation_id.clone(),
        confirmation_id,
        workspace_id: command.workspace_id.clone(),
        local_file_id: command.local_file_id.clone(),
        reason: reason.to_owned(),
    }))
}

fn reconcile_conflict(
    transaction: &Transaction<'_>,
    command: &RecomputeFileStateCommand,
    conflict_state: ConflictState,
) -> Result<(), PersistenceError> {
    let active = transaction
        .query_row(
            "SELECT conflict_id, conflict_type FROM conflicts WHERE local_file_id=?1 AND state IN ('detected', 'in_progress')",
            [command.local_file_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error("read active conflict for reconciliation"))?;
    match (&active, command.input.active_conflict) {
        (None, Some(input)) if input.blocks_mutation() => {
            return Err(conflict_error(
                "computed state references an active conflict that does not exist",
            ));
        }
        (Some(_), None) => {
            return Err(conflict_error(
                "computed state omitted the current active conflict",
            ));
        }
        (Some((_, active_type)), Some(input))
            if input != ConflictState::Unresolved && active_type != input.as_str() =>
        {
            return Err(conflict_error(
                "computed state active conflict does not match the database",
            ));
        }
        _ => {}
    }
    if !conflict_state.blocks_mutation() {
        if let Some((conflict_id, _)) = active {
            transaction
                .execute(
                    "UPDATE conflicts SET state='superseded', resolved_at=?1, state_revision=state_revision+1 WHERE conflict_id=?2",
                    params![command.computed_at, conflict_id],
                )
                .map_err(database_error("supersede cleared conflict"))?;
        }
        return Ok(());
    }
    if conflict_state == ConflictState::Unresolved {
        return active.map_or_else(
            || {
                Err(conflict_error(
                    "unresolved state requires an active conflict",
                ))
            },
            |_| Ok(()),
        );
    }
    let conflict_type = conflict_state.as_str();
    if active
        .as_ref()
        .is_some_and(|(_, active_type)| active_type == conflict_type)
    {
        return Ok(());
    }
    if let Some((conflict_id, _)) = active {
        transaction
            .execute(
                "UPDATE conflicts SET state='superseded', resolved_at=?1, state_revision=state_revision+1 WHERE conflict_id=?2",
                params![command.computed_at, conflict_id],
            )
            .map_err(database_error("supersede changed conflict"))?;
    }
    if command.derived_conflict_id.is_empty() {
        return Err(configuration_error("derived conflict requires an identity"));
    }
    transaction
        .execute(
            "INSERT INTO conflicts(conflict_id, workspace_id, local_file_id, conflict_type, state, baseline_version_id, platform_version_id, baseline_checksum, platform_checksum, local_checksum, detected_at, state_revision) VALUES (?1, ?2, ?3, ?4, 'detected', ?5, ?6, ?7, ?8, ?9, ?10, 0)",
            params![command.derived_conflict_id, command.workspace_id.as_str(), command.local_file_id, conflict_type, command.input.baseline_version_id, command.input.latest_published_version_id, command.input.baseline_checksum_value, command.input.latest_published_checksum_value, command.input.local_checksum_value, command.computed_at],
        )
        .map_err(database_error("insert derived conflict"))?;
    Ok(())
}

fn capability_reasons_json(output: &FileStateOutput) -> String {
    let mut json = String::from("[");
    for (index, block) in output.capabilities.blocks.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        write!(
            &mut json,
            "{{\"operation\":\"{}\",\"reason\":\"{}\"}}",
            block.operation.as_str(),
            block.reason.as_str()
        )
        .expect("writing fixed capability values to String cannot fail");
    }
    json.push(']');
    json
}

/// 读取工作区文件记录。`present_only` 只取当前还在磁盘上的那些。
///
/// 增量比对只关心当前存在的记录，而历史上标为缺失的旧记录可能是它们的好几倍——
/// 本机实测 49,081 条里有 26,732 条是缺失的。把它们一起读出来，只是白白拉长
/// 持锁时间。
/// 读取工作区文件记录时的范围。
enum WorkspaceFileScope<'a> {
    All,
    PresentOnly,
    /// 只取在这次快照里首次出现的记录。分批写入首次索引时每批只需处理本批新增的
    /// 记录，按工作区全量读会让每一批都比上一批更慢。
    FirstSeenIn(&'a str),
}
