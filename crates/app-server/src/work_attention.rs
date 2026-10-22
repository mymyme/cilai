use contexthub_application::{
    AttentionCaseRecord, AttentionCaseRepository, CreateAttentionCaseCommand,
    TransitionAttentionCaseCommand, UnitOfWork,
};
use contexthub_domain::{AttentionCaseStatus, AttentionCaseType, EventId};
use contexthub_persistence_sqlite::{PersistenceError, PersistenceErrorKind, SqlitePersistence};
use contexthub_protocol::{
    AgentIncidentReportParams, AgentIncidentReportResult, AttentionCaseItem,
    AttentionCaseListParams, AttentionCaseListResult, AttentionCaseTransitionParams,
    AttentionCaseTransitionResult, DecisionRequestCreateParams, DecisionRequestCreateResult,
    ProjectBlockerCreateParams, ProjectBlockerCreateResult,
};

/// 读取项目阻塞、决策请求和智能体运行异常。
///
/// # Errors
/// 项目标识为空或关注事项不可读时返回错误。
pub fn list_cases(
    persistence: &mut SqlitePersistence,
    params: &AttentionCaseListParams,
) -> Result<AttentionCaseListResult, PersistenceError> {
    required(&params.project_id, "attention_project_required")?;
    persistence
        .transact(|repositories| repositories.attention_cases(&params.project_id))
        .map(|cases| AttentionCaseListResult {
            project_id: params.project_id.clone(),
            cases: cases.into_iter().map(attention_case_item).collect(),
        })
}

/// 创建项目阻塞事实。
///
/// # Errors
/// 合同、项目关联或幂等约束无效时返回错误。
pub fn create_blocker(
    persistence: &mut SqlitePersistence,
    params: &ProjectBlockerCreateParams,
    now: i64,
) -> Result<ProjectBlockerCreateResult, PersistenceError> {
    required_fields(&[
        (&params.project_id, "attention_project_required"),
        (&params.title, "blocker_title_required"),
        (&params.reason, "blocker_reason_required"),
        (&params.owner_label, "blocker_owner_required"),
        (&params.reporter_label, "attention_reporter_required"),
        (&params.client_operation_id, "client_operation_id_required"),
        (&params.request_fingerprint, "request_fingerprint_required"),
    ])?;
    let command = create_command(
        AttentionCaseType::ProjectBlocker,
        &params.project_id,
        params.phase_id.clone(),
        params.task_id.clone(),
        None,
        &params.title,
        &params.reason,
        "user",
        &params.owner_label,
        &params.reporter_label,
        params.priority.as_str(),
        params.due_at,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        &params.client_operation_id,
        &params.request_fingerprint,
        now,
    );
    persistence
        .transact(|repositories| {
            contexthub_application::create_attention_case(repositories, &command)
        })
        .map(|record| ProjectBlockerCreateResult {
            blocker: attention_case_item(record),
        })
}

/// 创建需要项目负责人回应的决策请求。
///
/// # Errors
/// 合同、项目关联或幂等约束无效时返回错误。
pub fn create_decision_request(
    persistence: &mut SqlitePersistence,
    params: &DecisionRequestCreateParams,
    now: i64,
) -> Result<DecisionRequestCreateResult, PersistenceError> {
    required_fields(&[
        (&params.project_id, "attention_project_required"),
        (&params.title, "decision_title_required"),
        (&params.question, "decision_question_required"),
        (&params.options, "decision_options_required"),
        (&params.impact, "decision_impact_required"),
        (&params.evidence, "decision_evidence_required"),
        (&params.recommendation, "decision_recommendation_required"),
        (&params.consequence, "decision_consequence_required"),
        (&params.decision_owner_label, "decision_owner_required"),
        (&params.requester_label, "decision_requester_required"),
        (&params.client_operation_id, "client_operation_id_required"),
        (&params.request_fingerprint, "request_fingerprint_required"),
    ])?;
    let command = create_command(
        AttentionCaseType::DecisionRequest,
        &params.project_id,
        params.phase_id.clone(),
        None,
        params.deliverable_id.clone(),
        &params.title,
        &params.question,
        "project_owner",
        &params.decision_owner_label,
        &params.requester_label,
        params.priority.as_str(),
        params.due_at,
        None,
        None,
        Some(params.options.clone()),
        Some(params.impact.clone()),
        Some(params.evidence.clone()),
        Some(params.recommendation.clone()),
        Some(params.consequence.clone()),
        &params.client_operation_id,
        &params.request_fingerprint,
        now,
    );
    persistence
        .transact(|repositories| {
            contexthub_application::create_attention_case(repositories, &command)
        })
        .map(|record| DecisionRequestCreateResult {
            decision_request: attention_case_item(record),
        })
}

/// 接收外部 Agent 执行器上报的异常事实，不在本进程内执行 Agent。
///
/// # Errors
/// 运行身份、项目关联或幂等约束无效时返回错误。
pub fn report_agent_incident(
    persistence: &mut SqlitePersistence,
    params: &AgentIncidentReportParams,
    now: i64,
) -> Result<AgentIncidentReportResult, PersistenceError> {
    required_fields(&[
        (&params.project_id, "attention_project_required"),
        (&params.run_id, "agent_run_id_required"),
        (&params.agent_label, "agent_label_required"),
        (&params.title, "agent_incident_title_required"),
        (&params.error_summary, "agent_incident_summary_required"),
        (
            &params.responsibility_label,
            "agent_incident_owner_required",
        ),
        (&params.reporter_label, "attention_reporter_required"),
        (&params.client_operation_id, "client_operation_id_required"),
        (&params.request_fingerprint, "request_fingerprint_required"),
    ])?;
    let command = create_command(
        AttentionCaseType::AgentRunIncident,
        &params.project_id,
        params.phase_id.clone(),
        params.task_id.clone(),
        None,
        &params.title,
        &params.error_summary,
        "user",
        &params.responsibility_label,
        &params.reporter_label,
        params.priority.as_str(),
        None,
        Some(params.run_id.clone()),
        Some(params.agent_label.clone()),
        None,
        None,
        None,
        None,
        None,
        &params.client_operation_id,
        &params.request_fingerprint,
        now,
    );
    persistence
        .transact(|repositories| {
            contexthub_application::create_attention_case(repositories, &command)
        })
        .map(|record| AgentIncidentReportResult {
            incident: attention_case_item(record),
        })
}

/// 确认或解决关注事项。
///
/// # Errors
/// 状态机、修订、处置说明或幂等约束无效时返回错误。
pub fn transition_case(
    persistence: &mut SqlitePersistence,
    params: &AttentionCaseTransitionParams,
    now: i64,
) -> Result<AttentionCaseTransitionResult, PersistenceError> {
    required_fields(&[
        (&params.attention_case_id, "attention_case_required"),
        (&params.actor_label, "attention_actor_required"),
        (&params.client_operation_id, "client_operation_id_required"),
        (&params.request_fingerprint, "request_fingerprint_required"),
    ])?;
    let target_status = AttentionCaseStatus::try_from(params.target_status.as_str())
        .map_err(|()| configuration_error("attention_target_status_invalid"))?;
    let decision_action = params
        .decision_action
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let delegated_to = params
        .delegated_to
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if target_status == AttentionCaseStatus::Open && decision_action.as_deref() != Some("delegated")
    {
        return Err(configuration_error("attention_target_status_invalid"));
    }
    if target_status == AttentionCaseStatus::Resolved || decision_action.is_some() {
        required(&params.resolution_summary, "attention_resolution_required")?;
    }
    let id = uuid::Uuid::new_v4();
    let command = TransitionAttentionCaseCommand {
        transition_id: format!("attention-transition-{id}"),
        attention_case_id: params.attention_case_id.clone(),
        target_status,
        expected_revision: params.expected_revision,
        actor_label: params.actor_label.trim().to_owned(),
        resolution_summary: params.resolution_summary.trim().to_owned(),
        decision_action,
        delegated_to,
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("attention-transition-event-{id}")),
        correlation_id: format!("attention-transition-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::transition_attention_case(repositories, &command)
        })
        .map(|record| AttentionCaseTransitionResult {
            attention_case: attention_case_item(record),
        })
}

#[allow(clippy::too_many_arguments)]
fn create_command(
    case_type: AttentionCaseType,
    project_id: &str,
    phase_id: Option<String>,
    task_id: Option<String>,
    deliverable_id: Option<String>,
    title: &str,
    detail: &str,
    responsibility_type: &str,
    responsibility_label: &str,
    reporter_label: &str,
    priority: &str,
    due_at: Option<i64>,
    agent_run_id: Option<String>,
    agent_label: Option<String>,
    decision_options: Option<String>,
    decision_impact: Option<String>,
    decision_evidence: Option<String>,
    decision_recommendation: Option<String>,
    decision_consequence: Option<String>,
    client_operation_id: &str,
    request_fingerprint: &str,
    now: i64,
) -> CreateAttentionCaseCommand {
    let id = uuid::Uuid::new_v4();
    CreateAttentionCaseCommand {
        attention_case_id: format!("attention-{id}"),
        case_type,
        project_id: project_id.to_owned(),
        phase_id,
        task_id,
        deliverable_id,
        title: title.trim().to_owned(),
        detail: detail.trim().to_owned(),
        responsibility_type: responsibility_type.to_owned(),
        responsibility_label: responsibility_label.trim().to_owned(),
        reporter_label: reporter_label.trim().to_owned(),
        priority: priority.to_owned(),
        due_at,
        agent_run_id,
        agent_label,
        decision_options,
        decision_impact,
        decision_evidence,
        decision_recommendation,
        decision_consequence,
        client_operation_id: client_operation_id.to_owned(),
        request_fingerprint: request_fingerprint.to_owned(),
        event_id: EventId::new(format!("attention-create-event-{id}")),
        correlation_id: format!("attention-create-correlation-{id}"),
        occurred_at: now,
    }
}

fn attention_case_item(record: AttentionCaseRecord) -> AttentionCaseItem {
    AttentionCaseItem {
        attention_case_id: record.attention_case_id,
        case_type: record.case_type.as_str().to_owned(),
        project_id: record.project_id,
        phase_id: record.phase_id,
        task_id: record.task_id,
        deliverable_id: record.deliverable_id,
        title: record.title,
        detail: record.detail,
        status: record.status.as_str().to_owned(),
        responsibility_type: record.responsibility_type,
        responsibility_label: record.responsibility_label,
        reporter_label: record.reporter_label,
        priority: record.priority,
        due_at: record.due_at,
        agent_run_id: record.agent_run_id,
        agent_label: record.agent_label,
        resolution_summary: record.resolution_summary,
        decision_options: record.decision_options,
        decision_impact: record.decision_impact,
        decision_evidence: record.decision_evidence,
        decision_recommendation: record.decision_recommendation,
        decision_consequence: record.decision_consequence,
        decision_action: record.decision_action,
        decision_action_summary: record.decision_action_summary,
        decision_actor_label: record.decision_actor_label,
        decision_delegated_to: record.decision_delegated_to,
        decision_action_at: record.decision_action_at,
        state_revision: record.state_revision,
        created_at: record.created_at,
        updated_at: record.updated_at,
        resolved_at: record.resolved_at,
    }
}

fn required_fields(values: &[(&String, &'static str)]) -> Result<(), PersistenceError> {
    for (value, reason) in values {
        required(value, reason)?;
    }
    Ok(())
}

fn required(value: &str, reason: &'static str) -> Result<(), PersistenceError> {
    if value.trim().is_empty() {
        return Err(configuration_error(reason));
    }
    Ok(())
}

fn configuration_error(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Configuration, reason, reason)
}
