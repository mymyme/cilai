use super::{
    PersistenceError, PersistenceErrorKind, Row, SqliteRepositoryTransaction, TaskAttemptId,
    TaskAttemptRecord, TaskAttemptState, TaskId, TaskProgress, TaskRecord, TaskRepository,
    TaskState, TaskTransition, WorkspaceId, configuration_error, conflict_error, database_error,
    params,
};
use rusqlite::OptionalExtension;

impl TaskRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn insert_task(&mut self, task: &TaskRecord) -> Result<(), Self::Error> {
        validate_new_task(task)?;
        self.transaction
            .execute(
                "INSERT INTO tasks(task_id, task_type, workspace_id, state, progress_completed, progress_total, current_stage, cancellable, payload_json, result_ref_type, result_ref_id, correlation_id, created_by_client, created_at, started_at, completed_at, heartbeat_at, error_reason, error_detail_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                params![
                    task.task_id.as_str(),
                    task.task_type,
                    task.workspace_id.as_ref().map(WorkspaceId::as_str),
                    task.state.as_str(),
                    task.progress_completed,
                    task.progress_total,
                    task.current_stage,
                    task.cancellable,
                    task.payload_json,
                    task.result_ref_type,
                    task.result_ref_id,
                    task.correlation_id,
                    task.created_by_client,
                    task.created_at,
                    task.started_at,
                    task.completed_at,
                    task.heartbeat_at,
                    task.error_reason,
                    task.error_detail_json,
                ],
            )
            .map_err(database_error("insert task"))?;
        Ok(())
    }

    fn task(&mut self, task_id: &TaskId) -> Result<Option<TaskRecord>, Self::Error> {
        self.transaction
            .query_row(
                "SELECT task_id, task_type, workspace_id, state, progress_completed, progress_total, current_stage, cancellable, payload_json, result_ref_type, result_ref_id, correlation_id, created_by_client, created_at, started_at, completed_at, heartbeat_at, error_reason, error_detail_json FROM tasks WHERE task_id=?1",
                [task_id.as_str()],
                read_task,
            )
            .optional()
            .map_err(database_error("read task"))
    }

    fn nonterminal_tasks(&mut self) -> Result<Vec<TaskRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT task_id, task_type, workspace_id, state, progress_completed, progress_total, current_stage, cancellable, payload_json, result_ref_type, result_ref_id, correlation_id, created_by_client, created_at, started_at, completed_at, heartbeat_at, error_reason, error_detail_json FROM tasks WHERE state IN ('queued', 'running', 'waiting_for_user', 'interrupted', 'outcome_unknown') ORDER BY created_at, task_id",
            )
            .map_err(database_error("prepare nonterminal task recovery query"))?;
        statement
            .query_map([], read_task)
            .map_err(database_error("query nonterminal tasks"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read nonterminal tasks"))
    }

    fn start_task(
        &mut self,
        task_id: &TaskId,
        attempt_id: &TaskAttemptId,
        server_instance_id: &str,
        started_at: i64,
    ) -> Result<(TaskRecord, TaskAttemptRecord), Self::Error> {
        if task_id.as_str().is_empty()
            || attempt_id.as_str().is_empty()
            || server_instance_id.is_empty()
            || started_at < 0
        {
            return Err(configuration_error("task start fields are invalid"));
        }
        let task = self.task(task_id)?.ok_or_else(|| {
            PersistenceError::new(PersistenceErrorKind::Conflict, "task does not exist")
        })?;
        if !matches!(task.state, TaskState::Queued | TaskState::Interrupted) {
            return Err(conflict_error("task cannot start from its current state"));
        }
        let has_running_attempt: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_attempts WHERE task_id=?1 AND state='running')",
                [task_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("check active task attempt"))?;
        if has_running_attempt {
            return Err(conflict_error("task already has a running attempt"));
        }
        let attempt_no: i64 = self
            .transaction
            .query_row(
                "SELECT COALESCE(MAX(attempt_no), 0) + 1 FROM task_attempts WHERE task_id=?1",
                [task_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("allocate task attempt number"))?;
        let changed = self
            .transaction
            .execute(
                "UPDATE tasks SET state='running', started_at=COALESCE(started_at, ?1), completed_at=NULL, heartbeat_at=?1, error_reason=NULL, error_detail_json=NULL WHERE task_id=?2 AND state=?3",
                params![started_at, task_id.as_str(), task.state.as_str()],
            )
            .map_err(database_error("start task"))?;
        if changed != 1 {
            return Err(conflict_error("task state changed before start"));
        }
        self.transaction
            .execute(
                "INSERT INTO task_attempts(attempt_id, task_id, attempt_no, server_instance_id, state, started_at) VALUES (?1, ?2, ?3, ?4, 'running', ?5)",
                params![attempt_id.as_str(), task_id.as_str(), attempt_no, server_instance_id, started_at],
            )
            .map_err(database_error("insert task attempt"))?;
        let started = self.task(task_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "started task disappeared inside transaction",
            )
        })?;
        Ok((
            started,
            TaskAttemptRecord {
                attempt_id: attempt_id.clone(),
                task_id: task_id.clone(),
                attempt_no,
                server_instance_id: server_instance_id.to_owned(),
                state: TaskAttemptState::Running,
                started_at,
                ended_at: None,
                error_reason: None,
            },
        ))
    }

    fn update_task_progress(&mut self, progress: &TaskProgress) -> Result<TaskRecord, Self::Error> {
        if progress.progress_completed < 0
            || progress.progress_total.is_some_and(|total| total < 0)
            || progress
                .progress_total
                .is_some_and(|total| progress.progress_completed > total)
            || progress.heartbeat_at < 0
        {
            return Err(configuration_error("task progress fields are invalid"));
        }
        let task = self.task(&progress.task_id)?.ok_or_else(|| {
            PersistenceError::new(PersistenceErrorKind::Conflict, "task does not exist")
        })?;
        if task.state != TaskState::Running {
            return Err(conflict_error(
                "task progress can only change while running",
            ));
        }
        if progress.progress_completed < task.progress_completed {
            return Err(conflict_error("task progress must not move backwards"));
        }
        let effective_total = match (task.progress_total, progress.progress_total) {
            (None, incoming) => incoming,
            (Some(current), None) => Some(current),
            (Some(current), Some(incoming)) if current == incoming => Some(current),
            (Some(_), Some(_)) => return Err(conflict_error("task progress total is immutable")),
        };
        if effective_total.is_some_and(|total| progress.progress_completed > total) {
            return Err(conflict_error("task progress exceeds its total"));
        }
        let changed = self
            .transaction
            .execute(
                "UPDATE tasks SET progress_completed=?1, progress_total=?2, current_stage=?3, heartbeat_at=?4 WHERE task_id=?5 AND state='running' AND progress_completed<=?1",
                params![progress.progress_completed, effective_total, progress.current_stage, progress.heartbeat_at, progress.task_id.as_str()],
            )
            .map_err(database_error("update task progress"))?;
        if changed != 1 {
            return Err(conflict_error(
                "task state or progress changed before update",
            ));
        }
        self.task(&progress.task_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "updated task disappeared inside transaction",
            )
        })
    }

    fn transition_task(&mut self, transition: &TaskTransition) -> Result<TaskRecord, Self::Error> {
        validate_task_transition(transition)?;
        let task = self.task(&transition.task_id)?.ok_or_else(|| {
            PersistenceError::new(PersistenceErrorKind::Conflict, "task does not exist")
        })?;
        if task.state != transition.expected_state {
            return Err(conflict_error("task state does not match expected state"));
        }
        if !task.state.can_transition_to(transition.target_state) {
            return Err(conflict_error("task state transition is not allowed"));
        }
        if transition.target_state == TaskState::Running
            && transition.expected_state == TaskState::Interrupted
        {
            return Err(conflict_error(
                "an interrupted task must create a new attempt through start_task",
            ));
        }
        if transition.target_state == TaskState::Cancelled && !task.cancellable {
            return Err(conflict_error("task is not cancellable"));
        }
        let completes = matches!(
            transition.target_state,
            TaskState::Succeeded
                | TaskState::Failed
                | TaskState::Cancelled
                | TaskState::Interrupted
                | TaskState::OutcomeUnknown
        );
        let changed = self
            .transaction
            .execute(
                "UPDATE tasks SET state=?1, completed_at=?2, heartbeat_at=?3, result_ref_type=?4, result_ref_id=?5, error_reason=?6, error_detail_json=?7 WHERE task_id=?8 AND state=?9",
                params![
                    transition.target_state.as_str(),
                    completes.then_some(transition.occurred_at),
                    transition.occurred_at,
                    transition.result_ref_type,
                    transition.result_ref_id,
                    transition.error_reason,
                    transition.error_detail_json,
                    transition.task_id.as_str(),
                    transition.expected_state.as_str(),
                ],
            )
            .map_err(database_error("transition task"))?;
        if changed != 1 {
            return Err(conflict_error("task state changed before transition"));
        }
        if completes
            && !matches!(
                transition.expected_state,
                TaskState::Queued | TaskState::OutcomeUnknown
            )
        {
            let attempt_state = match transition.target_state {
                TaskState::Succeeded => TaskAttemptState::Succeeded,
                TaskState::Failed => TaskAttemptState::Failed,
                TaskState::Cancelled | TaskState::Interrupted | TaskState::OutcomeUnknown => {
                    TaskAttemptState::Interrupted
                }
                _ => unreachable!("completing transition has a terminal attempt state"),
            };
            let attempts = self
                .transaction
                .execute(
                    "UPDATE task_attempts SET state=?1, ended_at=?2, error_reason=?3 WHERE task_id=?4 AND state='running'",
                    params![attempt_state.as_str(), transition.occurred_at, transition.error_reason, transition.task_id.as_str()],
                )
                .map_err(database_error("finish task attempt"))?;
            if attempts != 1 {
                return Err(PersistenceError::new(
                    PersistenceErrorKind::Database,
                    "running task does not have exactly one active attempt",
                ));
            }
        }
        self.task(&transition.task_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "transitioned task disappeared inside transaction",
            )
        })
    }
}

fn validate_new_task(task: &TaskRecord) -> Result<(), PersistenceError> {
    let valid_progress = task.progress_completed >= 0
        && task
            .progress_total
            .is_none_or(|total| total >= 0 && task.progress_completed <= total);
    if task.task_id.as_str().is_empty()
        || task.task_type.is_empty()
        || task.correlation_id.is_empty()
        || task.created_at < 0
        || !valid_progress
    {
        return Err(configuration_error("new task fields are invalid"));
    }
    if task.state != TaskState::Queued
        || task.started_at.is_some()
        || task.completed_at.is_some()
        || task.heartbeat_at.is_some()
        || task.result_ref_type.is_some()
        || task.result_ref_id.is_some()
        || task.error_reason.is_some()
        || task.error_detail_json.is_some()
    {
        return Err(configuration_error(
            "new task must be queued without runtime or result fields",
        ));
    }
    Ok(())
}

fn validate_task_transition(transition: &TaskTransition) -> Result<(), PersistenceError> {
    if transition.task_id.as_str().is_empty() || transition.occurred_at < 0 {
        return Err(configuration_error("task transition fields are invalid"));
    }
    if transition.result_ref_type.is_some() != transition.result_ref_id.is_some() {
        return Err(configuration_error(
            "task result reference type and id must be provided together",
        ));
    }
    if transition.target_state == TaskState::Succeeded {
        if transition.error_reason.is_some() || transition.error_detail_json.is_some() {
            return Err(configuration_error(
                "succeeded task transition must not contain an error",
            ));
        }
    } else if transition.result_ref_type.is_some() {
        return Err(configuration_error(
            "only a succeeded task transition may contain a result reference",
        ));
    }
    if matches!(
        transition.target_state,
        TaskState::Failed
            | TaskState::Cancelled
            | TaskState::Interrupted
            | TaskState::OutcomeUnknown
    ) && transition
        .error_reason
        .as_ref()
        .is_none_or(String::is_empty)
    {
        return Err(configuration_error(
            "failed, cancelled, interrupted, or outcome-unknown transition requires a reason",
        ));
    }
    if matches!(
        transition.target_state,
        TaskState::Running | TaskState::WaitingForUser
    ) && (transition.error_reason.is_some() || transition.error_detail_json.is_some())
    {
        return Err(configuration_error(
            "non-completing task transition must not contain an error",
        ));
    }
    Ok(())
}

fn read_task(row: &Row<'_>) -> rusqlite::Result<TaskRecord> {
    let raw_state = row.get::<_, String>(3)?;
    let state = TaskState::try_from(raw_state.as_str()).map_err(|()| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unknown task state",
            )),
        )
    })?;
    Ok(TaskRecord {
        task_id: TaskId::new(row.get::<_, String>(0)?),
        task_type: row.get(1)?,
        workspace_id: row.get::<_, Option<String>>(2)?.map(WorkspaceId::new),
        state,
        progress_completed: row.get(4)?,
        progress_total: row.get(5)?,
        current_stage: row.get(6)?,
        cancellable: row.get(7)?,
        payload_json: row.get(8)?,
        result_ref_type: row.get(9)?,
        result_ref_id: row.get(10)?,
        correlation_id: row.get(11)?,
        created_by_client: row.get(12)?,
        created_at: row.get(13)?,
        started_at: row.get(14)?,
        completed_at: row.get(15)?,
        heartbeat_at: row.get(16)?,
        error_reason: row.get(17)?,
        error_detail_json: row.get(18)?,
    })
}
