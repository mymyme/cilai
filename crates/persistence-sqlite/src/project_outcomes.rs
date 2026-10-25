use super::{
    AcceptanceDecision, AttachProjectWorkspaceCommand, CreateDeliverableCommand,
    CreateDeliverableVersionCommand, CreateProjectCommand, DeleteProjectCommand,
    DeleteWorkspaceCommand, DeletedProjectSummary, DeletedWorkspaceSummary,
    DeliverableCarrierRecord, DeliverableFileBindingRecord, DeliverableRecord,
    DeliverableReviewRecord, DeliverableStatus, DeliverableVersionRecord, DeliverableVersionStatus,
    DetachProjectWorkspaceCommand, HashSet, PersistenceError, PersistenceErrorKind,
    ProjectOutcomeRepository, ProjectRecord, ProjectStatus, ReviewDeliverableVersionCommand,
    SqliteRepositoryTransaction, Transaction, UpdateProjectCommand, WorkspaceId,
    configuration_error, database_error, params,
};
use rusqlite::OptionalExtension;

impl ProjectOutcomeRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn projects_for_workspace(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ProjectRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT p.project_id, p.display_name, p.goal, p.status, p.state_revision, p.created_at, p.updated_at, primary_link.workspace_id FROM projects p JOIN project_workspaces requested_link ON requested_link.project_id=p.project_id LEFT JOIN project_workspaces primary_link ON primary_link.project_id=p.project_id AND primary_link.role='primary' WHERE requested_link.workspace_id=?1 ORDER BY p.created_at, p.project_id",
            )
            .map_err(database_error("prepare projects for workspace"))?;
        let rows = statement
            .query_map([workspace_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(database_error("query projects for workspace"))?;
        rows.map(|row| {
            let (
                project_id,
                display_name,
                goal,
                status,
                state_revision,
                created_at,
                updated_at,
                primary_workspace_id,
            ) = row.map_err(database_error("read project"))?;
            Ok(ProjectRecord {
                project_id,
                display_name,
                goal,
                status: ProjectStatus::try_from(status.as_str())
                    .map_err(|()| project_outcome_state_error("invalid_project_status"))?,
                primary_workspace_id: primary_workspace_id.map(WorkspaceId::new),
                state_revision,
                created_at,
                updated_at,
                replayed: false,
            })
        })
        .collect()
    }

    fn project_by_id(&mut self, project_id: &str) -> Result<ProjectRecord, Self::Error> {
        let exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
                [project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check project exists"))?;
        if !exists {
            return Err(configuration_error("project_not_found"));
        }
        read_project(&self.transaction, project_id, false)
    }

    fn workspaces_for_project(
        &mut self,
        project_id: &str,
    ) -> Result<Vec<WorkspaceId>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT workspace_id FROM project_workspaces WHERE project_id=?1 ORDER BY role, workspace_id",
            )
            .map_err(database_error("prepare workspaces for project"))?;
        let rows = statement
            .query_map([project_id], |row| row.get::<_, String>(0))
            .map_err(database_error("query workspaces for project"))?;
        rows.map(|row| {
            row.map(WorkspaceId::new)
                .map_err(database_error("read project workspace"))
        })
        .collect()
    }

    fn insert_project(
        &mut self,
        command: &CreateProjectCommand,
    ) -> Result<ProjectRecord, Self::Error> {
        if let Some((project_id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT project_id, request_fingerprint FROM projects WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read project idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(project_outcome_conflict("project_idempotency_conflict"));
            }
            return read_project(&self.transaction, &project_id, true);
        }
        if let Some(workspace_id) = &command.primary_workspace_id {
            let workspace_exists = self
                .transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                    [workspace_id.as_str()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(database_error("check project workspace"))?;
            if !workspace_exists {
                return Err(project_outcome_conflict("project_workspace_not_found"));
            }
        }
        self.transaction
            .execute(
                "INSERT INTO projects(project_id, display_name, goal, status, state_revision, client_operation_id, request_fingerprint, created_at, updated_at) VALUES (?1, ?2, ?3, 'active', 0, ?4, ?5, ?6, ?6)",
                params![
                    command.project_id,
                    command.display_name,
                    command.goal,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert project"))?;
        if let Some(workspace_id) = &command.primary_workspace_id {
            self.transaction
                .execute(
                    "INSERT INTO project_workspaces(project_id, workspace_id, role, created_at) VALUES (?1, ?2, 'primary', ?3)",
                    params![command.project_id, workspace_id.as_str(), command.occurred_at],
                )
                .map_err(database_error("insert project workspace"))?;
        }
        read_project(&self.transaction, &command.project_id, false)
    }

    fn update_project(
        &mut self,
        command: &UpdateProjectCommand,
    ) -> Result<ProjectRecord, Self::Error> {
        let current: Option<i64> = self
            .transaction
            .query_row(
                "SELECT state_revision FROM projects WHERE project_id=?1",
                [&command.project_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error("read project revision"))?;
        let Some(current) = current else {
            return Err(project_outcome_conflict("project_not_found"));
        };
        // The editor was showing a particular revision; refusing a stale one is what keeps a
        // second editor from overwriting the first without either of them noticing.
        if current != command.expected_revision {
            return Err(project_outcome_conflict("project_revision_conflict"));
        }
        self.transaction
            .execute(
                "UPDATE projects SET display_name=?2, goal=?3, state_revision=state_revision+1, updated_at=?4 WHERE project_id=?1",
                params![
                    command.project_id,
                    command.display_name,
                    command.goal,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("update project"))?;
        read_project(&self.transaction, &command.project_id, false)
    }

    fn attach_project_workspace(
        &mut self,
        command: &AttachProjectWorkspaceCommand,
    ) -> Result<ProjectRecord, Self::Error> {
        let project_exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
                [&command.project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check attach target project"))?;
        if !project_exists {
            return Err(project_outcome_conflict("project_not_found"));
        }
        let workspace_exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [command.workspace_id.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check attach workspace"))?;
        if !workspace_exists {
            return Err(project_outcome_conflict("project_workspace_not_found"));
        }
        let attached = self.transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_workspaces WHERE project_id=?1 AND workspace_id=?2)",
            params![command.project_id, command.workspace_id.as_str()],
            |row| row.get::<_, bool>(0),
        ).map_err(database_error("read workspace project binding"))?;
        if attached {
            return read_project(&self.transaction, &command.project_id, true);
        }
        let role = if self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM project_workspaces WHERE project_id=?1)",
                [&command.project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check project workspace count"))?
        {
            "supporting"
        } else {
            "primary"
        };
        self.transaction
            .execute(
                "INSERT INTO project_workspaces(project_id, workspace_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    command.project_id,
                    command.workspace_id.as_str(),
                    role,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert supporting project workspace"))?;
        if role == "primary" {
            self.transaction
                .execute(
                    "UPDATE my_work_items SET primary_workspace_id=?2 WHERE project_id=?1",
                    params![command.project_id, command.workspace_id.as_str()],
                )
                .map_err(database_error("refresh project work item workspace"))?;
        }
        read_project(&self.transaction, &command.project_id, false)
    }

    fn delete_workspace(
        &mut self,
        command: &DeleteWorkspaceCommand,
    ) -> Result<DeletedWorkspaceSummary, Self::Error> {
        let id = command.workspace_id.as_str();
        let exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check workspace before delete"))?;
        if !exists {
            return Err(project_outcome_conflict("workspace_not_found"));
        }
        let still_bound = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM project_workspaces WHERE workspace_id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check workspace project binding"))?;
        if still_bound {
            return Err(project_outcome_conflict("workspace_still_in_project"));
        }
        // Files here may still be the carrier of an accepted deliverable version; removing them
        // would silently damage another project's acceptance record.
        let referenced = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM deliverable_file_bindings WHERE workspace_id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check deliverable references"))?;
        if referenced {
            return Err(project_outcome_conflict(
                "workspace_referenced_by_deliverable",
            ));
        }

        let removed_files: i64 = self
            .transaction
            .query_row(
                "SELECT COUNT(*) FROM local_files WHERE workspace_id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(database_error("count workspace files"))?;
        let removed_events: i64 = self
            .transaction
            .query_row(
                "SELECT COUNT(*) FROM durable_events WHERE workspace_id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(database_error("count workspace events"))?;

        // local_files references itself and documents, so a strict leaf-first order is fragile.
        // Deferring foreign keys moves the check to commit: a genuine violation still fails, but
        // the statement order inside the transaction no longer has to be perfect.
        self.transaction
            .execute_batch("PRAGMA defer_foreign_keys = ON")
            .map_err(database_error("defer foreign keys"))?;
        for statement in [
            "DELETE FROM sync_plan_items WHERE plan_id IN (SELECT plan_id FROM sync_plans WHERE workspace_id=?1)",
            "DELETE FROM sync_executions WHERE plan_id IN (SELECT plan_id FROM sync_plans WHERE workspace_id=?1)",
            "DELETE FROM sync_plans WHERE workspace_id=?1",
            "DELETE FROM task_attempts WHERE task_id IN (SELECT task_id FROM tasks WHERE workspace_id=?1)",
            "DELETE FROM tasks WHERE workspace_id=?1",
            "DELETE FROM checksum_cache WHERE workspace_id=?1",
            "DELETE FROM conflicts WHERE workspace_id=?1",
            "DELETE FROM file_confirmation_text_snapshots WHERE workspace_id=?1",
            "DELETE FROM file_confirmations WHERE workspace_id=?1",
            "DELETE FROM file_states WHERE local_file_id IN (SELECT local_file_id FROM local_files WHERE workspace_id=?1)",
            "DELETE FROM local_file_observations WHERE snapshot_id IN (SELECT snapshot_id FROM local_scan_snapshots WHERE workspace_id=?1)",
            "DELETE FROM document_versions WHERE document_id IN (SELECT document_id FROM documents WHERE workspace_id=?1)",
            "DELETE FROM local_files WHERE workspace_id=?1",
            "DELETE FROM documents WHERE workspace_id=?1",
            "DELETE FROM local_scan_snapshots WHERE workspace_id=?1",
            "DELETE FROM platform_snapshots WHERE workspace_id=?1",
            "DELETE FROM workspace_platform_bindings WHERE workspace_id=?1",
            "DELETE FROM workspace_exports WHERE workspace_id=?1",
            "DELETE FROM my_work_items WHERE primary_workspace_id=?1",
            "DELETE FROM durable_events WHERE workspace_id=?1",
            "DELETE FROM workspaces WHERE workspace_id=?1",
        ] {
            self.transaction
                .execute(statement, [id])
                .map_err(database_error("delete workspace graph"))?;
        }

        Ok(DeletedWorkspaceSummary {
            workspace_id: id.to_owned(),
            removed_files,
            removed_events,
        })
    }

    fn detach_project_workspace(
        &mut self,
        command: &DetachProjectWorkspaceCommand,
    ) -> Result<ProjectRecord, Self::Error> {
        let project_exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
                [&command.project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check detach target project"))?;
        if !project_exists {
            return Err(project_outcome_conflict("project_not_found"));
        }
        let binding = self
            .transaction
            .query_row(
                "SELECT role FROM project_workspaces WHERE project_id=?1 AND workspace_id=?2",
                params![command.project_id, command.workspace_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read workspace binding for detach"))?;
        let Some(role) = binding else {
            // Already unbound: treat as a replay so a retried request is not an error.
            return read_project(&self.transaction, &command.project_id, true);
        };
        self.transaction
            .execute(
                "DELETE FROM project_workspaces WHERE project_id=?1 AND workspace_id=?2",
                params![command.project_id, command.workspace_id.as_str()],
            )
            .map_err(database_error("delete project workspace binding"))?;
        if role == "primary" {
            let replacement = self
                .transaction
                .query_row(
                    "SELECT workspace_id FROM project_workspaces WHERE project_id=?1 ORDER BY created_at, workspace_id LIMIT 1",
                    [&command.project_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(database_error("read replacement project workspace"))?;
            if let Some(replacement) = &replacement {
                self.transaction
                    .execute(
                        "UPDATE project_workspaces SET role='primary' WHERE project_id=?1 AND workspace_id=?2",
                        params![command.project_id, replacement],
                    )
                    .map_err(database_error("promote replacement project workspace"))?;
            }
            self.transaction
                .execute(
                    "UPDATE my_work_items SET primary_workspace_id=?2 WHERE project_id=?1",
                    params![command.project_id, replacement],
                )
                .map_err(database_error("refresh project work item workspace"))?;
        }
        read_project(&self.transaction, &command.project_id, false)
    }

    fn delete_project(
        &mut self,
        command: &DeleteProjectCommand,
    ) -> Result<DeletedProjectSummary, Self::Error> {
        let project_exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
                [&command.project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check project before delete"))?;
        if !project_exists {
            return Err(project_outcome_conflict("project_not_found"));
        }

        let count_of = |label: &'static str, sql: &str| -> Result<i64, PersistenceError> {
            self.transaction
                .query_row(sql, [&command.project_id], |row| row.get::<_, i64>(0))
                .map_err(database_error(label))
        };
        let removed_phases = count_of(
            "count project phases",
            "SELECT COUNT(*) FROM project_phases WHERE project_id=?1",
        )?;
        let removed_tasks = count_of(
            "count project tasks",
            "SELECT COUNT(*) FROM business_tasks WHERE project_id=?1",
        )?;
        let removed_deliverables = count_of(
            "count project deliverables",
            "SELECT COUNT(*) FROM deliverables WHERE project_id=?1",
        )?;
        let removed_attention_cases = count_of(
            "count project attention cases",
            "SELECT COUNT(*) FROM attention_cases WHERE project_id=?1",
        )?;

        let mut released = self
            .transaction
            .prepare("SELECT workspace_id FROM project_workspaces WHERE project_id=?1")
            .map_err(database_error("prepare released workspaces"))?;
        let released_workspace_ids = released
            .query_map([&command.project_id], |row| row.get::<_, String>(0))
            .map_err(database_error("read released workspaces"))?
            .collect::<Result<Vec<String>, _>>()
            .map_err(database_error("collect released workspaces"))?
            .into_iter()
            .map(WorkspaceId::new)
            .collect::<Vec<WorkspaceId>>();
        drop(released);

        // Deleted leaf-first: every foreign key in this graph is ON DELETE RESTRICT, so a wrong
        // order is refused by the database rather than silently cascading.
        for statement in [
            "DELETE FROM my_work_items WHERE project_id=?1",
            "DELETE FROM attention_case_transitions WHERE attention_case_id IN (SELECT attention_case_id FROM attention_cases WHERE project_id=?1)",
            "DELETE FROM attention_cases WHERE project_id=?1",
            "DELETE FROM business_task_moves WHERE business_task_id IN (SELECT business_task_id FROM business_tasks WHERE project_id=?1)",
            "DELETE FROM business_task_transitions WHERE business_task_id IN (SELECT business_task_id FROM business_tasks WHERE project_id=?1)",
            "DELETE FROM business_tasks WHERE project_id=?1",
            "DELETE FROM phase_transitions WHERE phase_id IN (SELECT phase_id FROM project_phases WHERE project_id=?1)",
            "DELETE FROM phase_dependencies WHERE plan_id IN (SELECT plan_id FROM phase_plans WHERE project_id=?1)",
            "DELETE FROM project_phases WHERE project_id=?1",
            "DELETE FROM phase_plans WHERE project_id=?1",
            "DELETE FROM deliverable_acceptances WHERE deliverable_id IN (SELECT deliverable_id FROM deliverables WHERE project_id=?1)",
            "DELETE FROM deliverable_file_bindings WHERE version_id IN (SELECT version_id FROM deliverable_versions WHERE deliverable_id IN (SELECT deliverable_id FROM deliverables WHERE project_id=?1))",
            "DELETE FROM deliverable_version_carriers WHERE version_id IN (SELECT version_id FROM deliverable_versions WHERE deliverable_id IN (SELECT deliverable_id FROM deliverables WHERE project_id=?1))",
            "DELETE FROM deliverable_version_carriers WHERE source_project_id=?1",
            "DELETE FROM deliverable_versions WHERE deliverable_id IN (SELECT deliverable_id FROM deliverables WHERE project_id=?1)",
            "DELETE FROM deliverables WHERE project_id=?1",
            "DELETE FROM project_workspaces WHERE project_id=?1",
            "DELETE FROM projects WHERE project_id=?1",
        ] {
            self.transaction
                .execute(statement, [&command.project_id])
                .map_err(database_error("delete project graph"))?;
        }

        Ok(DeletedProjectSummary {
            project_id: command.project_id.clone(),
            released_workspace_ids,
            removed_phases,
            removed_tasks,
            removed_deliverables,
            removed_attention_cases,
        })
    }

    fn deliverables_for_project(
        &mut self,
        project_id: &str,
    ) -> Result<Vec<DeliverableRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT deliverable_id FROM deliverables WHERE project_id=?1 ORDER BY updated_at DESC, deliverable_id",
            )
            .map_err(database_error("prepare project deliverables"))?;
        let ids = statement
            .query_map([project_id], |row| row.get::<_, String>(0))
            .map_err(database_error("query project deliverables"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read project deliverable id"))?;
        ids.into_iter()
            .map(|id| read_deliverable(&self.transaction, &id, false))
            .collect()
    }

    fn insert_deliverable(
        &mut self,
        command: &CreateDeliverableCommand,
    ) -> Result<DeliverableRecord, Self::Error> {
        if let Some((deliverable_id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT deliverable_id, request_fingerprint FROM deliverables WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read deliverable idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(project_outcome_conflict("deliverable_idempotency_conflict"));
            }
            return read_deliverable(&self.transaction, &deliverable_id, true);
        }
        let project_status = self
            .transaction
            .query_row(
                "SELECT status FROM projects WHERE project_id=?1",
                [&command.project_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read deliverable project"))?
            .ok_or_else(|| project_outcome_conflict("project_not_found"))?;
        if project_status != "active" {
            return Err(project_outcome_conflict("project_not_editable"));
        }
        self.transaction
            .execute(
                "INSERT INTO deliverables(deliverable_id, project_id, title, deliverable_type, owner_label, acceptance_criteria, status, current_accepted_version_id, state_revision, client_operation_id, request_fingerprint, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'draft', NULL, 0, ?7, ?8, ?9, ?9)",
                params![
                    command.deliverable_id,
                    command.project_id,
                    command.title,
                    command.deliverable_type,
                    command.owner_label,
                    command.acceptance_criteria,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert deliverable"))?;
        self.transaction
            .execute(
                "UPDATE projects SET state_revision=state_revision+1, updated_at=?2 WHERE project_id=?1",
                params![command.project_id, command.occurred_at],
            )
            .map_err(database_error("advance project revision for deliverable"))?;
        read_deliverable(&self.transaction, &command.deliverable_id, false)
    }

    fn versions_for_deliverable(
        &mut self,
        deliverable_id: &str,
    ) -> Result<Vec<DeliverableVersionRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT version_id FROM deliverable_versions WHERE deliverable_id=?1 ORDER BY version_no DESC",
            )
            .map_err(database_error("prepare deliverable versions"))?;
        let ids = statement
            .query_map([deliverable_id], |row| row.get::<_, String>(0))
            .map_err(database_error("query deliverable versions"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read deliverable version id"))?;
        ids.into_iter()
            .map(|id| read_deliverable_version(&self.transaction, &id, false))
            .collect()
    }

    fn insert_deliverable_version(
        &mut self,
        command: &CreateDeliverableVersionCommand,
    ) -> Result<DeliverableVersionRecord, Self::Error> {
        validate_deliverable_version_carrier_inputs(command)?;
        if let Some((version_id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT version_id, request_fingerprint FROM deliverable_versions WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read deliverable version idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(project_outcome_conflict("deliverable_version_idempotency_conflict"));
            }
            return read_deliverable_version(&self.transaction, &version_id, true);
        }
        let project_id = self
            .transaction
            .query_row(
                "SELECT project_id FROM deliverables WHERE deliverable_id=?1",
                [&command.deliverable_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read version deliverable"))?
            .ok_or_else(|| project_outcome_conflict("deliverable_not_found"))?;
        let version_no = self
            .transaction
            .query_row(
                "SELECT COALESCE(MAX(version_no), 0) + 1 FROM deliverable_versions WHERE deliverable_id=?1",
                [&command.deliverable_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(database_error("allocate deliverable version"))?;
        self.transaction
            .execute(
                "INSERT INTO deliverable_versions(version_id, deliverable_id, version_no, status, summary, client_operation_id, request_fingerprint, created_at) VALUES (?1, ?2, ?3, 'candidate', ?4, ?5, ?6, ?7)",
                params![
                    command.version_id,
                    command.deliverable_id,
                    version_no,
                    command.summary,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert deliverable version"))?;
        insert_deliverable_file_bindings(&self.transaction, command, &project_id)?;
        insert_deliverable_version_carriers(&self.transaction, command, &project_id)?;
        self.transaction
            .execute(
                "UPDATE deliverables SET status='candidate', state_revision=state_revision+1, updated_at=?2 WHERE deliverable_id=?1",
                params![command.deliverable_id, command.occurred_at],
            )
            .map_err(database_error("advance deliverable candidate"))?;
        read_deliverable_version(&self.transaction, &command.version_id, false)
    }

    fn review_deliverable_version(
        &mut self,
        command: &ReviewDeliverableVersionCommand,
    ) -> Result<DeliverableReviewRecord, Self::Error> {
        if let Some((acceptance_id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT acceptance_id, request_fingerprint FROM deliverable_acceptances WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read deliverable review idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(project_outcome_conflict("deliverable_review_idempotency_conflict"));
            }
            return read_deliverable_review(&self.transaction, &acceptance_id, true);
        }
        let revision = self
            .transaction
            .query_row(
                "SELECT state_revision FROM deliverables WHERE deliverable_id=?1",
                [&command.deliverable_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(database_error("read review deliverable revision"))?
            .ok_or_else(|| project_outcome_conflict("deliverable_not_found"))?;
        if revision != command.expected_deliverable_revision {
            return Err(project_outcome_conflict("deliverable_revision_conflict"));
        }
        let version_status = self
            .transaction
            .query_row(
                "SELECT status FROM deliverable_versions WHERE version_id=?1 AND deliverable_id=?2",
                params![command.version_id, command.deliverable_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read reviewed version"))?
            .ok_or_else(|| project_outcome_conflict("deliverable_version_not_found"))?;
        if version_status != "candidate" {
            return Err(project_outcome_conflict(
                "deliverable_version_not_candidate",
            ));
        }
        self.transaction
            .execute(
                "INSERT INTO deliverable_acceptances(acceptance_id, deliverable_id, version_id, decision, reviewer_label, comment, client_operation_id, request_fingerprint, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    command.acceptance_id,
                    command.deliverable_id,
                    command.version_id,
                    command.decision.as_str(),
                    command.reviewer_label,
                    command.comment,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert deliverable review"))?;
        match command.decision {
            AcceptanceDecision::Accepted => {
                self.transaction
                    .execute(
                        "UPDATE deliverable_versions SET status='superseded' WHERE deliverable_id=?1 AND status='accepted'",
                        [&command.deliverable_id],
                    )
                    .map_err(database_error("supersede accepted deliverable version"))?;
                self.transaction
                    .execute(
                        "UPDATE deliverable_versions SET status='accepted', accepted_at=?2 WHERE version_id=?1",
                        params![command.version_id, command.occurred_at],
                    )
                    .map_err(database_error("accept deliverable version"))?;
                self.transaction
                    .execute(
                        "UPDATE deliverables SET status='accepted', current_accepted_version_id=?2, state_revision=state_revision+1, updated_at=?3 WHERE deliverable_id=?1",
                        params![command.deliverable_id, command.version_id, command.occurred_at],
                    )
                    .map_err(database_error("set accepted deliverable version"))?;
            }
            AcceptanceDecision::Rejected => {
                self.transaction
                    .execute(
                        "UPDATE deliverable_versions SET status='rejected', rejected_at=?2 WHERE version_id=?1",
                        params![command.version_id, command.occurred_at],
                    )
                    .map_err(database_error("reject deliverable version"))?;
                self.transaction
                    .execute(
                        "UPDATE deliverables SET status='revision_needed', state_revision=state_revision+1, updated_at=?2 WHERE deliverable_id=?1",
                        params![command.deliverable_id, command.occurred_at],
                    )
                    .map_err(database_error("set deliverable revision needed"))?;
            }
        }
        read_deliverable_review(&self.transaction, &command.acceptance_id, false)
    }
}

fn validate_deliverable_version_carrier_inputs(
    command: &CreateDeliverableVersionCommand,
) -> Result<(), PersistenceError> {
    if command.local_file_ids.is_empty() && command.carriers.is_empty() {
        return Err(project_outcome_conflict(
            "deliverable_version_carrier_required",
        ));
    }
    let mut unique_ids = command.local_file_ids.clone();
    unique_ids.sort();
    unique_ids.dedup();
    if unique_ids.len() != command.local_file_ids.len() {
        return Err(project_outcome_conflict(
            "deliverable_version_files_duplicated",
        ));
    }
    let mut carrier_ids = HashSet::new();
    for carrier in &command.carriers {
        if !carrier_ids.insert(&carrier.carrier_id)
            || !matches!(
                carrier.carrier_type.as_str(),
                "directory" | "commit" | "external" | "project_version"
            )
            || !matches!(carrier.carrier_role.as_str(), "primary" | "supporting")
            || carrier.label.trim().is_empty()
            || carrier.reference.trim().is_empty()
            || carrier.version_label.trim().is_empty()
            || (carrier.carrier_type == "project_version") != carrier.source_version_id.is_some()
        {
            return Err(project_outcome_conflict(
                "invalid_deliverable_version_carrier",
            ));
        }
    }
    Ok(())
}

fn insert_deliverable_version_carriers(
    transaction: &Transaction<'_>,
    command: &CreateDeliverableVersionCommand,
    project_id: &str,
) -> Result<(), PersistenceError> {
    for carrier in &command.carriers {
        let (source_project_id, source_deliverable_id, source_version_id) = if carrier.carrier_type
            == "project_version"
        {
            let source = transaction
                    .query_row(
                        "SELECT d.project_id,v.deliverable_id,v.version_id
                         FROM deliverable_versions v
                         JOIN deliverables d ON d.deliverable_id=v.deliverable_id
                         WHERE v.version_id=?1 AND v.status IN ('accepted','superseded') AND d.project_id<>?2",
                        params![carrier.source_version_id, project_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                    )
                    .optional()
                    .map_err(database_error("read referenced project version"))?
                    .ok_or_else(|| project_outcome_conflict("project_version_reference_invalid"))?;
            (Some(source.0), Some(source.1), Some(source.2))
        } else {
            if carrier.carrier_type == "directory" {
                let valid: bool = transaction
                        .query_row(
                            "SELECT EXISTS(
                                SELECT 1 FROM local_files file
                                JOIN project_workspaces workspace ON workspace.workspace_id=file.workspace_id
                                WHERE file.local_file_id=?1 AND workspace.project_id=?2
                                  AND file.file_type='directory' AND file.presence_state='present'
                            )",
                            params![carrier.reference, project_id],
                            |row| row.get(0),
                        )
                        .map_err(database_error("validate deliverable directory carrier"))?;
                if !valid {
                    return Err(project_outcome_conflict(
                        "deliverable_directory_not_available",
                    ));
                }
            }
            (None, None, None)
        };
        transaction
            .execute(
                "INSERT INTO deliverable_version_carriers(carrier_id,version_id,carrier_type,carrier_role,label,reference,version_label,source_project_id,source_deliverable_id,source_version_id,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    carrier.carrier_id,
                    command.version_id,
                    carrier.carrier_type,
                    carrier.carrier_role,
                    carrier.label.trim(),
                    carrier.reference.trim(),
                    carrier.version_label.trim(),
                    source_project_id,
                    source_deliverable_id,
                    source_version_id,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert deliverable version carrier"))?;
    }
    Ok(())
}

fn insert_deliverable_file_bindings(
    transaction: &Transaction<'_>,
    command: &CreateDeliverableVersionCommand,
    project_id: &str,
) -> Result<(), PersistenceError> {
    for (index, local_file_id) in command.local_file_ids.iter().enumerate() {
        let file = transaction
            .query_row(
                "SELECT lf.workspace_id, lf.relative_path, lf.state_revision, lf.local_checksum_algorithm, lf.local_checksum_value FROM local_files lf JOIN project_workspaces pw ON pw.workspace_id=lf.workspace_id WHERE lf.local_file_id=?1 AND pw.project_id=?2 AND lf.file_type='file' AND lf.materialization_state='materialized' AND lf.scan_state='current'",
                params![local_file_id, project_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error("read candidate version file"))?
            .ok_or_else(|| project_outcome_conflict("deliverable_version_file_not_available"))?;
        let checksum_algorithm = file
            .3
            .ok_or_else(|| project_outcome_conflict("deliverable_version_file_checksum_missing"))?;
        let checksum_value = file
            .4
            .ok_or_else(|| project_outcome_conflict("deliverable_version_file_checksum_missing"))?;
        transaction
            .execute(
                "INSERT INTO deliverable_file_bindings(version_id, local_file_id, workspace_id, relative_path_at_binding, file_revision_at_binding, checksum_algorithm, checksum_value, binding_role, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    command.version_id,
                    local_file_id,
                    file.0,
                    file.1,
                    file.2,
                    checksum_algorithm,
                    checksum_value,
                    if index == 0 { "primary" } else { "supporting" },
                    command.occurred_at,
                ],
            )
            .map_err(database_error("bind deliverable version file"))?;
    }
    Ok(())
}

fn read_project(
    transaction: &Transaction<'_>,
    project_id: &str,
    replayed: bool,
) -> Result<ProjectRecord, PersistenceError> {
    let row = transaction
        .query_row(
            "SELECT p.display_name, p.goal, p.status, p.state_revision, p.created_at, p.updated_at, pw.workspace_id FROM projects p LEFT JOIN project_workspaces pw ON pw.project_id=p.project_id AND pw.role='primary' WHERE p.project_id=?1",
            [project_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .map_err(database_error("read project"))?;
    Ok(ProjectRecord {
        project_id: project_id.to_owned(),
        display_name: row.0,
        goal: row.1,
        status: ProjectStatus::try_from(row.2.as_str())
            .map_err(|()| project_outcome_state_error("invalid_project_status"))?,
        state_revision: row.3,
        created_at: row.4,
        updated_at: row.5,
        primary_workspace_id: row.6.map(WorkspaceId::new),
        replayed,
    })
}

fn read_deliverable(
    transaction: &Transaction<'_>,
    deliverable_id: &str,
    replayed: bool,
) -> Result<DeliverableRecord, PersistenceError> {
    let row = transaction
        .query_row(
            "SELECT project_id, title, deliverable_type, owner_label, acceptance_criteria, status, current_accepted_version_id, state_revision, created_at, updated_at FROM deliverables WHERE deliverable_id=?1",
            [deliverable_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?, row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?, row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?, row.get::<_, i64>(9)?,
                ))
            },
        )
        .map_err(database_error("read deliverable"))?;
    Ok(DeliverableRecord {
        deliverable_id: deliverable_id.to_owned(),
        project_id: row.0,
        title: row.1,
        deliverable_type: row.2,
        owner_label: row.3,
        acceptance_criteria: row.4,
        status: DeliverableStatus::try_from(row.5.as_str())
            .map_err(|()| project_outcome_state_error("invalid_deliverable_status"))?,
        current_accepted_version_id: row.6,
        state_revision: row.7,
        created_at: row.8,
        updated_at: row.9,
        replayed,
    })
}

fn read_deliverable_version(
    transaction: &Transaction<'_>,
    version_id: &str,
    replayed: bool,
) -> Result<DeliverableVersionRecord, PersistenceError> {
    let row = transaction
        .query_row(
            "SELECT deliverable_id, version_no, status, summary, created_at, accepted_at, rejected_at FROM deliverable_versions WHERE version_id=?1",
            [version_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?, row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .map_err(database_error("read deliverable version"))?;
    let mut statement = transaction
        .prepare(
            "SELECT local_file_id, workspace_id, relative_path_at_binding, file_revision_at_binding, checksum_algorithm, checksum_value, binding_role FROM deliverable_file_bindings WHERE version_id=?1 ORDER BY CASE binding_role WHEN 'primary' THEN 0 ELSE 1 END, relative_path_at_binding, local_file_id",
        )
        .map_err(database_error("prepare deliverable bindings"))?;
    let bindings = statement
        .query_map([version_id], |binding| {
            Ok(DeliverableFileBindingRecord {
                version_id: version_id.to_owned(),
                local_file_id: binding.get(0)?,
                workspace_id: WorkspaceId::new(binding.get::<_, String>(1)?),
                relative_path: binding.get(2)?,
                file_revision: binding.get(3)?,
                checksum_algorithm: binding.get(4)?,
                checksum_value: binding.get(5)?,
                binding_role: binding.get(6)?,
            })
        })
        .map_err(database_error("query deliverable bindings"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read deliverable bindings"))?;
    let mut carrier_statement = transaction
        .prepare(
            "SELECT carrier_id,carrier_type,carrier_role,label,reference,version_label,source_project_id,source_deliverable_id,source_version_id
             FROM deliverable_version_carriers WHERE version_id=?1
             ORDER BY CASE carrier_role WHEN 'primary' THEN 0 ELSE 1 END,carrier_type,carrier_id",
        )
        .map_err(database_error("prepare deliverable carriers"))?;
    let carriers = carrier_statement
        .query_map([version_id], |carrier| {
            Ok(DeliverableCarrierRecord {
                carrier_id: carrier.get(0)?,
                version_id: version_id.to_owned(),
                carrier_type: carrier.get(1)?,
                carrier_role: carrier.get(2)?,
                label: carrier.get(3)?,
                reference: carrier.get(4)?,
                version_label: carrier.get(5)?,
                source_project_id: carrier.get(6)?,
                source_deliverable_id: carrier.get(7)?,
                source_version_id: carrier.get(8)?,
            })
        })
        .map_err(database_error("query deliverable carriers"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read deliverable carriers"))?;
    Ok(DeliverableVersionRecord {
        version_id: version_id.to_owned(),
        deliverable_id: row.0,
        version_no: row.1,
        status: DeliverableVersionStatus::try_from(row.2.as_str())
            .map_err(|()| project_outcome_state_error("invalid_deliverable_version_status"))?,
        summary: row.3,
        created_at: row.4,
        accepted_at: row.5,
        rejected_at: row.6,
        bindings,
        carriers,
        replayed,
    })
}

fn read_deliverable_review(
    transaction: &Transaction<'_>,
    acceptance_id: &str,
    replayed: bool,
) -> Result<DeliverableReviewRecord, PersistenceError> {
    let row = transaction
        .query_row(
            "SELECT deliverable_id, version_id, decision, reviewer_label, comment, created_at FROM deliverable_acceptances WHERE acceptance_id=?1",
            [acceptance_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?, row.get::<_, i64>(5)?,
                ))
            },
        )
        .map_err(database_error("read deliverable review"))?;
    Ok(DeliverableReviewRecord {
        acceptance_id: acceptance_id.to_owned(),
        deliverable: read_deliverable(transaction, &row.0, false)?,
        version: read_deliverable_version(transaction, &row.1, false)?,
        decision: AcceptanceDecision::try_from(row.2.as_str())
            .map_err(|()| project_outcome_state_error("invalid_acceptance_decision"))?,
        reviewer_label: row.3,
        comment: row.4,
        created_at: row.5,
        replayed,
    })
}

fn project_outcome_conflict(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}

fn project_outcome_state_error(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Database, reason, reason)
}
