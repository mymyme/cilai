use contexthub_domain::WorkspaceId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectPortfolioPhaseRecord {
    pub phase_id: String,
    pub name: String,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectPortfolioItemRecord {
    pub project_id: String,
    pub display_name: String,
    pub goal: String,
    pub status: String,
    pub workspace_id: Option<WorkspaceId>,
    pub workspace_display_name: Option<String>,
    pub workspace_ids: Vec<WorkspaceId>,
    pub state_revision: i64,
    pub updated_at: i64,
    pub has_plan: bool,
    pub active_phases: Vec<ProjectPortfolioPhaseRecord>,
    pub next_phase_name: Option<String>,
    pub task_total: i64,
    pub task_completed: i64,
    pub deliverable_total: i64,
    pub deliverable_accepted: i64,
    pub blocker_count: i64,
    pub decision_count: i64,
    pub incident_count: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectPortfolioQuery {
    pub workspace_ids: Vec<WorkspaceId>,
}

pub trait ProjectPortfolioRepository {
    type Error;

    /// 按工作区批量读取跨项目总览投影，一次事务内完成，避免调用方逐项目往返。
    ///
    /// # Errors
    ///
    /// 数据无法读取或领域状态无法转换时返回错误。
    fn portfolio_items(
        &mut self,
        query: &ProjectPortfolioQuery,
    ) -> Result<Vec<ProjectPortfolioItemRecord>, Self::Error>;
}

/// 读取跨项目总览投影，不拥有或改写来源业务状态。
///
/// # Errors
/// 投影仓储读取失败时返回仓储错误。
pub fn get_project_portfolio<R>(
    repository: &mut R,
    query: &ProjectPortfolioQuery,
) -> Result<Vec<ProjectPortfolioItemRecord>, R::Error>
where
    R: ProjectPortfolioRepository,
{
    repository.portfolio_items(query)
}
