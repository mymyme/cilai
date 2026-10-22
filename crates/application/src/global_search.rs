#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalSearchRecord {
    pub object_type: String,
    pub object_id: String,
    pub title: String,
    pub subtitle: String,
    pub project_id: Option<String>,
    pub workspace_id: String,
    pub phase_id: Option<String>,
    pub deliverable_id: Option<String>,
    pub local_file_id: Option<String>,
    pub relative_path: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalSearchQuery {
    pub query: String,
    pub limit: usize,
    pub after: Option<(i64, String, String)>,
}

pub trait GlobalSearchRepository {
    type Error;

    /// # Errors
    /// 搜索索引或来源事实不可读时返回错误。
    fn global_search(
        &mut self,
        query: &GlobalSearchQuery,
    ) -> Result<Vec<GlobalSearchRecord>, Self::Error>;
}

/// 在本地可访问的唯一来源事实中执行统一搜索。
///
/// # Errors
/// 来源事实不可读时返回仓储错误。
pub fn global_search<R: GlobalSearchRepository>(
    repositories: &mut R,
    query: &GlobalSearchQuery,
) -> Result<Vec<GlobalSearchRecord>, R::Error> {
    repositories.global_search(query)
}
