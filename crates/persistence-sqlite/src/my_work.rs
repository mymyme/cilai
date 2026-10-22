use super::{PersistenceError, SqliteRepositoryTransaction, database_error};
use contexthub_application::{MyWorkItemRecord, MyWorkQuery, MyWorkQueryResult, MyWorkRepository};
use rusqlite::params;

const VISIBLE_WORK_ITEMS: &str = "WITH conversation_state AS (
    SELECT conversation.conversation_id AS conversation_id,
           COALESCE(json_extract(conversation.conversation_json,'$.archived'),0) AS archived
    FROM ai_business_conversations conversation
), ai_created_tasks AS (
    SELECT DISTINCT business_task_id
    FROM conversation_work_classifications
    WHERE task_action='create' AND business_task_id IS NOT NULL
), archived_runs AS (
    SELECT run.run_id AS run_id
    FROM ai_business_runs run
    JOIN conversation_state conversation ON conversation.conversation_id=run.conversation_id
    WHERE conversation.archived=1
), latest_visible_contributions AS (
    SELECT classification.business_task_id,classification.conversation_status,classification.stage
    FROM conversation_work_classifications classification
    JOIN conversation_state conversation ON conversation.conversation_id=classification.conversation_id
    WHERE conversation.archived=0 AND classification.business_task_id IS NOT NULL
      AND NOT EXISTS (
          SELECT 1 FROM conversation_work_classifications newer
          WHERE newer.business_task_id=classification.business_task_id
            AND newer.conversation_id=classification.conversation_id
            AND (newer.created_at>classification.created_at OR
                 (newer.created_at=classification.created_at AND newer.classification_id>classification.classification_id))
      )
), visible_ai_task_focus AS (
    SELECT business_task_id,
           CASE
             WHEN SUM(CASE WHEN conversation_status='active' THEN 1 ELSE 0 END)=0 THEN NULL
             WHEN SUM(CASE WHEN conversation_status='active' AND stage='decision' THEN 1 ELSE 0 END)>0 THEN 'decision'
             WHEN SUM(CASE WHEN conversation_status='active' AND stage='review' THEN 1 ELSE 0 END)>0 THEN 'review'
             WHEN SUM(CASE WHEN conversation_status='active' AND stage='in_progress' THEN 1 ELSE 0 END)>0 THEN 'in_progress'
             ELSE 'todo'
           END AS focus_group
    FROM latest_visible_contributions GROUP BY business_task_id
), visible_work_items AS (
    SELECT item.*,
           CASE WHEN origin.business_task_id IS NOT NULL THEN ai_focus.focus_group ELSE item.focus_group END AS visible_focus_group
    FROM my_work_items item
    LEFT JOIN ai_created_tasks origin ON origin.business_task_id=item.task_id AND item.source_type='business_task'
    LEFT JOIN visible_ai_task_focus ai_focus ON ai_focus.business_task_id=item.task_id
    WHERE origin.business_task_id IS NULL OR ai_focus.focus_group IS NOT NULL
), current_work_items AS (
    SELECT item.* FROM visible_work_items item
    WHERE item.source_type<>'agent_run_incident'
), enriched_work_items AS (
    SELECT item.*,
           (SELECT agent_run_id FROM attention_cases attention
            WHERE attention.project_id=item.project_id AND attention.case_type='agent_run_incident'
              AND attention.status<>'resolved'
              AND attention.client_operation_id NOT LIKE 'desktop-classification-incident-%'
              AND ((item.source_type='business_task' AND attention.task_id=item.task_id)
                OR (item.source_type='agent_run_incident' AND attention.attention_case_id=item.attention_case_id))
              AND NOT EXISTS (SELECT 1 FROM archived_runs archived WHERE archived.run_id=attention.agent_run_id)
            ORDER BY attention.created_at DESC,attention.attention_case_id LIMIT 1) AS blocking_run_id,
           (SELECT detail FROM attention_cases attention
            WHERE attention.project_id=item.project_id AND attention.case_type='agent_run_incident'
              AND attention.status<>'resolved'
              AND attention.client_operation_id NOT LIKE 'desktop-classification-incident-%'
              AND ((item.source_type='business_task' AND attention.task_id=item.task_id)
                OR (item.source_type='agent_run_incident' AND attention.attention_case_id=item.attention_case_id))
              AND NOT EXISTS (SELECT 1 FROM archived_runs archived WHERE archived.run_id=attention.agent_run_id)
            ORDER BY attention.created_at DESC,attention.attention_case_id LIMIT 1) AS blocking_reason
    FROM current_work_items item
)";

impl MyWorkRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn my_work_items(&mut self, query: &MyWorkQuery) -> Result<MyWorkQueryResult, Self::Error> {
        let project_id = query.project_id.as_deref();
        let focus_group = query.focus_group.as_deref();
        let total = self
            .transaction
            .query_row(
                &format!("{VISIBLE_WORK_ITEMS}
                SELECT COUNT(*) FROM enriched_work_items
                WHERE (?1 IS NULL OR project_id=?1)
                  AND (?2 IS NULL OR CASE WHEN blocking_run_id IS NOT NULL THEN 'todo' ELSE visible_focus_group END=?2)"),
                params![project_id, focus_group],
                |row| row.get::<_, i64>(0),
            )
            .map_err(database_error("count my work items"))?;
        let total = u64::try_from(total).map_err(|_| {
            PersistenceError::with_reason(
                super::PersistenceErrorKind::Database,
                "my_work_total_invalid",
                "my_work_total_invalid",
            )
        })?;
        let mut statement = self
            .transaction
            .prepare(
                &format!("{VISIBLE_WORK_ITEMS}
                 SELECT work_item_id,source_type,source_id,project_id,project_name,primary_workspace_id,phase_id,phase_name,task_id,deliverable_id,deliverable_title,attention_case_id,title,action_type,
                    CASE WHEN blocking_run_id IS NOT NULL THEN 'todo' ELSE visible_focus_group END,
                    responsibility_type,responsibility_label,priority,due_at,source_revision,updated_at,blocking_run_id,blocking_reason
                 FROM enriched_work_items
                 WHERE (?1 IS NULL OR project_id=?1)
                   AND (?2 IS NULL OR CASE WHEN blocking_run_id IS NOT NULL THEN 'todo' ELSE visible_focus_group END=?2)
                 ORDER BY
                    CASE WHEN blocking_run_id IS NOT NULL THEN 0 ELSE CASE visible_focus_group WHEN 'todo' THEN 0 WHEN 'in_progress' THEN 1 WHEN 'review' THEN 2 WHEN 'decision' THEN 3 END END,
                    CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 END,
                    due_at IS NULL, due_at, updated_at, work_item_id
                 LIMIT ?3"),
            )
            .map_err(database_error("prepare my work items"))?;
        let items = statement
            .query_map(
                params![project_id, focus_group, i64::from(query.limit)],
                |row| {
                    Ok(MyWorkItemRecord {
                        blocking_run_id: row.get(21)?,
                        blocking_reason: row.get(22)?,
                        work_item_id: row.get(0)?,
                        source_type: row.get(1)?,
                        source_id: row.get(2)?,
                        project_id: row.get(3)?,
                        project_name: row.get(4)?,
                        primary_workspace_id: row.get(5)?,
                        phase_id: row.get(6)?,
                        phase_name: row.get(7)?,
                        task_id: row.get(8)?,
                        deliverable_id: row.get(9)?,
                        deliverable_title: row.get(10)?,
                        attention_case_id: row.get(11)?,
                        title: row.get(12)?,
                        action_type: row.get(13)?,
                        focus_group: row.get(14)?,
                        responsibility_type: row.get(15)?,
                        responsibility_label: row.get(16)?,
                        priority: row.get(17)?,
                        due_at: row.get(18)?,
                        source_revision: row.get(19)?,
                        updated_at: row.get(20)?,
                    })
                },
            )
            .map_err(database_error("query my work items"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read my work items"))?;
        Ok(MyWorkQueryResult { items, total })
    }
}
