use super::{PersistenceError, PersistenceErrorKind, SqliteRepositoryTransaction, database_error};
use contexthub_application::{
    AttentionCaseRecord, AttentionCaseRepository, CreateAttentionCaseCommand,
    TransitionAttentionCaseCommand,
};
use contexthub_domain::{AttentionCaseStatus, AttentionCaseType};
use rusqlite::{OptionalExtension, Row, Transaction, params};

impl AttentionCaseRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn attention_cases(
        &mut self,
        project_id: &str,
    ) -> Result<Vec<AttentionCaseRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT attention_case_id FROM attention_cases WHERE project_id=?1 ORDER BY status='resolved', CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 ELSE 3 END, updated_at DESC, attention_case_id",
            )
            .map_err(database_error("prepare attention case list"))?;
        let ids = statement
            .query_map([project_id], |row| row.get::<_, String>(0))
            .map_err(database_error("query attention case list"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read attention case ids"))?;
        ids.into_iter()
            .map(|id| read_attention_case(&self.transaction, &id, false))
            .collect()
    }

    fn insert_attention_case(
        &mut self,
        command: &CreateAttentionCaseCommand,
    ) -> Result<AttentionCaseRecord, Self::Error> {
        validate_create(command)?;
        if let Some((id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT attention_case_id,request_fingerprint FROM attention_cases WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read attention create idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(conflict("attention_create_idempotency_conflict"));
            }
            return read_attention_case(&self.transaction, &id, true);
        }
        validate_references(&self.transaction, command)?;
        if command.case_type == AttentionCaseType::AgentRunIncident {
            let duplicate: bool = self
                .transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM attention_cases WHERE project_id=?1 AND agent_run_id=?2)",
                    params![command.project_id, command.agent_run_id],
                    |row| row.get(0),
                )
                .map_err(database_error("check agent incident run identity"))?;
            if duplicate {
                return Err(conflict("agent_run_incident_exists"));
            }
        }
        self.transaction
            .execute(
                "INSERT INTO attention_cases(attention_case_id,case_type,project_id,phase_id,task_id,deliverable_id,title,detail,status,responsibility_type,responsibility_label,reporter_label,priority,due_at,agent_run_id,agent_label,resolution_summary,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,resolved_at,decision_options,decision_impact,decision_evidence,decision_recommendation,decision_consequence)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'open',?9,?10,?11,?12,?13,?14,?15,NULL,0,?16,?17,?18,?18,NULL,?19,?20,?21,?22,?23)",
                params![
                    command.attention_case_id,
                    command.case_type.as_str(),
                    command.project_id,
                    command.phase_id,
                    command.task_id,
                    command.deliverable_id,
                    command.title,
                    command.detail,
                    command.responsibility_type,
                    command.responsibility_label,
                    command.reporter_label,
                    command.priority,
                    command.due_at,
                    command.agent_run_id,
                    command.agent_label,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                    command.decision_options,
                    command.decision_impact,
                    command.decision_evidence,
                    command.decision_recommendation,
                    command.decision_consequence,
                ],
            )
            .map_err(database_error("insert attention case"))?;
        read_attention_case(&self.transaction, &command.attention_case_id, false)
    }

    fn transition_attention_case(
        &mut self,
        command: &TransitionAttentionCaseCommand,
    ) -> Result<AttentionCaseRecord, Self::Error> {
        if command.actor_label.trim().is_empty() {
            return Err(conflict("attention_actor_required"));
        }
        if let Some((id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT attention_case_id,request_fingerprint FROM attention_case_transitions WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read attention transition idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(conflict("attention_transition_idempotency_conflict"));
            }
            return read_attention_case(&self.transaction, &id, true);
        }
        let current = read_attention_case(&self.transaction, &command.attention_case_id, false)?;
        if current.state_revision != command.expected_revision {
            return Err(conflict("attention_revision_conflict"));
        }
        validate_transition_contract(&current, command)?;
        let delegated = command.decision_action.as_deref() == Some("delegated");
        if !delegated
            && !current
                .status
                .can_transition_to(current.case_type, command.target_status)
        {
            return Err(conflict("invalid_attention_transition"));
        }
        self.transaction
            .execute(
                "INSERT INTO attention_case_transitions(transition_id,attention_case_id,from_status,to_status,actor_label,resolution_summary,decision_action,delegated_to,client_operation_id,request_fingerprint,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    command.transition_id,
                    command.attention_case_id,
                    current.status.as_str(),
                    command.target_status.as_str(),
                    command.actor_label,
                    command.resolution_summary,
                    command.decision_action,
                    command.delegated_to,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert attention transition"))?;
        let resolved = command.target_status == AttentionCaseStatus::Resolved;
        self.transaction
            .execute(
                "UPDATE attention_cases
                 SET status=?2,resolution_summary=?3,state_revision=state_revision+1,updated_at=?4,resolved_at=?5,
                     decision_action=?6,decision_action_summary=?7,decision_actor_label=?8,
                     decision_delegated_to=?9,decision_action_at=?10,
                     responsibility_label=COALESCE(?9,responsibility_label)
                 WHERE attention_case_id=?1",
                params![
                    command.attention_case_id,
                    command.target_status.as_str(),
                    resolved.then_some(command.resolution_summary.as_str()),
                    command.occurred_at,
                    resolved.then_some(command.occurred_at),
                    command.decision_action,
                    command
                        .decision_action
                        .as_ref()
                        .map(|_| command.resolution_summary.as_str()),
                    command
                        .decision_action
                        .as_ref()
                        .map(|_| command.actor_label.as_str()),
                    command.delegated_to,
                    command
                        .decision_action
                        .as_ref()
                        .map(|_| command.occurred_at),
                ],
            )
            .map_err(database_error("update attention case"))?;
        read_attention_case(&self.transaction, &command.attention_case_id, false)
    }
}

fn validate_transition_contract(
    current: &AttentionCaseRecord,
    command: &TransitionAttentionCaseCommand,
) -> Result<(), PersistenceError> {
    let resolution_present = !command.resolution_summary.trim().is_empty();
    if current.case_type != AttentionCaseType::DecisionRequest {
        if command.decision_action.is_some() || command.delegated_to.is_some() {
            return Err(conflict("decision_action_type_invalid"));
        }
        if command.target_status == AttentionCaseStatus::Resolved && !resolution_present {
            return Err(conflict("attention_resolution_required"));
        }
        return Ok(());
    }
    let action = command
        .decision_action
        .as_deref()
        .ok_or_else(|| conflict("decision_action_required"))?;
    if !matches!(
        action,
        "approved" | "change_requested" | "rejected" | "delegated"
    ) || !resolution_present
    {
        return Err(conflict("decision_action_invalid"));
    }
    if action == "delegated" {
        if command.target_status != AttentionCaseStatus::Open
            || command
                .delegated_to
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(conflict("decision_delegation_invalid"));
        }
    } else if command.target_status != AttentionCaseStatus::Resolved
        || command.delegated_to.is_some()
    {
        return Err(conflict("decision_resolution_invalid"));
    }
    Ok(())
}

fn validate_create(command: &CreateAttentionCaseCommand) -> Result<(), PersistenceError> {
    for value in [
        command.attention_case_id.as_str(),
        command.project_id.as_str(),
        command.title.as_str(),
        command.detail.as_str(),
        command.responsibility_label.as_str(),
        command.reporter_label.as_str(),
        command.client_operation_id.as_str(),
        command.request_fingerprint.as_str(),
    ] {
        if value.trim().is_empty() {
            return Err(conflict("attention_contract_incomplete"));
        }
    }
    if !matches!(
        command.responsibility_type.as_str(),
        "user" | "collaboration" | "project_owner"
    ) {
        return Err(conflict("attention_responsibility_invalid"));
    }
    if !matches!(
        command.priority.as_str(),
        "low" | "medium" | "high" | "critical"
    ) {
        return Err(conflict("attention_priority_invalid"));
    }
    let has_agent_identity = command
        .agent_run_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && command
            .agent_label
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    if (command.case_type == AttentionCaseType::AgentRunIncident) != has_agent_identity {
        return Err(conflict("agent_incident_identity_invalid"));
    }
    let decision_fields = [
        command.decision_options.as_deref(),
        command.decision_impact.as_deref(),
        command.decision_evidence.as_deref(),
        command.decision_recommendation.as_deref(),
        command.decision_consequence.as_deref(),
    ];
    if command.case_type == AttentionCaseType::DecisionRequest {
        if decision_fields
            .iter()
            .any(|value| value.is_none_or(|item| item.trim().is_empty()))
        {
            return Err(conflict("decision_card_incomplete"));
        }
    } else if decision_fields.iter().any(Option::is_some) {
        return Err(conflict("decision_card_type_invalid"));
    }
    Ok(())
}

fn validate_references(
    transaction: &Transaction<'_>,
    command: &CreateAttentionCaseCommand,
) -> Result<(), PersistenceError> {
    let active: bool = transaction
        .query_row(
            "SELECT status='active' FROM projects WHERE project_id=?1",
            [&command.project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error("read attention project"))?
        .ok_or_else(|| conflict("project_not_found"))?;
    if !active {
        return Err(conflict("project_not_editable"));
    }
    for (id, sql, reason, operation) in [
        (
            command.phase_id.as_deref(),
            "SELECT EXISTS(SELECT 1 FROM project_phases WHERE phase_id=?1 AND project_id=?2)",
            "attention_phase_project_mismatch",
            "validate attention phase",
        ),
        (
            command.task_id.as_deref(),
            "SELECT EXISTS(SELECT 1 FROM business_tasks WHERE business_task_id=?1 AND project_id=?2)",
            "attention_task_project_mismatch",
            "validate attention task",
        ),
        (
            command.deliverable_id.as_deref(),
            "SELECT EXISTS(SELECT 1 FROM deliverables WHERE deliverable_id=?1 AND project_id=?2)",
            "attention_deliverable_project_mismatch",
            "validate attention deliverable",
        ),
    ] {
        if let Some(id) = id {
            let valid: bool = transaction
                .query_row(sql, params![id, command.project_id], |row| row.get(0))
                .map_err(database_error(operation))?;
            if !valid {
                return Err(conflict(reason));
            }
        }
    }
    Ok(())
}

fn read_attention_case(
    transaction: &Transaction<'_>,
    id: &str,
    replayed: bool,
) -> Result<AttentionCaseRecord, PersistenceError> {
    transaction
        .query_row(
            "SELECT attention_case_id,case_type,project_id,phase_id,task_id,deliverable_id,title,detail,status,responsibility_type,responsibility_label,reporter_label,priority,due_at,agent_run_id,agent_label,resolution_summary,state_revision,created_at,updated_at,resolved_at,decision_options,decision_impact,decision_evidence,decision_recommendation,decision_consequence,decision_action,decision_action_summary,decision_actor_label,decision_delegated_to,decision_action_at FROM attention_cases WHERE attention_case_id=?1",
            [id],
            |row| row_to_attention_case(row, replayed),
        )
        .optional()
        .map_err(database_error("read attention case"))?
        .ok_or_else(|| conflict("attention_case_not_found"))
}

fn row_to_attention_case(row: &Row<'_>, replayed: bool) -> rusqlite::Result<AttentionCaseRecord> {
    let case_type = row.get::<_, String>(1)?;
    let status = row.get::<_, String>(8)?;
    Ok(AttentionCaseRecord {
        attention_case_id: row.get(0)?,
        case_type: AttentionCaseType::try_from(case_type.as_str()).map_err(|()| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                "invalid attention case type".into(),
            )
        })?,
        project_id: row.get(2)?,
        phase_id: row.get(3)?,
        task_id: row.get(4)?,
        deliverable_id: row.get(5)?,
        title: row.get(6)?,
        detail: row.get(7)?,
        status: AttentionCaseStatus::try_from(status.as_str()).map_err(|()| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                "invalid attention case status".into(),
            )
        })?,
        responsibility_type: row.get(9)?,
        responsibility_label: row.get(10)?,
        reporter_label: row.get(11)?,
        priority: row.get(12)?,
        due_at: row.get(13)?,
        agent_run_id: row.get(14)?,
        agent_label: row.get(15)?,
        resolution_summary: row.get(16)?,
        state_revision: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
        resolved_at: row.get(20)?,
        decision_options: row.get(21)?,
        decision_impact: row.get(22)?,
        decision_evidence: row.get(23)?,
        decision_recommendation: row.get(24)?,
        decision_consequence: row.get(25)?,
        decision_action: row.get(26)?,
        decision_action_summary: row.get(27)?,
        decision_actor_label: row.get(28)?,
        decision_delegated_to: row.get(29)?,
        decision_action_at: row.get(30)?,
        replayed,
    })
}

fn conflict(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}
