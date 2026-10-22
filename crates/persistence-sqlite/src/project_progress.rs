use super::{PersistenceError, PersistenceErrorKind, SqliteRepositoryTransaction, database_error};
use contexthub_application::{
    ActivatePhasePlanCommand, BusinessTaskContributionRecord, BusinessTaskRecord,
    CreateBusinessTaskCommand, CreatePhasePlanCommand, MoveBusinessTaskCommand, PhasePlanRecord,
    ProjectPhaseRecord, ProjectProgressRepository, TransitionBusinessTaskCommand,
    TransitionProjectPhaseCommand,
};
use contexthub_domain::{BusinessTaskStatus, PhasePlanStatus, ProjectPhaseStatus};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::HashSet;

impl ProjectProgressRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn phase_plans(&mut self, project_id: &str) -> Result<Vec<PhasePlanRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare("SELECT plan_id FROM phase_plans WHERE project_id=?1 ORDER BY version_no DESC")
            .map_err(database_error("prepare phase plans"))?;
        let ids = statement
            .query_map([project_id], |row| row.get::<_, String>(0))
            .map_err(database_error("query phase plans"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read phase plan ids"))?;
        ids.into_iter()
            .map(|id| read_phase_plan(&self.transaction, &id, false))
            .collect()
    }

    fn business_task(&mut self, task_id: &str) -> Result<BusinessTaskRecord, Self::Error> {
        read_task(&self.transaction, task_id, false)
    }

    fn business_task_contributions(
        &mut self,
        task_id: &str,
    ) -> Result<Vec<BusinessTaskContributionRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT run_id,conversation_id,conversation_status,stage,outcome_title,outcome_summary,reason,confidence,created_at
                 FROM conversation_work_classifications
                 WHERE business_task_id=?1
                 ORDER BY created_at DESC,classification_id DESC",
            )
            .map_err(database_error("prepare business task contributions"))?;
        statement
            .query_map([task_id], |row| {
                Ok(BusinessTaskContributionRecord {
                    run_id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    conversation_status: row.get(2)?,
                    stage: row.get(3)?,
                    outcome_title: row.get(4)?,
                    outcome_summary: row.get(5)?,
                    reason: row.get(6)?,
                    confidence: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(database_error("query business task contributions"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read business task contributions"))
    }

    fn project_tasks(&mut self, project_id: &str) -> Result<Vec<BusinessTaskRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT business_task_id FROM business_tasks
                 WHERE project_id=?1
                 ORDER BY CASE WHEN status IN ('completed','terminated') THEN 1 ELSE 0 END,
                          updated_at DESC,business_task_id",
            )
            .map_err(database_error("prepare project tasks"))?;
        let ids = statement
            .query_map([project_id], |row| row.get::<_, String>(0))
            .map_err(database_error("query project tasks"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read project task ids"))?;
        ids.into_iter()
            .map(|id| read_task(&self.transaction, &id, false))
            .collect()
    }

    fn insert_phase_plan(
        &mut self,
        command: &CreatePhasePlanCommand,
    ) -> Result<PhasePlanRecord, Self::Error> {
        validate_phase_inputs(command)?;
        if let Some((id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT plan_id, request_fingerprint FROM phase_plans WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read phase plan idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(conflict("phase_plan_idempotency_conflict"));
            }
            return read_phase_plan(&self.transaction, &id, true);
        }
        let project_active = self
            .transaction
            .query_row(
                "SELECT status='active' FROM projects WHERE project_id=?1",
                [&command.project_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(database_error("read phase plan project"))?
            .ok_or_else(|| conflict("project_not_found"))?;
        if !project_active {
            return Err(conflict("project_not_editable"));
        }
        validate_required_deliverables(&self.transaction, command)?;
        let version_no = self
            .transaction
            .query_row(
                "SELECT COALESCE(MAX(version_no),0)+1 FROM phase_plans WHERE project_id=?1",
                [&command.project_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(database_error("allocate phase plan version"))?;
        self.transaction.execute(
            "INSERT INTO phase_plans(plan_id,project_id,version_no,status,summary,state_revision,client_operation_id,request_fingerprint,created_at) VALUES (?1,?2,?3,'draft',?4,0,?5,?6,?7)",
            params![command.plan_id, command.project_id, version_no, command.summary, command.client_operation_id, command.request_fingerprint, command.occurred_at])
            .map_err(database_error("insert phase plan"))?;
        for phase in &command.phases {
            self.transaction.execute(
                "INSERT INTO project_phases(phase_id,plan_id,project_id,name,goal,acceptance_criteria,required_deliverable_id,sort_order,is_main,status,state_revision,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'planned',0,?10,?10)",
                params![phase.phase_id, command.plan_id, command.project_id, phase.name, phase.goal, phase.acceptance_criteria, phase.required_deliverable_id, phase.sort_order, i64::from(phase.sort_order == 0), command.occurred_at])
                .map_err(database_error("insert project phase"))?;
        }
        for phase in &command.phases {
            for predecessor_phase_id in &phase.predecessor_phase_ids {
                self.transaction.execute("INSERT INTO phase_dependencies(plan_id,predecessor_phase_id,successor_phase_id,dependency_type) VALUES (?1,?2,?3,'finish_to_start')",
                    params![command.plan_id, predecessor_phase_id, phase.phase_id]).map_err(database_error("insert phase dependency"))?;
            }
        }
        read_phase_plan(&self.transaction, &command.plan_id, false)
    }

    fn activate_phase_plan(
        &mut self,
        command: &ActivatePhasePlanCommand,
    ) -> Result<PhasePlanRecord, Self::Error> {
        let replay = self.transaction.query_row("SELECT activation_client_operation_id, activation_request_fingerprint FROM phase_plans WHERE plan_id=?1", [&command.plan_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)))
            .optional().map_err(database_error("read phase plan activation"))?.ok_or_else(|| conflict("phase_plan_not_found"))?;
        if replay.0.as_deref() == Some(&command.client_operation_id) {
            if replay.1.as_deref() != Some(&command.request_fingerprint) {
                return Err(conflict("phase_plan_activation_idempotency_conflict"));
            }
            return read_phase_plan(&self.transaction, &command.plan_id, true);
        }
        let (project_id, status, revision) = self
            .transaction
            .query_row(
                "SELECT project_id,status,state_revision FROM phase_plans WHERE plan_id=?1",
                [&command.plan_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .map_err(database_error("read phase plan activation state"))?;
        if status != "draft" {
            return Err(conflict("phase_plan_not_draft"));
        }
        if revision != command.expected_revision {
            return Err(conflict("phase_plan_revision_conflict"));
        }
        let unfinished_active: bool = self.transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM phase_plans pp JOIN project_phases ph ON ph.plan_id=pp.plan_id WHERE pp.project_id=?1 AND pp.status='active' AND ph.status NOT IN ('completed','terminated'))",
            [&project_id], |row| row.get(0)).map_err(database_error("check active phase plan"))?;
        if unfinished_active {
            return Err(conflict("active_phase_plan_has_unfinished_phases"));
        }
        self.transaction.execute("UPDATE phase_plans SET status='superseded',state_revision=state_revision+1 WHERE project_id=?1 AND status='active'", [&project_id])
            .map_err(database_error("supersede phase plan"))?;
        self.transaction.execute("UPDATE phase_plans SET status='active',state_revision=state_revision+1,activation_client_operation_id=?2,activation_request_fingerprint=?3,activated_at=?4 WHERE plan_id=?1",
            params![command.plan_id, command.client_operation_id, command.request_fingerprint, command.occurred_at]).map_err(database_error("activate phase plan"))?;
        read_phase_plan(&self.transaction, &command.plan_id, false)
    }

    fn transition_project_phase(
        &mut self,
        command: &TransitionProjectPhaseCommand,
    ) -> Result<ProjectPhaseRecord, Self::Error> {
        if command.actor_label.trim().is_empty() {
            return Err(conflict("phase_actor_required"));
        }
        if let Some((phase_id, fingerprint)) = self.transaction.query_row("SELECT phase_id,request_fingerprint FROM phase_transitions WHERE client_operation_id=?1", [&command.client_operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).optional().map_err(database_error("read phase transition idempotency"))? {
            if fingerprint != command.request_fingerprint { return Err(conflict("phase_transition_idempotency_conflict")); }
            return read_phase(&self.transaction, &phase_id, true);
        }
        let phase = read_phase(&self.transaction, &command.phase_id, false)?;
        if phase.state_revision != command.expected_revision {
            return Err(conflict("phase_revision_conflict"));
        }
        if !phase.status.can_transition_to(command.target_status) {
            return Err(conflict("invalid_phase_transition"));
        }
        validate_phase_transition(&self.transaction, &phase, command)?;
        let accepted_version = if command.target_status == ProjectPhaseStatus::Completed {
            phase.required_deliverable_version_id.clone()
        } else {
            None
        };
        self.transaction.execute("INSERT INTO phase_transitions(transition_id,phase_id,from_status,to_status,actor_label,comment,acceptance_criteria_snapshot,accepted_deliverable_version_id,client_operation_id,request_fingerprint,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![command.transition_id, command.phase_id, phase.status.as_str(), command.target_status.as_str(), command.actor_label, command.comment,
                if command.target_status == ProjectPhaseStatus::Completed { Some(phase.acceptance_criteria.as_str()) } else { None }, accepted_version,
                command.client_operation_id, command.request_fingerprint, command.occurred_at]).map_err(database_error("insert phase transition"))?;
        self.transaction.execute("UPDATE project_phases SET status=?2,state_revision=state_revision+1,is_main=?3,updated_at=?4 WHERE phase_id=?1",
            params![command.phase_id, command.target_status.as_str(), i64::from(command.target_status == ProjectPhaseStatus::InProgress), command.occurred_at])
            .map_err(database_error("update project phase"))?;
        if command.target_status == ProjectPhaseStatus::InProgress {
            self.transaction
                .execute(
                    "UPDATE project_phases SET is_main=0 WHERE project_id=?1 AND phase_id<>?2",
                    params![phase.project_id, command.phase_id],
                )
                .map_err(database_error("update main phase"))?;
        }
        read_phase(&self.transaction, &command.phase_id, false)
    }

    fn insert_business_task(
        &mut self,
        command: &CreateBusinessTaskCommand,
    ) -> Result<BusinessTaskRecord, Self::Error> {
        if let Some((id, fingerprint)) = self.transaction.query_row("SELECT business_task_id,request_fingerprint FROM business_tasks WHERE client_operation_id=?1", [&command.client_operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).optional().map_err(database_error("read business task idempotency"))? {
            if fingerprint != command.request_fingerprint { return Err(conflict("business_task_idempotency_conflict")); }
            return read_task(&self.transaction, &id, true);
        }
        validate_task_contract(command)?;
        if let Some(phase_id) = command.phase_id.as_deref() {
            let phase_valid: bool = self.transaction.query_row("SELECT EXISTS(SELECT 1 FROM project_phases ph JOIN phase_plans pp ON pp.plan_id=ph.plan_id WHERE ph.phase_id=?1 AND ph.project_id=?2 AND ph.status='in_progress' AND pp.status='active')",
                params![phase_id, command.project_id], |row| row.get(0)).map_err(database_error("validate task phase"))?;
            if !phase_valid {
                return Err(conflict("business_task_phase_not_active"));
            }
        }
        self.transaction.execute("INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'candidate',0,?13,?14,?15,?15)",
            params![command.business_task_id, command.project_id, command.phase_id, command.title, command.objective, command.input_context, command.expected_output, command.completion_criteria, command.assignee_type, command.assignee_label, command.priority, command.due_at, command.client_operation_id, command.request_fingerprint, command.occurred_at])
            .map_err(database_error("insert business task"))?;
        read_task(&self.transaction, &command.business_task_id, false)
    }

    fn transition_business_task(
        &mut self,
        command: &TransitionBusinessTaskCommand,
    ) -> Result<BusinessTaskRecord, Self::Error> {
        if command.actor_label.trim().is_empty() {
            return Err(conflict("business_task_actor_required"));
        }
        if let Some((id, fingerprint)) = self.transaction.query_row("SELECT business_task_id,request_fingerprint FROM business_task_transitions WHERE client_operation_id=?1", [&command.client_operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).optional().map_err(database_error("read task transition idempotency"))? {
            if fingerprint != command.request_fingerprint { return Err(conflict("business_task_transition_idempotency_conflict")); }
            return read_task(&self.transaction, &id, true);
        }
        let task = read_task(&self.transaction, &command.business_task_id, false)?;
        if task.state_revision != command.expected_revision {
            return Err(conflict("business_task_revision_conflict"));
        }
        if !task.status.can_transition_to(command.target_status) {
            return Err(conflict("invalid_business_task_transition"));
        }
        if command.target_status == BusinessTaskStatus::InProgress
            && let Some(phase_id) = task.phase_id.as_deref()
        {
            let phase_active: bool = self
                .transaction
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM project_phases phase
                        JOIN phase_plans plan ON plan.plan_id=phase.plan_id
                        WHERE phase.phase_id=?1 AND phase.status='in_progress' AND plan.status='active'
                    )",
                    [phase_id],
                    |row| row.get(0),
                )
                .map_err(database_error("validate task execution phase"))?;
            if !phase_active {
                return Err(conflict("business_task_phase_not_active"));
            }
        }
        if matches!(
            command.target_status,
            BusinessTaskStatus::Completed | BusinessTaskStatus::Rework
        ) && (command.actor_label.trim().is_empty() || command.comment.trim().is_empty())
        {
            return Err(conflict("business_task_acceptance_required"));
        }
        self.transaction.execute("INSERT INTO business_task_transitions(transition_id,business_task_id,from_status,to_status,actor_label,comment,client_operation_id,request_fingerprint,created_at,attempt_no) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![command.transition_id, command.business_task_id, task.status.as_str(), command.target_status.as_str(), command.actor_label, command.comment, command.client_operation_id, command.request_fingerprint, command.occurred_at, task.attempt_no])
            .map_err(database_error("insert business task transition"))?;
        self.transaction.execute("UPDATE business_tasks SET status=?2,attempt_no=attempt_no+CASE WHEN ?2='rework' THEN 1 ELSE 0 END,state_revision=state_revision+1,updated_at=?3 WHERE business_task_id=?1",
            params![command.business_task_id, command.target_status.as_str(), command.occurred_at]).map_err(database_error("update business task"))?;
        read_task(&self.transaction, &command.business_task_id, false)
    }

    fn move_business_task(
        &mut self,
        command: &MoveBusinessTaskCommand,
    ) -> Result<BusinessTaskRecord, Self::Error> {
        for (value, reason) in [
            (&command.actor_label, "business_task_move_actor_required"),
            (
                &command.dependency_impact,
                "business_task_move_dependency_impact_required",
            ),
            (
                &command.deliverable_impact,
                "business_task_move_deliverable_impact_required",
            ),
            (
                &command.acceptance_impact,
                "business_task_move_acceptance_impact_required",
            ),
        ] {
            if value.trim().is_empty() {
                return Err(conflict(reason));
            }
        }
        if let Some((task_id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT business_task_id,request_fingerprint FROM business_task_moves WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read task move idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(conflict("business_task_move_idempotency_conflict"));
            }
            return read_task(&self.transaction, &task_id, true);
        }
        let task = read_task(&self.transaction, &command.business_task_id, false)?;
        if task.state_revision != command.expected_revision {
            return Err(conflict("business_task_revision_conflict"));
        }
        if !matches!(
            task.status,
            BusinessTaskStatus::Candidate | BusinessTaskStatus::Todo | BusinessTaskStatus::Rework
        ) {
            return Err(conflict("business_task_move_status_forbidden"));
        }
        let source_phase_id = task
            .phase_id
            .as_deref()
            .ok_or_else(|| conflict("business_task_move_source_phase_required"))?;
        if source_phase_id == command.target_phase_id {
            return Err(conflict("business_task_move_target_unchanged"));
        }
        let target_valid: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM project_phases source
                    JOIN phase_plans plan ON plan.plan_id=source.plan_id AND plan.status='active'
                    JOIN project_phases target ON target.plan_id=source.plan_id
                    WHERE source.phase_id=?1 AND target.phase_id=?2
                      AND source.project_id=target.project_id
                      AND target.status IN ('planned','in_progress','paused')
                )",
                params![source_phase_id, command.target_phase_id],
                |row| row.get(0),
            )
            .map_err(database_error("validate task move target"))?;
        if !target_valid {
            return Err(conflict("business_task_move_target_invalid"));
        }
        self.transaction
            .execute(
                "INSERT INTO business_task_moves(move_id,business_task_id,from_phase_id,to_phase_id,dependency_impact,deliverable_impact,acceptance_impact,actor_label,client_operation_id,request_fingerprint,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    command.move_id,
                    command.business_task_id,
                    source_phase_id,
                    command.target_phase_id,
                    command.dependency_impact.trim(),
                    command.deliverable_impact.trim(),
                    command.acceptance_impact.trim(),
                    command.actor_label.trim(),
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert business task move"))?;
        self.transaction
            .execute(
                "UPDATE business_tasks SET phase_id=?2,state_revision=state_revision+1,updated_at=?3 WHERE business_task_id=?1",
                params![command.business_task_id, command.target_phase_id, command.occurred_at],
            )
            .map_err(database_error("move business task"))?;
        read_task(&self.transaction, &command.business_task_id, false)
    }
}

fn validate_phase_inputs(command: &CreatePhasePlanCommand) -> Result<(), PersistenceError> {
    if command.phases.is_empty() {
        return Err(conflict("phase_plan_phases_required"));
    }
    for (index, phase) in command.phases.iter().enumerate() {
        let mut predecessors = HashSet::new();
        if phase.name.trim().is_empty()
            || phase.goal.trim().is_empty()
            || phase.acceptance_criteria.trim().is_empty()
            || phase.sort_order != i64::try_from(index).unwrap_or(-1)
        {
            return Err(conflict("invalid_phase_plan_phase"));
        }
        if phase.predecessor_phase_ids.iter().any(|predecessor| {
            !predecessors.insert(predecessor)
                || !command.phases[..index]
                    .iter()
                    .any(|candidate| candidate.phase_id == *predecessor)
        }) {
            return Err(conflict("invalid_phase_predecessor"));
        }
    }
    Ok(())
}

fn validate_required_deliverables(
    transaction: &Transaction<'_>,
    command: &CreatePhasePlanCommand,
) -> Result<(), PersistenceError> {
    for phase in &command.phases {
        let Some(deliverable_id) = phase.required_deliverable_id.as_ref() else {
            continue;
        };
        let valid: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM deliverables WHERE deliverable_id=?1 AND project_id=?2)",
            params![deliverable_id, command.project_id], |row| row.get(0)).map_err(database_error("validate phase deliverable"))?;
        if !valid {
            return Err(conflict("phase_required_deliverable_not_found"));
        }
    }
    Ok(())
}

fn validate_task_contract(command: &CreateBusinessTaskCommand) -> Result<(), PersistenceError> {
    if command.title.trim().is_empty()
        || command.objective.trim().is_empty()
        || command.expected_output.trim().is_empty()
        || command.completion_criteria.trim().is_empty()
        || command.assignee_label.trim().is_empty()
        || !matches!(
            command.assignee_type.as_str(),
            "user" | "agent" | "collaboration"
        )
        || !matches!(
            command.priority.as_str(),
            "low" | "medium" | "high" | "critical"
        )
    {
        return Err(conflict("invalid_business_task_contract"));
    }
    Ok(())
}

fn validate_phase_transition(
    transaction: &Transaction<'_>,
    phase: &ProjectPhaseRecord,
    command: &TransitionProjectPhaseCommand,
) -> Result<(), PersistenceError> {
    let target = command.target_status;
    if (matches!(
        target,
        ProjectPhaseStatus::Paused | ProjectPhaseStatus::Terminated
    ) || phase.status == ProjectPhaseStatus::Review && target == ProjectPhaseStatus::InProgress)
        && command.comment.trim().is_empty()
    {
        return Err(conflict("phase_change_reason_required"));
    }
    if target == ProjectPhaseStatus::InProgress && phase.status == ProjectPhaseStatus::Planned {
        let blocked: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM phase_dependencies d JOIN project_phases p ON p.phase_id=d.predecessor_phase_id WHERE d.successor_phase_id=?1 AND p.status<>'completed')", [&phase.phase_id], |row| row.get(0))
            .map_err(database_error("check phase predecessors"))?;
        if blocked {
            return Err(conflict("phase_predecessor_incomplete"));
        }
    }
    if target == ProjectPhaseStatus::Review {
        let (completed, incomplete) = transaction.query_row("SELECT COALESCE(SUM(CASE WHEN status='completed' THEN 1 ELSE 0 END),0), COALESCE(SUM(CASE WHEN status NOT IN ('completed','terminated') THEN 1 ELSE 0 END),0) FROM business_tasks WHERE phase_id=?1", [&phase.phase_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))).map_err(database_error("check phase tasks"))?;
        if completed == 0 || incomplete > 0 {
            return Err(conflict("phase_tasks_not_completed"));
        }
        if phase.required_deliverable_id.is_some()
            && phase.required_deliverable_version_id.is_none()
        {
            return Err(conflict("phase_deliverable_not_accepted"));
        }
    }
    if target == ProjectPhaseStatus::Completed {
        if phase.required_deliverable_id.is_some()
            && phase.required_deliverable_version_id.is_none()
        {
            return Err(conflict("phase_deliverable_not_accepted"));
        }
        if command.actor_label.trim().is_empty() || command.comment.trim().is_empty() {
            return Err(conflict("phase_acceptance_required"));
        }
    }
    Ok(())
}

fn read_phase_plan(
    transaction: &Transaction<'_>,
    plan_id: &str,
    replayed: bool,
) -> Result<PhasePlanRecord, PersistenceError> {
    let row = transaction.query_row("SELECT project_id,version_no,status,summary,state_revision,created_at,activated_at FROM phase_plans WHERE plan_id=?1", [plan_id], |row| Ok((row.get::<_, String>(0)?,row.get::<_, i64>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?,row.get::<_, i64>(4)?,row.get::<_, i64>(5)?,row.get::<_, Option<i64>>(6)?)))
        .map_err(database_error("read phase plan"))?;
    let mut statement = transaction
        .prepare(
            "SELECT phase_id FROM project_phases WHERE plan_id=?1 ORDER BY sort_order,phase_id",
        )
        .map_err(database_error("prepare plan phases"))?;
    let ids = statement
        .query_map([plan_id], |item| item.get::<_, String>(0))
        .map_err(database_error("query plan phases"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read plan phase ids"))?;
    let phases = ids
        .into_iter()
        .map(|id| read_phase(transaction, &id, false))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PhasePlanRecord {
        plan_id: plan_id.to_owned(),
        project_id: row.0,
        version_no: row.1,
        status: PhasePlanStatus::try_from(row.2.as_str())
            .map_err(|()| state_error("invalid_phase_plan_status"))?,
        summary: row.3,
        state_revision: row.4,
        created_at: row.5,
        activated_at: row.6,
        phases,
        replayed,
    })
}

fn read_phase(
    transaction: &Transaction<'_>,
    phase_id: &str,
    replayed: bool,
) -> Result<ProjectPhaseRecord, PersistenceError> {
    let row = transaction.query_row("SELECT ph.plan_id,ph.project_id,ph.name,ph.goal,ph.acceptance_criteria,ph.required_deliverable_id,d.title,d.current_accepted_version_id,ph.sort_order,ph.is_main,ph.status,ph.state_revision FROM project_phases ph LEFT JOIN deliverables d ON d.deliverable_id=ph.required_deliverable_id WHERE ph.phase_id=?1", [phase_id],
        |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?,row.get::<_, String>(4)?,row.get::<_, Option<String>>(5)?,row.get::<_, Option<String>>(6)?,row.get::<_, Option<String>>(7)?,row.get::<_, i64>(8)?,row.get::<_, bool>(9)?,row.get::<_, String>(10)?,row.get::<_, i64>(11)?)))
        .map_err(database_error("read project phase"))?;
    let mut statement = transaction.prepare("SELECT business_task_id FROM business_tasks WHERE phase_id=?1 ORDER BY created_at,business_task_id").map_err(database_error("prepare phase tasks"))?;
    let ids = statement
        .query_map([phase_id], |item| item.get::<_, String>(0))
        .map_err(database_error("query phase tasks"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read phase task ids"))?;
    let tasks = ids
        .into_iter()
        .map(|id| read_task(transaction, &id, false))
        .collect::<Result<Vec<_>, _>>()?;
    let mut dependency_statement = transaction
        .prepare("SELECT predecessor_phase_id FROM phase_dependencies WHERE successor_phase_id=?1 ORDER BY predecessor_phase_id")
        .map_err(database_error("prepare phase predecessors"))?;
    let predecessor_phase_ids = dependency_statement
        .query_map([phase_id], |item| item.get::<_, String>(0))
        .map_err(database_error("query phase predecessors"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read phase predecessors"))?;
    Ok(ProjectPhaseRecord {
        phase_id: phase_id.to_owned(),
        plan_id: row.0,
        project_id: row.1,
        name: row.2,
        goal: row.3,
        acceptance_criteria: row.4,
        required_deliverable_id: row.5,
        required_deliverable_title: row.6,
        required_deliverable_version_id: row.7,
        sort_order: row.8,
        predecessor_phase_ids,
        is_main: row.9,
        status: ProjectPhaseStatus::try_from(row.10.as_str())
            .map_err(|()| state_error("invalid_project_phase_status"))?,
        state_revision: row.11,
        tasks,
        replayed,
    })
}

fn read_task(
    transaction: &Transaction<'_>,
    task_id: &str,
    replayed: bool,
) -> Result<BusinessTaskRecord, PersistenceError> {
    let row = transaction.query_row("SELECT project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,attempt_no,status,state_revision,updated_at FROM business_tasks WHERE business_task_id=?1", [task_id],
        |row| Ok((row.get::<_, String>(0)?,row.get::<_, Option<String>>(1)?,row.get::<_, String>(2)?,row.get::<_, String>(3)?,row.get::<_, String>(4)?,row.get::<_, String>(5)?,row.get::<_, String>(6)?,row.get::<_, String>(7)?,row.get::<_, String>(8)?,row.get::<_, String>(9)?,row.get::<_, Option<i64>>(10)?,row.get::<_, i64>(11)?,row.get::<_, String>(12)?,row.get::<_, i64>(13)?,row.get::<_, i64>(14)?)))
        .map_err(database_error("read business task"))?;
    Ok(BusinessTaskRecord {
        business_task_id: task_id.to_owned(),
        project_id: row.0,
        phase_id: row.1,
        title: row.2,
        objective: row.3,
        input_context: row.4,
        expected_output: row.5,
        completion_criteria: row.6,
        assignee_type: row.7,
        assignee_label: row.8,
        priority: row.9,
        due_at: row.10,
        attempt_no: row.11,
        status: BusinessTaskStatus::try_from(row.12.as_str())
            .map_err(|()| state_error("invalid_business_task_status"))?,
        state_revision: row.13,
        updated_at: row.14,
        replayed,
    })
}

fn conflict(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}
fn state_error(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Database, reason, reason)
}
