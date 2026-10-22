use crate::{DurableEventRepository, NewDurableEvent};
use contexthub_domain::{AttentionCaseStatus, AttentionCaseType, EventId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttentionCaseRecord {
    pub attention_case_id: String,
    pub case_type: AttentionCaseType,
    pub project_id: String,
    pub phase_id: Option<String>,
    pub task_id: Option<String>,
    pub deliverable_id: Option<String>,
    pub title: String,
    pub detail: String,
    pub status: AttentionCaseStatus,
    pub responsibility_type: String,
    pub responsibility_label: String,
    pub reporter_label: String,
    pub priority: String,
    pub due_at: Option<i64>,
    pub agent_run_id: Option<String>,
    pub agent_label: Option<String>,
    pub resolution_summary: Option<String>,
    pub decision_options: Option<String>,
    pub decision_impact: Option<String>,
    pub decision_evidence: Option<String>,
    pub decision_recommendation: Option<String>,
    pub decision_consequence: Option<String>,
    pub decision_action: Option<String>,
    pub decision_action_summary: Option<String>,
    pub decision_actor_label: Option<String>,
    pub decision_delegated_to: Option<String>,
    pub decision_action_at: Option<i64>,
    pub state_revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub resolved_at: Option<i64>,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateAttentionCaseCommand {
    pub attention_case_id: String,
    pub case_type: AttentionCaseType,
    pub project_id: String,
    pub phase_id: Option<String>,
    pub task_id: Option<String>,
    pub deliverable_id: Option<String>,
    pub title: String,
    pub detail: String,
    pub responsibility_type: String,
    pub responsibility_label: String,
    pub reporter_label: String,
    pub priority: String,
    pub due_at: Option<i64>,
    pub agent_run_id: Option<String>,
    pub agent_label: Option<String>,
    pub decision_options: Option<String>,
    pub decision_impact: Option<String>,
    pub decision_evidence: Option<String>,
    pub decision_recommendation: Option<String>,
    pub decision_consequence: Option<String>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionAttentionCaseCommand {
    pub transition_id: String,
    pub attention_case_id: String,
    pub target_status: AttentionCaseStatus,
    pub expected_revision: i64,
    pub actor_label: String,
    pub resolution_summary: String,
    pub decision_action: Option<String>,
    pub delegated_to: Option<String>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

pub trait AttentionCaseRepository {
    type Error;

    /// # Errors
    /// 项目关注事项不可读或持久化状态无法转换时返回错误。
    fn attention_cases(
        &mut self,
        project_id: &str,
    ) -> Result<Vec<AttentionCaseRecord>, Self::Error>;

    /// # Errors
    /// 项目、关联对象、幂等键或事项合同无效时返回错误。
    fn insert_attention_case(
        &mut self,
        command: &CreateAttentionCaseCommand,
    ) -> Result<AttentionCaseRecord, Self::Error>;

    /// # Errors
    /// 状态机、修订、处置说明或幂等键无效时返回错误。
    fn transition_attention_case(
        &mut self,
        command: &TransitionAttentionCaseCommand,
    ) -> Result<AttentionCaseRecord, Self::Error>;
}

/// 创建具有正式来源和审计身份的项目关注事项。
///
/// # Errors
/// 关注事项或事件无法在同一事务中提交时返回仓储错误。
pub fn create_attention_case<R>(
    repositories: &mut R,
    command: &CreateAttentionCaseCommand,
) -> Result<AttentionCaseRecord, <R as AttentionCaseRepository>::Error>
where
    R: AttentionCaseRepository
        + DurableEventRepository<Error = <R as AttentionCaseRepository>::Error>,
{
    let record = repositories.insert_attention_case(command)?;
    if !record.replayed {
        append_attention_event(
            repositories,
            command.event_id.clone(),
            match record.case_type {
                AttentionCaseType::ProjectBlocker => "project_blocker.opened",
                AttentionCaseType::DecisionRequest => "decision_request.opened",
                AttentionCaseType::AgentRunIncident => "agent_run_incident.reported",
            },
            &record.attention_case_id,
            &command.correlation_id,
            command.occurred_at,
        )?;
    }
    Ok(record)
}

/// 按事项类型状态机确认或解决项目关注事项。
///
/// # Errors
/// 状态转换或事件无法在同一事务中提交时返回仓储错误。
pub fn transition_attention_case<R>(
    repositories: &mut R,
    command: &TransitionAttentionCaseCommand,
) -> Result<AttentionCaseRecord, <R as AttentionCaseRepository>::Error>
where
    R: AttentionCaseRepository
        + DurableEventRepository<Error = <R as AttentionCaseRepository>::Error>,
{
    let record = repositories.transition_attention_case(command)?;
    if !record.replayed {
        append_attention_event(
            repositories,
            command.event_id.clone(),
            if command.decision_action.as_deref() == Some("delegated") {
                "decision_request.delegated"
            } else if record.status == AttentionCaseStatus::Resolved {
                "attention_case.resolved"
            } else {
                "attention_case.acknowledged"
            },
            &record.attention_case_id,
            &command.correlation_id,
            command.occurred_at,
        )?;
    }
    Ok(record)
}

fn append_attention_event<R: DurableEventRepository>(
    repositories: &mut R,
    event_id: EventId,
    event_type: &str,
    object_id: &str,
    correlation_id: &str,
    occurred_at: i64,
) -> Result<(), R::Error> {
    repositories.append_event(&NewDurableEvent {
        event_id,
        workspace_id: None,
        event_type: event_type.to_owned(),
        object_type: Some("attention_case".to_owned()),
        object_id: Some(object_id.to_owned()),
        payload_json: "{}".to_owned(),
        correlation_id: Some(correlation_id.to_owned()),
        occurred_at,
        created_at: occurred_at,
    })?;
    Ok(())
}
