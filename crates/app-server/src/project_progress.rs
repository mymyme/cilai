use contexthub_application::{
    ActivatePhasePlanCommand, BusinessTaskRecord, CreateBusinessTaskCommand,
    CreatePhasePlanCommand, MoveBusinessTaskCommand, PhaseDraftInput, PhasePlanRecord,
    ProjectPhaseRecord, ProjectProgressRepository, TransitionBusinessTaskCommand,
    TransitionProjectPhaseCommand, UnitOfWork,
};
use contexthub_domain::{BusinessTaskStatus, EventId, ProjectPhaseStatus};
use contexthub_persistence_sqlite::{PersistenceError, PersistenceErrorKind, SqlitePersistence};
use contexthub_protocol::{
    BusinessTaskContributionItem, BusinessTaskCreateParams, BusinessTaskCreateResult,
    BusinessTaskGetParams, BusinessTaskGetResult, BusinessTaskItem, BusinessTaskMoveParams,
    BusinessTaskMoveResult, BusinessTaskTransitionParams, BusinessTaskTransitionResult,
    PhasePlanActivateParams, PhasePlanActivateResult, PhasePlanCreateParams, PhasePlanCreateResult,
    PhasePlanItem, ProjectPhaseItem, ProjectPhaseTransitionParams, ProjectPhaseTransitionResult,
    ProjectProgressGetParams, ProjectProgressGetResult,
};

/// 按任务身份读取正式任务合同，包括项目级、尚未绑定阶段的 AI 自动归类任务。
///
/// # Errors
/// 任务标识为空、任务不存在或任务事实不可读时返回错误。
pub fn get_task(
    persistence: &mut SqlitePersistence,
    params: &BusinessTaskGetParams,
) -> Result<BusinessTaskGetResult, PersistenceError> {
    required(&params.business_task_id, "business_task_id_required")?;
    persistence
        .transact(|repositories| {
            let task = repositories.business_task(&params.business_task_id)?;
            let contributions =
                repositories.business_task_contributions(&params.business_task_id)?;
            Ok((task, contributions))
        })
        .map(|(task, contributions)| BusinessTaskGetResult {
            task: business_task_item(task),
            contributions: contributions
                .into_iter()
                .map(|item| BusinessTaskContributionItem {
                    run_id: item.run_id,
                    conversation_id: item.conversation_id,
                    conversation_status: item.conversation_status,
                    stage: item.stage,
                    outcome_title: item.outcome_title,
                    outcome_summary: item.outcome_summary,
                    reason: item.reason,
                    confidence: item.confidence,
                    created_at: item.created_at,
                })
                .collect(),
        })
}

/// 读取项目全部阶段计划及其业务任务。
///
/// # Errors
/// 项目标识为空或项目推进事实不可读时返回错误。
pub fn get_progress(
    persistence: &mut SqlitePersistence,
    params: &ProjectProgressGetParams,
) -> Result<ProjectProgressGetResult, PersistenceError> {
    required(&params.project_id, "progress_project_required")?;
    persistence
        .transact(|repositories| {
            let plans = repositories.phase_plans(&params.project_id)?;
            let tasks = repositories.project_tasks(&params.project_id)?;
            Ok((plans, tasks))
        })
        .map(|(plans, tasks)| ProjectProgressGetResult {
            project_id: params.project_id.clone(),
            plans: plans.into_iter().map(phase_plan_item).collect(),
            tasks: tasks.into_iter().map(business_task_item).collect(),
        })
}

/// 创建不可直接执行的阶段计划草案。
///
/// # Errors
/// 计划合同、交付物引用或幂等约束不满足时返回错误。
pub fn create_phase_plan(
    persistence: &mut SqlitePersistence,
    params: &PhasePlanCreateParams,
    now: i64,
) -> Result<PhasePlanCreateResult, PersistenceError> {
    required(&params.project_id, "phase_plan_project_required")?;
    required(&params.summary, "phase_plan_summary_required")?;
    required(&params.client_operation_id, "client_operation_id_required")?;
    required(&params.request_fingerprint, "request_fingerprint_required")?;
    if params.phases.is_empty() {
        return Err(configuration_error("phase_plan_phases_required"));
    }
    let id = uuid::Uuid::new_v4();
    let phase_ids = (0..params.phases.len())
        .map(|index| format!("phase-{id}-{index}"))
        .collect::<Vec<_>>();
    let phases = params
        .phases
        .iter()
        .enumerate()
        .map(|(index, phase)| {
            required(&phase.name, "phase_name_required")?;
            required(&phase.goal, "phase_goal_required")?;
            required(
                &phase.acceptance_criteria,
                "phase_acceptance_criteria_required",
            )?;
            if phase.predecessor_indexes.iter().any(|predecessor| {
                usize::try_from(*predecessor).map_or(true, |value| value >= index)
            }) {
                return Err(configuration_error("phase_predecessor_index_invalid"));
            }
            Ok(PhaseDraftInput {
                phase_id: phase_ids[index].clone(),
                name: phase.name.trim().to_owned(),
                goal: phase.goal.trim().to_owned(),
                acceptance_criteria: phase.acceptance_criteria.trim().to_owned(),
                required_deliverable_id: phase
                    .required_deliverable_id
                    .as_ref()
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
                sort_order: i64::try_from(index)
                    .map_err(|_| configuration_error("too_many_project_phases"))?,
                predecessor_phase_ids: phase
                    .predecessor_indexes
                    .iter()
                    .map(|predecessor| {
                        usize::try_from(*predecessor)
                            .ok()
                            .and_then(|value| phase_ids.get(value))
                            .cloned()
                            .ok_or_else(|| configuration_error("phase_predecessor_index_invalid"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            })
        })
        .collect::<Result<Vec<_>, PersistenceError>>()?;
    let command = CreatePhasePlanCommand {
        plan_id: format!("phase-plan-{id}"),
        project_id: params.project_id.clone(),
        summary: params.summary.trim().to_owned(),
        phases,
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("phase-plan-event-{id}")),
        correlation_id: format!("phase-plan-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| contexthub_application::create_phase_plan(repositories, &command))
        .map(|plan| PhasePlanCreateResult {
            plan: phase_plan_item(plan),
        })
}

/// 激活阶段计划草案。
///
/// # Errors
/// 计划不是草案、已有未完成计划或修订冲突时返回错误。
pub fn activate_phase_plan(
    persistence: &mut SqlitePersistence,
    params: &PhasePlanActivateParams,
    now: i64,
) -> Result<PhasePlanActivateResult, PersistenceError> {
    required(&params.plan_id, "phase_plan_required")?;
    required(&params.client_operation_id, "client_operation_id_required")?;
    required(&params.request_fingerprint, "request_fingerprint_required")?;
    let id = uuid::Uuid::new_v4();
    let command = ActivatePhasePlanCommand {
        plan_id: params.plan_id.clone(),
        expected_revision: params.expected_revision,
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("phase-plan-activate-event-{id}")),
        correlation_id: format!("phase-plan-activate-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::activate_phase_plan(repositories, &command)
        })
        .map(|plan| PhasePlanActivateResult {
            plan: phase_plan_item(plan),
        })
}

/// 转换项目阶段状态。
///
/// # Errors
/// 状态机、依赖、任务、交付物验收或修订门禁不满足时返回错误。
pub fn transition_phase(
    persistence: &mut SqlitePersistence,
    params: &ProjectPhaseTransitionParams,
    now: i64,
) -> Result<ProjectPhaseTransitionResult, PersistenceError> {
    let target_status = ProjectPhaseStatus::try_from(params.target_status.as_str())
        .map_err(|()| configuration_error("invalid_project_phase_status"))?;
    required(&params.phase_id, "phase_required")?;
    required(&params.actor_label, "phase_actor_required")?;
    required(&params.client_operation_id, "client_operation_id_required")?;
    required(&params.request_fingerprint, "request_fingerprint_required")?;
    let id = uuid::Uuid::new_v4();
    let command = TransitionProjectPhaseCommand {
        transition_id: format!("phase-transition-{id}"),
        phase_id: params.phase_id.clone(),
        target_status,
        expected_revision: params.expected_revision,
        actor_label: params.actor_label.trim().to_owned(),
        comment: params.comment.trim().to_owned(),
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("phase-transition-event-{id}")),
        correlation_id: format!("phase-transition-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::transition_project_phase(repositories, &command)
        })
        .map(|phase| ProjectPhaseTransitionResult {
            phase: project_phase_item(phase),
        })
}

/// 在进行中的项目阶段创建业务任务。
///
/// # Errors
/// 任务合同、阶段状态或幂等约束不满足时返回错误。
pub fn create_task(
    persistence: &mut SqlitePersistence,
    params: &BusinessTaskCreateParams,
    now: i64,
) -> Result<BusinessTaskCreateResult, PersistenceError> {
    for (value, reason) in [
        (&params.project_id, "business_task_project_required"),
        (&params.phase_id, "business_task_phase_required"),
        (&params.title, "business_task_title_required"),
        (&params.objective, "business_task_objective_required"),
        (&params.expected_output, "business_task_output_required"),
        (
            &params.completion_criteria,
            "business_task_completion_criteria_required",
        ),
        (&params.assignee_label, "business_task_assignee_required"),
        (&params.client_operation_id, "client_operation_id_required"),
        (&params.request_fingerprint, "request_fingerprint_required"),
    ] {
        required(value, reason)?;
    }
    let id = uuid::Uuid::new_v4();
    let command = CreateBusinessTaskCommand {
        business_task_id: format!("business-task-{id}"),
        project_id: params.project_id.clone(),
        phase_id: Some(params.phase_id.clone()),
        title: params.title.trim().to_owned(),
        objective: params.objective.trim().to_owned(),
        input_context: params.input_context.trim().to_owned(),
        expected_output: params.expected_output.trim().to_owned(),
        completion_criteria: params.completion_criteria.trim().to_owned(),
        assignee_type: params.assignee_type.as_str().to_owned(),
        assignee_label: params.assignee_label.trim().to_owned(),
        priority: params.priority.as_str().to_owned(),
        due_at: params.due_at,
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("business-task-event-{id}")),
        correlation_id: format!("business-task-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::create_business_task(repositories, &command)
        })
        .map(|task| BusinessTaskCreateResult {
            task: business_task_item(task),
        })
}

/// 转换业务任务状态。
///
/// # Errors
/// 状态机、任务验收、修订或幂等门禁不满足时返回错误。
pub fn transition_task(
    persistence: &mut SqlitePersistence,
    params: &BusinessTaskTransitionParams,
    now: i64,
) -> Result<BusinessTaskTransitionResult, PersistenceError> {
    let target_status = BusinessTaskStatus::try_from(params.target_status.as_str())
        .map_err(|()| configuration_error("invalid_business_task_status"))?;
    required(&params.business_task_id, "business_task_required")?;
    required(&params.actor_label, "business_task_actor_required")?;
    required(&params.client_operation_id, "client_operation_id_required")?;
    required(&params.request_fingerprint, "request_fingerprint_required")?;
    let id = uuid::Uuid::new_v4();
    let command = TransitionBusinessTaskCommand {
        transition_id: format!("business-task-transition-{id}"),
        business_task_id: params.business_task_id.clone(),
        target_status,
        expected_revision: params.expected_revision,
        actor_label: params.actor_label.trim().to_owned(),
        comment: params.comment.trim().to_owned(),
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("business-task-transition-event-{id}")),
        correlation_id: format!("business-task-transition-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::transition_business_task(repositories, &command)
        })
        .map(|task| BusinessTaskTransitionResult {
            task: business_task_item(task),
        })
}

/// 在同一有效计划内移动业务任务。
///
/// # Errors
/// 任务状态、目标阶段、影响说明、修订或幂等门禁不满足时返回错误。
pub fn move_task(
    persistence: &mut SqlitePersistence,
    params: &BusinessTaskMoveParams,
    now: i64,
) -> Result<BusinessTaskMoveResult, PersistenceError> {
    for (value, reason) in [
        (&params.business_task_id, "business_task_required"),
        (
            &params.target_phase_id,
            "business_task_target_phase_required",
        ),
        (&params.actor_label, "business_task_move_actor_required"),
        (
            &params.dependency_impact,
            "business_task_move_dependency_impact_required",
        ),
        (
            &params.deliverable_impact,
            "business_task_move_deliverable_impact_required",
        ),
        (
            &params.acceptance_impact,
            "business_task_move_acceptance_impact_required",
        ),
        (&params.client_operation_id, "client_operation_id_required"),
        (&params.request_fingerprint, "request_fingerprint_required"),
    ] {
        required(value, reason)?;
    }
    let id = uuid::Uuid::new_v4();
    let command = MoveBusinessTaskCommand {
        move_id: format!("business-task-move-{id}"),
        business_task_id: params.business_task_id.clone(),
        target_phase_id: params.target_phase_id.clone(),
        expected_revision: params.expected_revision,
        actor_label: params.actor_label.trim().to_owned(),
        dependency_impact: params.dependency_impact.trim().to_owned(),
        deliverable_impact: params.deliverable_impact.trim().to_owned(),
        acceptance_impact: params.acceptance_impact.trim().to_owned(),
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("business-task-move-event-{id}")),
        correlation_id: format!("business-task-move-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| contexthub_application::move_business_task(repositories, &command))
        .map(|task| BusinessTaskMoveResult {
            task: business_task_item(task),
        })
}

fn phase_plan_item(plan: PhasePlanRecord) -> PhasePlanItem {
    PhasePlanItem {
        plan_id: plan.plan_id,
        project_id: plan.project_id,
        version_no: plan.version_no,
        status: plan.status.as_str().to_owned(),
        summary: plan.summary,
        state_revision: plan.state_revision,
        created_at: plan.created_at,
        activated_at: plan.activated_at,
        phases: plan.phases.into_iter().map(project_phase_item).collect(),
    }
}

fn project_phase_item(phase: ProjectPhaseRecord) -> ProjectPhaseItem {
    ProjectPhaseItem {
        phase_id: phase.phase_id,
        plan_id: phase.plan_id,
        project_id: phase.project_id,
        name: phase.name,
        goal: phase.goal,
        acceptance_criteria: phase.acceptance_criteria,
        required_deliverable_id: phase.required_deliverable_id,
        required_deliverable_title: phase.required_deliverable_title,
        required_deliverable_version_id: phase.required_deliverable_version_id,
        sort_order: phase.sort_order,
        predecessor_phase_ids: phase.predecessor_phase_ids,
        is_main: phase.is_main,
        status: phase.status.as_str().to_owned(),
        state_revision: phase.state_revision,
        tasks: phase.tasks.into_iter().map(business_task_item).collect(),
    }
}

fn business_task_item(task: BusinessTaskRecord) -> BusinessTaskItem {
    BusinessTaskItem {
        business_task_id: task.business_task_id,
        project_id: task.project_id,
        phase_id: task.phase_id,
        title: task.title,
        objective: task.objective,
        input_context: task.input_context,
        expected_output: task.expected_output,
        completion_criteria: task.completion_criteria,
        assignee_type: task.assignee_type,
        assignee_label: task.assignee_label,
        priority: task.priority,
        due_at: task.due_at,
        attempt_no: task.attempt_no,
        status: task.status.as_str().to_owned(),
        state_revision: task.state_revision,
        updated_at: task.updated_at,
    }
}

fn required(value: &str, reason: &'static str) -> Result<(), PersistenceError> {
    if value.trim().is_empty() {
        Err(configuration_error(reason))
    } else {
        Ok(())
    }
}

fn configuration_error(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Configuration, reason, reason)
}
