use crate::{DurableEventRepository, NewDurableEvent};
use contexthub_domain::{BusinessTaskStatus, EventId, PhasePlanStatus, ProjectPhaseStatus};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhaseDraftInput {
    pub phase_id: String,
    pub name: String,
    pub goal: String,
    pub acceptance_criteria: String,
    pub required_deliverable_id: Option<String>,
    pub sort_order: i64,
    pub predecessor_phase_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BusinessTaskRecord {
    pub business_task_id: String,
    pub project_id: String,
    pub phase_id: Option<String>,
    pub title: String,
    pub objective: String,
    pub input_context: String,
    pub expected_output: String,
    pub completion_criteria: String,
    pub assignee_type: String,
    pub assignee_label: String,
    pub priority: String,
    pub due_at: Option<i64>,
    pub attempt_no: i64,
    pub status: BusinessTaskStatus,
    pub state_revision: i64,
    pub updated_at: i64,
    pub replayed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BusinessTaskContributionRecord {
    pub run_id: String,
    pub conversation_id: String,
    pub conversation_status: String,
    pub stage: Option<String>,
    pub outcome_title: Option<String>,
    pub outcome_summary: Option<String>,
    pub reason: String,
    pub confidence: f64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectPhaseRecord {
    pub phase_id: String,
    pub plan_id: String,
    pub project_id: String,
    pub name: String,
    pub goal: String,
    pub acceptance_criteria: String,
    pub required_deliverable_id: Option<String>,
    pub required_deliverable_title: Option<String>,
    pub required_deliverable_version_id: Option<String>,
    pub sort_order: i64,
    pub predecessor_phase_ids: Vec<String>,
    pub is_main: bool,
    pub status: ProjectPhaseStatus,
    pub state_revision: i64,
    pub tasks: Vec<BusinessTaskRecord>,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhasePlanRecord {
    pub plan_id: String,
    pub project_id: String,
    pub version_no: i64,
    pub status: PhasePlanStatus,
    pub summary: String,
    pub state_revision: i64,
    pub created_at: i64,
    pub activated_at: Option<i64>,
    pub phases: Vec<ProjectPhaseRecord>,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatePhasePlanCommand {
    pub plan_id: String,
    pub project_id: String,
    pub summary: String,
    pub phases: Vec<PhaseDraftInput>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivatePhasePlanCommand {
    pub plan_id: String,
    pub expected_revision: i64,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionProjectPhaseCommand {
    pub transition_id: String,
    pub phase_id: String,
    pub target_status: ProjectPhaseStatus,
    pub expected_revision: i64,
    pub actor_label: String,
    pub comment: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateBusinessTaskCommand {
    pub business_task_id: String,
    pub project_id: String,
    pub phase_id: Option<String>,
    pub title: String,
    pub objective: String,
    pub input_context: String,
    pub expected_output: String,
    pub completion_criteria: String,
    pub assignee_type: String,
    pub assignee_label: String,
    pub priority: String,
    pub due_at: Option<i64>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionBusinessTaskCommand {
    pub transition_id: String,
    pub business_task_id: String,
    pub target_status: BusinessTaskStatus,
    pub expected_revision: i64,
    pub actor_label: String,
    pub comment: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoveBusinessTaskCommand {
    pub move_id: String,
    pub business_task_id: String,
    pub target_phase_id: String,
    pub expected_revision: i64,
    pub actor_label: String,
    pub dependency_impact: String,
    pub deliverable_impact: String,
    pub acceptance_impact: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

pub trait ProjectProgressRepository {
    type Error;

    /// # Errors
    /// 数据无法读取或状态无法转换时返回错误。
    fn phase_plans(&mut self, project_id: &str) -> Result<Vec<PhasePlanRecord>, Self::Error>;
    /// # Errors
    /// 任务不存在、任务标识无效或任务事实不可读时返回错误。
    fn business_task(&mut self, task_id: &str) -> Result<BusinessTaskRecord, Self::Error>;
    /// # Errors
    /// 任务关联的对话运行记录无法读取时返回错误。
    fn business_task_contributions(
        &mut self,
        task_id: &str,
    ) -> Result<Vec<BusinessTaskContributionRecord>, Self::Error>;
    /// # Errors
    /// 项目不存在或项目任务不可读时返回错误。
    fn project_tasks(&mut self, project_id: &str) -> Result<Vec<BusinessTaskRecord>, Self::Error>;
    /// # Errors
    /// 项目、阶段、交付物、幂等或数据约束不满足时返回错误。
    fn insert_phase_plan(
        &mut self,
        command: &CreatePhasePlanCommand,
    ) -> Result<PhasePlanRecord, Self::Error>;
    /// # Errors
    /// 草案状态、修订、幂等或数据约束不满足时返回错误。
    fn activate_phase_plan(
        &mut self,
        command: &ActivatePhasePlanCommand,
    ) -> Result<PhasePlanRecord, Self::Error>;
    /// # Errors
    /// 阶段状态、依赖、验收关口、修订或幂等约束不满足时返回错误。
    fn transition_project_phase(
        &mut self,
        command: &TransitionProjectPhaseCommand,
    ) -> Result<ProjectPhaseRecord, Self::Error>;
    /// # Errors
    /// 阶段、任务合同、幂等或数据约束不满足时返回错误。
    fn insert_business_task(
        &mut self,
        command: &CreateBusinessTaskCommand,
    ) -> Result<BusinessTaskRecord, Self::Error>;
    /// # Errors
    /// 任务状态、修订、验收或幂等约束不满足时返回错误。
    fn transition_business_task(
        &mut self,
        command: &TransitionBusinessTaskCommand,
    ) -> Result<BusinessTaskRecord, Self::Error>;
    /// # Errors
    /// 任务状态、目标阶段、影响说明、修订或幂等约束不满足时返回错误。
    fn move_business_task(
        &mut self,
        command: &MoveBusinessTaskCommand,
    ) -> Result<BusinessTaskRecord, Self::Error>;
}

/// 创建阶段计划草案并追加耐久事件。
///
/// # Errors
/// 项目、阶段合同、交付物引用、幂等键或事件写入无效时返回仓储错误。
pub fn create_phase_plan<R>(
    repositories: &mut R,
    command: &CreatePhasePlanCommand,
) -> Result<PhasePlanRecord, <R as ProjectProgressRepository>::Error>
where
    R: ProjectProgressRepository
        + DurableEventRepository<Error = <R as ProjectProgressRepository>::Error>,
{
    let plan = repositories.insert_phase_plan(command)?;
    if !plan.replayed {
        append_progress_event(
            repositories,
            command.event_id.clone(),
            "phase_plan.draft_created",
            "phase_plan",
            &plan.plan_id,
            &command.correlation_id,
            command.occurred_at,
        )?;
    }
    Ok(plan)
}

/// 激活阶段计划并追加耐久事件。
///
/// # Errors
/// 计划状态、修订、未完成计划、幂等键或事件写入无效时返回仓储错误。
pub fn activate_phase_plan<R>(
    repositories: &mut R,
    command: &ActivatePhasePlanCommand,
) -> Result<PhasePlanRecord, <R as ProjectProgressRepository>::Error>
where
    R: ProjectProgressRepository
        + DurableEventRepository<Error = <R as ProjectProgressRepository>::Error>,
{
    let plan = repositories.activate_phase_plan(command)?;
    if !plan.replayed {
        append_progress_event(
            repositories,
            command.event_id.clone(),
            "phase_plan.activated",
            "phase_plan",
            &plan.plan_id,
            &command.correlation_id,
            command.occurred_at,
        )?;
    }
    Ok(plan)
}

/// 转换项目阶段状态并追加耐久事件。
///
/// # Errors
/// 阶段依赖、任务、交付物验收、修订、幂等键或事件写入无效时返回仓储错误。
pub fn transition_project_phase<R>(
    repositories: &mut R,
    command: &TransitionProjectPhaseCommand,
) -> Result<ProjectPhaseRecord, <R as ProjectProgressRepository>::Error>
where
    R: ProjectProgressRepository
        + DurableEventRepository<Error = <R as ProjectProgressRepository>::Error>,
{
    let phase = repositories.transition_project_phase(command)?;
    if !phase.replayed {
        append_progress_event(
            repositories,
            command.event_id.clone(),
            "project_phase.changed",
            "project_phase",
            &phase.phase_id,
            &command.correlation_id,
            command.occurred_at,
        )?;
    }
    Ok(phase)
}

/// 创建阶段业务任务并追加耐久事件。
///
/// # Errors
/// 任务合同、阶段状态、幂等键或事件写入无效时返回仓储错误。
pub fn create_business_task<R>(
    repositories: &mut R,
    command: &CreateBusinessTaskCommand,
) -> Result<BusinessTaskRecord, <R as ProjectProgressRepository>::Error>
where
    R: ProjectProgressRepository
        + DurableEventRepository<Error = <R as ProjectProgressRepository>::Error>,
{
    let task = repositories.insert_business_task(command)?;
    if !task.replayed {
        append_progress_event(
            repositories,
            command.event_id.clone(),
            "business_task.created",
            "business_task",
            &task.business_task_id,
            &command.correlation_id,
            command.occurred_at,
        )?;
    }
    Ok(task)
}

/// 转换业务任务状态并追加耐久事件。
///
/// # Errors
/// 任务状态、验收信息、修订、幂等键或事件写入无效时返回仓储错误。
pub fn transition_business_task<R>(
    repositories: &mut R,
    command: &TransitionBusinessTaskCommand,
) -> Result<BusinessTaskRecord, <R as ProjectProgressRepository>::Error>
where
    R: ProjectProgressRepository
        + DurableEventRepository<Error = <R as ProjectProgressRepository>::Error>,
{
    let task = repositories.transition_business_task(command)?;
    if !task.replayed {
        append_progress_event(
            repositories,
            command.event_id.clone(),
            "business_task.changed",
            "business_task",
            &task.business_task_id,
            &command.correlation_id,
            command.occurred_at,
        )?;
    }
    Ok(task)
}

/// 在同一有效阶段计划内移动业务任务并追加耐久事件。
///
/// # Errors
/// 任务状态、目标阶段、影响说明、修订、幂等键或事件写入无效时返回仓储错误。
pub fn move_business_task<R>(
    repositories: &mut R,
    command: &MoveBusinessTaskCommand,
) -> Result<BusinessTaskRecord, <R as ProjectProgressRepository>::Error>
where
    R: ProjectProgressRepository
        + DurableEventRepository<Error = <R as ProjectProgressRepository>::Error>,
{
    let task = repositories.move_business_task(command)?;
    if !task.replayed {
        append_progress_event(
            repositories,
            command.event_id.clone(),
            "business_task.moved",
            "business_task",
            &task.business_task_id,
            &command.correlation_id,
            command.occurred_at,
        )?;
    }
    Ok(task)
}

fn append_progress_event<R: DurableEventRepository>(
    repositories: &mut R,
    event_id: EventId,
    event_type: &str,
    object_type: &str,
    object_id: &str,
    correlation_id: &str,
    occurred_at: i64,
) -> Result<(), R::Error> {
    repositories.append_event(&NewDurableEvent {
        event_id,
        workspace_id: None,
        event_type: event_type.to_owned(),
        object_type: Some(object_type.to_owned()),
        object_id: Some(object_id.to_owned()),
        payload_json: "{}".to_owned(),
        correlation_id: Some(correlation_id.to_owned()),
        occurred_at,
        created_at: occurred_at,
    })?;
    Ok(())
}
