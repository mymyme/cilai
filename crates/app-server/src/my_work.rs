use contexthub_application::{MyWorkItemRecord, MyWorkQuery, UnitOfWork};
use contexthub_persistence_sqlite::{PersistenceError, PersistenceErrorKind, SqlitePersistence};
use contexthub_protocol::{MyWorkGetParams, MyWorkGetResult, MyWorkItem};

/// 读取跨项目个人工作项投影。
///
/// # Errors
/// 筛选、条数上限或投影数据无效时返回错误。
pub fn get_my_work(
    persistence: &mut SqlitePersistence,
    params: &MyWorkGetParams,
) -> Result<MyWorkGetResult, PersistenceError> {
    if !(1..=200).contains(&params.limit) {
        return Err(configuration_error("my_work_limit_invalid"));
    }
    if params
        .project_id
        .as_ref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err(configuration_error("my_work_project_invalid"));
    }
    if params
        .focus_group
        .as_deref()
        .is_some_and(|value| !matches!(value, "todo" | "in_progress" | "review" | "decision"))
    {
        return Err(configuration_error("my_work_focus_group_invalid"));
    }
    let query = MyWorkQuery {
        project_id: params.project_id.clone(),
        focus_group: params.focus_group.clone(),
        limit: params.limit,
    };
    persistence
        .transact(|repositories| contexthub_application::get_my_work(repositories, &query))
        .map(|result| MyWorkGetResult {
            items: result.items.into_iter().map(my_work_item).collect(),
            total: result.total,
        })
}

fn my_work_item(record: MyWorkItemRecord) -> MyWorkItem {
    MyWorkItem {
        blocking_run_id: record.blocking_run_id,
        blocking_reason: record.blocking_reason,
        work_item_id: record.work_item_id,
        source_type: record.source_type,
        source_id: record.source_id,
        project_id: record.project_id,
        project_name: record.project_name,
        primary_workspace_id: record.primary_workspace_id,
        phase_id: record.phase_id,
        phase_name: record.phase_name,
        task_id: record.task_id,
        deliverable_id: record.deliverable_id,
        deliverable_title: record.deliverable_title,
        attention_case_id: record.attention_case_id,
        title: record.title,
        action_type: record.action_type,
        focus_group: record.focus_group,
        responsibility_type: record.responsibility_type,
        responsibility_label: record.responsibility_label,
        priority: record.priority,
        due_at: record.due_at,
        source_revision: record.source_revision,
        updated_at: record.updated_at,
    }
}

fn configuration_error(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Configuration, reason, reason)
}
