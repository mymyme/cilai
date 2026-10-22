use contexthub_application::{GlobalSearchQuery, UnitOfWork, WorkspaceRepository, global_search};
use contexthub_persistence_sqlite::{PersistenceError, SqlitePersistence};
use contexthub_protocol::{GlobalSearchItem, GlobalSearchParams, GlobalSearchResult};

/// 查询当前本地用户可访问的项目、阶段、任务、交付物、决策和文件来源事实。
///
/// # Errors
/// 查询参数或本地事实读取失败时返回稳定错误。
pub fn query(
    persistence: &mut SqlitePersistence,
    params: &GlobalSearchParams,
) -> Result<GlobalSearchResult, PersistenceError> {
    Ok(query_page(persistence, params, None)?.0)
}

pub(super) type SearchPosition = (i64, String, String);

pub(super) fn query_page(
    persistence: &mut SqlitePersistence,
    params: &GlobalSearchParams,
    after: Option<SearchPosition>,
) -> Result<(GlobalSearchResult, Option<SearchPosition>), PersistenceError> {
    let value = params.query.trim();
    if value.chars().count() < 2 || !(1..=100).contains(&params.limit) {
        return Err(PersistenceError::with_reason(
            contexthub_persistence_sqlite::PersistenceErrorKind::Configuration,
            "global_search_params_invalid",
            "global_search_params_invalid",
        ));
    }
    let records = persistence.transact(|repositories| {
        global_search(
            repositories,
            &GlobalSearchQuery {
                query: value.to_owned(),
                limit: params.limit as usize + 1,
                after,
            },
        )
    })?;
    let has_more = records.len() > params.limit as usize;
    let records = records
        .into_iter()
        .take(params.limit as usize)
        .collect::<Vec<_>>();
    let next = if has_more {
        records.last().map(|record| {
            (
                record.updated_at,
                record.object_type.clone(),
                record.object_id.clone(),
            )
        })
    } else {
        None
    };
    let workspaces = persistence.transact(|repositories| repositories.workspaces())?;
    let items = records
        .into_iter()
        .filter(|record| {
            if record.object_type != "file" {
                return true;
            }
            let Some(workspace) = workspaces
                .iter()
                .find(|workspace| workspace.workspace_id.as_str() == record.workspace_id)
            else {
                return false;
            };
            if workspace.workspace_kind.as_str() != "local" {
                return true;
            }
            let Some(path) = &record.relative_path else {
                return false;
            };
            match std::fs::metadata(std::path::Path::new(&workspace.local_root).join(path)) {
                Ok(_) => true,
                Err(error) => !matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ),
            }
        })
        .map(|record| GlobalSearchItem {
            object_type: record.object_type,
            object_id: record.object_id,
            title: record.title,
            subtitle: record.subtitle,
            project_id: record.project_id,
            workspace_id: record.workspace_id,
            phase_id: record.phase_id,
            deliverable_id: record.deliverable_id,
            local_file_id: record.local_file_id,
            relative_path: record.relative_path,
            updated_at: record.updated_at,
        })
        .collect::<Vec<_>>();
    Ok((
        GlobalSearchResult {
            total: items.len() as u64,
            items,
        },
        next,
    ))
}
