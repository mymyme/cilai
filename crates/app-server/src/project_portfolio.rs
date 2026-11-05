use contexthub_application::{
    ProjectPortfolioItemRecord, ProjectPortfolioPhaseRecord, UnitOfWork, WorkspaceRecord,
    WorkspaceRepository,
};
use contexthub_domain::WorkspaceId;
use contexthub_persistence_sqlite::{PersistenceError, PersistenceErrorKind, SqlitePersistence};
use contexthub_protocol::{
    ProjectPortfolioItem, ProjectPortfolioListParams, ProjectPortfolioListResult,
    ProjectPortfolioPhaseItem, WorkspaceSummary,
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
    persistence.transact(|repositories| {
        let records = contexthub_application::get_project_portfolio(
            repositories,
            &contexthub_application::ProjectPortfolioQuery { workspace_ids },
        )?;
        let mut seen = std::collections::HashSet::new();
        let ids = records
            .iter()
            .flat_map(|record| {
                record
                    .workspace_ids
                    .iter()
                    .chain(record.workspace_id.iter())
            })
            .filter(|id| seen.insert(id.as_str()));
        let workspaces = registered_workspaces(repositories, ids)?;
        Ok(ProjectPortfolioListResult {
            items: records.into_iter().map(portfolio_item).collect(),
            workspaces,
        })
    })
}

pub(crate) fn registered_workspaces<'a>(
    repositories: &mut impl WorkspaceRepository<Error = PersistenceError>,
    ids: impl Iterator<Item = &'a WorkspaceId>,
) -> Result<Vec<WorkspaceSummary>, PersistenceError> {
    ids.map(|id| {
        let workspace = repositories.workspace(id)?.ok_or_else(|| {
            PersistenceError::with_reason(
                PersistenceErrorKind::Database,
                "project_workspace_registration_missing",
                "project references a missing workspace registration",
            )
        })?;
        let binding = repositories.platform_binding(id)?;
        Ok(workspace_summary(workspace, binding))
    })
    .collect()
}

pub(crate) fn workspace_summary(
    workspace: WorkspaceRecord,
    binding: Option<contexthub_application::WorkspacePlatformBindingRecord>,
) -> WorkspaceSummary {
    WorkspaceSummary {
        workspace_id: workspace.workspace_id.as_str().to_owned(),
        workspace_kind: workspace.workspace_kind.as_str().to_owned(),
        display_name: workspace.display_name,
        local_root: workspace.local_root,
        local_status: workspace.local_status.as_str().to_owned(),
        state_revision: workspace.state_revision,
        platform_bound: binding
            .as_ref()
            .is_some_and(|value| value.binding_state.as_str() == "bound"),
        platform_revision: binding
            .as_ref()
            .and_then(|value| value.platform_snapshot_revision.clone()),
        team_space_id: binding
            .as_ref()
            .and_then(|value| value.team_space_id.clone()),
        managed_root: binding
            .as_ref()
            .and_then(|value| value.managed_root.clone()),
        device_id: binding.as_ref().and_then(|value| value.device_id.clone()),
        baseline_revision: binding
            .as_ref()
            .and_then(|value| value.baseline_revision.clone()),
        change_cursor: binding
            .as_ref()
            .and_then(|value| value.change_cursor.clone()),
        permission_role: binding
            .as_ref()
            .and_then(|value| value.permission_role.clone()),
        permission_revision: binding.and_then(|value| value.permission_revision),
    }
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
