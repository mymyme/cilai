use super::{PersistenceError, SqliteRepositoryTransaction, configuration_error, database_error};
use contexthub_application::{GlobalSearchQuery, GlobalSearchRecord, GlobalSearchRepository};
use rusqlite::params;

impl GlobalSearchRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn global_search(
        &mut self,
        query: &GlobalSearchQuery,
    ) -> Result<Vec<GlobalSearchRecord>, Self::Error> {
        let pattern = format!("%{}%", query.query.trim());
        let limit = i64::try_from(query.limit)
            .map_err(|_| configuration_error("global search limit is too large"))?;
        let mut statement = self
            .transaction
            .prepare(SEARCH_SQL)
            .map_err(database_error("prepare global search"))?;
        statement
            .query_map(
                params![
                    pattern,
                    limit,
                    query.after.as_ref().map(|key| key.0),
                    query.after.as_ref().map(|key| &key.1),
                    query.after.as_ref().map(|key| &key.2)
                ],
                |row| {
                    Ok(GlobalSearchRecord {
                        object_type: row.get(0)?,
                        object_id: row.get(1)?,
                        title: row.get(2)?,
                        subtitle: row.get(3)?,
                        project_id: row.get(4)?,
                        workspace_id: row.get(5)?,
                        phase_id: row.get(6)?,
                        deliverable_id: row.get(7)?,
                        local_file_id: row.get(8)?,
                        relative_path: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .map_err(database_error("query global search"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read global search"))
    }
}

const SEARCH_SQL: &str = "
WITH search_results AS (
    SELECT 'project' object_type, p.project_id object_id, p.display_name title, p.goal subtitle,
           p.project_id, COALESCE(pw.workspace_id,'') workspace_id, NULL phase_id, NULL deliverable_id,
           NULL local_file_id, NULL relative_path, p.updated_at
    FROM projects p LEFT JOIN project_workspaces pw ON pw.project_id=p.project_id AND pw.role='primary'
    WHERE p.display_name LIKE ?1 OR p.goal LIKE ?1
    UNION ALL
    SELECT 'phase', ph.phase_id, ph.name, ph.goal, ph.project_id, COALESCE(pw.workspace_id,''),
           ph.phase_id, ph.required_deliverable_id, NULL, NULL, ph.updated_at
    FROM project_phases ph LEFT JOIN project_workspaces pw ON pw.project_id=ph.project_id AND pw.role='primary'
    WHERE ph.name LIKE ?1 OR ph.goal LIKE ?1 OR ph.acceptance_criteria LIKE ?1
    UNION ALL
    SELECT 'task', t.business_task_id, t.title, t.objective, t.project_id, COALESCE(pw.workspace_id,''),
           t.phase_id, ph.required_deliverable_id, NULL, NULL, t.updated_at
    FROM business_tasks t
    JOIN project_phases ph ON ph.phase_id=t.phase_id
    LEFT JOIN project_workspaces pw ON pw.project_id=t.project_id AND pw.role='primary'
    WHERE t.title LIKE ?1 OR t.objective LIKE ?1 OR t.expected_output LIKE ?1
    UNION ALL
    SELECT 'deliverable', d.deliverable_id, d.title, d.acceptance_criteria, d.project_id,
           COALESCE(pw.workspace_id,''), NULL, d.deliverable_id, NULL, NULL, d.updated_at
    FROM deliverables d LEFT JOIN project_workspaces pw ON pw.project_id=d.project_id AND pw.role='primary'
    WHERE d.title LIKE ?1 OR d.acceptance_criteria LIKE ?1 OR d.owner_label LIKE ?1
    UNION ALL
    SELECT 'decision', a.attention_case_id, a.title, a.detail, a.project_id, COALESCE(pw.workspace_id,''),
           a.phase_id, a.deliverable_id, NULL, NULL, a.updated_at
    FROM attention_cases a LEFT JOIN project_workspaces pw ON pw.project_id=a.project_id AND pw.role='primary'
    WHERE a.case_type='decision_request' AND (a.title LIKE ?1 OR a.detail LIKE ?1 OR a.decision_options LIKE ?1)
    UNION ALL
    SELECT 'file', f.local_file_id, f.relative_path, w.display_name, NULL, f.workspace_id,
           NULL, NULL, f.local_file_id, f.relative_path, f.updated_at
    FROM local_files f JOIN workspaces w ON w.workspace_id=f.workspace_id
    WHERE f.relative_path LIKE ?1 AND f.materialization_state<>'missing'
      AND f.relative_path NOT GLOB '.*'
      AND f.relative_path NOT GLOB '*/.*'
)
SELECT object_type,object_id,title,subtitle,project_id,workspace_id,phase_id,deliverable_id,
       local_file_id,relative_path,updated_at
FROM search_results
WHERE ?3 IS NULL OR updated_at < ?3 OR (updated_at = ?3 AND (object_type > ?4 OR (object_type = ?4 AND object_id > ?5)))
ORDER BY updated_at DESC, object_type, object_id
LIMIT ?2";
