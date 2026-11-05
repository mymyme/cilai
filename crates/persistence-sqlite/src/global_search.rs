use super::{
    PersistenceError, SqlitePersistence, SqliteRepositoryTransaction, configuration_error,
    database_error,
};
use contexthub_application::{GlobalSearchQuery, GlobalSearchRecord, GlobalSearchRepository};
use contexthub_application::{WorkspaceRecord, WorkspaceRepository};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params};

/// Local App Server 内部的独立只读检索连接；不开放业务写事务。
pub struct GlobalSearchReader {
    connection: Connection,
}

impl SqlitePersistence {
    /// 为锁外检索创建当前个人资料的只读连接。
    ///
    /// # Errors
    /// 数据库不可读或只读约束设置失败时返回错误。
    pub fn global_search_reader(&self) -> Result<GlobalSearchReader, PersistenceError> {
        let connection =
            Connection::open_with_flags(&self.database_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(database_error("open read-only global search"))?;
        connection
            .busy_timeout(super::BUSY_TIMEOUT)
            .map_err(database_error("configure search timeout"))?;
        connection
            .execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
            .map_err(database_error("configure read-only global search"))?;
        Ok(GlobalSearchReader { connection })
    }
}

impl GlobalSearchReader {
    /// 读取当前个人资料的工作区归属。
    ///
    /// # Errors
    /// 只读查询失败时返回错误。
    pub fn workspaces(&mut self) -> Result<Vec<WorkspaceRecord>, PersistenceError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error("begin read-only workspace query"))?;
        SqliteRepositoryTransaction { transaction }.workspaces()
    }

    /// 在短只读事务内获取候选；文件验证在事务释放后执行。
    ///
    /// # Errors
    /// 只读查询失败时返回错误。
    pub fn search(
        &mut self,
        query: &GlobalSearchQuery,
    ) -> Result<Vec<GlobalSearchRecord>, PersistenceError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(database_error("begin read-only global search"))?;
        SqliteRepositoryTransaction { transaction }.global_search(query)
    }
}

impl GlobalSearchRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn global_search(
        &mut self,
        query: &GlobalSearchQuery,
    ) -> Result<Vec<GlobalSearchRecord>, Self::Error> {
        let value = query.query.trim();
        let pattern = format!(
            "%{}%",
            value
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
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
                    query.after.as_ref().map(|key| &key.2),
                    format!("%{value}%"),
                    value,
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
                        body_match: row.get(11)?,
                        expected_size_bytes: row
                            .get::<_, Option<i64>>(12)?
                            .map(|value| {
                                u64::try_from(value).map_err(|_| {
                                    rusqlite::Error::IntegralValueOutOfRange(12, value)
                                })
                            })
                            .transpose()?,
                        expected_checksum: row.get(13)?,
                        expected_file_identity: row.get(14)?,
                    })
                },
            )
            .map_err(database_error("query global search"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read global search"))
    }
}

const SEARCH_SQL: &str = "
WITH text_matches AS MATERIALIZED (
    SELECT d.checksum_value,
           substr(ft.content, max(1, instr(lower(ft.content), lower(?7)) - 40), 160) excerpt
    FROM file_search_text ft JOIN file_search_documents d ON d.document_id=ft.rowid
    WHERE ft.content LIKE ?6 AND instr(lower(ft.content), lower(?7)) > 0
), search_results AS (
    SELECT 'project' object_type, p.project_id object_id, p.display_name title, p.goal subtitle,
           p.project_id, COALESCE(pw.workspace_id,'') workspace_id, NULL phase_id, NULL deliverable_id,
           NULL local_file_id, NULL relative_path, p.updated_at, 0 body_match,
           NULL expected_size_bytes, NULL expected_checksum, NULL expected_file_identity
    FROM projects p LEFT JOIN project_workspaces pw ON pw.project_id=p.project_id AND pw.role='primary'
    WHERE p.display_name LIKE ?1 ESCAPE '\\' OR p.goal LIKE ?1 ESCAPE '\\'
    UNION ALL
    SELECT 'phase', ph.phase_id, ph.name, ph.goal, ph.project_id, COALESCE(pw.workspace_id,''),
           ph.phase_id, ph.required_deliverable_id, NULL, NULL, ph.updated_at, 0, NULL, NULL, NULL
    FROM project_phases ph LEFT JOIN project_workspaces pw ON pw.project_id=ph.project_id AND pw.role='primary'
    WHERE ph.name LIKE ?1 ESCAPE '\\' OR ph.goal LIKE ?1 ESCAPE '\\' OR ph.acceptance_criteria LIKE ?1 ESCAPE '\\'
    UNION ALL
    SELECT 'task', t.business_task_id, t.title, t.objective, t.project_id, COALESCE(pw.workspace_id,''),
           t.phase_id, ph.required_deliverable_id, NULL, NULL, t.updated_at, 0, NULL, NULL, NULL
    FROM business_tasks t
    LEFT JOIN project_phases ph ON ph.phase_id=t.phase_id
    LEFT JOIN project_workspaces pw ON pw.project_id=t.project_id AND pw.role='primary'
    WHERE t.title LIKE ?1 ESCAPE '\\' OR t.objective LIKE ?1 ESCAPE '\\' OR t.expected_output LIKE ?1 ESCAPE '\\'
    UNION ALL
    SELECT 'deliverable', d.deliverable_id, d.title, d.acceptance_criteria, d.project_id,
           COALESCE(pw.workspace_id,''), NULL, d.deliverable_id, NULL, NULL, d.updated_at, 0, NULL, NULL, NULL
    FROM deliverables d LEFT JOIN project_workspaces pw ON pw.project_id=d.project_id AND pw.role='primary'
    WHERE d.title LIKE ?1 ESCAPE '\\' OR d.acceptance_criteria LIKE ?1 ESCAPE '\\' OR d.owner_label LIKE ?1 ESCAPE '\\'
    UNION ALL
    SELECT 'decision', a.attention_case_id, a.title, a.detail, a.project_id, COALESCE(pw.workspace_id,''),
           a.phase_id, a.deliverable_id, NULL, NULL, a.updated_at, 0, NULL, NULL, NULL
    FROM attention_cases a LEFT JOIN project_workspaces pw ON pw.project_id=a.project_id AND pw.role='primary'
    WHERE a.case_type='decision_request' AND (a.title LIKE ?1 ESCAPE '\\' OR a.detail LIKE ?1 ESCAPE '\\' OR a.decision_options LIKE ?1 ESCAPE '\\')
    UNION ALL
    SELECT 'file', f.local_file_id, f.relative_path,
           CASE WHEN f.relative_path LIKE ?1 ESCAPE '\\' THEN w.display_name ELSE w.display_name || ' · ' || text_matches.excerpt END,
           NULL, f.workspace_id,
           NULL, NULL, f.local_file_id, f.relative_path, f.updated_at,
           NOT (f.relative_path LIKE ?1 ESCAPE '\\'), f.size_bytes, f.local_checksum_value, f.file_identity
    FROM local_files f JOIN workspaces w ON w.workspace_id=f.workspace_id
    LEFT JOIN text_matches ON text_matches.checksum_value=f.local_checksum_value AND f.local_checksum_algorithm='sha256'
    WHERE (f.relative_path LIKE ?1 ESCAPE '\\' OR text_matches.checksum_value IS NOT NULL) AND f.materialization_state<>'missing'
      AND f.relative_path NOT GLOB '.*'
      AND f.relative_path NOT GLOB '*/.*'
)
SELECT object_type,object_id,title,subtitle,project_id,workspace_id,phase_id,deliverable_id,
       local_file_id,relative_path,updated_at,body_match,expected_size_bytes,expected_checksum,expected_file_identity
FROM search_results
WHERE ?3 IS NULL OR updated_at < ?3 OR (updated_at = ?3 AND (object_type > ?4 OR (object_type = ?4 AND object_id > ?5)))
ORDER BY updated_at DESC, object_type, object_id
LIMIT ?2";

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_application::UnitOfWork;

    #[test]
    fn tasks_with_and_without_phases_keep_their_identity_across_search_pages() {
        let root = std::env::temp_dir().join(format!("search-tasks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut db = crate::SqlitePersistence::open(&root.join("contexthub.db"), "0.1.0").unwrap();
        db.connection.execute_batch(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project','项目','','active',0,'project','project',1,1);
             INSERT INTO deliverables(deliverable_id,project_id,title,deliverable_type,owner_label,acceptance_criteria,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('deliverable','project','交付物','document','负责人','标准','draft',0,'deliverable','deliverable',1,1);
             INSERT INTO phase_plans(plan_id,project_id,version_no,status,summary,state_revision,client_operation_id,request_fingerprint,created_at)
             VALUES ('plan','project',1,'active','计划',0,'plan','plan',1);
             INSERT INTO project_phases(phase_id,plan_id,project_id,name,goal,acceptance_criteria,required_deliverable_id,sort_order,is_main,status,state_revision,created_at,updated_at)
             VALUES ('phase','plan','project','阶段','目标','标准','deliverable',0,1,'in_progress',0,1,1);",
        ).unwrap();
        for index in 0..7 {
            let phase_id = (index % 2 == 0).then_some("phase");
            db.connection.execute(
                "INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
                 VALUES (?1,'project',?2,?3,'目标','','产出','标准','collaboration','AI','medium','todo',0,?1,?1,20,20)",
                params![format!("task-{index}"), phase_id, format!("搜索待办 {index}")],
            ).unwrap();
        }
        let mut after = None;
        let mut records = Vec::new();
        for _ in 0..5 {
            let page = db
                .transact(|repository| {
                    repository.global_search(&GlobalSearchQuery {
                        query: "搜索待办".to_owned(),
                        limit: 2,
                        after: after.clone(),
                    })
                })
                .unwrap();
            if page.is_empty() {
                break;
            }
            let last = page.last().unwrap();
            after = Some((
                last.updated_at,
                last.object_type.clone(),
                last.object_id.clone(),
            ));
            records.extend(page);
        }
        assert_eq!(records.len(), 7);
        for (index, record) in records.iter().enumerate() {
            assert_eq!(record.object_id, format!("task-{index}"));
            assert_eq!(record.object_type, "task");
            assert_eq!(record.project_id.as_deref(), Some("project"));
            assert!(record.workspace_id.is_empty());
            assert_eq!(
                record.phase_id.as_deref(),
                (index % 2 == 0).then_some("phase")
            );
            assert_eq!(
                record.deliverable_id.as_deref(),
                (index % 2 == 0).then_some("deliverable")
            );
        }
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}
