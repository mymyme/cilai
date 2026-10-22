#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MyWorkItemRecord {
    pub work_item_id: String,
    pub source_type: String,
    pub source_id: String,
    pub project_id: String,
    pub project_name: String,
    pub primary_workspace_id: Option<String>,
    pub phase_id: Option<String>,
    pub phase_name: Option<String>,
    pub task_id: Option<String>,
    pub deliverable_id: Option<String>,
    pub deliverable_title: Option<String>,
    pub attention_case_id: Option<String>,
    pub title: String,
    pub action_type: String,
    pub focus_group: String,
    pub responsibility_type: String,
    pub responsibility_label: String,
    pub priority: String,
    pub due_at: Option<i64>,
    pub source_revision: i64,
    pub updated_at: i64,
    pub blocking_run_id: Option<String>,
    pub blocking_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MyWorkQuery {
    pub project_id: Option<String>,
    pub focus_group: Option<String>,
    pub limit: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MyWorkQueryResult {
    pub items: Vec<MyWorkItemRecord>,
    pub total: u64,
}

pub trait MyWorkRepository {
    type Error;

    /// # Errors
    /// 投影不可读、筛选无效或数据库状态不一致时返回错误。
    fn my_work_items(&mut self, query: &MyWorkQuery) -> Result<MyWorkQueryResult, Self::Error>;
}

/// 读取跨项目个人工作项投影，不拥有或改写来源业务状态。
///
/// # Errors
/// 投影仓储读取失败时返回仓储错误。
pub fn get_my_work<R>(
    repository: &mut R,
    query: &MyWorkQuery,
) -> Result<MyWorkQueryResult, R::Error>
where
    R: MyWorkRepository,
{
    repository.my_work_items(query)
}
