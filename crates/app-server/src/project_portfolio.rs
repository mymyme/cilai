use contexthub_application::{ProjectPortfolioItemRecord, ProjectPortfolioPhaseRecord, UnitOfWork};
use contexthub_domain::WorkspaceId;
use contexthub_persistence_sqlite::{PersistenceError, SqlitePersistence};
use contexthub_protocol::{
    ProjectPortfolioItem, ProjectPortfolioListParams, ProjectPortfolioListResult,
    ProjectPortfolioPhaseItem,
};

/// 按工作区批量读取跨项目总览投影，一次事务内完成，避免调用方逐项目往返。
///
/// # Errors
///
/// 数据库事实无法读取或领域状态无法转换时返回错误。
pub fn list_portfolio(
    persistence: &mut SqlitePersistence,
    params: &ProjectPortfolioListParams,
) -> Result<ProjectPortfolioListResult, PersistenceError> {
    let workspace_ids = params
        .workspace_ids
        .iter()
        .cloned()
        .map(WorkspaceId::new)
        .collect::<Vec<_>>();
    persistence
        .transact(|repositories| {
            contexthub_application::get_project_portfolio(
                repositories,
                &contexthub_application::ProjectPortfolioQuery { workspace_ids },
            )
        })
        .map(|records| ProjectPortfolioListResult {
            items: records.into_iter().map(portfolio_item).collect(),
        })
}

fn portfolio_item(record: ProjectPortfolioItemRecord) -> ProjectPortfolioItem {
    ProjectPortfolioItem {
        project_id: record.project_id,
        display_name: record.display_name,
        goal: record.goal,
        status: record.status,
        workspace_id: record
            .workspace_id
            .map(|workspace| workspace.as_str().to_owned()),
        workspace_display_name: record.workspace_display_name,
        workspace_ids: record
            .workspace_ids
            .into_iter()
            .map(|workspace| workspace.as_str().to_owned())
            .collect(),
        state_revision: record.state_revision,
        updated_at: record.updated_at,
        has_plan: record.has_plan,
        active_phases: record
            .active_phases
            .into_iter()
            .map(portfolio_phase)
            .collect(),
        next_phase_name: record.next_phase_name,
        task_total: record.task_total,
        task_completed: record.task_completed,
        deliverable_total: record.deliverable_total,
        deliverable_accepted: record.deliverable_accepted,
        blocker_count: record.blocker_count,
        decision_count: record.decision_count,
        incident_count: record.incident_count,
    }
}

fn portfolio_phase(record: ProjectPortfolioPhaseRecord) -> ProjectPortfolioPhaseItem {
    ProjectPortfolioPhaseItem {
        phase_id: record.phase_id,
        name: record.name,
        status: record.status,
    }
}
