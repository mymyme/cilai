use super::*;
use crate::test_historical_accounts::insert_historical_account;
use crate::test_historical_platform::{
    HistoricalPlatformSnapshot, insert_historical_platform_binding,
    remove_historical_platform_binding, seed_historical_platform_snapshot,
};
use contexthub_application::{
    ActivatePhasePlanCommand, AttentionCaseRepository, CreateAttentionCaseCommand,
    CreateBusinessTaskCommand, CreatePhasePlanCommand, DeliverableCarrierInput, GlobalSearchQuery,
    LocalFileObservation, MoveBusinessTaskCommand, MyWorkItemRecord, MyWorkQuery, PhaseDraftInput,
    ProjectPhaseRecord, ProjectProgressRepository, TransitionAttentionCaseCommand,
    TransitionBusinessTaskCommand, TransitionProjectPhaseCommand, export_workspace_projection,
};
use contexthub_domain::{
    AccountId, ActiveConfirmationFact, AttentionCaseStatus, AttentionCaseType,
    BaselinePathRelation, BusinessTaskStatus, FilePermission, FilePermissions, FileStateInput,
    LocalNodeKind, LocalScanState, MaterializationState, MetadataValidity, PhasePlanStatus,
    PlatformConnection, PlatformDocumentState, ProjectPhaseStatus, SubmissionState, WorkspaceKind,
};
use contexthub_workspace_fs::WorkspaceProjectionWriter;

fn apply_fixture_migration(connection: &Connection, migration: &Migration) {
    if migration.rebuilds_foreign_key_parent {
        connection
            .pragma_update(None, "foreign_keys", false)
            .unwrap();
    }
    connection.execute_batch(migration.sql).unwrap();
    if migration.rebuilds_foreign_key_parent {
        let violations: i64 = connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0);
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
    }
}

fn ai_turn_state(conversation_id: &str, run_id: &str) -> serde_json::Value {
    serde_json::json!({
        "agents": [],
        "conversations": [{"conversationId": conversation_id}],
        "messages": [{"messageId": format!("reply-{run_id}"), "role": "assistant", "content": "候选结果"}],
        "runs": [{"runId": run_id, "status": "completed"}]
    })
}

fn with_ai_state(command: &mut AiTurnFinalizeCommand, state: &serde_json::Value) {
    command.changes = AiStateChanges::full_from_state(state).unwrap();
}

fn ai_create_task_command(
    conversation_id: &str,
    run_id: &str,
    task_id: &str,
    expected_revision: i64,
) -> AiTurnFinalizeCommand {
    let raw = serde_json::json!({
        "action": "apply",
        "taskAction": "create",
        "taskId": null,
        "taskTitle": "自动归类对话",
        "objective": "归类本轮工作",
        "expectedOutput": "任务和阶段投影",
        "completionCriteria": "工作台可见",
        "conversationStatus": "active",
        "stage": "in_progress",
        "outcomeTitle": "已完成自动归类",
        "outcomeSummary": "本轮工作已关联任务并进入进行中",
        "reason": "已开始实质执行",
        "confidence": 0.96
    });
    AiTurnFinalizeCommand {
        ledger_request: None,
        expected_revision,
        changes: AiStateChanges::full_from_state(&ai_turn_state(conversation_id, run_id)).unwrap(),
        conversation_id: conversation_id.to_owned(),
        run_id: run_id.to_owned(),
        project_id: Some("project-ai".to_owned()),
        previous_task_id: None,
        agent_label: "ContextHub Agent".to_owned(),
        new_task_id: Some(task_id.to_owned()),
        classification: AiTurnClassificationRecord {
            action: "apply".to_owned(),
            task_action: "create".to_owned(),
            task_id: None,
            task_title: Some("自动归类对话".to_owned()),
            objective: Some("归类本轮工作".to_owned()),
            expected_output: Some("任务和阶段投影".to_owned()),
            completion_criteria: Some("工作台可见".to_owned()),
            conversation_status: Some("active".to_owned()),
            stage: Some("in_progress".to_owned()),
            outcome_title: Some("已完成自动归类".to_owned()),
            outcome_summary: Some("本轮工作已关联任务并进入进行中".to_owned()),
            reason: "已开始实质执行".to_owned(),
            confidence: 0.96,
            raw,
        },
        occurred_at: 10,
    }
}

/// 删除工作区（`delete_workspace`）与删除项目（`delete_project`）会逐行删除的表。
/// 往这两条删除路径里加新表时，也要把它加进来。
const ROW_DELETED_TABLES: &[&str] = &[
    "attention_case_transitions",
    "attention_cases",
    "business_task_moves",
    "business_task_transitions",
    "business_tasks",
    "checksum_cache",
    "conflicts",
    "deliverable_acceptances",
    "deliverable_file_bindings",
    "deliverable_version_carriers",
    "deliverable_versions",
    "deliverables",
    "document_versions",
    "documents",
    "durable_events",
    "file_confirmation_text_snapshots",
    "file_confirmations",
    "file_states",
    "local_file_observations",
    "local_files",
    "local_scan_snapshots",
    "my_work_items",
    "phase_dependencies",
    "phase_plans",
    "phase_transitions",
    "platform_snapshots",
    "project_phases",
    "project_workspaces",
    "projects",
    "sync_executions",
    "sync_plan_items",
    "sync_plans",
    "task_attempts",
    "tasks",
    "workspace_exports",
    "workspace_platform_bindings",
    "workspaces",
];

#[test]
fn 删除路径上逐行检查的外键都有可用索引() {
    // 删除工作区或项目时，上面这些表会被逐行删除，SQLite 每删一行都按外键查一遍子表。
    // 子表的引用字段没有以它开头的无条件索引时，每次查找都是整表扫描：曾经删一个
    // 一万三千文件的工作区要 54 秒，全程占着服务主锁。
    let fixture = TestDatabase::new("fk-indexes");
    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let mut statement = persistence
        .connection
        .prepare(
            "SELECT m.name, fk.\"from\", fk.\"table\" FROM sqlite_master m, pragma_foreign_key_list(m.name) fk
             WHERE m.type = 'table'
               AND NOT EXISTS (
                 SELECT 1 FROM pragma_index_list(m.name) il, pragma_index_info(il.name) ii
                 WHERE il.partial = 0 AND ii.seqno = 0 AND ii.name = fk.\"from\"
               )
             ORDER BY m.name, fk.\"from\"",
        )
        .unwrap();
    let missing = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .filter(|(_, _, parent)| ROW_DELETED_TABLES.contains(&parent.as_str()))
        .map(|(table, column, parent)| format!("{table}.{column} -> {parent}"))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "foreign keys without a usable index: {missing:?}"
    );
}

#[test]
fn workbench_memory_reset_rejects_late_publication_and_import_receipts_survive_reopen() {
    let fixture = TestDatabase::new("owned-context-and-memory");
    let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let state = db
        .ai_history("memoryGet", &serde_json::Value::Null, 1)
        .unwrap();
    db.ai_history("memoryReset", &serde_json::Value::Null, 2)
        .unwrap();
    assert!(db.ai_history("memorySave", &state, 3).is_err());
    let import = serde_json::json!({"scope":"tenant-home","threadId":"t","turns":[{"id":"old","messages":[{"id":"m","text":"原文"}]}]});
    assert_eq!(
        db.ai_history("contextPrepare", &import, 4).unwrap()["replayed"],
        false
    );
    drop(db);
    let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    assert_eq!(
        db.ai_history("contextPrepare", &import, 5).unwrap()["replayed"],
        true
    );
    db.ai_history("contextCommit", &import, 6).unwrap();
    assert_eq!(
        db.ai_history("contextGet", &import, 7).unwrap()["turns"],
        import["turns"]
    );
    let mut foreign = import.clone();
    foreign["scope"] = serde_json::json!("other-home");
    assert!(db.ai_history("contextGet", &foreign, 8).unwrap().is_null());
    let mut changed = import.clone();
    changed["turns"] = serde_json::json!([]);
    assert!(db.ai_history("contextPrepare", &changed, 9).is_err());
}

#[test]
fn context_import_delete_is_scoped_idempotent_and_persists_after_reopen() {
    let fixture = TestDatabase::new("context-import-delete");
    let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let import = serde_json::json!({"scope":"profile-a","threadId":"thread","turns":[{"id":"handoff","messages":[{"text":"应用转交正文"}]}]});
    let mut other = import.clone();
    other["scope"] = serde_json::json!("profile-b");
    for value in [&import, &other] {
        db.ai_history("contextPrepare", value, 1).unwrap();
        db.ai_history("contextCommit", value, 2).unwrap();
    }
    assert_eq!(
        db.ai_history("contextDelete", &import, 3).unwrap()["deleted"],
        true
    );
    assert_eq!(
        db.ai_history("contextDelete", &import, 4).unwrap()["deleted"],
        false
    );
    assert!(!db.ai_history("contextGet", &other, 5).unwrap().is_null());
    drop(db);
    let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    assert!(db.ai_history("contextGet", &import, 6).unwrap().is_null());
    assert!(!db.ai_history("contextGet", &other, 7).unwrap().is_null());
}

#[test]
fn search_reader_runs_during_writes_and_releases_each_read_transaction() {
    let fixture = TestDatabase::new("search-reader");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let mut reader = persistence.global_search_reader().unwrap();
    let transaction = persistence
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    transaction.execute_batch("CREATE TABLE search_writer_probe(value INTEGER); INSERT INTO search_writer_probe VALUES(1);").unwrap();
    assert!(reader.workspaces().unwrap().is_empty());
    assert!(
        reader
            .search(&GlobalSearchQuery {
                query: "正文".to_owned(),
                limit: 20,
                after: None,
            })
            .unwrap()
            .is_empty()
    );
    transaction.commit().unwrap();
    let value: i64 = persistence
        .connection
        .query_row("SELECT value FROM search_writer_probe", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(value, 1);
    assert!(reader.workspaces().unwrap().is_empty());
}

#[test]
fn new_database_has_frozen_configuration_and_migration_history() {
    let fixture = TestDatabase::new("configuration");
    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    assert_eq!(persistence.database_path(), fixture.path);
    assert_eq!(
        persistence.runtime_configuration().unwrap(),
        SqliteRuntimeConfiguration {
            journal_mode: "wal".to_owned(),
            foreign_keys: true,
            synchronous: 2,
            busy_timeout_ms: 5_000,
            temp_store: 2,
            trusted_schema: false,
        }
    );
    let mut statement = persistence
        .connection
        .prepare(
            "SELECT version, name, checksum, app_version FROM schema_migrations ORDER BY version",
        )
        .unwrap();
    let migrations = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(migrations.len(), MIGRATIONS.len());
    for (index, (version, name, checksum, app_version)) in migrations.iter().enumerate() {
        let expected = &MIGRATIONS[index];
        assert_eq!(*version, expected.version);
        assert_eq!(name, expected.name);
        assert_eq!(checksum, &migration_checksum(expected.sql));
        assert_eq!(app_version, "0.1.0");
    }
    assert!(migrations.iter().all(|migration| migration.3 == "0.1.0"));
    contexthub_platform_adapter::security::verify_restricted_path(&fixture.path, false).unwrap();
}

#[test]
fn capture_v63_upgrade_preserves_prepared_handoff_and_original_owner() {
    let fixture = TestDatabase::new("capture-v63-prepared-handoff");
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    for migration in MIGRATIONS.iter().filter(|m| m.version <= 63) {
        apply_fixture_migration(&connection, migration);
        connection.execute("INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES(?1,?2,?3,1,'previous-client')",params![migration.version,migration.name,migration_checksum(migration.sql)]).unwrap();
    }
    let scope = serde_json::json!({"tenant_id":"tenant","global_user_id":"user","membership_id":"member","source_device_id":"source","receiver_device_id":"receiver"}).to_string();
    let input = serde_json::json!({"prompt":"原已确认需求","agentId":"agent","context":{"workspaceRoot":""},"conversationId":null,"modelOverride":null,"permissionMode":"request"}).to_string();
    connection.execute("INSERT INTO capture_transfers VALUES('transfer','original-owner',?1,'record',1,'digest','{}','received','receipt',1,2)",[&scope]).unwrap();
    connection.execute("INSERT INTO capture_analyses(analysis_id,operation_id,transfer_id,version,source_text,source_sha256,state,source_run_id,result_json,created_at) VALUES('analysis','op','transfer',1,'原已确认需求','digest','completed','analysis-run',?1,3)",[serde_json::json!({"requirements":[],"todos":[],"questions":[]}).to_string()]).unwrap();
    connection.execute("INSERT INTO capture_handoffs VALUES('old','analysis','candidate',1,?1,NULL,NULL,'prepared',4)",[&input]).unwrap();
    drop(connection);
    for _ in 0..2 {
        let db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let handoff: (String, Option<String>, Option<String>, String) = db.connection.query_row("SELECT input_json,conversation_id,run_id,state FROM capture_handoffs WHERE handoff_id='old'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
        assert_eq!(handoff, (input.clone(), None, None, "prepared".into()));
        let owner: (String, String) = db
            .connection
            .query_row(
                "SELECT scope_key,scope_json FROM capture_transfers WHERE transfer_id='transfer'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(owner, ("original-owner".into(), scope.clone()));
        let count: i64 = db.connection.query_row("SELECT COUNT(*) FROM schema_migrations WHERE version<=63 AND app_version='previous-client'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 63);
    }
}

#[test]
fn capture_v61_and_v63_upgrade_keeps_accepted_analysis_and_processing_facts() {
    for previous_version in [61, 63] {
        let fixture = TestDatabase::new(&format!("capture-v{previous_version}-execution"));
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in MIGRATIONS.iter().filter(|m| m.version <= previous_version) {
            apply_fixture_migration(&connection, migration);
            connection.execute("INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES(?1,?2,?3,123,'previous-client')",params![migration.version,migration.name,migration_checksum(migration.sql)]).unwrap();
        }
        connection.execute_batch(r#"INSERT INTO capture_transfers VALUES('transfer','owner','{}','record',1,'digest','{}','received','receipt',1,2);
            INSERT INTO capture_analyses(analysis_id,operation_id,transfer_id,version,source_text,source_sha256,state,source_run_id,created_at) VALUES('analysis','analysis-operation','transfer',1,'原有讨论','source-digest','prepared','accepted-run',3);
            INSERT INTO capture_processing VALUES('processing','transfer','speech-operation','speech-model','audio','{"frozen":true}','submitted',NULL,2,3);
            INSERT INTO capture_handoffs VALUES('handoff','analysis','candidate',1,'{"executionTarget":"agent"}','conversation','original-run','submitted',4);"#).unwrap();
        drop(connection);
        for _ in 0..2 {
            let db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
            let row:(String,String,String,Option<String>)=db.connection.query_row("SELECT source_text,source_run_id,state,execution_json FROM capture_analyses WHERE analysis_id='analysis'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            assert_eq!(
                row,
                (
                    "原有讨论".into(),
                    "accepted-run".into(),
                    "prepared".into(),
                    None
                )
            );
            let processing:(String,String)=db.connection.query_row("SELECT state,request_json FROM capture_processing WHERE processing_id='processing'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(
                processing,
                ("submitted".into(), r#"{"frozen":true}"#.into())
            );
            let handoff: (String, String) = db
                .connection
                .query_row(
                    "SELECT run_id,input_json FROM capture_handoffs WHERE handoff_id='handoff'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(
                handoff,
                (
                    "original-run".into(),
                    r#"{"executionTarget":"agent"}"#.into()
                )
            );
            let count:i64=db.connection.query_row("SELECT COUNT(*) FROM schema_migrations WHERE version<=?1 AND app_version='previous-client'",[previous_version],|r|r.get(0)).unwrap();
            assert_eq!(count, previous_version);
        }
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "升级及重启后的文件和项目关联在同一数据库生命周期验收"
)]
fn legacy_cloud_mirrors_become_independent_local_spaces_and_keep_files_and_project_links() {
    let fixture = TestDatabase::new("retire-cloud-mirror");
    let local_root = fixture.path.with_extension("files");
    std::fs::create_dir_all(&local_root).unwrap();
    std::fs::write(local_root.join("keep.txt"), b"user-modified-local-copy").unwrap();
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    for migration in &MIGRATIONS[..53] {
        apply_fixture_migration(&connection, migration);
        connection.execute("INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')", params![migration.version,migration.name,migration_checksum(migration.sql)]).unwrap();
    }
    connection.execute_batch("INSERT INTO accounts(account_id,display_name,account_status,created_at,updated_at) VALUES ('mirror-account','成员','active',1,1);
        INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at) VALUES ('mirror-project','项目','迁移','active',0,'mirror-create','mirror-create',1,1);
        INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at,workspace_kind) VALUES
            ('legacy-mirror','原团队','/tmp/mirror-placeholder','ready',2,1,2,'cloud_team'),
            ('direct','直接云端','cilai-cloud://direct-team','ready',0,1,1,'cloud_team');
        INSERT INTO workspace_exports(workspace_id,desired_revision,exported_revision,export_state,updated_at) VALUES ('legacy-mirror',2,2,'current',2),('direct',0,0,'current',1);
        INSERT INTO workspace_platform_bindings(workspace_id,account_id,platform_workspace_id,project_id,platform_root,binding_state,created_at,updated_at,team_space_id,managed_root,device_id,permission_role,access_mode) VALUES
            ('legacy-mirror','mirror-account','mirror-team','mirror-project','/','bound',1,2,'mirror-team','/tmp/mirror-placeholder','old-device','editor','remote_direct'),
            ('direct','mirror-account','direct-team','mirror-project','/','bound',1,1,'direct-team',NULL,NULL,'viewer','remote_direct');
        INSERT INTO project_workspaces(project_id,workspace_id,role,created_at) VALUES ('mirror-project','legacy-mirror','primary',1);
        INSERT INTO local_files(local_file_id,workspace_id,relative_path,normalized_relative_path,materialization_state,file_type,observed_at,scan_state,state_revision,created_at,updated_at) VALUES ('keep-file','legacy-mirror','keep.txt','keep.txt','materialized','file',1,'current',0,1,1);
        INSERT INTO cloud_workspace_operations(client_operation_id,request_fingerprint,operation_kind,project_id,team_space_id,display_name,managed_root,device_id,state,workspace_id,created_at,updated_at) VALUES ('old-operation','fingerprint','attach_existing','mirror-project','mirror-team','原团队','/tmp/mirror-placeholder','old-device','pulling_remote','legacy-mirror',1,1);").unwrap();
    connection
        .execute(
            "UPDATE workspaces SET local_root=?1 WHERE workspace_id='legacy-mirror'",
            [local_root.to_string_lossy().as_ref()],
        )
        .unwrap();
    drop(connection);
    let persistence = SqlitePersistence::open(&fixture.path, "0.1.1").unwrap();
    let connection = &persistence.connection;
    let local: (String, String) = connection
        .query_row(
            "SELECT workspace_kind,local_root FROM workspaces WHERE workspace_id='legacy-mirror'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        local,
        (
            "local".to_owned(),
            local_root.to_string_lossy().into_owned()
        )
    );
    assert_eq!(
        std::fs::read(local_root.join("keep.txt")).unwrap(),
        b"user-modified-local-copy"
    );
    let remote: (String,String,String) = connection.query_row("SELECT w.workspace_kind,w.local_root,b.access_mode FROM workspaces w JOIN workspace_platform_bindings b ON b.workspace_id=w.workspace_id WHERE w.workspace_id='cloud-workspace-retired-legacy-mirror'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(
        remote,
        (
            "cloud_team".to_owned(),
            "cilai-cloud://mirror-team".to_owned(),
            "remote_direct".to_owned()
        )
    );
    assert_eq!(connection.query_row("SELECT binding_state FROM workspace_platform_bindings WHERE workspace_id='legacy-mirror'",[],|row|row.get::<_,String>(0)).unwrap(),"disconnected");
    assert_eq!(connection.query_row("SELECT state FROM cloud_workspace_operations WHERE client_operation_id='old-operation'",[],|row|row.get::<_,String>(0)).unwrap(),"failed");
    assert_eq!(
        connection
            .query_row(
                "SELECT workspace_id FROM local_files WHERE local_file_id='keep-file'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "legacy-mirror"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM project_workspaces WHERE project_id='mirror-project'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT workspace_kind FROM workspaces WHERE workspace_id='direct'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "cloud_team"
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    drop(persistence);
    let reopened = SqlitePersistence::open(&fixture.path, "0.1.1").unwrap();
    assert_eq!(
        reopened
            .connection
            .query_row("SELECT count(*) FROM cloud_mirror_retirements", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    std::fs::remove_dir_all(local_root).unwrap();
}

#[test]
fn released_migration_twenty_nine_stays_frozen_and_upgrades_to_thirty() {
    const RELEASED_MIGRATION_29_CHECKSUM: &str =
        "eb6c70b1ee4fb95af8774cd20e4f1d5091fc5d08333cd01bc88a8fbd6edd039a";
    let released = &MIGRATIONS[28];
    assert_eq!(released.version, 29);
    assert_eq!(released.name, "add_cloud_workspace_baseline_transfer");
    assert_eq!(
        migration_checksum(released.sql),
        RELEASED_MIGRATION_29_CHECKSUM
    );

    let fixture = TestDatabase::new("upgrade-v29");
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    for migration in &MIGRATIONS[..29] {
        apply_fixture_migration(&connection, migration);
        connection
            .execute(
                "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                params![
                    migration.version,
                    migration.name,
                    migration_checksum(migration.sql)
                ],
            )
            .unwrap();
    }
    drop(connection);

    let persistence = SqlitePersistence::open(&fixture.path, "0.1.1").unwrap();
    let migration_29_checksum: String = persistence
        .connection
        .query_row(
            "SELECT checksum FROM schema_migrations WHERE version=29",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(migration_29_checksum, RELEASED_MIGRATION_29_CHECKSUM);
    let migration_30_count: i64 = persistence
        .connection
        .query_row(
            "SELECT count(*) FROM schema_migrations WHERE version=30 AND name='add_cloud_workspace_pending_operation_index'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(migration_30_count, 1);
    let index_count: i64 = persistence
        .connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE type='index' AND name='idx_cloud_workspace_one_pending_per_project'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(index_count, 1);
}

fn prepare_local_confirmation_upgrade_fixture(connection: &Connection) {
    configure_connection(connection).unwrap();
    for migration in &MIGRATIONS[..31] {
        apply_fixture_migration(connection, migration);
        connection.execute(
            "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
            params![migration.version, migration.name, migration_checksum(migration.sql)],
        ).unwrap();
    }
    connection.execute_batch(
        "INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at,workspace_kind)
         VALUES ('w-local','本地','/tmp/local','ready',0,1,1,'local'),
                ('w-cloud','云端','/tmp/cloud','ready',0,1,1,'cloud_team');
         INSERT INTO local_files(local_file_id,workspace_id,relative_path,normalized_relative_path,materialization_state,file_type,observed_at,scan_state,state_revision,created_at,updated_at)
         VALUES ('f-local','w-local','a.md','a.md','materialized','file',1,'current',0,1,1),
                ('f-cloud','w-cloud','b.md','b.md','materialized','file',1,'current',0,1,1),
                ('f-converted','w-cloud','c.md','c.md','materialized','file',1,'current',0,1,1);
         INSERT INTO file_states(local_file_id,baseline_relation,local_change_state,confirmation_state,submission_state,conflict_state,display_state,capability_bits,capability_reasons_json,computed_at,state_revision)
         VALUES ('f-local','platform_absent','added','pending','none','none','confirmation_required',4,'[]',1,0),
                ('f-cloud','platform_absent','added','pending','none','none','confirmation_required',4,'[]',1,0);
         INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('p-cloud','云端项目','验证转换历史','active',0,'project-operation','project-fingerprint',1,1);
         INSERT INTO cloud_workspace_operations(client_operation_id,request_fingerprint,operation_kind,project_id,source_workspace_id,display_name,managed_root,device_id,state,workspace_id,created_at,updated_at)
         VALUES ('conversion','conversion-fingerprint','initialize_local','p-cloud','w-cloud','云端','/tmp/cloud','device','bound','w-cloud',1,2);
         INSERT INTO file_confirmations(confirmation_id,workspace_id,local_file_id,confirmed_by_account_id,confirmed_checksum_algorithm,confirmed_checksum_value,target_scope,result,source_client,created_at,invalidated_at)
         VALUES ('old-confirmation','w-local','f-local','local-profile:default','sha256','old-content','content','confirmed','desktop',1,NULL),
                ('converted-local-confirmation','w-cloud','f-converted','local-profile:default','sha256','old-cloud-content','content','confirmed','desktop',1,NULL),
                ('cloud-confirmation','w-cloud','f-cloud','local-profile:default','sha256','cloud-content','content','confirmed','desktop',3,NULL);",
    ).unwrap();
}

#[test]
fn local_confirmation_upgrade_retains_history_without_a_pending_action() {
    let fixture = TestDatabase::new("upgrade-local-file-status");
    let connection = Connection::open(&fixture.path).unwrap();
    prepare_local_confirmation_upgrade_fixture(&connection);
    drop(connection);

    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.1").unwrap();
    let local: (String, String, i64) = persistence.connection.query_row(
        "SELECT confirmation_state,display_state,capability_bits FROM file_states WHERE local_file_id='f-local'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(
        local,
        ("not_required".to_owned(), "local_ready".to_owned(), 0)
    );
    let cloud: (String, String) = persistence.connection.query_row(
        "SELECT confirmation_state,display_state FROM file_states WHERE local_file_id='f-cloud'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(
        cloud,
        ("pending".to_owned(), "confirmation_required".to_owned())
    );
    let history_count: i64 = persistence
        .connection
        .query_row(
            "SELECT count(*) FROM file_confirmations WHERE confirmation_id='old-confirmation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(history_count, 1);
    let scopes: Vec<(String, String)> = {
        let mut statement = persistence.connection.prepare(
            "SELECT confirmation_id, workspace_kind_at_confirmation FROM file_confirmations ORDER BY confirmation_id"
        ).unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        scopes,
        [
            ("cloud-confirmation".to_owned(), "cloud_team".to_owned()),
            (
                "converted-local-confirmation".to_owned(),
                "local".to_owned()
            ),
            ("old-confirmation".to_owned(), "local".to_owned()),
        ]
    );
    let converted_active_cloud_confirmations: i64 = persistence.connection.query_row(
        "SELECT count(*) FROM file_confirmations WHERE local_file_id='f-converted' AND invalidated_at IS NULL AND workspace_kind_at_confirmation='cloud_team'",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(converted_active_cloud_confirmations, 0);
    let candidate = persistence
        .transact(|repositories| {
            repositories.confirmation_candidate(&WorkspaceId::new("w-local"), "f-local")
        })
        .unwrap()
        .unwrap();
    assert!(candidate.confirmation_allowed);
}

#[test]
fn legacy_unknown_cloud_outcome_is_read_only_and_preserves_actual_fields() {
    let fixture = TestDatabase::new("legacy-cloud-outcome");
    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence.connection.execute_batch(
        "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('legacy-project','历史项目','核对已有记录','active',0,'legacy-project-op','legacy-project-fingerprint',1,1);
         INSERT INTO cloud_workspace_operations(client_operation_id,request_fingerprint,operation_kind,project_id,display_name,managed_root,device_id,state,failure_reason,created_at,updated_at)
         VALUES ('legacy-unknown','legacy-fingerprint','create_empty','legacy-project','旧目录','/legacy/retained','legacy-device','creating_remote','remote_outcome_unknown',10,12);"
    ).unwrap();
    let before = persistence.connection.total_changes();
    let record = persistence
        .cloud_workspace_operation("legacy-unknown")
        .unwrap()
        .unwrap();
    assert_eq!(record.client_operation_id, "legacy-unknown");
    assert_eq!(record.request_fingerprint, "legacy-fingerprint");
    assert_eq!(record.project_id, "legacy-project");
    assert_eq!(record.operation_kind, "create_empty");
    assert_eq!(record.managed_root, "/legacy/retained");
    assert_eq!(record.device_id, "legacy-device");
    assert_eq!(record.state, "creating_remote");
    assert_eq!(
        record.failure_reason.as_deref(),
        Some("remote_outcome_unknown")
    );
    assert_eq!(record.created_at, 10);
    assert_eq!(record.updated_at, 12);
    assert!(record.workspace_id.is_none());
    assert!(record.remote_binding_id.is_none());
    assert!(
        persistence
            .cloud_workspace_operation("not-present")
            .unwrap()
            .is_none()
    );
    assert_eq!(persistence.connection.total_changes(), before);
}

#[test]
fn active_cloud_bindings_cannot_reuse_a_managed_root_on_one_device() {
    let fixture = TestDatabase::new("cloud-workspace-managed-root");
    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence
        .connection
        .execute_batch(
            "INSERT INTO accounts(account_id,display_name,account_status,created_at,updated_at)
             VALUES ('account-cloud','成员','active',1,1);
             INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at,workspace_kind)
             VALUES ('workspace-cloud-1','团队一','/tmp/cloud-one','creating',0,1,1,'cloud_team'),
                    ('workspace-cloud-2','团队二','/tmp/cloud-two','creating',0,1,1,'cloud_team');
             INSERT INTO workspace_platform_bindings(
                workspace_id,account_id,platform_workspace_id,project_id,platform_root,binding_state,created_at,updated_at,
                team_space_id,managed_root,device_id,permission_role)
             VALUES ('workspace-cloud-1','account-cloud','team-1','project-1','/','binding',1,1,
                     'team-1','/tmp/managed-root','device-1','owner');",
        )
        .unwrap();
    assert!(
        persistence
            .connection
            .execute(
                "INSERT INTO workspace_platform_bindings(
                    workspace_id,account_id,platform_workspace_id,project_id,platform_root,binding_state,created_at,updated_at,
                    team_space_id,managed_root,device_id,permission_role)
                 VALUES ('workspace-cloud-2','account-cloud','team-2','project-2','/','binding',1,1,
                         'team-2','/tmp/managed-root','device-1','editor')",
                [],
            )
            .is_err()
    );
}

#[test]
fn archive_updates_only_metadata_and_rejects_stale_or_missing_targets() {
    let fixture = TestDatabase::new("archive-metadata");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let state = serde_json::json!({
        "conversations": [{"conversationId":"a","archived":false,"updatedAt":1}, {"conversationId":"b","archived":false,"updatedAt":1}],
        "messages":[{"messageId":"m","content":"正文".repeat(100_000)}], "runs":[]
    });
    persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
    persistence.connection.execute_batch("CREATE TRIGGER archive_payload_must_not_change BEFORE UPDATE ON ai_runtime_state BEGIN SELECT RAISE(ABORT,'payload rewritten'); END;").unwrap();
    let (revision, saved, changed) = persistence
        .set_ai_conversations_archived(1, &["a".to_owned()], true, 2)
        .unwrap();
    assert_eq!(revision, 2);
    assert_eq!(saved[0]["archived"], true);
    assert_eq!(changed, vec!["a"]);
    let (same_revision, same_saved, same_changed) = persistence
        .set_ai_conversations_archived(2, &["a".to_owned()], true, 3)
        .unwrap();
    assert_eq!(same_revision, 2);
    assert_eq!(same_saved, saved);
    assert!(same_changed.is_empty());
    let before = persistence.ai_runtime_state().unwrap();
    assert_eq!(before.1["messages"], state["messages"]);
    assert_eq!(
        persistence
            .set_ai_conversations_archived(1, &["b".to_owned()], true, 4)
            .unwrap_err()
            .reason(),
        Some("ai_state_revision_conflict")
    );
    assert!(
        persistence
            .set_ai_conversations_archived(2, &["b".to_owned(), "missing".to_owned()], true, 4)
            .is_err()
    );
    assert_eq!(persistence.ai_runtime_state().unwrap(), before);
    persistence
        .connection
        .execute_batch("DROP TRIGGER archive_payload_must_not_change;")
        .unwrap();
    assert_eq!(
        persistence
            .replace_ai_runtime_state(1, &state, 5)
            .unwrap_err()
            .reason(),
        Some("ai_state_revision_conflict")
    );
    let restored = persistence
        .set_ai_conversations_archived(2, &["a".to_owned()], false, 6)
        .unwrap();
    assert_eq!(restored.0, 3);
    drop(persistence);
    let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    assert_eq!(
        reopened.ai_runtime_state().unwrap().1["conversations"][0]["archived"],
        false
    );
}

#[test]
fn conversation_metadata_migration_preserves_history_order_and_revision() {
    let fixture = TestDatabase::new("archive-migration");
    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute_batch(include_str!(
            "../../../migrations/0014_add_ai_runtime_state.sql"
        ))
        .unwrap();
    let state = serde_json::json!({"conversations":[
        {"conversationId":"z","archived":true,"updatedAt":7,"context":{"projectId":"p"}},
        {"conversationId":"a","archived":false,"updatedAt":3}],
        "messages":[{"messageId":"m","content":"保留正文"}],"runs":[{"runId":"r","status":"completed"}]});
    connection
        .execute(
            "UPDATE ai_runtime_state SET state_revision=42,state_json=?1",
            [state.to_string()],
        )
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../../migrations/0034_separate_ai_conversation_metadata.sql"
        ))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../../migrations/0038_separate_ai_messages_and_runs.sql"
        ))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../../../migrations/0055_add_ai_business_ledger.sql"
        ))
        .unwrap();
    let persistence = SqlitePersistence {
        connection,
        database_path: fixture.path.clone(),
        file_content_index: None,
    };
    assert_eq!(persistence.ai_runtime_state().unwrap(), (42, state));
    let payload: String = persistence
        .connection
        .query_row("SELECT state_json FROM ai_runtime_state", [], |row| {
            row.get(0)
        })
        .unwrap();
    let payload = serde_json::from_str::<serde_json::Value>(&payload).unwrap();
    assert!(payload.get("conversations").is_none());
    assert!(payload.get("messages").is_none());
    assert!(payload.get("runs").is_none());
}

fn ai_entities(
    upserts: Vec<(i64, serde_json::Value)>,
    removed: &[&str],
    total: i64,
) -> AiStateEntityChanges {
    AiStateEntityChanges {
        upserts,
        removed_ids: removed.iter().map(|id| (*id).to_owned()).collect(),
        total,
    }
}

#[test]
fn ai_state_delta_rewrites_only_changed_entries() {
    let fixture = TestDatabase::new("ai-state-delta");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let state = serde_json::json!({
        "agents": [{"agentId": "agent-1"}],
        "conversations": [{"conversationId":"c","archived":false}],
        "messages": [
            {"messageId":"m1","conversationId":"c","role":"user","content":"正文".repeat(100_000)},
            {"messageId":"m2","conversationId":"c","role":"user","content":"第二条"}
        ],
        "runs": [{"runId":"r1","conversationId":"c","status":"completed"}]
    });
    persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
    // 大消息和其余字段都不应在增量保存里被改写
    persistence.connection.execute_batch(
        "CREATE TRIGGER payload_must_not_change BEFORE UPDATE ON ai_runtime_state BEGIN SELECT RAISE(ABORT,'payload rewritten'); END;
         CREATE TRIGGER long_message_must_not_change BEFORE UPDATE ON ai_messages WHEN old.message_id='m1' BEGIN SELECT RAISE(ABORT,'message rewritten'); END;",
    ).unwrap();
    let run = serde_json::json!({"runId":"r2","conversationId":"c","status":"running"});
    let message =
        serde_json::json!({"messageId":"m3","conversationId":"c","role":"user","content":"新消息"});
    let revision = persistence
        .apply_ai_runtime_changes(
            1,
            &AiStateChanges {
                full: false,
                payload: None,
                conversations: ai_entities(Vec::new(), &[], 1),
                messages: ai_entities(vec![(1, message.clone())], &["m2"], 2),
                runs: ai_entities(vec![(1, run.clone())], &[], 2),
            },
            2,
        )
        .unwrap();
    assert_eq!(revision, 2);
    let (_, saved) = persistence.ai_runtime_state().unwrap();
    assert_eq!(saved["agents"], state["agents"]);
    assert_eq!(saved["messages"][0], state["messages"][0]);
    assert_eq!(saved["messages"][1], message);
    assert_eq!(saved["messages"].as_array().unwrap().len(), 2);
    assert_eq!(saved["runs"][1], run);
}

#[test]
fn repeated_application_context_is_stored_once_and_read_back_whole() {
    let fixture = TestDatabase::new("ai-state-context-dedupe");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let context = serde_json::json!({
        "applicationId": "com.cilai.archive",
        "writeTools": [{"toolName": "save", "inputSchema": {"type": "object", "properties": {"x": {"type": "string"}}}}],
        "searchableApplications": [{"applicationId": "com.cilai.archive"}]
    });
    let state = serde_json::json!({
        "conversations": [], "runs": [],
        "messages": [
            {"messageId":"m1","role":"user","content":"一","applicationContext":context},
            {"messageId":"m2","role":"user","content":"二","applicationContext":context},
            {"messageId":"m3","role":"user","content":"三","applicationContext":null}
        ]
    });
    persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
    let contexts = |persistence: &SqlitePersistence| -> i64 {
        persistence
            .connection
            .query_row("SELECT count(*) FROM ai_application_contexts", [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    assert_eq!(contexts(&persistence), 1);
    assert_eq!(persistence.ai_runtime_state().unwrap(), (1, state.clone()));

    let without_context = serde_json::json!({
        "conversations": [], "runs": [],
        "messages": [state["messages"][2].clone()]
    });
    persistence
        .replace_ai_runtime_state(1, &without_context, 2)
        .unwrap();
    assert_eq!(contexts(&persistence), 0);

    let stored_form = serde_json::json!({
        "conversations": [], "runs": [],
        "messages": [{"messageId":"m4","role":"user","content":"四","applicationContextId":1}]
    });
    let rejected = persistence
        .replace_ai_runtime_state(2, &stored_form, 3)
        .unwrap_err();
    assert_eq!(rejected.reason(), Some("ai_state_invalid"));
}

#[test]
fn context_migration_keeps_every_message_context_intact() {
    let fixture = TestDatabase::new("ai-context-migration");
    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute_batch(include_str!(
            "../../../migrations/0014_add_ai_runtime_state.sql"
        ))
        .unwrap();
    let context = serde_json::json!({"applicationId":"a","writeTools":[{"toolName":"save"}]});
    let state = serde_json::json!({"conversations":[],"runs":[],"messages":[
        {"messageId":"m1","content":"一","applicationContext":context},
        {"messageId":"m2","content":"二","applicationContext":context},
        {"messageId":"m3","content":"三","applicationContext":null}]});
    connection
        .execute(
            "UPDATE ai_runtime_state SET state_json=?1",
            [state.to_string()],
        )
        .unwrap();
    for sql in [
        include_str!("../../../migrations/0034_separate_ai_conversation_metadata.sql"),
        include_str!("../../../migrations/0038_separate_ai_messages_and_runs.sql"),
        include_str!("../../../migrations/0039_deduplicate_ai_message_application_contexts.sql"),
    ] {
        connection.execute_batch(sql).unwrap();
    }
    connection
        .execute_batch(include_str!(
            "../../../migrations/0055_add_ai_business_ledger.sql"
        ))
        .unwrap();
    let persistence = SqlitePersistence {
        connection,
        database_path: fixture.path.clone(),
        file_content_index: None,
    };
    assert_eq!(persistence.ai_runtime_state().unwrap().1, state);
    let rows: i64 = persistence
        .connection
        .query_row("SELECT count(*) FROM ai_application_contexts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn ai_state_delta_rejects_a_baseline_that_differs_from_storage() {
    let fixture = TestDatabase::new("ai-state-delta-mismatch");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let state = serde_json::json!({
        "conversations": [], "runs": [],
        "messages": [{"messageId":"m1","role":"user","content":"正文"}]
    });
    persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
    let unchanged = |messages| AiStateChanges {
        full: false,
        payload: None,
        conversations: ai_entities(Vec::new(), &[], 0),
        messages,
        runs: ai_entities(Vec::new(), &[], 0),
    };
    let wrong_total = persistence
        .apply_ai_runtime_changes(1, &unchanged(ai_entities(Vec::new(), &[], 2)), 2)
        .unwrap_err();
    assert_eq!(wrong_total.reason(), Some("ai_state_delta_mismatch"));
    let missing_removal = persistence
        .apply_ai_runtime_changes(1, &unchanged(ai_entities(Vec::new(), &["absent"], 1)), 2)
        .unwrap_err();
    assert_eq!(missing_removal.reason(), Some("ai_state_delta_mismatch"));
    let mut embedded = unchanged(ai_entities(Vec::new(), &[], 1));
    embedded.payload = Some(serde_json::json!({"messages": []}));
    let embedded = persistence
        .apply_ai_runtime_changes(1, &embedded, 2)
        .unwrap_err();
    assert_eq!(embedded.reason(), Some("ai_state_invalid"));
    assert_eq!(persistence.ai_runtime_state().unwrap(), (1, state));
}

#[test]
fn ai_state_delta_queues_skill_delivery_once_the_reply_is_saved() {
    let fixture = TestDatabase::new("ai-state-delta-skill");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let state = serde_json::json!({
        "conversations": [{"conversationId":"c","context":{"workspaceRoot":"/tmp/p"}}],
        "messages": [],
        "runs": [{"runId":"r","conversationId":"c","status":"completed","artifacts":[]}]
    });
    persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
    assert!(persistence.skill_installation("r").unwrap().is_none());
    let reply = serde_json::json!({"messageId":"reply","conversationId":"c","runId":"r","role":"assistant","content":"技能"});
    persistence
        .apply_ai_runtime_changes(
            1,
            &AiStateChanges {
                full: false,
                payload: None,
                conversations: ai_entities(Vec::new(), &[], 1),
                messages: ai_entities(vec![(0, reply)], &[], 1),
                runs: ai_entities(Vec::new(), &[], 1),
            },
            2,
        )
        .unwrap();
    let queued = persistence.skill_installation("r").unwrap().unwrap();
    assert_eq!(queued.status, "pending");
    let source: serde_json::Value = serde_json::from_str(&queued.source_json).unwrap();
    assert_eq!(source["message"]["messageId"], "reply");
    assert_eq!(source["workspaceRoot"], "/tmp/p");
    assert!(persistence.skill_source_run_available("r").unwrap());
}

#[test]
fn ai_runtime_state_uses_revision_checked_atomic_replacement() {
    let fixture = TestDatabase::new("ai-runtime-state");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let (revision, initial) = persistence.ai_runtime_state().unwrap();
    assert_eq!(revision, 0);
    assert_eq!(initial["runs"], serde_json::json!([]));

    let updated = serde_json::json!({
        "agents": [{"agentId": "agent-1"}],
        "conversations": [],
        "messages": [],
        "runs": [],
        "permissionRequests": [],
        "inboxReadItemIds": ["run:failed-1"],
        "automationRules": []
    });
    let next_revision = persistence
        .replace_ai_runtime_state(revision, &updated, 100)
        .unwrap();
    assert_eq!(next_revision, 1);
    assert_eq!(
        persistence.ai_runtime_state().unwrap(),
        (1, updated.clone())
    );

    let conflict = persistence
        .replace_ai_runtime_state(revision, &updated, 101)
        .unwrap_err();
    assert_eq!(conflict.reason(), Some("ai_state_revision_conflict"));
    assert_eq!(persistence.ai_runtime_state().unwrap(), (1, updated));
}

#[test]
fn lossy_history_replacement_rolls_back_every_record_and_revision() {
    let fixture = TestDatabase::new("text-integrity");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let clean = serde_json::json!({"conversations":[{"conversationId":"c","title":"测试技能"}],"messages":[],"runs":[{"runId":"r","conversationId":"c","artifacts":[{"path":"测试技能/蓝图.png"}]}]});
    persistence.replace_ai_runtime_state(0, &clean, 1).unwrap();
    let mut damaged = clean.clone();
    damaged["conversations"][0]["title"] = serde_json::json!("合法的另一个修改");
    damaged["runs"][0]["artifacts"][0]["path"] = serde_json::json!("测试��能/蓝图.png");
    assert_eq!(
        persistence
            .replace_ai_runtime_state(1, &damaged, 2)
            .unwrap_err()
            .reason(),
        Some("ai_text_corruption_detected")
    );
    assert_eq!(persistence.ai_runtime_state().unwrap(), (1, clean));
}

#[test]
fn text_recovery_checks_source_and_atomically_preserves_business_identity() {
    let fixture = TestDatabase::new("text-recovery");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let source = persistence
        .ai_history("source", &serde_json::Value::Null, 0)
        .unwrap();
    persistence.ai_history("activate",&serde_json::json!({"sourceDigest":source["sourceDigest"],"ledger":{"commandId":"activate","records":[]},"legacyDeliveries":[]}),1).unwrap();
    persistence.ai_history("bind",&serde_json::json!({"conversationId":"c","nativeThreadId":"old-thread","title":"测试技能"}),2).unwrap();
    let record = serde_json::json!({"runId":"r","conversationId":"c","artifacts":[{"path":"测试��能/蓝图.png"}]});
    persistence.apply_ai_business_batch(&serde_json::json!({"commandId":"seed","records":[{"collection":"runs","id":"r","position":0,"expectedDigest":null,"value":record}]}),3).unwrap();
    persistence
        .connection
        .execute(
            "INSERT INTO ai_run_native_refs VALUES('r','old-thread','old-turn')",
            [],
        )
        .unwrap();
    let before = persistence.ai_business_ledger().unwrap();
    let hash = super::ai_business_ledger::digest(&before);
    let mut repaired = record.clone();
    repaired["artifacts"][0]["path"] = serde_json::json!("测试技能/蓝图.png");
    let request = serde_json::json!({"operationId":"repair","expectedLedgerDigest":hash,"ledger":{"commandId":"repair-ledger","records":[{"collection":"runs","id":"r","position":0,"expectedDigest":before["runs"][0]["digest"],"value":repaired}]},"transitions":[{"conversationId":"c","previousThreadId":"old-thread","nativeThreadId":"restored-thread","title":"测试技能","archived":false,"runs":[{"runId":"r","nativeTurnId":"r"}],"messages":[]}]});
    let mut stale = request.clone();
    stale["expectedLedgerDigest"] = serde_json::json!("stale");
    assert_eq!(
        persistence
            .ai_history("recoverText", &stale, 4)
            .unwrap_err()
            .reason(),
        Some("ai_text_recovery_source_changed")
    );
    let mut incomplete = request.clone();
    incomplete["transitions"][0]["runs"] = serde_json::json!([]);
    assert_eq!(
        persistence
            .ai_history("recoverText", &incomplete, 4)
            .unwrap_err()
            .reason(),
        Some("ai_text_recovery_incomplete")
    );
    assert_eq!(persistence.ai_business_ledger().unwrap(), before);
    persistence.ai_history("recoverText", &request, 5).unwrap();
    let after = persistence.ai_business_ledger().unwrap();
    assert_eq!(after["runs"][0]["id"], "r");
    assert_eq!(after["runs"][0]["value"], repaired);
    assert_eq!(after["runRefs"][0]["nativeThreadId"], "restored-thread");
    let mut lossy = record;
    lossy["artifacts"][0]["path"] = serde_json::json!("测试���能/蓝图.png");
    assert_eq!(persistence.apply_ai_business_batch(&serde_json::json!({"commandId":"lossy","records":[{"collection":"runs","id":"r","position":0,"expectedDigest":after["runs"][0]["digest"],"value":lossy}]}),6).unwrap_err().reason(),Some("ai_text_corruption_detected"));
    assert_eq!(persistence.ai_business_ledger().unwrap(), after);
}

#[test]
fn native_cutover_keeps_business_transactions_after_removing_legacy_tables() {
    let fixture = TestDatabase::new("native-cutover");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let source = persistence
        .ai_history("source", &serde_json::Value::Null, 0)
        .unwrap();
    persistence.ai_history("activate",&serde_json::json!({"sourceDigest":source["sourceDigest"],"ledger":{"commandId":"activate","records":[]},"legacyDeliveries":[]}),1).unwrap();
    persistence.connection.execute_batch("DROP TABLE ai_conversations; DROP TABLE ai_messages; DROP TABLE ai_runs; DROP TABLE ai_runtime_state; DROP TABLE ai_runtime_revision;").unwrap();
    persistence.connection.execute("INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at) VALUES ('project-ai','AI 项目','验证归类','active',0,'op-project-ai','fp-project-ai',1,1)",[]).unwrap();
    persistence.ai_history("bind",&serde_json::json!({"conversationId":"conversation-ai","nativeThreadId":"native-thread","title":"业务核对"}),1).unwrap();
    let mut command = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
    let values = [
        (
            "conversations",
            "conversation-ai",
            serde_json::json!({"conversationId":"conversation-ai","context":{"projectId":"project-ai","workspaceRoot":"/tmp"},"updatedAt":2}),
        ),
        (
            "runs",
            "run-ai",
            serde_json::json!({"runId":"run-ai","conversationId":"conversation-ai","status":"completed","artifacts":[],"skillIds":[]}),
        ),
        (
            "messages",
            "reply-run-ai",
            serde_json::json!({"messageId":"reply-run-ai","conversationId":"conversation-ai","runId":"run-ai","role":"assistant","contentDigest":"body-digest","contentBytes":12}),
        ),
    ];
    command.ledger_request = Some(
        serde_json::json!({"commandId":"finalize-native","records":values.into_iter().map(|(collection,id,value)|serde_json::json!({"collection":collection,"id":id,"position":0,"value":value,"expectedDigest":null})).collect::<Vec<_>>(),"deliveries":[{"runId":"run-ai","messageId":"reply-run-ai","contentDigest":"body-digest","skillDelivery":{"Ok":null},"capabilityDelivery":{"Ok":null}}]}),
    );
    let result = persistence.finalize_ai_turn(&command).unwrap();
    assert_eq!(result.task_id.as_deref(), Some("task-ai"));
    assert!(persistence.finalize_ai_turn(&command).unwrap().replayed);
    let mut conflict = command.clone();
    conflict.ledger_request.as_mut().unwrap()["records"][0]["value"]["updatedAt"] =
        serde_json::json!(3);
    assert_eq!(
        persistence
            .finalize_ai_turn(&conflict)
            .unwrap_err()
            .reason(),
        Some("ai_ledger_command_conflict")
    );
    assert_eq!(
        persistence.ai_business_ledger().unwrap()["messages"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        persistence.ai_runtime_state().unwrap_err().reason(),
        Some("ai_legacy_storage_retired")
    );
    assert!(
        persistence
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM my_work_items WHERE task_id='task-ai')",
                [],
                |row| row.get::<_, bool>(0)
            )
            .unwrap()
    );
    let source: String = persistence
        .connection
        .query_row(
            "SELECT source_json FROM ai_skill_installations WHERE run_id='run-ai'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !serde_json::from_str::<serde_json::Value>(&source).unwrap()["message"]
            .as_object()
            .unwrap()
            .contains_key("content")
    );
    drop(persistence);
    let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    assert_eq!(
        reopened.ai_business_ledger().unwrap()["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn ai_turn_finalization_atomically_creates_and_projects_a_phase_free_task() {
    let fixture = TestDatabase::new("ai-turn-finalization");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence
        .connection
        .execute(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-ai','AI 项目','验证自动归类','active',0,'op-project-ai','fp-project-ai',1,1)",
            [],
        )
        .unwrap();
    let command = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
    let result = persistence.finalize_ai_turn(&command).unwrap();
    assert_eq!(result.state_revision, 1);
    assert_eq!(result.task_id.as_deref(), Some("task-ai"));
    assert_eq!(
        persistence.ai_runtime_state().unwrap(),
        (1, ai_turn_state("conversation-ai", "run-ai"))
    );
    let task_projection = persistence
        .connection
        .query_row(
            "SELECT task.phase_id,task.status,work.focus_group,work.action_type,task.assignee_type,task.assignee_label
             FROM business_tasks task
             JOIN my_work_items work ON work.task_id=task.business_task_id
             WHERE task.business_task_id='task-ai'",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        task_projection,
        (
            None,
            "in_progress".to_owned(),
            "in_progress".to_owned(),
            "task_execute".to_owned(),
            "collaboration".to_owned(),
            "AI 对话协作".to_owned()
        )
    );
    assert_eq!(
        persistence
            .ai_work_task_candidates("project-ai")
            .unwrap()
            .len(),
        1
    );
    let contributions = persistence
        .transact(|repositories| repositories.business_task_contributions("task-ai"))
        .unwrap();
    assert_eq!(contributions.len(), 1);
    assert_eq!(contributions[0].run_id, "run-ai");
    assert_eq!(contributions[0].conversation_id, "conversation-ai");
    assert_eq!(contributions[0].stage.as_deref(), Some("in_progress"));
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "失败投影及后续恢复必须在同一份运行状态中完整验证"
)]
fn failed_tool_call_blocks_the_classified_task_until_a_later_success() {
    let fixture = TestDatabase::new("ai-tool-failure-incident");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence
        .connection
        .execute(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-ai','AI 项目','验证工具失败归类','active',0,'op-project-ai','fp-project-ai',1,1)",
            [],
        )
        .unwrap();
    let mut failed = ai_create_task_command("conversation-ai", "run-failed", "task-ai", 0);
    let failed_state = serde_json::json!({
        "agents":[],
        "conversations":[{
            "conversationId":"conversation-ai",
            "title":"生成产品配图",
            "archived":false,
            "context":{"projectId":"project-ai","taskId":"task-ai"}
        }],
        "messages":[{
            "messageId":"reply-failed",
            "conversationId":"conversation-ai",
            "runId":"run-failed",
            "role":"assistant",
            "content":"生图失败"
        }],
        "runs":[{
            "runId":"run-failed",
            "conversationId":"conversation-ai",
            "status":"completed",
            "model":"agnes-3.0-flash",
            "events":[{
                "sequence":13,
                "eventType":"item.completed",
                "phase":"reply",
                "kind":"mcp",
                "status":"failed",
                "summary":"generate_image 调用失败"
            }]
        }]
    });
    with_ai_state(&mut failed, &failed_state);
    persistence.finalize_ai_turn(&failed).unwrap();

    let incident: (String, String, String) = persistence
        .connection
        .query_row(
            "SELECT status,task_id,detail FROM attention_cases WHERE agent_run_id='run-failed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        incident,
        (
            "open".to_owned(),
            "task-ai".to_owned(),
            "generate_image 调用失败".to_owned()
        )
    );
    let blocked = my_work_items(&mut persistence)
        .into_iter()
        .find(|item| item.task_id.as_deref() == Some("task-ai"))
        .unwrap();
    assert_eq!(blocked.focus_group, "todo");
    assert_eq!(blocked.blocking_run_id.as_deref(), Some("run-failed"));
    assert_eq!(
        blocked.blocking_reason.as_deref(),
        Some("generate_image 调用失败")
    );

    let mut recovered = failed.clone();
    recovered.expected_revision = 1;
    recovered.run_id = "run-recovered".to_owned();
    recovered.new_task_id = None;
    recovered.previous_task_id = Some("task-ai".to_owned());
    recovered.occurred_at = 20;
    recovered.classification.action = "keep".to_owned();
    recovered.classification.task_action = "keep".to_owned();
    recovered.classification.task_id = Some("task-ai".to_owned());
    recovered.classification.objective = None;
    recovered.classification.expected_output = None;
    recovered.classification.completion_criteria = None;
    recovered.classification.raw = serde_json::json!({"run":"recovered"});
    let mut recovered_state = failed_state;
    recovered_state["messages"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "messageId":"reply-recovered",
            "conversationId":"conversation-ai",
            "runId":"run-recovered",
            "role":"assistant",
            "content":"生图成功"
        }));
    recovered_state["runs"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "runId":"run-recovered",
            "conversationId":"conversation-ai",
            "status":"completed",
            "model":"agnes-3.0-flash",
            "events":[{
                "sequence":8,
                "eventType":"item.completed",
                "phase":"reply",
                "kind":"mcp",
                "status":"completed",
                "summary":"已使用 generate_image"
            }]
        }));
    with_ai_state(&mut recovered, &recovered_state);
    persistence.finalize_ai_turn(&recovered).unwrap();

    let resolved: (String, Option<i64>) = persistence
        .connection
        .query_row(
            "SELECT status,resolved_at FROM attention_cases WHERE agent_run_id='run-failed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(resolved, ("resolved".to_owned(), Some(20)));
    let unblocked = my_work_items(&mut persistence)
        .into_iter()
        .find(|item| item.task_id.as_deref() == Some("task-ai"))
        .unwrap();
    assert_eq!(unblocked.focus_group, "in_progress");
    assert!(unblocked.blocking_run_id.is_none());
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "归档、恢复与再次归类需在同一份数据上依次验证我的工作投影"
)]
fn archived_ai_conversations_leave_my_work_without_changing_task_history() {
    let fixture = TestDatabase::new("archived-ai-my-work");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence.connection.execute_batch(
        "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('project-ai','AI 项目','验证归档','active',0,'op-project-ai','fp-project-ai',1,1);",
    ).unwrap();
    let mut first = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
    first.classification.stage = Some("decision".to_owned());
    first.classification.raw["stage"] = serde_json::json!("decision");
    let mut first_state = ai_turn_state("conversation-ai", "run-ai");
    first_state["conversations"][0]["archived"] = serde_json::json!(false);
    first_state["runs"][0]["conversationId"] = serde_json::json!("conversation-ai");
    with_ai_state(&mut first, &first_state);
    persistence.finalize_ai_turn(&first).unwrap();
    persistence.connection.execute_batch(
        "INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
         VALUES ('task-manual','project-ai',NULL,'人工任务','目标','','产出','标准','user','员工','medium',NULL,'todo',0,'manual-task-create','fp-manual',2,2,1);",
    ).unwrap();
    assert_eq!(my_work_items(&mut persistence).len(), 2);
    assert_eq!(
        my_work_items(&mut persistence)
            .iter()
            .find(|item| item.task_id.as_deref() == Some("task-ai"))
            .unwrap()
            .focus_group,
        "decision"
    );

    let mut archived = first_state.clone();
    archived["conversations"][0]["archived"] = serde_json::json!(true);
    persistence
        .replace_ai_runtime_state(1, &archived, 11)
        .unwrap();
    let query = MyWorkQuery {
        project_id: None,
        focus_group: None,
        limit: 200,
    };
    let archived_work = persistence
        .transact(|repositories| contexthub_application::get_my_work(repositories, &query))
        .unwrap();
    assert_eq!(archived_work.total, 1);
    assert_eq!(
        archived_work.items[0].task_id.as_deref(),
        Some("task-manual")
    );
    assert_eq!(
        persistence
            .connection
            .query_row(
                "SELECT status FROM business_tasks WHERE business_task_id='task-ai'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "decision"
    );
    assert_eq!(
        persistence
            .transact(|repositories| repositories.business_task_contributions("task-ai"))
            .unwrap()
            .len(),
        1
    );

    let mut restored = archived.clone();
    restored["conversations"][0]["archived"] = serde_json::json!(false);
    persistence
        .replace_ai_runtime_state(2, &restored, 12)
        .unwrap();
    assert_eq!(my_work_items(&mut persistence).len(), 2);

    let mut second = first.clone();
    second.expected_revision = 3;
    second.conversation_id = "conversation-later".to_owned();
    second.run_id = "run-later".to_owned();
    second.new_task_id = None;
    second.previous_task_id = None;
    second.occurred_at = 13;
    second.classification.action = "apply".to_owned();
    second.classification.task_action = "link".to_owned();
    second.classification.task_id = Some("task-ai".to_owned());
    second.classification.objective = None;
    second.classification.expected_output = None;
    second.classification.completion_criteria = None;
    second.classification.stage = Some("todo".to_owned());
    second.classification.raw = serde_json::json!({"run": "later"});
    let mut second_state = first_state.clone();
    second_state["conversations"][0]["archived"] = serde_json::json!(true);
    second_state["conversations"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"conversationId":"conversation-later","archived":false}));
    second_state["runs"].as_array_mut().unwrap().push(serde_json::json!({"runId":"run-later","conversationId":"conversation-later","status":"completed"}));
    with_ai_state(&mut second, &second_state);
    persistence.finalize_ai_turn(&second).unwrap();
    persistence.connection.execute_batch(
        "INSERT INTO attention_cases(attention_case_id,case_type,project_id,task_id,title,detail,status,responsibility_type,responsibility_label,reporter_label,priority,agent_run_id,agent_label,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('incident-ai','agent_run_incident','project-ai','task-ai','旧运行异常','历史失败','open','user','员工','desktop','medium','run-ai','Codex',0,'op-incident-ai','fp-incident-ai',14,14);",
    ).unwrap();
    let visible = my_work_items(&mut persistence);
    assert_eq!(visible.len(), 2);
    let ai_item = visible
        .iter()
        .find(|item| item.task_id.as_deref() == Some("task-ai"))
        .unwrap();
    assert_eq!(ai_item.focus_group, "todo");
    assert!(ai_item.blocking_run_id.is_none());
    let decision_query = MyWorkQuery {
        project_id: None,
        focus_group: Some("decision".to_owned()),
        limit: 200,
    };
    let decision_work = persistence
        .transact(|repositories| contexthub_application::get_my_work(repositories, &decision_query))
        .unwrap();
    assert_eq!(decision_work.total, 0);
    assert!(decision_work.items.is_empty());
}

#[test]
fn ai_turn_finalization_rejects_a_duplicate_project_task() {
    let fixture = TestDatabase::new("ai-turn-duplicate-task");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence
        .connection
        .execute(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-ai','AI 项目','验证自动归类','active',0,'op-project-ai','fp-project-ai',1,1)",
            [],
        )
        .unwrap();
    let first = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
    persistence.finalize_ai_turn(&first).unwrap();
    let mut duplicate =
        ai_create_task_command("conversation-ai-2", "run-ai-2", "task-ai-duplicate", 1);
    duplicate.occurred_at = 11;
    assert_eq!(
        persistence
            .finalize_ai_turn(&duplicate)
            .unwrap_err()
            .reason(),
        Some("ai_work_task_duplicate")
    );
    assert_eq!(
        persistence
            .ai_work_task_candidates("project-ai")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn completed_conversations_close_and_later_work_reactivates_the_same_task() {
    let fixture = TestDatabase::new("ai-turn-completion-reactivation");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence
        .connection
        .execute(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-ai','AI 项目','验证自动归类','active',0,'op-project-ai','fp-project-ai',1,1)",
            [],
        )
        .unwrap();
    let first = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
    persistence.finalize_ai_turn(&first).unwrap();

    let mut completed = first.clone();
    completed.expected_revision = 1;
    completed.run_id = "run-ai-completed".to_owned();
    completed.new_task_id = None;
    completed.previous_task_id = Some("task-ai".to_owned());
    completed.occurred_at = 11;
    completed.classification = AiTurnClassificationRecord {
        action: "keep".to_owned(),
        task_action: "keep".to_owned(),
        task_id: Some("task-ai".to_owned()),
        task_title: Some("自动归类对话".to_owned()),
        objective: None,
        expected_output: None,
        completion_criteria: None,
        conversation_status: Some("completed".to_owned()),
        stage: None,
        outcome_title: Some("本轮工作完成".to_owned()),
        outcome_summary: Some("当前对话不再需要后续处理".to_owned()),
        reason: "本轮已经结束".to_owned(),
        confidence: 0.98,
        raw: serde_json::json!({"run": "completed"}),
    };
    persistence.finalize_ai_turn(&completed).unwrap();
    let closed = persistence
        .connection
        .query_row(
            "SELECT status,attempt_no,EXISTS(SELECT 1 FROM my_work_items WHERE task_id='task-ai') FROM business_tasks WHERE business_task_id='task-ai'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, bool>(2)?)),
        )
        .unwrap();
    assert_eq!(closed, ("completed".to_owned(), 1, false));
    assert_eq!(
        persistence.ai_work_task_candidates("project-ai").unwrap()[0].stage,
        "completed"
    );

    let mut reactivated = completed.clone();
    reactivated.expected_revision = 2;
    reactivated.conversation_id = "conversation-ai-later".to_owned();
    reactivated.run_id = "run-ai-later".to_owned();
    reactivated.previous_task_id = None;
    reactivated.occurred_at = 12;
    reactivated.classification.action = "apply".to_owned();
    reactivated.classification.task_action = "link".to_owned();
    reactivated.classification.conversation_status = Some("active".to_owned());
    reactivated.classification.stage = Some("todo".to_owned());
    reactivated.classification.outcome_title = Some("发现后续问题".to_owned());
    reactivated.classification.outcome_summary = Some("同一任务需要继续处理".to_owned());
    reactivated.classification.raw = serde_json::json!({"run": "reactivated"});
    persistence.finalize_ai_turn(&reactivated).unwrap();
    let reopened = persistence
        .connection
        .query_row(
            "SELECT status,attempt_no,EXISTS(SELECT 1 FROM my_work_items WHERE task_id='task-ai') FROM business_tasks WHERE business_task_id='task-ai'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, bool>(2)?)),
        )
        .unwrap();
    assert_eq!(reopened, ("todo".to_owned(), 2, true));
}

#[test]
fn version_twenty_four_separates_ai_model_from_task_responsibility() {
    let fixture = TestDatabase::new("upgrade-v24-task-responsibility");
    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence.connection.execute_batch(
        "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('project-ai','AI 项目','验证责任迁移','active',0,'op-project-ai','fp-project-ai',1,1);
         INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
         VALUES ('task-auto','project-ai',NULL,'自动任务','目标','','产出','标准','agent','Qwen3.6','medium',NULL,'in_progress',0,'ai-task-create:run-1','fp-auto',2,2,1);
         INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
         VALUES ('task-manual','project-ai',NULL,'人工任务','目标','','产出','标准','agent','研发 Agent','medium',NULL,'in_progress',0,'manual-task-create','fp-manual',2,2,1);",
    ).unwrap();

    persistence
        .connection
        .execute_batch(MIGRATIONS[23].sql)
        .unwrap();

    let responsibility = |task_id: &str| {
        persistence
            .connection
            .query_row(
                "SELECT task.assignee_type,task.assignee_label,work.responsibility_type,work.responsibility_label
                 FROM business_tasks task JOIN my_work_items work ON work.task_id=task.business_task_id
                 WHERE task.business_task_id=?1",
                [task_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
            )
            .unwrap()
    };
    assert_eq!(
        responsibility("task-auto"),
        (
            "collaboration".to_owned(),
            "AI 对话协作".to_owned(),
            "collaboration".to_owned(),
            "AI 对话协作".to_owned(),
        )
    );
    assert_eq!(
        responsibility("task-manual"),
        (
            "agent".to_owned(),
            "研发 Agent".to_owned(),
            "agent".to_owned(),
            "研发 Agent".to_owned(),
        )
    );
}

#[test]
fn version_twenty_six_resolves_only_frontend_inferred_run_incidents() {
    let fixture = TestDatabase::new("upgrade-v26-run-incidents");
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    for migration in &MIGRATIONS[..25] {
        apply_fixture_migration(&connection, migration);
        connection
            .execute(
                "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                params![migration.version, migration.name, migration_checksum(migration.sql)],
            )
            .unwrap();
    }
    connection.execute_batch(
        "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('project-ai','AI 项目','验证运行异常归属','active',0,'op-project-ai','fp-project-ai',1,1);
         INSERT INTO attention_cases(attention_case_id,case_type,project_id,title,detail,status,responsibility_type,responsibility_label,reporter_label,priority,agent_run_id,agent_label,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('frontend-incident','agent_run_incident','project-ai','历史失败','容量不足','open','user','user','desktop','medium','run-frontend','Codex',0,'desktop-agent-incident-run-frontend','fp-frontend',2,2);
         INSERT INTO attention_cases(attention_case_id,case_type,project_id,title,detail,status,responsibility_type,responsibility_label,reporter_label,priority,agent_run_id,agent_label,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('runtime-incident','agent_run_incident','project-ai','任务重试失败','运行失败','open','user','user','desktop','medium','run-runtime','Codex',0,'desktop-run-incident-run-runtime','fp-runtime',3,3);",
    ).unwrap();
    drop(connection);

    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let status = |case_id: &str| {
        persistence
            .connection
            .query_row(
                "SELECT status,resolution_summary FROM attention_cases WHERE attention_case_id=?1",
                [case_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .unwrap()
    };
    let frontend = status("frontend-incident");
    assert_eq!(frontend.0, "resolved");
    assert!(frontend.1.unwrap().contains("不再作为任务阻塞依据"));
    assert_eq!(status("runtime-incident"), ("open".to_owned(), None));
    let transition_count: i64 = persistence
        .connection
        .query_row(
            "SELECT COUNT(*) FROM attention_case_transitions WHERE attention_case_id='frontend-incident'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(transition_count, 1);
}

#[test]
fn upgrading_legacy_classification_failure_preserves_history_without_work_item() {
    let fixture = TestDatabase::new("upgrade-v41-classification-incident");
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    for migration in &MIGRATIONS[..40] {
        apply_fixture_migration(&connection, migration);
        connection
            .execute(
                "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                params![migration.version, migration.name, migration_checksum(migration.sql)],
            )
            .unwrap();
    }
    connection.execute_batch(
        "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('project-ai','图片项目','验证归类失败投影','active',0,'op-project-ai','fp-project-ai',1,1);",
    ).unwrap();
    let conversation = serde_json::json!({
        "conversationId":"conversation-ai",
        "title":"生成产品配图",
        "archived":false,
        "context":{"projectId":"project-ai","phaseId":"removed-phase","taskId":null}
    });
    let run = serde_json::json!({
        "runId":"run-classification-failed",
        "conversationId":"conversation-ai",
        "inputMessageId":"message-ai",
        "status":"completed",
        "classificationStatus":"failed",
        "classificationError":"工作归类输出未完成",
        "startedAt":10,
        "completedAt":20
    });
    connection
        .execute(
            "INSERT INTO ai_conversations(conversation_id,position,conversation_json) VALUES ('conversation-ai',0,?1)",
            [conversation.to_string()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO ai_runs(run_id,position,run_json) VALUES ('run-classification-failed',0,?1)",
            [run.to_string()],
        )
        .unwrap();
    drop(connection);

    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    assert!(my_work_items(&mut persistence).is_empty());
    let incident: (String, Option<String>) = persistence
        .connection
        .query_row(
            "SELECT status,task_id FROM attention_cases WHERE agent_run_id='run-classification-failed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(incident, ("open".to_owned(), None));
}

#[test]
fn version_forty_two_projects_existing_failed_tool_call_onto_its_task() {
    let fixture = TestDatabase::new("upgrade-v42-tool-failure-incident");
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    for migration in &MIGRATIONS[..41] {
        apply_fixture_migration(&connection, migration);
        connection
            .execute(
                "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                params![migration.version, migration.name, migration_checksum(migration.sql)],
            )
            .unwrap();
    }
    connection.execute_batch(
        "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('project-ai','图片项目','验证工具失败投影','active',0,'op-project-ai','fp-project-ai',1,1);
         INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
         VALUES ('task-ai','project-ai',NULL,'创建生图技能','目标','','产出','标准','collaboration','AI 对话协作','medium',NULL,'in_progress',0,'ai-task-create:run-failed','fp-task',2,2,1);",
    ).unwrap();
    let conversation = serde_json::json!({
        "conversationId":"conversation-ai",
        "title":"创建生图技能",
        "archived":false,
        "context":{"projectId":"project-ai","taskId":"task-ai"}
    });
    let run = serde_json::json!({
        "runId":"run-failed",
        "conversationId":"conversation-ai",
        "status":"completed",
        "model":"agnes-3.0-flash",
        "completedAt":20,
        "events":[{
            "sequence":13,
            "eventType":"item.completed",
            "phase":"reply",
            "kind":"mcp",
            "status":"failed",
            "summary":"generate_image 调用失败"
        }]
    });
    connection
        .execute(
            "INSERT INTO ai_conversations(conversation_id,position,conversation_json) VALUES ('conversation-ai',0,?1)",
            [conversation.to_string()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO ai_runs(run_id,position,run_json) VALUES ('run-failed',0,?1)",
            [run.to_string()],
        )
        .unwrap();
    connection.execute(
        "INSERT INTO conversation_work_classifications(
            classification_id,run_id,conversation_id,project_id,business_task_id,
            classification_action,task_action,conversation_status,stage,outcome_title,
            outcome_summary,reason,confidence,raw_json,created_at
         ) VALUES ('classification:run-failed','run-failed','conversation-ai','project-ai','task-ai',
                   'keep','keep','active','in_progress','生图失败','需要处理','生图工具失败',0.9,'{}',20)",
        [],
    ).unwrap();
    drop(connection);

    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let incident: (String, String, String) = persistence
        .connection
        .query_row(
            "SELECT status,task_id,detail FROM attention_cases WHERE agent_run_id='run-failed'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        incident,
        (
            "open".to_owned(),
            "task-ai".to_owned(),
            "generate_image 调用失败".to_owned()
        )
    );
    let item = my_work_items(&mut persistence)
        .into_iter()
        .find(|item| item.task_id.as_deref() == Some("task-ai"))
        .unwrap();
    assert_eq!(item.focus_group, "todo");
    assert_eq!(item.blocking_run_id.as_deref(), Some("run-failed"));
    assert_eq!(
        item.blocking_reason.as_deref(),
        Some("generate_image 调用失败")
    );
}

fn seed_version_forty_four_shared_workspace(path: &Path) -> i64 {
    let connection = Connection::open(path).unwrap();
    for migration in &MIGRATIONS[..44] {
        apply_fixture_migration(&connection, migration);
        connection.execute(
            "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
            params![migration.version, migration.name, migration_checksum(migration.sql)],
        ).unwrap();
    }
    connection.execute_batch(
        "INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at)
         VALUES ('shared','共享目录','/tmp/shared','ready',0,1,1);
         INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('a','甲','验收','active',0,'a','a',1,1),('b','乙','验收','active',0,'b','b',1,1);
         INSERT INTO project_workspaces VALUES ('a','shared','primary',1);
         INSERT INTO direct_cloud_workspace_operations VALUES ('direct','direct','a','team','shared',1);
         INSERT INTO cloud_workspace_operations(client_operation_id,request_fingerprint,operation_kind,project_id,team_space_id,display_name,managed_root,device_id,state,workspace_id,created_at,updated_at)
         VALUES ('legacy','legacy','attach_existing','a','team','空间','/tmp/shared','device','bound','shared',1,1);
         INSERT INTO cloud_workspace_transfer_entries(client_operation_id,local_relative_path,remote_relative_path,entry_kind,size_bytes,checksum_sha256,transfer_state,updated_at)
         VALUES ('legacy','keep.txt','keep.txt','file',4,'hash','completed',1);"
    ).unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO project_workspaces VALUES ('b','shared','primary',2)",
                []
            )
            .is_err()
    );
    let triggers: i64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='trigger'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    triggers
}

#[test]
fn version_forty_four_links_upgrade_to_shared_workspaces_and_keep_operation_history() {
    let fixture = TestDatabase::new("upgrade-v44-shared-workspaces");
    let triggers = seed_version_forty_four_shared_workspace(&fixture.path);
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let connection = &persistence.connection;
    connection
        .execute(
            "INSERT INTO project_workspaces VALUES ('b','shared','primary',2)",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO project_workspaces VALUES ('b','shared','supporting',3)",
                []
            )
            .is_err()
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='trigger'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        triggers
    );
    assert_eq!(
        connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("PRAGMA legacy_alter_table", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert_eq!(connection.query_row("SELECT count(*) FROM cloud_workspace_transfer_entries WHERE client_operation_id='legacy'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    let delete = DeleteProjectCommand {
        project_id: "a".to_owned(),
        client_operation_id: "delete-a".to_owned(),
        request_fingerprint: "delete-a".to_owned(),
        event_id: EventId::new("delete-a"),
        correlation_id: "delete-a".to_owned(),
        occurred_at: 3,
    };
    persistence
        .transact(|repositories| repositories.delete_project(&delete))
        .unwrap();
    assert_eq!(
        persistence
            .transact(
                |repositories| repositories.projects_for_workspace(&WorkspaceId::new("shared"))
            )
            .unwrap()[0]
            .project_id,
        "b"
    );
    assert_eq!(
        persistence
            .connection
            .query_row(
                "SELECT count(*) FROM direct_cloud_workspace_operations",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        persistence
            .cloud_workspace_operation("legacy")
            .unwrap()
            .unwrap()
            .project_id,
        "a"
    );
    drop(persistence);
    SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
}

#[test]
fn version_eleven_actions_are_rebuilt_into_my_work_projection() {
    let fixture = TestDatabase::new("upgrade-v11-my-work");
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    for migration in &MIGRATIONS[..11] {
        apply_fixture_migration(&connection, migration);
        connection
            .execute(
                "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                params![migration.version, migration.name, migration_checksum(migration.sql)],
            )
            .unwrap();
    }
    connection.execute_batch(
        "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
         VALUES ('w-upgrade','升级工作区','/tmp/w-upgrade',NULL,'ready',0,NULL,1,1);
         INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('p-upgrade','升级项目','验证投影','active',0,'op-p-upgrade','fp-p-upgrade',1,1);
         INSERT INTO project_workspaces(project_id,workspace_id,role,created_at)
         VALUES ('p-upgrade','w-upgrade','primary',1);
         INSERT INTO deliverables(deliverable_id,project_id,title,deliverable_type,owner_label,acceptance_criteria,status,current_accepted_version_id,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('d-upgrade','p-upgrade','升级成果','document','Mika','可验收','candidate',NULL,1,'op-d-upgrade','fp-d-upgrade',2,2);
         INSERT INTO deliverable_versions(version_id,deliverable_id,version_no,status,summary,client_operation_id,request_fingerprint,created_at)
         VALUES ('dv-upgrade','d-upgrade',1,'candidate','候选','op-dv-upgrade','fp-dv-upgrade',3);
         INSERT INTO phase_plans(plan_id,project_id,version_no,status,summary,state_revision,client_operation_id,request_fingerprint,created_at,activated_at)
         VALUES ('pp-upgrade','p-upgrade',1,'active','升级计划',1,'op-pp-upgrade','fp-pp-upgrade',4,4);
         INSERT INTO project_phases(phase_id,plan_id,project_id,name,goal,acceptance_criteria,required_deliverable_id,sort_order,is_main,status,state_revision,created_at,updated_at)
         VALUES ('ph-upgrade','pp-upgrade','p-upgrade','升级阶段','形成成果','通过验收','d-upgrade',0,1,'review',2,4,6);
         INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('bt-upgrade','p-upgrade','ph-upgrade','升级任务','验证','上下文','结果','完成','user','Mika','high',100,'in_progress',2,'op-bt-upgrade','fp-bt-upgrade',5,7);",
    ).unwrap();
    drop(connection);

    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let items = my_work_items(&mut persistence);
    assert_eq!(items.len(), 3);
    assert!(
        items
            .iter()
            .any(|item| item.source_type == "business_task" && item.focus_group == "in_progress")
    );
    assert!(
        items
            .iter()
            .any(|item| item.source_type == "deliverable_version" && item.focus_group == "review")
    );
    assert!(
        items
            .iter()
            .any(|item| item.source_type == "project_phase" && item.focus_group == "decision")
    );
    assert!(
        items
            .iter()
            .all(|item| item.primary_workspace_id.as_deref() == Some("w-upgrade"))
    );
}

#[test]
fn existing_version_59_database_preserves_context_and_memory_on_reopen() {
    let fixture = TestDatabase::new("existing-v59");
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version <= 58)
    {
        apply_fixture_migration(&connection, migration);
        connection
            .execute(
                "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (?1, ?2, ?3, 1, '0.1.0')",
                params![migration.version, migration.name, migration_checksum(migration.sql)],
            )
            .unwrap();
    }
    // 模拟已经由先前客户端写入的第 59 版资料库，保留原迁移记录与数据。
    let existing = Migration {
        version: 59,
        name: "add_workbench_context_imports",
        sql: include_str!("../../../migrations/0059_add_workbench_context_imports.sql"),
        rebuilds_foreign_key_parent: false,
    };
    assert_eq!(
        migration_checksum(existing.sql),
        "46b9783b457fb519cdcb740223f9ee30a2dc494ea83a83a376e37d19bd2c10bd"
    );
    apply_fixture_migration(&connection, &existing);
    connection
        .execute(
            "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (59, ?1, ?2, 123, 'previous-client')",
            params![existing.name, migration_checksum(existing.sql)],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO ai_context_imports VALUES ('workbench', 'thread-fixture', 'digest-fixture', '{\"source\":\"fixture\"}', 'committed', 456)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO ai_memory_state VALUES ('account', 7, '{\"memory\":\"fixture\"}')",
            [],
        )
        .unwrap();
    drop(connection);

    for _ in 0..2 {
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let history: (i64, String) = persistence
            .connection
            .query_row(
                "SELECT applied_at, app_version FROM schema_migrations WHERE version = 59",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(history, (123, "previous-client".to_owned()));
        let context: (String, String, i64) = persistence
            .connection
            .query_row(
                "SELECT source_json, status, updated_at FROM ai_context_imports WHERE scope = 'workbench' AND thread_id = 'thread-fixture'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            context,
            (
                "{\"source\":\"fixture\"}".to_owned(),
                "committed".to_owned(),
                456
            )
        );
        let memory: (i64, String) = persistence
            .connection
            .query_row(
                "SELECT revision, value_json FROM ai_memory_state WHERE state_key = 'account'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(memory, (7, "{\"memory\":\"fixture\"}".to_owned()));
    }
}

#[test]
fn database_with_future_migration_is_rejected_without_removing_history() {
    let fixture = TestDatabase::new("future-migration");
    drop(SqlitePersistence::open(&fixture.path, "0.1.0").unwrap());
    let future_version = MIGRATIONS.last().unwrap().version + 1;
    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (?1, 'future-migration', ?2, 456, 'future-client')",
            params![future_version, "0".repeat(64)],
        )
        .unwrap();
    drop(connection);
    let error = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap_err();
    assert_eq!(error.kind(), PersistenceErrorKind::Migration);
    assert!(error.to_string().contains("newer than this App Server"));
    let connection = Connection::open(&fixture.path).unwrap();
    let retained_version: i64 = connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(retained_version, future_version);
}

#[test]
fn reopening_is_idempotent_and_changed_history_is_rejected() {
    let fixture = TestDatabase::new("history");
    drop(SqlitePersistence::open(&fixture.path, "0.1.0").unwrap());
    drop(SqlitePersistence::open(&fixture.path, "0.1.0").unwrap());

    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute(
            "UPDATE schema_migrations SET checksum = ?1 WHERE version = 1",
            ["0".repeat(64)],
        )
        .unwrap();
    drop(connection);
    let error = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap_err();
    assert_eq!(error.kind(), PersistenceErrorKind::Migration);
    assert!(error.to_string().contains("does not match"));
}

#[test]
fn version_one_database_upgrades_to_core_schema_without_reapplying_history() {
    let fixture = TestDatabase::new("upgrade-v1");
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    connection.execute_batch(MIGRATIONS[0].sql).unwrap();
    connection
        .execute(
            "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (1, ?1, ?2, 1, '0.1.0')",
            params![MIGRATIONS[0].name, migration_checksum(MIGRATIONS[0].sql)],
        )
        .unwrap();
    drop(connection);

    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let versions: String = persistence
        .connection
        .query_row(
            "SELECT group_concat(version, ',') FROM (SELECT version FROM schema_migrations ORDER BY version)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let expected_versions = MIGRATIONS
        .iter()
        .map(|migration| migration.version.to_string())
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(versions, expected_versions);
    let account_table_exists: bool = persistence
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='accounts')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(account_table_exists);
}

#[test]
fn deleting_a_project_removes_process_data_releases_workspaces_and_keeps_history() {
    let fixture = TestDatabase::new("project-delete");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_project_outcome_file(&mut persistence);

    let create = CreateProjectCommand {
        project_id: "project-delete-1".to_owned(),
        display_name: "待删除项目".to_owned(),
        goal: "验证删除范围".to_owned(),
        primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
        client_operation_id: "op-delete-project".to_owned(),
        request_fingerprint: "fp-delete-project".to_owned(),
        event_id: EventId::new("event-delete-project"),
        correlation_id: "correlation-delete-project".to_owned(),
        occurred_at: 3,
    };
    persistence
        .transact(|repositories| contexthub_application::create_project(repositories, &create))
        .unwrap();

    let events_before: i64 = persistence
        .connection
        .query_row("SELECT COUNT(*) FROM durable_events", [], |row| row.get(0))
        .unwrap();

    let delete = DeleteProjectCommand {
        project_id: "project-delete-1".to_owned(),
        client_operation_id: "op-delete-1".to_owned(),
        request_fingerprint: "fp-delete-1".to_owned(),
        event_id: EventId::new("event-delete-1"),
        correlation_id: "correlation-delete-1".to_owned(),
        occurred_at: 4,
    };
    let summary = persistence
        .transact(|repositories| contexthub_application::delete_project(repositories, &delete))
        .unwrap();
    assert_eq!(summary.project_id, "project-delete-1");
    assert_eq!(
        summary
            .released_workspace_ids
            .iter()
            .map(|workspace| workspace.as_str().to_owned())
            .collect::<Vec<String>>(),
        vec!["w-snapshot".to_owned()]
    );

    // The project and its binding are gone, so the workspace can be reused.
    let project_rows: i64 = persistence
        .connection
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(project_rows, 0);
    let binding_rows: i64 = persistence
        .connection
        .query_row("SELECT COUNT(*) FROM project_workspaces", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(binding_rows, 0);

    // The workspace itself survives: the files on disk stay browsable.
    let workspace_rows: i64 = persistence
        .connection
        .query_row(
            "SELECT COUNT(*) FROM workspaces WHERE workspace_id='w-snapshot'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(workspace_rows, 1);

    // History is an audit trail: deleting a project does not rewrite what happened.
    let events_after: i64 = persistence
        .connection
        .query_row("SELECT COUNT(*) FROM durable_events", [], |row| row.get(0))
        .unwrap();
    assert!(events_after > events_before);

    // Deleting again is refused rather than silently succeeding.
    let error = persistence
        .transact(|repositories| contexthub_application::delete_project(repositories, &delete))
        .unwrap_err();
    assert!(format!("{error:?}").contains("project_not_found"));
}

#[test]
fn deleting_an_unowned_workspace_clears_its_records_but_refuses_while_still_in_a_project() {
    let fixture = TestDatabase::new("workspace-delete");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_project_outcome_file(&mut persistence);
    let create = CreateProjectCommand {
        project_id: "project-ws-delete".to_owned(),
        display_name: "占用项目".to_owned(),
        goal: "占用主文件夹".to_owned(),
        primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
        client_operation_id: "op-ws-delete".to_owned(),
        request_fingerprint: "fp-ws-delete".to_owned(),
        event_id: EventId::new("event-ws-delete"),
        correlation_id: "correlation-ws-delete".to_owned(),
        occurred_at: 3,
    };
    persistence
        .transact(|repositories| contexthub_application::create_project(repositories, &create))
        .unwrap();

    // A workspace that still anchors a project must not be deletable.
    let bound = DeleteWorkspaceCommand {
        workspace_id: WorkspaceId::new("w-snapshot"),
        client_operation_id: "op-del-bound".to_owned(),
        request_fingerprint: "fp-del-bound".to_owned(),
        occurred_at: 4,
    };
    let error = persistence
        .transact(|repositories| {
            contexthub_application::delete_workspace(
                repositories,
                &bound,
                EventId::new("event-del-bound"),
            )
        })
        .unwrap_err();
    assert!(format!("{error:?}").contains("workspace_still_in_project"));

    // An unowned workspace can be removed, taking its own history with it.
    persistence
        .connection
        .execute_batch(
            "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
             VALUES ('w-orphan','闲置文件夹','/tmp/w-orphan',NULL,'ready',0,NULL,1,1);
             INSERT INTO durable_events(event_id,global_sequence,workspace_id,workspace_sequence,event_type,object_type,object_id,payload_json,correlation_id,occurred_at,created_at)
             VALUES ('event-orphan-1',9001,'w-orphan',1,'workspace.connected','workspace','w-orphan','{}',NULL,1,1);",
        )
        .unwrap();
    let orphan = DeleteWorkspaceCommand {
        workspace_id: WorkspaceId::new("w-orphan"),
        client_operation_id: "op-del-orphan".to_owned(),
        request_fingerprint: "fp-del-orphan".to_owned(),
        occurred_at: 5,
    };
    let summary = persistence
        .transact(|repositories| {
            contexthub_application::delete_workspace(
                repositories,
                &orphan,
                EventId::new("event-del-orphan"),
            )
        })
        .unwrap();
    assert_eq!(summary.removed_events, 1);

    let remaining: i64 = persistence
        .connection
        .query_row(
            "SELECT COUNT(*) FROM workspaces WHERE workspace_id='w-orphan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
    let orphan_events: i64 = persistence
        .connection
        .query_row(
            "SELECT COUNT(*) FROM durable_events WHERE workspace_id='w-orphan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(orphan_events, 0);
    // The deletion itself stays in the audit trail, recorded without a workspace reference.
    let deletion_recorded: i64 = persistence
        .connection
        .query_row(
            "SELECT COUNT(*) FROM durable_events WHERE event_type='workspace.deleted' AND object_id='w-orphan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(deletion_recorded, 1);
}

#[test]
fn project_can_be_created_without_a_workspace_and_attach_one_later() {
    let fixture = TestDatabase::new("project-without-workspace");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_project_outcome_file(&mut persistence);
    let create = CreateProjectCommand {
        project_id: "project-empty-1".to_owned(),
        display_name: "空项目".to_owned(),
        goal: "先规划后关联文件".to_owned(),
        primary_workspace_id: None,
        client_operation_id: "op-empty-project".to_owned(),
        request_fingerprint: "fp-empty-project".to_owned(),
        event_id: EventId::new("event-empty-project"),
        correlation_id: "correlation-empty-project".to_owned(),
        occurred_at: 3,
    };
    let project = persistence
        .transact(|repositories| contexthub_application::create_project(repositories, &create))
        .unwrap();
    assert_eq!(project.primary_workspace_id, None);

    let attach = AttachProjectWorkspaceCommand {
        project_id: create.project_id.clone(),
        workspace_id: WorkspaceId::new("w-snapshot"),
        client_operation_id: "op-empty-attach".to_owned(),
        request_fingerprint: "fp-empty-attach".to_owned(),
        event_id: EventId::new("event-empty-attach"),
        correlation_id: "correlation-empty-attach".to_owned(),
        occurred_at: 4,
    };
    let attached = persistence
        .transact(|repositories| {
            contexthub_application::attach_project_workspace(repositories, &attach)
        })
        .unwrap();
    assert_eq!(
        attached
            .primary_workspace_id
            .as_ref()
            .map(WorkspaceId::as_str),
        Some("w-snapshot")
    );
}

#[test]
fn every_project_workspace_can_be_detached_and_the_project_may_be_empty() {
    let fixture = TestDatabase::new("project-detach-workspace");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_project_outcome_file(&mut persistence);
    persistence
        .connection
        .execute_batch(
            "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
             VALUES ('w-second','第二文件夹','/tmp/w-second',NULL,'ready',0,NULL,1,1);",
        )
        .unwrap();
    let create = CreateProjectCommand {
        project_id: "project-detach-1".to_owned(),
        display_name: "多文件夹项目".to_owned(),
        goal: "验证移出".to_owned(),
        primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
        client_operation_id: "op-detach-project".to_owned(),
        request_fingerprint: "fp-detach-project".to_owned(),
        event_id: EventId::new("event-detach-project"),
        correlation_id: "correlation-detach-project".to_owned(),
        occurred_at: 3,
    };
    persistence
        .transact(|repositories| contexthub_application::create_project(repositories, &create))
        .unwrap();
    let attach = AttachProjectWorkspaceCommand {
        project_id: "project-detach-1".to_owned(),
        workspace_id: WorkspaceId::new("w-second"),
        client_operation_id: "op-detach-attach".to_owned(),
        request_fingerprint: "fp-detach-attach".to_owned(),
        event_id: EventId::new("event-detach-attach"),
        correlation_id: "correlation-detach-attach".to_owned(),
        occurred_at: 4,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::attach_project_workspace(repositories, &attach)
        })
        .unwrap();

    let detach_primary = DetachProjectWorkspaceCommand {
        project_id: "project-detach-1".to_owned(),
        workspace_id: WorkspaceId::new("w-snapshot"),
        client_operation_id: "op-detach-primary".to_owned(),
        request_fingerprint: "fp-detach-primary".to_owned(),
        event_id: EventId::new("event-detach-primary"),
        correlation_id: "correlation-detach-primary".to_owned(),
        occurred_at: 5,
    };
    let promoted = persistence
        .transact(|repositories| {
            contexthub_application::detach_project_workspace(repositories, &detach_primary)
        })
        .unwrap();
    assert_eq!(
        promoted
            .primary_workspace_id
            .as_ref()
            .map(WorkspaceId::as_str),
        Some("w-second")
    );

    let detach = DetachProjectWorkspaceCommand {
        workspace_id: WorkspaceId::new("w-second"),
        client_operation_id: "op-detach-1".to_owned(),
        request_fingerprint: "fp-detach-1".to_owned(),
        event_id: EventId::new("event-detach-1"),
        correlation_id: "correlation-detach-1".to_owned(),
        ..detach_primary.clone()
    };
    let detached = persistence
        .transact(|repositories| {
            contexthub_application::detach_project_workspace(repositories, &detach)
        })
        .unwrap();
    assert!(!detached.replayed);

    // The last folder can also leave; the project remains as an empty business container.
    let remaining: i64 = persistence
        .connection
        .query_row(
            "SELECT COUNT(*) FROM project_workspaces WHERE project_id='project-detach-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
    assert_eq!(detached.primary_workspace_id, None);

    // Repeating the detach is a replay rather than an error.
    let replayed = persistence
        .transact(|repositories| {
            contexthub_application::detach_project_workspace(repositories, &detach)
        })
        .unwrap();
    assert!(replayed.replayed);
}

#[test]
fn attaching_a_workspace_extends_a_project_without_duplicate_links() {
    let fixture = TestDatabase::new("project-attach-workspace");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_project_outcome_file(&mut persistence);
    persistence
        .connection
        .execute_batch(
            "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
             VALUES ('w-second','第二工作区','/tmp/w-second',NULL,'ready',0,NULL,1,1);
             INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
             VALUES ('w-third','第三工作区','/tmp/w-third',NULL,'ready',0,NULL,1,1);",
        )
        .unwrap();

    let create = CreateProjectCommand {
        project_id: "project-attach-1".to_owned(),
        display_name: "多工作区项目".to_owned(),
        goal: "验证一个项目持有多个工作区".to_owned(),
        primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
        client_operation_id: "op-attach-project".to_owned(),
        request_fingerprint: "fp-attach-project".to_owned(),
        event_id: EventId::new("event-attach-project"),
        correlation_id: "correlation-attach-project".to_owned(),
        occurred_at: 3,
    };
    persistence
        .transact(|repositories| contexthub_application::create_project(repositories, &create))
        .unwrap();

    let attach = AttachProjectWorkspaceCommand {
        project_id: "project-attach-1".to_owned(),
        workspace_id: WorkspaceId::new("w-second"),
        client_operation_id: "op-attach-1".to_owned(),
        request_fingerprint: "fp-attach-1".to_owned(),
        event_id: EventId::new("event-attach-1"),
        correlation_id: "correlation-attach-1".to_owned(),
        occurred_at: 4,
    };
    let attached = persistence
        .transact(|repositories| {
            contexthub_application::attach_project_workspace(repositories, &attach)
        })
        .unwrap();
    assert!(!attached.replayed);

    // The project now holds both workspaces, which is the whole point of the capability.
    let workspaces = persistence
        .transact(|repositories| repositories.workspaces_for_project("project-attach-1"))
        .unwrap();
    let mut ids: Vec<String> = workspaces
        .iter()
        .map(|workspace| workspace.as_str().to_owned())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["w-second".to_owned(), "w-snapshot".to_owned()]);

    // Re-attaching the same pair is a replay, not a duplicate row.
    let replayed = persistence
        .transact(|repositories| {
            contexthub_application::attach_project_workspace(repositories, &attach)
        })
        .unwrap();
    assert!(replayed.replayed);
    let binding_count: i64 = persistence
        .connection
        .query_row(
            "SELECT COUNT(*) FROM project_workspaces WHERE project_id='project-attach-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(binding_count, 2);
}

#[test]
fn attaching_a_workspace_shares_across_projects_and_refuses_unknown_workspaces() {
    let fixture = TestDatabase::new("project-attach-workspace-rejects");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_project_outcome_file(&mut persistence);
    persistence
        .connection
        .execute_batch(
            "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
             VALUES ('w-third','第三工作区','/tmp/w-third',NULL,'ready',0,NULL,1,1);",
        )
        .unwrap();
    let create = CreateProjectCommand {
        project_id: "project-attach-1".to_owned(),
        display_name: "多工作区项目".to_owned(),
        goal: "验证一个项目持有多个工作区".to_owned(),
        primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
        client_operation_id: "op-attach-project".to_owned(),
        request_fingerprint: "fp-attach-project".to_owned(),
        event_id: EventId::new("event-attach-project"),
        correlation_id: "correlation-attach-project".to_owned(),
        occurred_at: 3,
    };
    persistence
        .transact(|repositories| contexthub_application::create_project(repositories, &create))
        .unwrap();

    let second_project = CreateProjectCommand {
        project_id: "project-attach-2".to_owned(),
        display_name: "另一个项目".to_owned(),
        goal: "占用第三工作区".to_owned(),
        primary_workspace_id: Some(WorkspaceId::new("w-third")),
        client_operation_id: "op-attach-project-2".to_owned(),
        request_fingerprint: "fp-attach-project-2".to_owned(),
        event_id: EventId::new("event-attach-project-2"),
        correlation_id: "correlation-attach-project-2".to_owned(),
        occurred_at: 5,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::create_project(repositories, &second_project)
        })
        .unwrap();
    let stolen = AttachProjectWorkspaceCommand {
        project_id: "project-attach-1".to_owned(),
        workspace_id: WorkspaceId::new("w-third"),
        client_operation_id: "op-attach-2".to_owned(),
        request_fingerprint: "fp-attach-2".to_owned(),
        event_id: EventId::new("event-attach-2"),
        correlation_id: "correlation-attach-2".to_owned(),
        occurred_at: 6,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::attach_project_workspace(repositories, &stolen)
        })
        .unwrap();
    assert_eq!(
        persistence
            .transact(
                |repositories| repositories.projects_for_workspace(&WorkspaceId::new("w-third"))
            )
            .unwrap()
            .len(),
        2
    );

    // Unknown workspaces are refused too, so a typo cannot silently create a dangling binding.
    let missing = AttachProjectWorkspaceCommand {
        workspace_id: WorkspaceId::new("w-does-not-exist"),
        client_operation_id: "op-attach-3".to_owned(),
        request_fingerprint: "fp-attach-3".to_owned(),
        event_id: EventId::new("event-attach-3"),
        correlation_id: "correlation-attach-3".to_owned(),
        ..stolen.clone()
    };
    let error = persistence
        .transact(|repositories| {
            contexthub_application::attach_project_workspace(repositories, &missing)
        })
        .unwrap_err();
    assert!(format!("{error:?}").contains("project_workspace_not_found"));
}

#[test]
fn project_rename_advances_the_revision_and_refuses_a_stale_one() {
    let fixture = TestDatabase::new("project-update");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_project_outcome_file(&mut persistence);
    let created = persistence
        .transact(|repositories| {
            contexthub_application::create_project(
                repositories,
                &CreateProjectCommand {
                    project_id: "project-rename-1".to_owned(),
                    display_name: "旧名".to_owned(),
                    goal: "旧目标".to_owned(),
                    primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
                    client_operation_id: "op-rename-1".to_owned(),
                    request_fingerprint: "fp-rename-1".to_owned(),
                    event_id: EventId::new("event-rename-1"),
                    correlation_id: "correlation-rename-1".to_owned(),
                    occurred_at: 3,
                },
            )
        })
        .unwrap();

    let renamed = persistence
        .transact(|repositories| {
            contexthub_application::update_project(
                repositories,
                &UpdateProjectCommand {
                    project_id: created.project_id.clone(),
                    display_name: "新名".to_owned(),
                    goal: "新目标".to_owned(),
                    expected_revision: created.state_revision,
                    event_id: EventId::new("event-rename-2"),
                    correlation_id: "correlation-rename-2".to_owned(),
                    occurred_at: 4,
                },
            )
        })
        .unwrap();
    assert_eq!(renamed.display_name, "新名");
    assert_eq!(renamed.goal, "新目标");
    assert_eq!(renamed.state_revision, created.state_revision + 1);

    // Re-sending the same edit now carries a revision the project has moved past; accepting it
    // would silently overwrite whatever happened in between.
    let stale = persistence.transact(|repositories| {
        contexthub_application::update_project(
            repositories,
            &UpdateProjectCommand {
                project_id: created.project_id.clone(),
                display_name: "更新的名".to_owned(),
                goal: "更新的目标".to_owned(),
                expected_revision: created.state_revision,
                event_id: EventId::new("event-rename-3"),
                correlation_id: "correlation-rename-3".to_owned(),
                occurred_at: 5,
            },
        )
    });
    assert!(stale.is_err(), "stale revision must be refused");
}

#[test]
fn project_rename_refuses_an_unknown_project() {
    let fixture = TestDatabase::new("project-update-missing");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_project_outcome_file(&mut persistence);
    let result = persistence.transact(|repositories| {
        contexthub_application::update_project(
            repositories,
            &UpdateProjectCommand {
                project_id: "project-does-not-exist".to_owned(),
                display_name: "任意".to_owned(),
                goal: "任意".to_owned(),
                expected_revision: 0,
                event_id: EventId::new("event-rename-missing"),
                correlation_id: "correlation-rename-missing".to_owned(),
                occurred_at: 4,
            },
        )
    });
    assert!(result.is_err());
}

#[test]
fn project_deliverable_file_version_and_acceptance_form_one_persisted_flow() {
    let fixture = TestDatabase::new("project-outcomes");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_project_outcome_file(&mut persistence);

    let project_command = CreateProjectCommand {
        project_id: "project-local-1".to_owned(),
        display_name: "本地项目".to_owned(),
        goal: "形成可验收成果".to_owned(),
        primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
        client_operation_id: "op-project-1".to_owned(),
        request_fingerprint: "fp-project-1".to_owned(),
        event_id: EventId::new("event-project-1"),
        correlation_id: "correlation-project-1".to_owned(),
        occurred_at: 3,
    };
    let project = persistence
        .transact(|repositories| {
            contexthub_application::create_project(repositories, &project_command)
        })
        .unwrap();
    assert_eq!(project.status, ProjectStatus::Active);
    let replayed_project = persistence
        .transact(|repositories| {
            contexthub_application::create_project(repositories, &project_command)
        })
        .unwrap();
    assert!(replayed_project.replayed);

    let deliverable_command = CreateDeliverableCommand {
        deliverable_id: "deliverable-1".to_owned(),
        project_id: project.project_id.clone(),
        title: "需求方案".to_owned(),
        deliverable_type: "document".to_owned(),
        owner_label: "Mika".to_owned(),
        acceptance_criteria: "范围、决策和边界完整".to_owned(),
        client_operation_id: "op-deliverable-1".to_owned(),
        request_fingerprint: "fp-deliverable-1".to_owned(),
        event_id: EventId::new("event-deliverable-1"),
        correlation_id: "correlation-deliverable-1".to_owned(),
        occurred_at: 4,
    };
    let deliverable = persistence
        .transact(|repositories| {
            contexthub_application::create_deliverable(repositories, &deliverable_command)
        })
        .unwrap();
    assert_eq!(deliverable.status, DeliverableStatus::Draft);

    let version_command = CreateDeliverableVersionCommand {
        version_id: "deliverable-version-1".to_owned(),
        deliverable_id: deliverable.deliverable_id.clone(),
        summary: "首轮候选".to_owned(),
        local_file_ids: vec!["project-file".to_owned()],
        carriers: vec![commit_carrier()],
        client_operation_id: "op-version-1".to_owned(),
        request_fingerprint: "fp-version-1".to_owned(),
        event_id: EventId::new("event-version-1"),
        correlation_id: "correlation-version-1".to_owned(),
        occurred_at: 5,
    };
    let version = persistence
        .transact(|repositories| {
            contexthub_application::create_deliverable_version(repositories, &version_command)
        })
        .unwrap();
    assert_eq!(version.status, DeliverableVersionStatus::Candidate);
    assert_eq!(version.bindings[0].relative_path, "交付件/方案.md");
    assert_eq!(version.carriers.len(), 1);
    assert_eq!(version.carriers[0].carrier_type, "commit");
    assert_eq!(version.carriers[0].reference, "abc1234");
    let deliverable_review = my_work_items(&mut persistence);
    assert_eq!(deliverable_review.len(), 1);
    assert_eq!(deliverable_review[0].source_type, "deliverable_version");
    assert_eq!(deliverable_review[0].focus_group, "review");

    let review_command = ReviewDeliverableVersionCommand {
        acceptance_id: "acceptance-1".to_owned(),
        deliverable_id: deliverable.deliverable_id,
        version_id: version.version_id,
        decision: AcceptanceDecision::Accepted,
        reviewer_label: "Mika".to_owned(),
        comment: "符合验收标准".to_owned(),
        expected_deliverable_revision: 1,
        client_operation_id: "op-review-1".to_owned(),
        request_fingerprint: "fp-review-1".to_owned(),
        event_id: EventId::new("event-review-1"),
        correlation_id: "correlation-review-1".to_owned(),
        occurred_at: 6,
    };
    let review = persistence
        .transact(|repositories| {
            contexthub_application::review_deliverable_version(repositories, &review_command)
        })
        .unwrap();
    assert_eq!(review.version.status, DeliverableVersionStatus::Accepted);
    assert_eq!(review.deliverable.status, DeliverableStatus::Accepted);
    assert_eq!(
        review.deliverable.current_accepted_version_id.as_deref(),
        Some("deliverable-version-1")
    );
    assert_eq!(review.deliverable.state_revision, 2);
    assert!(my_work_items(&mut persistence).is_empty());

    assert_project_outcome_events(&persistence);
}

#[test]
fn phase_plan_tasks_dependencies_and_acceptance_form_one_persisted_flow() {
    let fixture = TestDatabase::new("project-progress");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);

    create_active_phase_plan(&mut persistence);

    let blocked = phase_transition("phase-2", ProjectPhaseStatus::InProgress, 0, 12, "blocked");
    let error = persistence
        .transact(|repositories| {
            contexthub_application::transition_project_phase(repositories, &blocked)
        })
        .unwrap_err();
    assert_eq!(error.reason(), Some("phase_predecessor_incomplete"));

    transition_phase(
        &mut persistence,
        "phase-1",
        ProjectPhaseStatus::InProgress,
        0,
        13,
    );
    create_and_move_business_task(&mut persistence);
    advance_business_task_through_rework(&mut persistence);
    let attempts = persistence
        .transact(|repositories| repositories.phase_plans("project-progress-1"))
        .unwrap()[0]
        .phases[0]
        .tasks[0]
        .attempt_no;
    assert_eq!(attempts, 2);

    transition_phase(
        &mut persistence,
        "phase-1",
        ProjectPhaseStatus::Review,
        1,
        24,
    );
    let phase_review = my_work_items(&mut persistence);
    assert_eq!(phase_review.len(), 1);
    assert_eq!(phase_review[0].source_type, "project_phase");
    assert_eq!(phase_review[0].focus_group, "decision");
    assert_eq!(phase_review[0].phase_name.as_deref(), Some("需求定义"));
    transition_phase(
        &mut persistence,
        "phase-1",
        ProjectPhaseStatus::Completed,
        2,
        25,
    );
    assert!(my_work_items(&mut persistence).is_empty());
    let second = transition_phase(
        &mut persistence,
        "phase-2",
        ProjectPhaseStatus::InProgress,
        0,
        26,
    );
    assert!(second.is_main);
    assert_eq!(second.status, ProjectPhaseStatus::InProgress);
}

#[test]
fn phase_without_deliverable_can_be_planned_reviewed_and_completed() {
    let fixture = TestDatabase::new("project-progress-without-deliverable");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);
    let mut phase = phase_input("phase-without-deliverable", "探索", 0);
    phase.required_deliverable_id = None;
    let plan = persistence
        .transact(|repositories| {
            contexthub_application::create_phase_plan(
                repositories,
                &CreatePhasePlanCommand {
                    plan_id: "phase-plan-without-deliverable".to_owned(),
                    project_id: "project-progress-1".to_owned(),
                    summary: "先探索再决定产出".to_owned(),
                    phases: vec![phase],
                    client_operation_id: "op-phase-plan-without-deliverable".to_owned(),
                    request_fingerprint: "fp-phase-plan-without-deliverable".to_owned(),
                    event_id: EventId::new("event-phase-plan-without-deliverable"),
                    correlation_id: "correlation-phase-plan-without-deliverable".to_owned(),
                    occurred_at: 10,
                },
            )
        })
        .unwrap();
    assert!(plan.phases[0].required_deliverable_id.is_none());
    persistence
        .transact(|repositories| {
            contexthub_application::activate_phase_plan(
                repositories,
                &ActivatePhasePlanCommand {
                    plan_id: plan.plan_id,
                    expected_revision: plan.state_revision,
                    client_operation_id: "op-activate-without-deliverable".to_owned(),
                    request_fingerprint: "fp-activate-without-deliverable".to_owned(),
                    event_id: EventId::new("event-activate-without-deliverable"),
                    correlation_id: "correlation-activate-without-deliverable".to_owned(),
                    occurred_at: 11,
                },
            )
        })
        .unwrap();
    transition_phase(
        &mut persistence,
        "phase-without-deliverable",
        ProjectPhaseStatus::InProgress,
        0,
        12,
    );
    persistence.connection.execute(
        "INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
         VALUES ('task-without-deliverable','project-progress-1','phase-without-deliverable','完成探索','取得结论','','结论','人工确认','user','Mika','medium',NULL,'completed',0,'op-task-without-deliverable','fp-task-without-deliverable',13,13,1)",
        [],
    ).unwrap();
    let review = transition_phase(
        &mut persistence,
        "phase-without-deliverable",
        ProjectPhaseStatus::Review,
        1,
        14,
    );
    assert_eq!(review.status, ProjectPhaseStatus::Review);
    let completed = transition_phase(
        &mut persistence,
        "phase-without-deliverable",
        ProjectPhaseStatus::Completed,
        2,
        15,
    );
    assert_eq!(completed.status, ProjectPhaseStatus::Completed);
}

#[test]
fn explicit_dependency_graph_allows_parallel_successor_phases() {
    let fixture = TestDatabase::new("project-progress-parallel");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);
    let first = phase_input("phase-1", "共同输入", 0);
    let second = phase_input("phase-2", "并行设计", 1);
    let mut third = phase_input("phase-3", "并行验证", 2);
    third.predecessor_phase_ids = vec!["phase-1".to_owned()];
    let command = CreatePhasePlanCommand {
        plan_id: "phase-plan-parallel".to_owned(),
        project_id: "project-progress-1".to_owned(),
        summary: "一个共同前置与两个并行后继".to_owned(),
        phases: vec![first, second, third],
        client_operation_id: "op-phase-plan-parallel".to_owned(),
        request_fingerprint: "fp-phase-plan-parallel".to_owned(),
        event_id: EventId::new("event-phase-plan-parallel"),
        correlation_id: "correlation-phase-plan-parallel".to_owned(),
        occurred_at: 10,
    };
    let plan = persistence
        .transact(|repositories| contexthub_application::create_phase_plan(repositories, &command))
        .unwrap();
    assert_eq!(plan.phases[2].predecessor_phase_ids, vec!["phase-1"]);
    let activation = ActivatePhasePlanCommand {
        plan_id: plan.plan_id,
        expected_revision: 0,
        client_operation_id: "op-activate-parallel".to_owned(),
        request_fingerprint: "fp-activate-parallel".to_owned(),
        event_id: EventId::new("event-activate-parallel"),
        correlation_id: "correlation-activate-parallel".to_owned(),
        occurred_at: 11,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::activate_phase_plan(repositories, &activation)
        })
        .unwrap();
    persistence
        .connection
        .execute(
            "UPDATE project_phases SET status='completed' WHERE phase_id='phase-1'",
            [],
        )
        .unwrap();
    let second = transition_phase(
        &mut persistence,
        "phase-2",
        ProjectPhaseStatus::InProgress,
        0,
        12,
    );
    let third = transition_phase(
        &mut persistence,
        "phase-3",
        ProjectPhaseStatus::InProgress,
        0,
        13,
    );
    assert_eq!(second.status, ProjectPhaseStatus::InProgress);
    assert_eq!(third.status, ProjectPhaseStatus::InProgress);
    let phases = persistence
        .transact(|repositories| repositories.phase_plans("project-progress-1"))
        .unwrap()[0]
        .phases
        .clone();
    assert_eq!(
        phases
            .iter()
            .filter(|phase| phase.status == ProjectPhaseStatus::InProgress)
            .count(),
        2
    );
}

#[test]
fn global_search_keyset_pages_all_rows_with_equal_timestamps() {
    let fixture = TestDatabase::new("global-search-pages");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);
    for index in 0..27 {
        persistence.connection.execute("INSERT INTO local_files(local_file_id,workspace_id,relative_path,normalized_relative_path,materialization_state,file_type,observed_at,scan_state,state_revision,created_at,updated_at) VALUES (?1,'w-snapshot',?2,?2,'materialized','file',20,'current',0,20,20)", params![format!("page-{index:02}"), format!("分页材料-{index:02}.md")]).unwrap();
    }
    let mut after = None;
    let mut ids = Vec::new();
    loop {
        let records = persistence
            .transact(|repositories| {
                contexthub_application::global_search(
                    repositories,
                    &GlobalSearchQuery {
                        query: "分页材料".to_owned(),
                        limit: 8,
                        after: after.clone(),
                    },
                )
            })
            .unwrap();
        if records.is_empty() {
            break;
        }
        let last = records.last().unwrap();
        after = Some((
            last.updated_at,
            last.object_type.clone(),
            last.object_id.clone(),
        ));
        ids.extend(records.into_iter().map(|record| record.object_id));
    }
    assert_eq!(ids.len(), 27);
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        27
    );
}

#[test]
fn global_search_includes_projects_without_folders() {
    let fixture = TestDatabase::new("global-search-project-no-folder");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);
    persistence
        .connection
        .execute(
            "DELETE FROM project_workspaces WHERE project_id='project-progress-1'",
            [],
        )
        .unwrap();
    persistence
        .connection
        .execute(
            "UPDATE projects SET display_name='无文件夹讨论' WHERE project_id='project-progress-1'",
            [],
        )
        .unwrap();
    let results = persistence
        .transact(|repositories| {
            contexthub_application::global_search(
                repositories,
                &GlobalSearchQuery {
                    query: "无文件夹讨论".to_owned(),
                    limit: 8,
                    after: None,
                },
            )
        })
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].object_type, "project");
    assert_eq!(results[0].object_id, "project-progress-1");
    assert!(results[0].workspace_id.is_empty());
}

#[test]
fn global_search_returns_typed_unique_source_results() {
    let fixture = TestDatabase::new("global-search");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);
    create_active_phase_plan(&mut persistence);
    let results = persistence
        .transact(|repositories| {
            contexthub_application::global_search(
                repositories,
                &GlobalSearchQuery {
                    after: None,
                    query: "需求".to_owned(),
                    limit: 100,
                },
            )
        })
        .unwrap();
    assert!(results.iter().any(|item| item.object_type == "phase"
        && item.title == "需求定义"
        && item.project_id.as_deref() == Some("project-progress-1")));
    assert!(results.iter().all(|item| !item.workspace_id.is_empty()));

    persistence.connection.execute_batch(
        "INSERT INTO local_files(local_file_id,workspace_id,relative_path,normalized_relative_path,materialization_state,file_type,observed_at,scan_state,state_revision,created_at,updated_at)
         VALUES ('visible-search-file','w-snapshot','文档/隐藏规则说明.md','文档/隐藏规则说明.md','materialized','file',20,'current',0,20,20),
                ('missing-search-file','w-snapshot','文档/已删除隐藏规则说明.md','文档/已删除隐藏规则说明.md','missing','file',20,'current',0,20,20),
                ('hidden-root-search-file','w-snapshot','.claude/隐藏规则说明.md','.claude/隐藏规则说明.md','materialized','file',20,'current',0,20,20),
                ('hidden-nested-search-file','w-snapshot','文档/.draft/隐藏规则说明.md','文档/.draft/隐藏规则说明.md','materialized','file',20,'current',0,20,20);",
    ).unwrap();
    let file_results = persistence
        .transact(|repositories| {
            contexthub_application::global_search(
                repositories,
                &GlobalSearchQuery {
                    after: None,
                    query: "隐藏规则".to_owned(),
                    limit: 100,
                },
            )
        })
        .unwrap();
    assert_eq!(
        file_results
            .iter()
            .filter(|item| item.object_type == "file")
            .map(|item| item.object_id.as_str())
            .collect::<Vec<_>>(),
        vec!["visible-search-file"]
    );
}

#[test]
fn attention_cases_project_to_my_work_and_follow_type_specific_state_machines() {
    let fixture = TestDatabase::new("attention-cases");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);
    create_active_phase_plan(&mut persistence);
    seed_attention_cases(&mut persistence);
    assert_attention_projection(&mut persistence);
    assert_attention_transitions(&mut persistence);
    assert_attention_events(&persistence);
}

#[test]
fn task_incident_attaches_to_original_task_without_duplicate_card() {
    let fixture = TestDatabase::new("task-incident");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);
    create_active_phase_plan(&mut persistence);
    transition_phase(
        &mut persistence,
        "phase-1",
        ProjectPhaseStatus::InProgress,
        0,
        13,
    );
    create_and_move_business_task(&mut persistence);
    transition_task(&mut persistence, BusinessTaskStatus::Todo, 2, 17);
    transition_task(&mut persistence, BusinessTaskStatus::InProgress, 3, 18);
    transition_task(&mut persistence, BusinessTaskStatus::Review, 4, 19);
    let mut command = attention_create(
        "task-incident",
        AttentionCaseType::AgentRunIncident,
        None,
        Some(("failed-task-run", "Codex")),
        30,
    );
    command.task_id = Some("business-task-1".to_owned());
    command.phase_id = None;
    persistence
        .transact(|repositories| {
            contexthub_application::create_attention_case(repositories, &command)
        })
        .unwrap();
    let items = my_work_items(&mut persistence);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source_type, "business_task");
    assert_eq!(items[0].focus_group, "todo");
    assert_eq!(items[0].blocking_run_id.as_deref(), Some("failed-task-run"));
    assert_eq!(items[0].blocking_reason.as_deref(), Some("需要及时处理"));
    persist_attention_transition(
        &mut persistence,
        &attention_transition(
            "resolve-task-incident",
            "task-incident",
            AttentionCaseStatus::Resolved,
            0,
            "依赖已修复",
            40,
        ),
    );
    let items = my_work_items(&mut persistence);
    assert_eq!(items.len(), 1);
    assert!(items[0].blocking_run_id.is_none());
    assert_eq!(items[0].focus_group, "review");
}

#[test]
fn failed_work_classification_does_not_project_as_business_work() {
    let fixture = TestDatabase::new("classification-incident");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);
    create_active_phase_plan(&mut persistence);
    let state = serde_json::json!({
        "conversations":[{
            "conversationId":"conversation",
            "title":"生成产品配图",
            "archived":false,
            "context":{"projectId":"project-progress-1","phaseId":"phase-1","taskId":null}
        }],
        "messages":[
            {"messageId":"original","conversationId":"conversation","role":"user","content":"生成产品配图"}
        ],
        "runs":[{
            "runId":"classification-failed",
            "conversationId":"conversation",
            "inputMessageId":"original",
            "status":"completed",
            "classificationStatus":"failed",
            "classificationError":"工作归类输出未完成",
            "startedAt":10,
            "completedAt":20,
            "provider":"codex",
            "model":"test"
        }]
    });
    persistence.replace_ai_runtime_state(0, &state, 20).unwrap();
    let mut command = attention_create(
        "classification-incident",
        AttentionCaseType::AgentRunIncident,
        None,
        Some(("classification-failed", "AI 工作归类")),
        20,
    );
    command.phase_id = Some("phase-1".to_owned());
    command.title = "恢复工作归类：生成产品配图".to_owned();
    command.detail = "工作归类输出未完成".to_owned();
    command.client_operation_id =
        "desktop-classification-incident-classification-failed".to_owned();
    persistence
        .transact(|repositories| {
            contexthub_application::create_attention_case(repositories, &command)
        })
        .unwrap();

    assert!(my_work_items(&mut persistence).is_empty());
    assert_eq!(persistence.ai_runtime_state().unwrap().1, state);
}

#[test]
fn repeated_failed_classifications_preserve_history_without_business_cards() {
    let fixture = TestDatabase::new("classification-incident-latest-only");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_progress_project(&mut persistence);
    create_active_phase_plan(&mut persistence);
    let state = serde_json::json!({
        "conversations":[{
            "conversationId":"conversation",
            "title":"生成产品配图",
            "archived":false,
            "context":{"projectId":"project-progress-1","phaseId":"phase-1","taskId":null}
        }],
        "messages":[
            {"messageId":"original","conversationId":"conversation","role":"user","content":"生成产品配图"},
            {"messageId":"retry","conversationId":"conversation","role":"user","content":"重试"}
        ],
        "runs":[
            {"runId":"classification-old","conversationId":"conversation","inputMessageId":"original","status":"completed","classificationStatus":"failed","classificationError":"第一次归类失败","startedAt":10,"completedAt":20,"provider":"codex","model":"test"},
            {"runId":"classification-latest","conversationId":"conversation","inputMessageId":"retry","status":"completed","classificationStatus":"failed","classificationError":"第二次归类失败","startedAt":30,"completedAt":40,"provider":"codex","model":"test"}
        ]
    });
    persistence.replace_ai_runtime_state(0, &state, 40).unwrap();
    for (case_id, run_id, detail, occurred_at) in [
        (
            "classification-old-case",
            "classification-old",
            "第一次归类失败",
            20,
        ),
        (
            "classification-latest-case",
            "classification-latest",
            "第二次归类失败",
            40,
        ),
    ] {
        let mut command = attention_create(
            case_id,
            AttentionCaseType::AgentRunIncident,
            None,
            Some((run_id, "AI 工作归类")),
            occurred_at,
        );
        command.phase_id = Some("phase-1".to_owned());
        command.title = "恢复工作归类：生成产品配图".to_owned();
        command.detail = detail.to_owned();
        command.client_operation_id = format!("desktop-classification-incident-{run_id}");
        persistence
            .transact(|repositories| {
                contexthub_application::create_attention_case(repositories, &command)
            })
            .unwrap();
    }

    assert!(my_work_items(&mut persistence).is_empty());

    persistence.replace_ai_runtime_state(1, &state, 50).unwrap();
    let statuses: (String, String) = persistence.connection.query_row(
        "SELECT older.status,latest.status FROM attention_cases older JOIN attention_cases latest ON latest.attention_case_id='classification-latest-case' WHERE older.attention_case_id='classification-old-case'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(statuses, ("resolved".to_owned(), "open".to_owned()));
}

#[test]
#[allow(clippy::too_many_lines)]
fn classification_recovery_requires_committed_results_and_survives_upgrade() {
    for upgrade in [false, true] {
        for (scenario, status, committed, superseded, expected) in [
            ("continue", "applied", true, false, "resolved"),
            ("not-committed", "applied", false, false, "open"),
            ("still-failed", "failed", false, false, "open"),
            ("replaced", "applied", true, true, "open"),
        ] {
            let fixture = TestDatabase::new(scenario);
            let mut persistence = if upgrade {
                let connection = Connection::open(&fixture.path).unwrap();
                configure_connection(&connection).unwrap();
                for migration in &MIGRATIONS[..48] {
                    apply_fixture_migration(&connection, migration);
                    connection.execute(
                        "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                        params![migration.version, migration.name, migration_checksum(migration.sql)],
                    ).unwrap();
                }
                connection.execute_batch("CREATE TEMP TABLE ai_storage_activation(storage_key TEXT); CREATE TEMP VIEW ai_business_conversations AS SELECT * FROM ai_conversations; CREATE TEMP VIEW ai_business_messages AS SELECT * FROM ai_messages; CREATE TEMP VIEW ai_business_runs AS SELECT * FROM ai_runs;").unwrap();
                SqlitePersistence {
                    connection,
                    database_path: fixture.path.clone(),
                    file_content_index: None,
                }
            } else {
                SqlitePersistence::open(&fixture.path, "0.1.0").unwrap()
            };
            seed_progress_project(&mut persistence);
            let mut state = serde_json::json!({
                "conversations":[{"conversationId":"conversation","context":{"projectId":"project-progress-1","taskId":null}}],
                "messages":[
                    {"messageId":"original","conversationId":"conversation","role":"user","content":"生成图片"},
                    {"messageId":"next","conversationId":"conversation","role":"user","content":"继续完成"}
                ],
                "runs":[
                    {"runId":"failed","conversationId":"conversation","inputMessageId":"original","status":"completed","classificationStatus":"failed","startedAt":10,"completedAt":20},
                    {"runId":"success","conversationId":"conversation","inputMessageId":"next","status":"completed","classificationStatus":status,"superseded":superseded,"startedAt":30,"completedAt":40}
                ]
            });
            persistence.replace_ai_runtime_state(0, &state, 40).unwrap();
            let mut command = attention_create(
                "classification-case",
                AttentionCaseType::AgentRunIncident,
                None,
                Some(("failed", "AI 工作归类")),
                20,
            );
            command.phase_id = None;
            command.client_operation_id = "desktop-classification-incident-failed".to_owned();
            persistence
                .transact(|repositories| {
                    contexthub_application::create_attention_case(repositories, &command)
                })
                .unwrap();
            if committed {
                persistence.connection.execute(
                    "INSERT INTO conversation_work_classifications(classification_id,run_id,conversation_id,project_id,business_task_id,classification_action,task_action,conversation_status,stage,outcome_title,outcome_summary,reason,confidence,raw_json,created_at) VALUES ('classified','success','conversation','project-progress-1',NULL,'unclassified','none',NULL,NULL,NULL,NULL,'已校验',1,'{}',40)", [],
                ).unwrap();
            }
            if upgrade {
                let projection = MIGRATIONS[40]
                    .sql
                    .split("INSERT OR IGNORE INTO attention_cases(")
                    .next()
                    .unwrap();
                persistence.connection.execute_batch(projection).unwrap();
                persistence.connection.execute("UPDATE attention_cases SET updated_at=updated_at WHERE attention_case_id='classification-case'", []).unwrap();
                assert_eq!(persistence.connection.query_row("SELECT COUNT(*) FROM my_work_items WHERE attention_case_id='classification-case'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
                drop(persistence);
                persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
            } else {
                state["conversations"][0]["updatedAt"] = serde_json::json!(50);
                persistence.replace_ai_runtime_state(1, &state, 50).unwrap();
            }
            let actual: String = persistence.connection.query_row("SELECT status FROM attention_cases WHERE attention_case_id='classification-case'", [], |row| row.get(0)).unwrap();
            assert_eq!(actual, expected, "{scenario}, upgrade={upgrade}");
            assert!(my_work_items(&mut persistence).is_empty());
            assert_eq!(persistence.ai_runtime_state().unwrap().1, state);
            drop(persistence);
            let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
            let transitions: i64 = reopened.connection.query_row("SELECT COUNT(*) FROM attention_case_transitions WHERE attention_case_id='classification-case'", [], |row| row.get(0)).unwrap();
            assert_eq!(transitions, i64::from(expected == "resolved"));
        }
    }
}

#[test]
fn incident_recovery_requires_matching_request_and_preserves_history() {
    for (scenario, prompt, retry, task, expected) in [
        ("same-request", "原始请求", false, None, "resolved"),
        ("unrelated", "无关的新请求", false, None, "open"),
        ("explicit-retry", "重试", true, None, "resolved"),
        (
            "changed-task",
            "原始请求",
            false,
            Some("other-task"),
            "open",
        ),
    ] {
        let fixture = TestDatabase::new(scenario);
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        create_active_phase_plan(&mut persistence);
        let command = attention_create(
            "incident",
            AttentionCaseType::AgentRunIncident,
            None,
            Some(("failed", "Codex")),
            20,
        );
        persistence
            .transact(|repositories| {
                contexthub_application::create_attention_case(repositories, &command)
            })
            .unwrap();
        let state = serde_json::json!({
            "conversations":[{"conversationId":"conversation","context":{"projectId":"project-progress-1","taskId":task}}],
            "messages":[
                {"messageId":"original","conversationId":"conversation","role":"user","content":"原始请求"},
                {"messageId":"next","conversationId":"conversation","role":"user","content":prompt}
            ],
            "runs":[
                {"runId":"failed","conversationId":"conversation","inputMessageId":"original","status":"failed","startedAt":10,"completedAt":20,"provider":"codex","model":"test"},
                {"runId":"success","conversationId":"conversation","inputMessageId":"next","retryOfRunId":if retry {Some("failed")} else {None},"status":"completed","startedAt":30,"completedAt":40,"provider":"other","model":"different"}
            ]
        });
        persistence.replace_ai_runtime_state(0, &state, 40).unwrap();
        assert!(my_work_items(&mut persistence).is_empty());
        let status: String = persistence
            .connection
            .query_row(
                "SELECT status FROM attention_cases WHERE attention_case_id='incident'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, expected, "{scenario}");
        assert_eq!(persistence.ai_runtime_state().unwrap().1, state);
        drop(persistence);
        let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let transitions: i64 = reopened
            .connection
            .query_row(
                "SELECT COUNT(*) FROM attention_case_transitions",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(transitions, i64::from(expected == "resolved"));
    }
}

#[test]
fn version_two_workspace_data_is_split_without_changing_local_identity_or_children() {
    let fixture = TestDatabase::new("upgrade-v2-workspace");
    let connection = Connection::open(&fixture.path).unwrap();
    configure_connection(&connection).unwrap();
    for migration in &MIGRATIONS[..2] {
        apply_fixture_migration(&connection, migration);
        connection
            .execute(
                "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (?1, ?2, ?3, 1, '0.1.0')",
                params![migration.version, migration.name, migration_checksum(migration.sql)],
            )
            .unwrap();
    }
    connection
        .execute_batch(
            "INSERT INTO accounts(account_id, display_name, account_status, created_at, updated_at) VALUES ('a-legacy', 'Legacy', 'active', 1, 1);
             INSERT INTO workspaces(workspace_id, account_id, project_id, display_name, local_root, local_root_identity, platform_root, status, state_revision, platform_snapshot_revision, last_platform_check_at, last_local_scan_at, created_at, updated_at) VALUES ('w-legacy', 'a-legacy', 'p-legacy', 'Legacy workspace', '/tmp/legacy', 'dev:1:ino:2', '/docs', 'conflict', 7, 'rev-9', 8, 9, 1, 10);
             INSERT INTO workspace_exports(workspace_id, desired_revision, exported_revision, export_state, updated_at) VALUES ('w-legacy', 2, 1, 'pending', 10);
             INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, materialization_state, file_type, observed_at, scan_state, state_revision, created_at, updated_at) VALUES ('f-legacy', 'w-legacy', 'a.md', 'a.md', 'materialized', 'file', 9, 'current', 0, 1, 9);
             INSERT INTO file_states(local_file_id, baseline_relation, local_change_state, confirmation_state, submission_state, conflict_state, display_state, capability_bits, capability_reasons_json, computed_at, state_revision) VALUES ('f-legacy', 'unknown', 'clean', 'not_required', 'none', 'none', 'current', 0, '[]', 9, 0);",
        )
        .unwrap();
    drop(connection);

    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let workspace: (String, String, String, String, i64, Option<i64>) = persistence
        .connection
        .query_row(
            "SELECT workspace_id, local_root, local_root_identity, local_status, state_revision, last_local_scan_at FROM workspaces WHERE workspace_id='w-legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .unwrap();
    assert_eq!(
        workspace,
        (
            "w-legacy".to_owned(),
            "/tmp/legacy".to_owned(),
            "dev:1:ino:2".to_owned(),
            "attention_required".to_owned(),
            7,
            Some(9),
        )
    );
    let binding: (String, Option<String>, Option<String>, String, String, Option<String>) = persistence
        .connection
        .query_row(
            "SELECT account_id, organization_id, platform_workspace_id, platform_root, binding_state, platform_snapshot_revision FROM workspace_platform_bindings WHERE workspace_id='w-legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .unwrap();
    assert_eq!(
        binding,
        (
            "a-legacy".to_owned(),
            None,
            None,
            "/docs".to_owned(),
            "bound".to_owned(),
            Some("rev-9".to_owned()),
        )
    );
    let child_workspace: String = persistence
        .connection
        .query_row(
            "SELECT workspace_id FROM local_files WHERE local_file_id='f-legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(child_workspace, "w-legacy");
    let retained_file_state: String = persistence
        .connection
        .query_row(
            "SELECT local_file_id FROM file_states WHERE local_file_id='f-legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained_file_state, "f-legacy");
    let violations: i64 = persistence
        .connection
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}

#[test]
fn m0_core_schema_has_required_tables_indexes_and_integrity() {
    let fixture = TestDatabase::new("core-schema");
    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let expected_tables = [
        "accounts",
        "app_metadata",
        "checksum_cache",
        "client_event_cursors",
        "confirmation_invalidations",
        "conflict_resolutions",
        "conflicts",
        "document_versions",
        "documents",
        "durable_events",
        "file_confirmation_text_snapshots",
        "file_confirmations",
        "file_states",
        "local_file_observations",
        "local_files",
        "local_scan_snapshots",
        "platform_snapshots",
        "schema_migrations",
        "sync_execution_items",
        "sync_executions",
        "sync_plan_items",
        "sync_plans",
        "task_attempts",
        "tasks",
        "workspace_exports",
        "workspace_platform_bindings",
        "workspaces",
    ];
    for table in expected_tables {
        let exists: bool = persistence
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "missing table {table}");
    }
    let expected_indexes = [
        "idx_confirmation_invalidations_history",
        "idx_file_confirmations_active",
        "idx_file_confirmations_client_operation",
        "idx_file_confirmations_history",
        "idx_file_confirmation_text_snapshots_file",
        "idx_sync_plans_client_operation",
        "idx_sync_executions_client_operation",
        "idx_conflict_resolutions_client_operation",
        "idx_local_files_workspace_order",
    ];
    for index in expected_indexes {
        let exists: bool = persistence
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='index' AND name=?1)",
                [index],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "missing index {index}");
    }
    let query_plan = persistence
        .connection
        .prepare(
            "EXPLAIN QUERY PLAN SELECT local_file_id FROM local_files WHERE workspace_id=?1 ORDER BY normalized_relative_path, local_file_id LIMIT ?2",
        )
        .unwrap()
        .query_map(params!["workspace", 501], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        query_plan
            .iter()
            .any(|detail| detail.contains("idx_local_files_workspace_order"))
    );
    assert!(
        query_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE"))
    );
    let integrity: String = persistence
        .connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    let foreign_key_violations: i64 = persistence
        .connection
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(foreign_key_violations, 0);
}

#[test]
fn workspace_file_pages_exclude_missing_history_and_keep_one_materialized_row_per_path() {
    let fixture = TestDatabase::new("workspace-file-path-projection");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence.connection.execute_batch(
        "INSERT INTO workspaces(workspace_id, display_name, local_root, local_status, state_revision, created_at, updated_at) VALUES ('w1', 'Workspace', '/tmp/w1', 'ready', 0, 1, 1);
         INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, materialization_state, file_type, observed_at, scan_state, state_revision, created_at, updated_at) VALUES
           ('a-missing', 'w1', 'a.md', 'a.md', 'missing', 'file', 1, 'current', 1, 1, 2),
           ('a-present', 'w1', 'a.md', 'a.md', 'materialized', 'file', 3, 'current', 0, 3, 3),
           ('b-missing-old', 'w1', 'b.md', 'b.md', 'missing', 'file', 1, 'current', 1, 1, 2),
           ('b-missing-new', 'w1', 'b.md', 'b.md', 'missing', 'file', 2, 'current', 2, 2, 4),
           ('c-present', 'w1', 'c.md', 'c.md', 'materialized', 'file', 5, 'current', 0, 5, 5);",
    ).unwrap();

    let first = persistence
        .transact(|repositories| {
            repositories.workspace_files_page(&WorkspaceId::new("w1"), None, 1)
        })
        .unwrap();
    assert_eq!(
        first
            .files
            .iter()
            .map(|file| file.local_file_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a-present"]
    );
    assert!(first.has_more);

    let second = persistence
        .transact(|repositories| {
            repositories.workspace_files_page(
                &WorkspaceId::new("w1"),
                first.next_cursor.as_deref(),
                1,
            )
        })
        .unwrap();
    assert_eq!(
        second
            .files
            .iter()
            .map(|file| file.local_file_id.as_str())
            .collect::<Vec<_>>(),
        vec!["c-present"]
    );
    assert!(!second.has_more);
}

#[test]
fn m0_core_constraints_reject_invalid_or_ambiguous_state() {
    let fixture = TestDatabase::new("core-constraints");
    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let connection = &persistence.connection;
    connection
        .execute(
            "INSERT INTO workspaces(workspace_id, display_name, local_root, local_status, state_revision, created_at, updated_at) VALUES ('w-local', 'Local', '/tmp/w-local', 'ready', 0, 1, 1)",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO workspace_platform_bindings(workspace_id, account_id, organization_id, platform_workspace_id, project_id, platform_root, binding_state, created_at, updated_at) VALUES ('w-local', 'missing', 'org', 'pw', 'p', '/', 'bound', 1, 1)",
                [],
            )
            .is_err()
    );
    connection
        .execute_batch(
            "INSERT INTO accounts(account_id, display_name, account_status, created_at, updated_at) VALUES ('a1', 'Account', 'active', 1, 1);
             INSERT INTO workspaces(workspace_id, display_name, local_root, local_status, state_revision, created_at, updated_at) VALUES ('w1', 'Workspace', '/tmp/w1', 'ready', 0, 1, 1);
             INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, materialization_state, file_type, observed_at, scan_state, state_revision, created_at, updated_at) VALUES ('f1', 'w1', 'a.md', 'a.md', 'materialized', 'file', 1, 'current', 0, 1, 1);",
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, materialization_state, file_type, observed_at, scan_state, state_revision, created_at, updated_at) VALUES ('f2', 'w1', './a.md', 'a.md', 'materialized', 'file', 1, 'current', 0, 1, 1)",
                [],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO tasks(task_id, task_type, workspace_id, state, progress_completed, cancellable, payload_json, correlation_id, created_by_client, created_at) VALUES ('t1', 'scan', 'w1', 'not-a-state', 0, 1, '{}', 'c1', 'desktop', 1)",
                [],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO file_confirmations(confirmation_id, workspace_id, local_file_id, confirmed_by_account_id, confirmed_checksum_algorithm, confirmed_checksum_value, target_scope, result, source_client, created_at) VALUES ('c1', 'w1', 'f1', 'a1', 'sha256', 'one', 'content', 'confirmed', 'desktop', 1)",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO file_confirmations(confirmation_id, workspace_id, local_file_id, confirmed_by_account_id, confirmed_checksum_algorithm, confirmed_checksum_value, target_scope, result, source_client, created_at) VALUES ('c2', 'w1', 'f1', 'a1', 'sha256', 'two', 'content', 'confirmed', 'cli', 2)",
                [],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO conflicts(conflict_id, workspace_id, local_file_id, conflict_type, state, detected_at, state_revision) VALUES ('x1', 'w1', 'f1', 'content_diverged', 'detected', 1, 0)",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "INSERT INTO conflicts(conflict_id, workspace_id, local_file_id, conflict_type, state, detected_at, state_revision) VALUES ('x2', 'w1', 'f1', 'delete_modify', 'in_progress', 2, 0)",
                [],
            )
            .is_err()
    );
}

#[test]
fn local_workspace_creation_needs_no_account_and_binding_lifecycle_preserves_history() {
    let fixture = TestDatabase::new("local-workspace-lifecycle");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let command = contexthub_application::CreateLocalWorkspaceCommand {
        workspace_id: WorkspaceId::new("w-local"),
        display_name: "Local workspace".to_owned(),
        canonical_local_root: "/tmp/w-local".to_owned(),
        local_root_identity: Some("dev:1:ino:20".to_owned()),
        scan_task_id: TaskId::new("task-scan-local"),
        event_id: EventId::new("event-workspace-local"),
        correlation_id: "corr-local".to_owned(),
        created_by_client: "desktop".to_owned(),
        occurred_at: 10,
    };
    let workspace = persistence
        .transact(|repositories| {
            contexthub_application::create_local_workspace(repositories, &command)
        })
        .unwrap();
    assert_eq!(workspace.local_status, LocalWorkspaceStatus::Scanning);
    assert_default_local_identity(&persistence.connection);
    let scan_task: (String, String, String) = persistence
        .connection
        .query_row(
            "SELECT task_type, state, workspace_id FROM tasks WHERE task_id='task-scan-local'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        scan_task,
        ("scan".to_owned(), "queued".to_owned(), "w-local".to_owned())
    );
    persistence
        .transact(|repositories| {
            insert_historical_account(&repositories.transaction, "a1", "Account", 11)?;
            insert_historical_platform_binding(
                &repositories.transaction,
                &WorkspacePlatformBindingRecord {
                    workspace_id: WorkspaceId::new("w-local"),
                    account_id: AccountId::new("a1"),
                    organization_id: Some("org1".to_owned()),
                    platform_workspace_id: Some("platform-w1".to_owned()),
                    project_id: "project1".to_owned(),
                    platform_root: "/docs".to_owned(),
                    binding_state: WorkspacePlatformBindingState::Bound,
                    platform_snapshot_revision: Some("rev1".to_owned()),
                    last_platform_check_at: Some(11),
                    team_space_id: None,
                    managed_root: None,
                    device_id: None,
                    baseline_revision: None,
                    change_cursor: None,
                    permission_role: None,
                    permission_revision: None,
                    created_at: 11,
                    updated_at: 11,
                },
            )
        })
        .unwrap();
    let bound_workspace = persistence
        .transact(|repositories| {
            assert!(
                repositories
                    .platform_binding(&WorkspaceId::new("w-local"))?
                    .is_some()
            );
            repositories
                .workspace(&WorkspaceId::new("w-local"))?
                .ok_or_else(|| {
                    PersistenceError::new(
                        PersistenceErrorKind::Database,
                        "workspace missing during binding test",
                    )
                })
        })
        .unwrap();
    assert_eq!(bound_workspace.workspace_id.as_str(), "w-local");
    persistence
        .transact(|repositories| {
            remove_historical_platform_binding(
                &repositories.transaction,
                &WorkspaceId::new("w-local"),
                31,
            )
        })
        .unwrap();
    let retained: (i64, i64, i64) = persistence
        .connection
        .query_row(
            "SELECT (SELECT count(*) FROM workspaces WHERE workspace_id='w-local'), (SELECT count(*) FROM tasks WHERE workspace_id='w-local'), (SELECT count(*) FROM durable_events WHERE workspace_id='w-local')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(retained, (1, 1, 1));
}

fn assert_default_local_identity(connection: &Connection) {
    let count = connection
        .query_row("SELECT count(*) FROM accounts", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    assert_eq!(count, 1);
    let identity = connection
        .query_row(
            "SELECT display_name, identity_kind FROM accounts WHERE account_id='local-profile:default'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap();
    assert_eq!(identity, ("本机操作人".to_owned(), "local".to_owned()));
}

#[test]
fn repository_unit_of_work_commits_state_and_event_atomically() {
    let fixture = TestDatabase::new("unit-of-work");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let workspace = WorkspaceRecord {
        workspace_id: WorkspaceId::new("w1"),
        workspace_kind: WorkspaceKind::Local,
        display_name: "Workspace".to_owned(),
        local_root: "/tmp/w1".to_owned(),
        local_root_identity: Some("dev:1:ino:1".to_owned()),
        local_status: LocalWorkspaceStatus::Creating,
        state_revision: 0,
        last_local_scan_at: None,
        created_at: 1,
        updated_at: 1,
    };
    let export = WorkspaceExportRecord {
        workspace_id: workspace.workspace_id.clone(),
        desired_revision: 0,
        exported_revision: 0,
        updated_at: 1,
    };
    persistence
        .transact(|repositories| {
            insert_historical_account(&repositories.transaction, "a1", "Account", 1)?;
            repositories.insert_workspace(&workspace)?;
            repositories.initialize_workspace_export(&export)?;
            let stored = repositories.append_event(&workspace_event("e1", 1))?;
            assert_eq!(stored.global_sequence, 1);
            assert_eq!(stored.workspace_sequence, Some(1));
            Ok(())
        })
        .unwrap();

    let rollback_error = persistence
        .transact(|repositories| {
            repositories.update_workspace_status(
                &WorkspaceId::new("w1"),
                0,
                LocalWorkspaceStatus::Ready,
                2,
            )?;
            repositories.append_event(&workspace_event("e1", 2))?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(rollback_error.kind(), PersistenceErrorKind::Database);
    let state_after_rollback: (String, i64) = persistence
        .connection
        .query_row(
            "SELECT local_status, state_revision FROM workspaces WHERE workspace_id='w1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state_after_rollback, ("creating".to_owned(), 0));
    let event_count: i64 = persistence
        .connection
        .query_row("SELECT count(*) FROM durable_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(event_count, 1);

    let updated = persistence
        .transact(|repositories| {
            let updated = repositories.update_workspace_status(
                &WorkspaceId::new("w1"),
                0,
                LocalWorkspaceStatus::Ready,
                3,
            )?;
            let event = repositories.append_event(&workspace_event("e2", 3))?;
            assert_eq!(event.workspace_sequence, Some(2));
            Ok(updated)
        })
        .unwrap();
    assert_eq!(updated.local_status, LocalWorkspaceStatus::Ready);
    assert_eq!(updated.state_revision, 1);

    let stale_error = persistence
        .transact(|repositories| {
            repositories.update_workspace_status(
                &WorkspaceId::new("w1"),
                0,
                LocalWorkspaceStatus::AttentionRequired,
                4,
            )?;
            repositories.append_event(&workspace_event("e3", 4))?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(stale_error.kind(), PersistenceErrorKind::Conflict);
    let final_event_count: i64 = persistence
        .connection
        .query_row("SELECT count(*) FROM durable_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(final_event_count, 2);
}

#[test]
fn 观测快照有上限不会无限增长() {
    // 每次扫描都写一份完整观测。手动扫描年代这只是慢性膨胀；接上文件监听之后
    // 变成急性：实测两万多文件的工作区每份 22,347 行，库一小时内涨到 3.3 GB。
    let fixture = TestDatabase::new("observation-retention");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, false);

    for generation in 1..=14 {
        persistence
            .transact(|repositories| {
                repositories.commit_local_snapshot(&local_snapshot(
                    &format!("ls-{generation}"),
                    generation,
                    generation * 1_000,
                    vec![local_observation(
                        "file-1",
                        "notes/a.md",
                        "dev:1:ino:10",
                        "one",
                    )],
                ))
            })
            .unwrap();
    }

    let snapshots_with_observations: i64 = persistence
        .connection
        .query_row(
            "SELECT COUNT(DISTINCT snapshot_id) FROM local_file_observations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(snapshots_with_observations, 10);
}

#[test]
fn 单条目快照只补录记录不把工作区标为索引完成() {
    let fixture = TestDatabase::new("local-snapshot-partial");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, false);
    let mut partial = local_snapshot(
        "ls-partial",
        1,
        10,
        vec![local_observation(
            "file-1",
            "notes/a.md",
            "dev:1:ino:10",
            "one",
        )],
    );
    partial.snapshot_kind = SnapshotKind::Incremental;
    partial.covers_whole_tree = false;
    let committed = persistence
        .transact(|repositories| repositories.commit_local_snapshot(&partial))
        .unwrap();
    assert_eq!(committed.created_local_file_ids, vec!["file-1"]);
    let workspace = persistence
        .transact(|repositories| repositories.workspace(&WorkspaceId::new("w-snapshot")))
        .unwrap()
        .unwrap();
    assert_eq!(workspace.local_status, LocalWorkspaceStatus::Scanning);
    assert_eq!(workspace.last_local_scan_at, None);

    let complete = local_snapshot(
        "ls-complete",
        2,
        20,
        vec![local_observation(
            "file-2",
            "notes/a.md",
            "dev:1:ino:10",
            "one",
        )],
    );
    persistence
        .transact(|repositories| repositories.commit_local_snapshot(&complete))
        .unwrap();
    let workspace = persistence
        .transact(|repositories| repositories.workspace(&WorkspaceId::new("w-snapshot")))
        .unwrap()
        .unwrap();
    assert_eq!(workspace.local_status, LocalWorkspaceStatus::Ready);
    assert_eq!(workspace.last_local_scan_at, Some(20));
}

#[test]
fn 按快照只读出本次新增的文件记录() {
    let fixture = TestDatabase::new("first-seen-scope");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, false);
    let first = local_snapshot(
        "ls-a",
        1,
        10,
        vec![local_observation("file-a", "a.md", "dev:1:ino:1", "one")],
    );
    persistence
        .transact(|repositories| repositories.commit_local_snapshot(&first))
        .unwrap();
    let mut second = local_snapshot(
        "ls-b",
        2,
        20,
        vec![
            local_observation("file-a2", "a.md", "dev:1:ino:1", "one"),
            local_observation("file-b", "b.md", "dev:1:ino:2", "two"),
        ],
    );
    second.snapshot_kind = SnapshotKind::Incremental;
    persistence
        .transact(|repositories| repositories.commit_local_snapshot(&second))
        .unwrap();
    let created = persistence
        .transact(|repositories| {
            repositories.workspace_files_first_seen_in(&WorkspaceId::new("w-snapshot"), "ls-b")
        })
        .unwrap();
    assert_eq!(
        created
            .iter()
            .map(|file| file.local_file_id.as_str())
            .collect::<Vec<_>>(),
        vec!["file-b"]
    );
}

#[test]
fn 有读取问题的快照不能推断删除也不能标为全量() {
    let fixture = TestDatabase::new("local-snapshot-partial-invalid");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, false);
    let mut full_partial = local_snapshot("ls-bad-1", 1, 10, vec![]);
    full_partial.covers_whole_tree = false;
    assert!(
        persistence
            .transact(|repositories| repositories.commit_local_snapshot(&full_partial))
            .is_err()
    );
    let mut removing_partial = local_snapshot("ls-bad-2", 1, 10, vec![]);
    removing_partial.snapshot_kind = SnapshotKind::Incremental;
    removing_partial.covers_whole_tree = false;
    removing_partial.issue_count = 1;
    removing_partial.removed_normalized_paths = vec!["notes/a.md".to_owned()];
    assert!(
        persistence
            .transact(|repositories| repositories.commit_local_snapshot(&removing_partial))
            .is_err()
    );
}

#[expect(
    clippy::too_many_lines,
    reason = "the acceptance test keeps the complete move, replacement and deletion history visible"
)]
#[test]
fn local_snapshots_distinguish_move_replacement_and_deletion() {
    let fixture = TestDatabase::new("local-snapshot-identity");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, false);

    let first = persistence
        .transact(|repositories| {
            repositories.commit_local_snapshot(&local_snapshot(
                "ls-1",
                1,
                10,
                vec![local_observation(
                    "file-1",
                    "notes/a.md",
                    "dev:1:ino:10",
                    "one",
                )],
            ))
        })
        .unwrap();
    assert_eq!(first.created_local_file_ids, vec!["file-1"]);

    let moved = persistence
        .transact(|repositories| {
            repositories.commit_local_snapshot(&local_snapshot(
                "ls-2",
                2,
                20,
                vec![local_observation(
                    "file-unused",
                    "archive/a.md",
                    "dev:1:ino:10",
                    "one",
                )],
            ))
        })
        .unwrap();
    assert_eq!(moved.retained_local_file_ids, vec!["file-1"]);
    assert!(moved.created_local_file_ids.is_empty());

    let replaced = persistence
        .transact(|repositories| {
            repositories.commit_local_snapshot(&local_snapshot(
                "ls-3",
                3,
                30,
                vec![local_observation(
                    "file-2",
                    "archive/a.md",
                    "dev:1:ino:20",
                    "two",
                )],
            ))
        })
        .unwrap();
    assert_eq!(replaced.created_local_file_ids, vec!["file-2"]);
    assert_eq!(replaced.missing_local_file_ids, vec!["file-1"]);

    let deleted = persistence
        .transact(|repositories| {
            repositories.commit_local_snapshot(&local_snapshot("ls-4", 4, 40, vec![]))
        })
        .unwrap();
    assert_eq!(deleted.missing_local_file_ids, vec!["file-2"]);
    let rows = persistence
        .connection
        .prepare(
            "SELECT local_file_id, normalized_relative_path, materialization_state, first_seen_snapshot_id, last_seen_snapshot_id, missing_since_snapshot_id FROM local_files ORDER BY local_file_id",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (
                "file-1".to_owned(),
                "archive/a.md".to_owned(),
                "missing".to_owned(),
                Some("ls-1".to_owned()),
                Some("ls-2".to_owned()),
                Some("ls-3".to_owned()),
            ),
            (
                "file-2".to_owned(),
                "archive/a.md".to_owned(),
                "missing".to_owned(),
                Some("ls-3".to_owned()),
                Some("ls-3".to_owned()),
                Some("ls-4".to_owned()),
            ),
        ]
    );
    let observation_count: i64 = persistence
        .connection
        .query_row("SELECT count(*) FROM local_file_observations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(observation_count, 3);
}

#[test]
fn incremental_snapshot_swaps_paths_without_losing_stable_identities() {
    let fixture = TestDatabase::new("local-snapshot-swap");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, false);
    persistence
        .transact(|repositories| {
            repositories.commit_local_snapshot(&local_snapshot(
                "ls-swap-1",
                1,
                10,
                vec![
                    local_observation("file-a", "a.md", "dev:1:ino:10", "a"),
                    local_observation("file-b", "b.md", "dev:1:ino:20", "b"),
                ],
            ))
        })
        .unwrap();
    let mut incremental = local_snapshot(
        "ls-swap-2",
        2,
        20,
        vec![
            local_observation("unused-a", "b.md", "dev:1:ino:10", "a"),
            local_observation("unused-b", "a.md", "dev:1:ino:20", "b"),
        ],
    );
    incremental.snapshot_kind = SnapshotKind::Incremental;
    let result = persistence
        .transact(|repositories| repositories.commit_local_snapshot(&incremental))
        .unwrap();
    assert_eq!(result.retained_local_file_ids, vec!["file-a", "file-b"]);
    assert!(result.created_local_file_ids.is_empty());
    assert!(result.missing_local_file_ids.is_empty());
    let paths: String = persistence
        .connection
        .query_row(
            "SELECT group_concat(local_file_id || ':' || normalized_relative_path, ',') FROM (SELECT local_file_id, normalized_relative_path FROM local_files WHERE materialization_state='materialized' ORDER BY local_file_id)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(paths, "file-a:b.md,file-b:a.md");
}

#[test]
fn platform_snapshots_keep_document_identity_and_immutable_versions() {
    let fixture = TestDatabase::new("platform-snapshot-identity");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, true);
    persistence
        .transact(|repositories| {
            seed_historical_platform_snapshot(
                &repositories.transaction,
                &platform_snapshot("ps-1", "revision-1", "docs/a.md", "version-1", 10),
            )
        })
        .unwrap();
    persistence
        .transact(|repositories| {
            seed_historical_platform_snapshot(
                &repositories.transaction,
                &platform_snapshot("ps-2", "revision-2", "archive/a.md", "version-2", 20),
            )
        })
        .unwrap();
    let document: (String, String, String) = persistence
        .connection
        .query_row(
            "SELECT document_id, normalized_platform_path, latest_published_version_id FROM documents",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        document,
        (
            "document-1".to_owned(),
            "archive/a.md".to_owned(),
            "version-2".to_owned(),
        )
    );
    let versions: String = persistence
        .connection
        .query_row(
            "SELECT group_concat(version_id, ',') FROM (SELECT version_id FROM document_versions ORDER BY version_id)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(versions, "version-1,version-2");

    let duplicate_version = persistence
        .transact(|repositories| {
            seed_historical_platform_snapshot(
                &repositories.transaction,
                &platform_snapshot("ps-3", "revision-3", "archive/a.md", "version-2", 30),
            )
        })
        .unwrap_err();
    assert_eq!(duplicate_version.kind(), PersistenceErrorKind::Database);
    let rolled_back_snapshot: i64 = persistence
        .connection
        .query_row(
            "SELECT count(*) FROM platform_snapshots WHERE snapshot_id='ps-3'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rolled_back_snapshot, 0);
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the acceptance test preserves the complete state, confirmation, conflict and rollback history"
)]
fn computed_file_state_persists_confirmation_invalidation_conflict_and_event_atomically() {
    let fixture = TestDatabase::new("computed-file-state");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, true);
    persistence
        .transact(|repositories| {
            seed_historical_platform_snapshot(
                &repositories.transaction,
                &platform_snapshot(
                    "state-platform-1",
                    "state-revision-1",
                    "docs/a.md",
                    "version-1",
                    10,
                ),
            )?;
            repositories.commit_local_snapshot(&local_snapshot(
                "state-local-1",
                1,
                11,
                vec![local_observation(
                    "state-file-1",
                    "docs/a.md",
                    "dev:1:ino:30",
                    "checksum-version-1",
                )],
            ))?;
            Ok(())
        })
        .unwrap();
    persistence
        .connection
        .execute(
            "UPDATE local_files SET document_id='document-1' WHERE local_file_id='state-file-1'",
            [],
        )
        .unwrap();

    let current = state_input("checksum-version-1", "version-1", "checksum-version-1");
    let persisted = persistence
        .transact(|repositories| {
            contexthub_application::recompute_file_state(
                repositories,
                &state_command("state-current", None, current, 12),
            )
        })
        .unwrap();
    assert_eq!(persisted.state_revision, 0);
    assert_eq!(persisted.output.display_state.as_str(), "synced");

    persistence
        .connection
        .execute(
            "INSERT INTO file_confirmations(confirmation_id, workspace_id, local_file_id, document_id, confirmed_by_account_id, confirmed_checksum_algorithm, confirmed_checksum_value, baseline_version_id, target_scope, result, source_client, created_at) VALUES ('confirmation-1', 'w-snapshot', 'state-file-1', 'document-1', 'account-snapshot', 'sha256', 'checksum-version-1', 'version-1', 'project', 'confirmed', 'desktop', 13)",
            [],
        )
        .unwrap();
    let mut modified = state_input("local-change", "version-1", "checksum-version-1");
    modified.active_confirmation = Some(ActiveConfirmationFact {
        checksum_algorithm: "sha256".to_owned(),
        checksum_value: "checksum-version-1".to_owned(),
        baseline_version_id: Some("version-1".to_owned()),
    });
    let invalidated = persistence
        .transact(|repositories| {
            contexthub_application::recompute_file_state(
                repositories,
                &state_command("state-modified", Some(0), modified, 14),
            )
        })
        .unwrap();
    assert_eq!(invalidated.state_revision, 1);
    assert_eq!(
        invalidated.output.confirmation_state,
        ConfirmationState::InvalidatedContentChanged
    );
    let invalidation: (String, String) = persistence
        .connection
        .query_row(
            "SELECT reason, current_checksum FROM confirmation_invalidations WHERE confirmation_id='confirmation-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        invalidation,
        (
            "local_content_changed".to_owned(),
            "local-change".to_owned()
        )
    );
    let invalidation_event: (String, String) = persistence
        .connection
        .query_row(
            "SELECT event_type, object_id FROM durable_events WHERE event_id='event-invalidation-state-modified'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        invalidation_event,
        (
            "file.confirmation_invalidated".to_owned(),
            "confirmation-1".to_owned()
        )
    );

    persistence
        .transact(|repositories| {
            seed_historical_platform_snapshot(
                &repositories.transaction,
                &platform_snapshot(
                    "state-platform-2",
                    "state-revision-2",
                    "docs/a.md",
                    "version-2",
                    15,
                ),
            )
        })
        .unwrap();
    let diverged = state_input("local-change", "version-2", "checksum-version-2");
    let conflict = persistence
        .transact(|repositories| {
            contexthub_application::recompute_file_state(
                repositories,
                &state_command("state-diverged", Some(1), diverged, 16),
            )
        })
        .unwrap();
    assert_eq!(
        conflict.output.conflict_state,
        ConflictState::ContentDiverged
    );
    let conflict_type: String = persistence
        .connection
        .query_row(
            "SELECT conflict_type FROM conflicts WHERE local_file_id='state-file-1' AND state='detected'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(conflict_type, "content_diverged");

    let omitted_conflict = persistence
        .transact(|repositories| {
            contexthub_application::recompute_file_state(
                repositories,
                &state_command(
                    "state-omitted-conflict",
                    Some(2),
                    state_input("local-change", "version-2", "checksum-version-2"),
                    17,
                ),
            )
        })
        .unwrap_err();
    assert_eq!(omitted_conflict.kind(), PersistenceErrorKind::Conflict);

    let stale = persistence
        .transact(|repositories| {
            contexthub_application::recompute_file_state(
                repositories,
                &state_command(
                    "state-stale",
                    Some(1),
                    state_input("local-change", "version-2", "checksum-version-2"),
                    18,
                ),
            )
        })
        .unwrap_err();
    assert_eq!(stale.kind(), PersistenceErrorKind::Conflict);
    let stale_event_count: i64 = persistence
        .connection
        .query_row(
            "SELECT count(*) FROM durable_events WHERE event_id='event-state-stale'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale_event_count, 0);
    let omitted_event_count: i64 = persistence
        .connection
        .query_row(
            "SELECT count(*) FROM durable_events WHERE event_id='event-state-omitted-conflict'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(omitted_event_count, 0);
    drop(persistence);
    let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let retained: (String, String, String) = reopened
        .connection
        .query_row(
            "SELECT c.state, fs.conflict_state, fs.confirmation_state FROM conflicts c JOIN file_states fs ON fs.local_file_id=c.local_file_id WHERE c.conflict_id='conflict-state-diverged'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        retained,
        (
            "detected".to_owned(),
            "content_diverged".to_owned(),
            "blocked_by_conflict".to_owned()
        )
    );
    let invalidated_at: Option<i64> = reopened
        .connection
        .query_row(
            "SELECT invalidated_at FROM file_confirmations WHERE confirmation_id='confirmation-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(invalidated_at, Some(14));
}

fn legacy_sync_rows(connection: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
    [
        "sync_plans",
        "sync_plan_items",
        "sync_executions",
        "sync_execution_items",
        "tasks",
    ]
    .into_iter()
    .map(|table| {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let count = statement.column_count();
        statement
            .query_map([], |row| (0..count).map(|index| row.get(index)).collect())
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    })
    .collect()
}

#[test]
fn legacy_sync_history_remains_unchanged_across_reopen() {
    let fixture = TestDatabase::new("retained-sync-history");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, false);
    persistence.connection.execute_batch(
        "INSERT INTO sync_plans(plan_id, workspace_id, direction, state, local_state_revision, created_by_account_id, created_by_client, created_at, expires_at, item_count, risk_summary_json, client_operation_id, request_fingerprint)
         VALUES ('old-plan','w-snapshot','submit','executing',7,'local-profile:default','desktop',10,20,1,'{\"blocked\":0}','old-plan-operation','old-plan-fingerprint');
         INSERT INTO sync_plan_items(plan_item_id, plan_id, relative_path, action, allowed, requires_confirmation, risk_level, ordinal, expected_local_checksum_value, frozen_confirmation_state)
         VALUES ('old-item','old-plan','historical.md','update',1,1,'medium',0,'original-checksum','confirmed');
         INSERT INTO tasks(task_id, task_type, workspace_id, state, progress_completed, progress_total, cancellable, payload_json, correlation_id, created_by_client, created_at)
         VALUES ('old-task','sync','w-snapshot','outcome_unknown',0,1,0,'{\"original\":true}','old-correlation','desktop',11);
         INSERT INTO sync_executions(execution_id, plan_id, task_id, state, success_count, failed_count, skipped_count, conflict_count, correlation_id, client_operation_id, request_fingerprint)
         VALUES ('old-execution','old-plan','old-task','outcome_unknown',0,0,0,0,'old-correlation','old-execution-operation','old-execution-fingerprint');
         INSERT INTO sync_execution_items(execution_item_id, execution_id, plan_item_id, state, attempt_count, platform_request_id, error_reason)
         VALUES ('old-execution-item','old-execution','old-item','outcome_unknown',2,'original-request','receipt-unconfirmed');"
    ).unwrap();
    let before = legacy_sync_rows(&persistence.connection);
    assert!(before.iter().all(|rows| rows.len() == 1));
    drop(persistence);
    let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    assert_eq!(legacy_sync_rows(&reopened.connection), before);
    let violations: i64 = reopened
        .connection
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}

#[test]
fn disabled_identity_invalidates_current_confirmations_with_durable_events() {
    let fixture = TestDatabase::new("identity-invalidation");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_snapshot_workspace(&mut persistence, false);
    persistence
        .transact(|repositories| {
            repositories.commit_local_snapshot(&local_snapshot(
                "identity-local-1",
                1,
                10,
                vec![local_observation(
                    "identity-file-1",
                    "note.md",
                    "dev:1:ino:88",
                    "identity-checksum",
                )],
            ))
        })
        .unwrap();
    persistence
        .connection
        .execute(
            "INSERT INTO file_confirmations(confirmation_id, workspace_id, local_file_id, confirmed_by_account_id, confirmed_checksum_algorithm, confirmed_checksum_value, target_scope, result, source_client, created_at) VALUES ('identity-confirmation', 'w-snapshot', 'identity-file-1', 'local-profile:default', 'sha256', 'identity-checksum', 'content', 'confirmed', 'desktop', 11)",
            [],
        )
        .unwrap();
    persistence
        .connection
        .execute(
            "UPDATE accounts SET account_status='disabled', updated_at=12 WHERE account_id='local-profile:default'",
            [],
        )
        .unwrap();
    let invalidations = persistence
        .transact(|repositories| {
            contexthub_application::invalidate_identity_confirmations(
                repositories,
                &InvalidateIdentityConfirmationsCommand {
                    identity_id: AccountId::new("local-profile:default"),
                    invalidation_id_prefix: "identity-invalidation:".to_owned(),
                    event_id_prefix: "identity-event:".to_owned(),
                    correlation_id: "identity-correlation".to_owned(),
                    occurred_at: 12,
                },
            )
        })
        .unwrap();
    assert_eq!(invalidations.len(), 1);
    assert_eq!(invalidations[0].reason, "identity_changed");
    let audit: (String, String) = persistence
        .connection
        .query_row(
            "SELECT reason, event_type FROM confirmation_invalidations JOIN durable_events ON durable_events.object_id=confirmation_invalidations.confirmation_id WHERE confirmation_invalidations.confirmation_id='identity-confirmation'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        audit,
        (
            "identity_changed".to_owned(),
            "file.confirmation_invalidated".to_owned()
        )
    );
}

#[test]
fn durable_event_store_replays_global_and_workspace_pages_without_gaps() {
    let fixture = TestDatabase::new("event-replay");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_event_store(&mut persistence);

    let first = persistence.replay_global_after(0, 2).unwrap();
    assert_eq!(event_sequences(&first), vec![1, 2]);
    assert_eq!(first.next_sequence, 2);
    assert!(first.has_more);
    let second = persistence
        .replay_global_after(first.next_sequence, 2)
        .unwrap();
    assert_eq!(event_sequences(&second), vec![3, 4]);
    assert!(second.has_more);
    let third = persistence
        .replay_global_after(second.next_sequence, 2)
        .unwrap();
    assert_eq!(event_sequences(&third), vec![5]);
    assert!(!third.has_more);

    let workspace_first = persistence
        .replay_workspace_after(&WorkspaceId::new("w1"), 0, 2)
        .unwrap();
    assert_eq!(
        workspace_first
            .events
            .iter()
            .map(|event| event.workspace_sequence.unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(workspace_first.next_sequence, 2);
    assert!(workspace_first.has_more);
    let workspace_second = persistence
        .replay_workspace_after(&WorkspaceId::new("w1"), workspace_first.next_sequence, 2)
        .unwrap();
    assert_eq!(workspace_second.events[0].event_id.as_str(), "e5");
    assert_eq!(workspace_second.next_sequence, 3);
}

#[test]
fn client_event_cursor_survives_reopen_and_rejects_unavailable_replay() {
    let fixture = TestDatabase::new("event-cursor");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    seed_event_store(&mut persistence);
    persistence
        .transact(|repositories| {
            assert_eq!(
                repositories.acknowledge_event_cursor(
                    "desktop-main",
                    &AccountId::new("a1"),
                    4,
                    10,
                )?,
                4
            );
            assert_eq!(
                repositories.acknowledge_event_cursor(
                    "desktop-main",
                    &AccountId::new("a1"),
                    2,
                    11,
                )?,
                4
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        persistence
            .client_cursor("desktop-main", &AccountId::new("a1"))
            .unwrap(),
        Some(4)
    );
    let future_cursor = persistence
        .transact(|repositories| {
            repositories.acknowledge_event_cursor("desktop-main", &AccountId::new("a1"), 6, 12)
        })
        .unwrap_err();
    assert_eq!(future_cursor.kind(), PersistenceErrorKind::Conflict);

    drop(persistence);
    let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    assert_eq!(
        reopened
            .client_cursor("desktop-main", &AccountId::new("a1"))
            .unwrap(),
        Some(4)
    );
    let missed = reopened.replay_global_after(4, 10).unwrap();
    assert_eq!(event_sequences(&missed), vec![5]);
    reopened
        .connection
        .execute("DELETE FROM durable_events WHERE global_sequence=1", [])
        .unwrap();
    let expired = reopened.replay_global_after(0, 10).unwrap_err();
    assert_eq!(expired.kind(), PersistenceErrorKind::ReplayUnavailable);
    let available = reopened.replay_global_available(10).unwrap();
    assert_eq!(event_sequences(&available), vec![2, 3, 4, 5]);
    let ahead = reopened.replay_global_after(6, 10).unwrap_err();
    assert_eq!(ahead.kind(), PersistenceErrorKind::ReplayUnavailable);
    let invalid_page = reopened.replay_global_after(4, 0).unwrap_err();
    assert_eq!(invalid_page.kind(), PersistenceErrorKind::Configuration);
}

#[test]
fn task_manager_persists_lifecycle_progress_result_and_attempt() {
    let fixture = TestDatabase::new("task-lifecycle");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let task = test_task("t1", true);
    persistence
        .transact(|repositories| repositories.insert_task(&task))
        .unwrap();
    let (started, attempt) = persistence
        .transact(|repositories| {
            repositories.start_task(
                &TaskId::new("t1"),
                &TaskAttemptId::new("ta1"),
                "server-1",
                2,
            )
        })
        .unwrap();
    assert_eq!(started.state, TaskState::Running);
    assert_eq!(started.started_at, Some(2));
    assert_eq!(attempt.attempt_no, 1);
    assert_eq!(attempt.state, TaskAttemptState::Running);

    let progressed = persistence
        .transact(|repositories| {
            repositories.update_task_progress(&TaskProgress {
                task_id: TaskId::new("t1"),
                progress_completed: 2,
                progress_total: Some(5),
                current_stage: Some("scan".to_owned()),
                heartbeat_at: 3,
            })
        })
        .unwrap();
    assert_eq!(progressed.progress_completed, 2);
    assert_eq!(progressed.progress_total, Some(5));
    assert_invalid_progress(&mut persistence, 1, Some(5));
    assert_invalid_progress(&mut persistence, 3, Some(6));

    for (expected_state, target_state, occurred_at) in [
        (TaskState::Running, TaskState::WaitingForUser, 5),
        (TaskState::WaitingForUser, TaskState::Running, 6),
    ] {
        persistence
            .transact(|repositories| {
                transition_without_result(
                    repositories,
                    "t1",
                    expected_state,
                    target_state,
                    occurred_at,
                )
            })
            .unwrap();
    }
    persistence
        .transact(|repositories| {
            let completed = repositories.transition_task(&TaskTransition {
                task_id: TaskId::new("t1"),
                expected_state: TaskState::Running,
                target_state: TaskState::Succeeded,
                occurred_at: 7,
                result_ref_type: Some("sync_plan".to_owned()),
                result_ref_id: Some("sp1".to_owned()),
                error_reason: None,
                error_detail_json: None,
            })?;
            repositories.append_event(&task_event("task-e1", "t1", 7))?;
            Ok(completed)
        })
        .unwrap();
    drop(persistence);

    let mut reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let completed = reopened
        .transact(|repositories| repositories.task(&TaskId::new("t1")))
        .unwrap()
        .unwrap();
    assert_eq!(completed.state, TaskState::Succeeded);
    assert_eq!(completed.progress_completed, 2);
    assert_eq!(completed.result_ref_id.as_deref(), Some("sp1"));
    assert_eq!(completed.completed_at, Some(7));
    let attempt_state: String = reopened
        .connection
        .query_row(
            "SELECT state FROM task_attempts WHERE attempt_id='ta1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(attempt_state, "succeeded");
}

#[test]
fn task_manager_rolls_back_terminal_event_and_restarts_interrupted_attempt() {
    let fixture = TestDatabase::new("task-restart");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence
        .transact(|repositories| {
            repositories.insert_task(&test_task("t2", true))?;
            repositories.start_task(
                &TaskId::new("t2"),
                &TaskAttemptId::new("ta21"),
                "server-1",
                2,
            )?;
            repositories.append_event(&task_event("duplicate", "t2", 2))?;
            Ok(())
        })
        .unwrap();
    let rollback = persistence
        .transact(|repositories| {
            repositories.transition_task(&failed_transition("t2", 3))?;
            repositories.append_event(&task_event("duplicate", "t2", 3))?;
            Ok(())
        })
        .unwrap_err();
    assert_eq!(rollback.kind(), PersistenceErrorKind::Database);
    let after_rollback = persistence
        .transact(|repositories| repositories.task(&TaskId::new("t2")))
        .unwrap()
        .unwrap();
    assert_eq!(after_rollback.state, TaskState::Running);

    persistence
        .transact(|repositories| {
            repositories.transition_task(&TaskTransition {
                task_id: TaskId::new("t2"),
                expected_state: TaskState::Running,
                target_state: TaskState::Interrupted,
                occurred_at: 4,
                result_ref_type: None,
                result_ref_id: None,
                error_reason: Some("server_stopped".to_owned()),
                error_detail_json: None,
            })?;
            let (_, second_attempt) = repositories.start_task(
                &TaskId::new("t2"),
                &TaskAttemptId::new("ta22"),
                "server-2",
                5,
            )?;
            assert_eq!(second_attempt.attempt_no, 2);
            repositories.transition_task(&TaskTransition {
                task_id: TaskId::new("t2"),
                expected_state: TaskState::Running,
                target_state: TaskState::Cancelled,
                occurred_at: 6,
                result_ref_type: None,
                result_ref_id: None,
                error_reason: Some("user_cancelled".to_owned()),
                error_detail_json: None,
            })?;
            Ok(())
        })
        .unwrap();
    let attempts = persistence
        .connection
        .prepare(
            "SELECT attempt_no, state FROM task_attempts WHERE task_id='t2' ORDER BY attempt_no",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        attempts,
        vec![(1, "interrupted".to_owned()), (2, "interrupted".to_owned())]
    );
}

#[test]
fn task_manager_reconciles_unknown_outcome_and_rejects_forbidden_cancellation() {
    let fixture = TestDatabase::new("task-reconcile");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    let reconciled = persistence
        .transact(|repositories| {
            repositories.insert_task(&test_task("t4", false))?;
            repositories.start_task(
                &TaskId::new("t4"),
                &TaskAttemptId::new("ta41"),
                "server-2",
                7,
            )?;
            repositories.transition_task(&TaskTransition {
                task_id: TaskId::new("t4"),
                expected_state: TaskState::Running,
                target_state: TaskState::OutcomeUnknown,
                occurred_at: 8,
                result_ref_type: None,
                result_ref_id: None,
                error_reason: Some("platform_result_unconfirmed".to_owned()),
                error_detail_json: None,
            })?;
            repositories.transition_task(&TaskTransition {
                task_id: TaskId::new("t4"),
                expected_state: TaskState::OutcomeUnknown,
                target_state: TaskState::Succeeded,
                occurred_at: 9,
                result_ref_type: Some("submission".to_owned()),
                result_ref_id: Some("submission-1".to_owned()),
                error_reason: None,
                error_detail_json: None,
            })
        })
        .unwrap();
    assert_eq!(reconciled.state, TaskState::Succeeded);
    assert_eq!(reconciled.completed_at, Some(9));

    persistence
        .transact(|repositories| repositories.insert_task(&test_task("t3", false)))
        .unwrap();
    let cancellation = persistence
        .transact(|repositories| {
            repositories.transition_task(&TaskTransition {
                task_id: TaskId::new("t3"),
                expected_state: TaskState::Queued,
                target_state: TaskState::Cancelled,
                occurred_at: 7,
                result_ref_type: None,
                result_ref_id: None,
                error_reason: Some("user_cancelled".to_owned()),
                error_detail_json: None,
            })
        })
        .unwrap_err();
    assert_eq!(cancellation.kind(), PersistenceErrorKind::Conflict);
}

fn seed_event_store(persistence: &mut SqlitePersistence) {
    persistence
        .transact(|repositories| {
            insert_historical_account(&repositories.transaction, "a1", "Account", 1)?;
            for (workspace_id, local_root) in [("w1", "/tmp/w1"), ("w2", "/tmp/w2")] {
                repositories.insert_workspace(&WorkspaceRecord {
                    workspace_id: WorkspaceId::new(workspace_id),
                    workspace_kind: WorkspaceKind::Local,
                    display_name: workspace_id.to_owned(),
                    local_root: local_root.to_owned(),
                    local_root_identity: None,
                    local_status: LocalWorkspaceStatus::Ready,
                    state_revision: 0,
                    last_local_scan_at: None,
                    created_at: 1,
                    updated_at: 1,
                })?;
            }
            for event in [
                replay_test_event("e1", Some("w1"), 1),
                replay_test_event("e2", None, 2),
                replay_test_event("e3", Some("w2"), 3),
                replay_test_event("e4", Some("w1"), 4),
                replay_test_event("e5", Some("w1"), 5),
            ] {
                repositories.append_event(&event)?;
            }
            Ok(())
        })
        .unwrap();
}

fn event_sequences(page: &EventReplayPage) -> Vec<i64> {
    page.events
        .iter()
        .map(|event| event.global_sequence)
        .collect()
}

fn replay_test_event(
    event_id: &str,
    workspace_id: Option<&str>,
    timestamp: i64,
) -> NewDurableEvent {
    NewDurableEvent {
        event_id: EventId::new(event_id),
        workspace_id: workspace_id.map(WorkspaceId::new),
        event_type: "test.event".to_owned(),
        object_type: None,
        object_id: None,
        payload_json: format!("{{\"at\":{timestamp}}}"),
        correlation_id: None,
        occurred_at: timestamp,
        created_at: timestamp,
    }
}

fn workspace_event(event_id: &str, timestamp: i64) -> NewDurableEvent {
    NewDurableEvent {
        event_id: EventId::new(event_id),
        workspace_id: Some(WorkspaceId::new("w1")),
        event_type: "workspace.changed".to_owned(),
        object_type: Some("workspace".to_owned()),
        object_id: Some("w1".to_owned()),
        payload_json: "{}".to_owned(),
        correlation_id: Some(format!("c{timestamp}")),
        occurred_at: timestamp,
        created_at: timestamp,
    }
}

fn seed_snapshot_workspace(persistence: &mut SqlitePersistence, bind_platform: bool) {
    persistence
        .transact(|repositories| {
            repositories.insert_workspace(&WorkspaceRecord {
                workspace_id: WorkspaceId::new("w-snapshot"),
                workspace_kind: WorkspaceKind::Local,
                display_name: "Snapshot workspace".to_owned(),
                local_root: "/tmp/w-snapshot".to_owned(),
                local_root_identity: Some("dev:1:ino:1".to_owned()),
                local_status: LocalWorkspaceStatus::Scanning,
                state_revision: 0,
                last_local_scan_at: None,
                created_at: 1,
                updated_at: 1,
            })?;
            repositories.initialize_workspace_export(&WorkspaceExportRecord {
                workspace_id: WorkspaceId::new("w-snapshot"),
                desired_revision: 1,
                exported_revision: 0,
                updated_at: 1,
            })?;
            if bind_platform {
                insert_historical_account(
                    &repositories.transaction,
                    "account-snapshot",
                    "Snapshot account",
                    1,
                )?;
                insert_historical_platform_binding(
                    &repositories.transaction,
                    &WorkspacePlatformBindingRecord {
                        workspace_id: WorkspaceId::new("w-snapshot"),
                        account_id: AccountId::new("account-snapshot"),
                        organization_id: Some("organization-1".to_owned()),
                        platform_workspace_id: Some("platform-workspace-1".to_owned()),
                        project_id: "project-1".to_owned(),
                        platform_root: "/docs".to_owned(),
                        binding_state: WorkspacePlatformBindingState::Bound,
                        platform_snapshot_revision: None,
                        last_platform_check_at: None,
                        team_space_id: None,
                        managed_root: None,
                        device_id: None,
                        baseline_revision: None,
                        change_cursor: None,
                        permission_role: None,
                        permission_revision: None,
                        created_at: 1,
                        updated_at: 1,
                    },
                )?;
            }
            Ok(())
        })
        .unwrap();
}

fn seed_project_outcome_file(persistence: &mut SqlitePersistence) {
    seed_snapshot_workspace(persistence, false);
    persistence
        .transact(|repositories| {
            repositories.commit_local_snapshot(&local_snapshot(
                "project-snapshot",
                1,
                2,
                vec![local_observation(
                    "project-file",
                    "交付件/方案.md",
                    "dev:1:ino:2",
                    "project-checksum",
                )],
            ))
        })
        .unwrap();
}

fn seed_progress_project(persistence: &mut SqlitePersistence) {
    seed_snapshot_workspace(persistence, false);
    persistence.connection.execute_batch(
        "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('project-progress-1','推进项目','形成可验收成果','active',0,'seed-project-progress','seed-project-progress',1,1);
         INSERT INTO project_workspaces(project_id,workspace_id,role,created_at)
         VALUES ('project-progress-1','w-snapshot','primary',1);
         INSERT INTO deliverables(deliverable_id,project_id,title,deliverable_type,owner_label,acceptance_criteria,status,current_accepted_version_id,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
         VALUES ('deliverable-progress-1','project-progress-1','阶段成果','document','Mika','内容完整','draft',NULL,0,'seed-deliverable-progress','seed-deliverable-progress',2,2);
         INSERT INTO deliverable_versions(version_id,deliverable_id,version_no,status,summary,client_operation_id,request_fingerprint,created_at,accepted_at)
         VALUES ('deliverable-progress-version-1','deliverable-progress-1',1,'accepted','已验收版本','seed-version-progress','seed-version-progress',3,4);
         UPDATE deliverables SET status='accepted',current_accepted_version_id='deliverable-progress-version-1',state_revision=1,updated_at=4
         WHERE deliverable_id='deliverable-progress-1';",
    ).unwrap();
}

fn phase_input(phase_id: &str, name: &str, sort_order: i64) -> PhaseDraftInput {
    PhaseDraftInput {
        phase_id: phase_id.to_owned(),
        name: name.to_owned(),
        goal: format!("完成{name}"),
        acceptance_criteria: format!("{name}已验收"),
        required_deliverable_id: Some("deliverable-progress-1".to_owned()),
        sort_order,
        predecessor_phase_ids: if sort_order > 0 {
            vec![format!("phase-{sort_order}")]
        } else {
            Vec::new()
        },
    }
}

fn create_active_phase_plan(persistence: &mut SqlitePersistence) {
    let plan_command = CreatePhasePlanCommand {
        plan_id: "phase-plan-1".to_owned(),
        project_id: "project-progress-1".to_owned(),
        summary: "从需求推进到验收".to_owned(),
        phases: vec![
            phase_input("phase-1", "需求定义", 0),
            phase_input("phase-2", "开发验收", 1),
        ],
        client_operation_id: "op-phase-plan-1".to_owned(),
        request_fingerprint: "fp-phase-plan-1".to_owned(),
        event_id: EventId::new("event-phase-plan-1"),
        correlation_id: "correlation-phase-plan-1".to_owned(),
        occurred_at: 10,
    };
    let draft = persistence
        .transact(|repositories| {
            contexthub_application::create_phase_plan(repositories, &plan_command)
        })
        .unwrap();
    assert_eq!(draft.status, PhasePlanStatus::Draft);
    let activation = ActivatePhasePlanCommand {
        plan_id: draft.plan_id,
        expected_revision: draft.state_revision,
        client_operation_id: "op-phase-activate-1".to_owned(),
        request_fingerprint: "fp-phase-activate-1".to_owned(),
        event_id: EventId::new("event-phase-activate-1"),
        correlation_id: "correlation-phase-activate-1".to_owned(),
        occurred_at: 11,
    };
    let active = persistence
        .transact(|repositories| {
            contexthub_application::activate_phase_plan(repositories, &activation)
        })
        .unwrap();
    assert_eq!(active.status, PhasePlanStatus::Active);
}

fn phase_transition(
    phase_id: &str,
    target_status: ProjectPhaseStatus,
    expected_revision: i64,
    occurred_at: i64,
    suffix: &str,
) -> TransitionProjectPhaseCommand {
    TransitionProjectPhaseCommand {
        transition_id: format!("phase-transition-{suffix}"),
        phase_id: phase_id.to_owned(),
        target_status,
        expected_revision,
        actor_label: "Mika".to_owned(),
        comment: "确认推进".to_owned(),
        client_operation_id: format!("op-phase-transition-{suffix}"),
        request_fingerprint: format!("fp-phase-transition-{suffix}"),
        event_id: EventId::new(format!("event-phase-transition-{suffix}")),
        correlation_id: format!("correlation-phase-transition-{suffix}"),
        occurred_at,
    }
}

fn transition_phase(
    persistence: &mut SqlitePersistence,
    phase_id: &str,
    target_status: ProjectPhaseStatus,
    expected_revision: i64,
    occurred_at: i64,
) -> ProjectPhaseRecord {
    let suffix = format!("{phase_id}-{occurred_at}");
    let command = phase_transition(
        phase_id,
        target_status,
        expected_revision,
        occurred_at,
        &suffix,
    );
    persistence
        .transact(|repositories| {
            contexthub_application::transition_project_phase(repositories, &command)
        })
        .unwrap()
}

fn commit_carrier() -> DeliverableCarrierInput {
    DeliverableCarrierInput {
        carrier_id: "carrier-commit-1".to_owned(),
        carrier_type: "commit".to_owned(),
        carrier_role: "supporting".to_owned(),
        label: "实现提交".to_owned(),
        reference: "abc1234".to_owned(),
        version_label: "提交 abc1234".to_owned(),
        source_version_id: None,
    }
}

fn create_and_move_business_task(persistence: &mut SqlitePersistence) {
    let command = CreateBusinessTaskCommand {
        business_task_id: "business-task-1".to_owned(),
        project_id: "project-progress-1".to_owned(),
        phase_id: Some("phase-1".to_owned()),
        title: "完成需求说明".to_owned(),
        objective: "明确开发边界".to_owned(),
        input_context: "用户访谈".to_owned(),
        expected_output: "需求说明".to_owned(),
        completion_criteria: "范围、例外和验收口径完整".to_owned(),
        assignee_type: "user".to_owned(),
        assignee_label: "Mika".to_owned(),
        priority: "high".to_owned(),
        due_at: Some(100),
        client_operation_id: "op-business-task-1".to_owned(),
        request_fingerprint: "fp-business-task-1".to_owned(),
        event_id: EventId::new("event-business-task-1"),
        correlation_id: "correlation-business-task-1".to_owned(),
        occurred_at: 14,
    };
    let task = persistence
        .transact(|repositories| {
            contexthub_application::create_business_task(repositories, &command)
        })
        .unwrap();
    assert_eq!(task.status, BusinessTaskStatus::Candidate);
    assert_eq!(my_work_items(persistence)[0].focus_group, "todo");
    for (revision, target_phase, occurred_at) in [(0, "phase-2", 15), (1, "phase-1", 16)] {
        move_business_task(persistence, revision, target_phase, occurred_at);
    }
    let move_count: i64 = persistence
        .connection
        .query_row("SELECT COUNT(*) FROM business_task_moves", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(move_count, 2);
}

fn move_business_task(
    persistence: &mut SqlitePersistence,
    expected_revision: i64,
    target_phase: &str,
    occurred_at: i64,
) {
    let command = MoveBusinessTaskCommand {
        move_id: format!("business-task-move-{target_phase}"),
        business_task_id: "business-task-1".to_owned(),
        target_phase_id: target_phase.to_owned(),
        expected_revision,
        actor_label: "Mika".to_owned(),
        dependency_impact: "依赖随目标阶段重新确认".to_owned(),
        deliverable_impact: "输出归属随阶段调整".to_owned(),
        acceptance_impact: "原阶段与目标阶段验收口径已确认".to_owned(),
        client_operation_id: format!("op-business-task-move-{target_phase}"),
        request_fingerprint: format!("fp-business-task-move-{target_phase}"),
        event_id: EventId::new(format!("event-business-task-move-{target_phase}")),
        correlation_id: format!("correlation-business-task-move-{target_phase}"),
        occurred_at,
    };
    let moved = persistence
        .transact(|repositories| contexthub_application::move_business_task(repositories, &command))
        .unwrap();
    assert_eq!(moved.phase_id.as_deref(), Some(target_phase));
}

fn advance_business_task_through_rework(persistence: &mut SqlitePersistence) {
    for (revision, target, now, expected_group) in [
        (2, BusinessTaskStatus::Todo, 17, Some("todo")),
        (3, BusinessTaskStatus::InProgress, 18, Some("in_progress")),
        (4, BusinessTaskStatus::Review, 19, Some("review")),
        (5, BusinessTaskStatus::Rework, 20, Some("todo")),
        (6, BusinessTaskStatus::InProgress, 21, Some("in_progress")),
        (7, BusinessTaskStatus::Review, 22, Some("review")),
        (8, BusinessTaskStatus::Completed, 23, None),
    ] {
        transition_task(persistence, target, revision, now);
        assert_eq!(
            my_work_items(persistence)
                .first()
                .map(|item| item.focus_group.as_str()),
            expected_group
        );
    }
}

fn transition_task(
    persistence: &mut SqlitePersistence,
    target_status: BusinessTaskStatus,
    expected_revision: i64,
    occurred_at: i64,
) {
    let suffix = format!("{}-{occurred_at}", target_status.as_str());
    let command = TransitionBusinessTaskCommand {
        transition_id: format!("business-task-transition-{suffix}"),
        business_task_id: "business-task-1".to_owned(),
        target_status,
        expected_revision,
        actor_label: "Mika".to_owned(),
        comment: "确认任务状态".to_owned(),
        client_operation_id: format!("op-business-task-transition-{suffix}"),
        request_fingerprint: format!("fp-business-task-transition-{suffix}"),
        event_id: EventId::new(format!("event-business-task-transition-{suffix}")),
        correlation_id: format!("correlation-business-task-transition-{suffix}"),
        occurred_at,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::transition_business_task(repositories, &command)
        })
        .unwrap();
}

fn attention_create(
    id: &str,
    case_type: AttentionCaseType,
    deliverable_id: Option<&str>,
    agent: Option<(&str, &str)>,
    occurred_at: i64,
) -> CreateAttentionCaseCommand {
    CreateAttentionCaseCommand {
        attention_case_id: id.to_owned(),
        case_type,
        project_id: "project-progress-1".to_owned(),
        phase_id: Some("phase-1".to_owned()),
        task_id: None,
        deliverable_id: deliverable_id.map(str::to_owned),
        title: format!("关注事项 {id}"),
        detail: "需要及时处理".to_owned(),
        responsibility_type: "user".to_owned(),
        responsibility_label: "Mika".to_owned(),
        reporter_label: "Mika".to_owned(),
        priority: "high".to_owned(),
        due_at: None,
        agent_run_id: agent.map(|value| value.0.to_owned()),
        agent_label: agent.map(|value| value.1.to_owned()),
        decision_options: (case_type == AttentionCaseType::DecisionRequest)
            .then(|| "方案 A；方案 B".to_owned()),
        decision_impact: (case_type == AttentionCaseType::DecisionRequest)
            .then(|| "影响项目路线".to_owned()),
        decision_evidence: (case_type == AttentionCaseType::DecisionRequest)
            .then(|| "已验收成果与风险记录".to_owned()),
        decision_recommendation: (case_type == AttentionCaseType::DecisionRequest)
            .then(|| "推荐方案 A".to_owned()),
        decision_consequence: (case_type == AttentionCaseType::DecisionRequest)
            .then(|| "延迟下一阶段".to_owned()),
        client_operation_id: format!("op-{id}"),
        request_fingerprint: format!("fp-{id}"),
        event_id: EventId::new(format!("event-{id}")),
        correlation_id: format!("correlation-{id}"),
        occurred_at,
    }
}

fn seed_attention_cases(persistence: &mut SqlitePersistence) {
    let blocker = attention_create(
        "blocker-1",
        AttentionCaseType::ProjectBlocker,
        None,
        None,
        20,
    );
    let commands = [
        blocker.clone(),
        attention_create(
            "decision-1",
            AttentionCaseType::DecisionRequest,
            Some("deliverable-progress-1"),
            None,
            21,
        ),
        attention_create(
            "agent-1",
            AttentionCaseType::AgentRunIncident,
            None,
            Some(("run-1", "Research Agent")),
            22,
        ),
    ];
    for command in &commands {
        persistence
            .transact(|repositories| {
                contexthub_application::create_attention_case(repositories, command)
            })
            .unwrap();
    }
    let replayed = persistence
        .transact(|repositories| {
            contexthub_application::create_attention_case(repositories, &blocker)
        })
        .unwrap();
    assert!(replayed.replayed);
}

fn assert_attention_projection(persistence: &mut SqlitePersistence) {
    let items = my_work_items(persistence);
    assert_eq!(items.len(), 2);
    for (source, group, action) in [
        ("project_blocker", "todo", "blocker_resolve"),
        ("decision_request", "decision", "decision_respond"),
    ] {
        assert!(items.iter().any(|item| item.source_type == source
            && item.focus_group == group
            && item.action_type == action));
    }
}

fn assert_attention_transitions(persistence: &mut SqlitePersistence) {
    let invalid = attention_transition(
        "invalid-blocker-ack",
        "blocker-1",
        AttentionCaseStatus::Acknowledged,
        0,
        "",
        23,
    );
    let error = persistence
        .transact(|repositories| {
            contexthub_application::transition_attention_case(repositories, &invalid)
        })
        .unwrap_err();
    assert_eq!(error.reason(), Some("invalid_attention_transition"));

    persist_attention_transition(
        persistence,
        &attention_transition(
            "agent-ack",
            "agent-1",
            AttentionCaseStatus::Acknowledged,
            0,
            "",
            24,
        ),
    );
    assert!(
        my_work_items(persistence)
            .iter()
            .all(|item| item.source_id != "agent-1")
    );
    let mut delegated = attention_transition(
        "decision-delegate",
        "decision-1",
        AttentionCaseStatus::Open,
        0,
        "由交付负责人做最终判断",
        25,
    );
    delegated.decision_action = Some("delegated".to_owned());
    delegated.delegated_to = Some("交付负责人".to_owned());
    persist_attention_transition(persistence, &delegated);
    let delegated_item = my_work_items(persistence)
        .into_iter()
        .find(|item| item.source_id == "decision-1")
        .unwrap();
    assert_eq!(delegated_item.responsibility_label, "交付负责人");
    for transition in [
        attention_transition(
            "blocker-resolve",
            "blocker-1",
            AttentionCaseStatus::Resolved,
            0,
            "依赖已补齐",
            26,
        ),
        attention_transition(
            "decision-resolve",
            "decision-1",
            AttentionCaseStatus::Resolved,
            1,
            "采用方案 A",
            27,
        ),
        attention_transition(
            "agent-resolve",
            "agent-1",
            AttentionCaseStatus::Resolved,
            1,
            "外部执行器已恢复",
            28,
        ),
    ] {
        persist_attention_transition(persistence, &transition);
    }
    let decision = persistence
        .transact(|repositories| repositories.attention_cases("project-progress-1"))
        .unwrap()
        .into_iter()
        .find(|item| item.attention_case_id == "decision-1")
        .unwrap();
    assert_eq!(decision.decision_action.as_deref(), Some("approved"));
    assert_eq!(
        decision.decision_action_summary.as_deref(),
        Some("采用方案 A")
    );
    assert_eq!(decision.decision_actor_label.as_deref(), Some("Mika"));
    assert!(decision.decision_delegated_to.is_none());
    assert!(my_work_items(persistence).is_empty());
}

fn persist_attention_transition(
    persistence: &mut SqlitePersistence,
    transition: &TransitionAttentionCaseCommand,
) {
    persistence
        .transact(|repositories| {
            contexthub_application::transition_attention_case(repositories, transition)
        })
        .unwrap();
}

fn assert_attention_events(persistence: &SqlitePersistence) {
    let event_types = persistence.connection.prepare("SELECT event_type FROM durable_events WHERE object_type='attention_case' ORDER BY global_sequence").unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(
        event_types,
        vec![
            "project_blocker.opened",
            "decision_request.opened",
            "agent_run_incident.reported",
            "attention_case.acknowledged",
            "decision_request.delegated",
            "attention_case.resolved",
            "attention_case.resolved",
            "attention_case.resolved"
        ]
    );
}

fn attention_transition(
    suffix: &str,
    attention_case_id: &str,
    target_status: AttentionCaseStatus,
    expected_revision: i64,
    resolution_summary: &str,
    occurred_at: i64,
) -> TransitionAttentionCaseCommand {
    TransitionAttentionCaseCommand {
        transition_id: format!("attention-transition-{suffix}"),
        attention_case_id: attention_case_id.to_owned(),
        target_status,
        expected_revision,
        actor_label: "Mika".to_owned(),
        resolution_summary: resolution_summary.to_owned(),
        decision_action: (attention_case_id == "decision-1").then(|| "approved".to_owned()),
        delegated_to: None,
        client_operation_id: format!("op-attention-transition-{suffix}"),
        request_fingerprint: format!("fp-attention-transition-{suffix}"),
        event_id: EventId::new(format!("event-attention-transition-{suffix}")),
        correlation_id: format!("correlation-attention-transition-{suffix}"),
        occurred_at,
    }
}

fn my_work_items(persistence: &mut SqlitePersistence) -> Vec<MyWorkItemRecord> {
    persistence
        .transact(|repositories| {
            contexthub_application::get_my_work(
                repositories,
                &MyWorkQuery {
                    project_id: None,
                    focus_group: None,
                    limit: 200,
                },
            )
        })
        .unwrap()
        .items
}

fn assert_project_outcome_events(persistence: &SqlitePersistence) {
    let event_types = persistence
        .connection
        .prepare("SELECT event_type FROM durable_events ORDER BY global_sequence")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(event_types.contains(&"project.created".to_owned()));
    assert!(event_types.contains(&"deliverable.version.accepted".to_owned()));
}

fn local_observation(
    proposed_id: &str,
    normalized_path: &str,
    identity: &str,
    checksum: &str,
) -> LocalFileObservation {
    LocalFileObservation {
        proposed_local_file_id: proposed_id.to_owned(),
        relative_path: normalized_path.to_owned(),
        normalized_relative_path: normalized_path.to_owned(),
        file_identity: Some(identity.to_owned()),
        file_type: "file".to_owned(),
        size_bytes: Some(10),
        modified_at_fs: Some(5),
        checksum_algorithm: Some("sha256".to_owned()),
        checksum_value: Some(checksum.to_owned()),
    }
}

fn local_snapshot(
    snapshot_id: &str,
    generation: i64,
    timestamp: i64,
    observations: Vec<LocalFileObservation>,
) -> LocalScanSnapshot {
    LocalScanSnapshot {
        snapshot_id: snapshot_id.to_owned(),
        workspace_id: WorkspaceId::new("w-snapshot"),
        generation,
        snapshot_kind: SnapshotKind::Full,
        observed_at: timestamp,
        completed_at: timestamp,
        issue_count: 0,
        event_id: EventId::new(format!("event-{snapshot_id}")),
        correlation_id: format!("correlation-{snapshot_id}"),
        observations,
        removed_normalized_paths: Vec::new(),
        covers_whole_tree: true,
    }
}

fn platform_snapshot(
    snapshot_id: &str,
    revision: &str,
    path: &str,
    version_id: &str,
    timestamp: i64,
) -> HistoricalPlatformSnapshot {
    HistoricalPlatformSnapshot {
        snapshot_id: snapshot_id.into(),
        revision: revision.into(),
        path: path.into(),
        version_id: version_id.into(),
        timestamp,
    }
}

fn state_input(
    local_checksum: &str,
    latest_version: &str,
    latest_checksum: &str,
) -> FileStateInput {
    FileStateInput {
        workspace_kind: contexthub_domain::WorkspaceKind::CloudTeam,
        platform_connection: PlatformConnection::Bound,
        platform_state: PlatformDocumentState::Published,
        materialization_state: MaterializationState::Materialized,
        scan_state: LocalScanState::Current,
        node_kind: LocalNodeKind::File,
        baseline_path_relation: BaselinePathRelation::Unchanged,
        baseline_version_id: Some("version-1".to_owned()),
        baseline_checksum_algorithm: Some("sha256".to_owned()),
        baseline_checksum_value: Some("checksum-version-1".to_owned()),
        latest_published_version_id: Some(latest_version.to_owned()),
        latest_published_checksum_algorithm: Some("sha256".to_owned()),
        latest_published_checksum_value: Some(latest_checksum.to_owned()),
        local_checksum_algorithm: Some("sha256".to_owned()),
        local_checksum_value: Some(local_checksum.to_owned()),
        active_confirmation: None,
        submission_state: SubmissionState::None,
        active_conflict: None,
        permissions: FilePermissions::from_allowed([
            FilePermission::Access,
            FilePermission::Confirm,
            FilePermission::Submit,
        ]),
        metadata_validity: MetadataValidity::Valid,
    }
}

fn state_command(
    label: &str,
    expected_state_revision: Option<i64>,
    input: FileStateInput,
    computed_at: i64,
) -> RecomputeFileStateCommand {
    RecomputeFileStateCommand {
        workspace_id: WorkspaceId::new("w-snapshot"),
        local_file_id: "state-file-1".to_owned(),
        expected_state_revision,
        input,
        confirmation_invalidation_id: format!("invalidation-{label}"),
        confirmation_invalidation_event_id: EventId::new(format!("event-invalidation-{label}")),
        derived_conflict_id: format!("conflict-{label}"),
        event_id: EventId::new(format!("event-{label}")),
        correlation_id: format!("correlation-{label}"),
        computed_at,
    }
}

fn test_task(task_id: &str, cancellable: bool) -> TaskRecord {
    TaskRecord {
        task_id: TaskId::new(task_id),
        task_type: "scan".to_owned(),
        workspace_id: None,
        state: TaskState::Queued,
        progress_completed: 0,
        progress_total: None,
        current_stage: None,
        cancellable,
        payload_json: "{}".to_owned(),
        result_ref_type: None,
        result_ref_id: None,
        correlation_id: format!("correlation-{task_id}"),
        created_by_client: "desktop".to_owned(),
        created_at: 1,
        started_at: None,
        completed_at: None,
        heartbeat_at: None,
        error_reason: None,
        error_detail_json: None,
    }
}

fn task_event(event_id: &str, task_id: &str, timestamp: i64) -> NewDurableEvent {
    NewDurableEvent {
        event_id: EventId::new(event_id),
        workspace_id: None,
        event_type: "task.changed".to_owned(),
        object_type: Some("task".to_owned()),
        object_id: Some(task_id.to_owned()),
        payload_json: "{}".to_owned(),
        correlation_id: Some(format!("correlation-{task_id}")),
        occurred_at: timestamp,
        created_at: timestamp,
    }
}

fn failed_transition(task_id: &str, timestamp: i64) -> TaskTransition {
    TaskTransition {
        task_id: TaskId::new(task_id),
        expected_state: TaskState::Running,
        target_state: TaskState::Failed,
        occurred_at: timestamp,
        result_ref_type: None,
        result_ref_id: None,
        error_reason: Some("execution_failed".to_owned()),
        error_detail_json: Some("{}".to_owned()),
    }
}

fn transition_without_result(
    repositories: &mut SqliteRepositoryTransaction<'_>,
    task_id: &str,
    expected_state: TaskState,
    target_state: TaskState,
    occurred_at: i64,
) -> Result<TaskRecord, PersistenceError> {
    repositories.transition_task(&TaskTransition {
        task_id: TaskId::new(task_id),
        expected_state,
        target_state,
        occurred_at,
        result_ref_type: None,
        result_ref_id: None,
        error_reason: None,
        error_detail_json: None,
    })
}

fn assert_invalid_progress(
    persistence: &mut SqlitePersistence,
    progress_completed: i64,
    progress_total: Option<i64>,
) {
    let error = persistence
        .transact(|repositories| {
            repositories.update_task_progress(&TaskProgress {
                task_id: TaskId::new("t1"),
                progress_completed,
                progress_total,
                current_stage: Some("scan".to_owned()),
                heartbeat_at: 4,
            })
        })
        .unwrap_err();
    assert_eq!(error.kind(), PersistenceErrorKind::Conflict);
}

#[test]
fn unknown_schema_without_history_is_not_adopted() {
    let fixture = TestDatabase::new("unknown-schema");
    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute_batch("CREATE TABLE unexpected(id INTEGER PRIMARY KEY);")
        .unwrap();
    drop(connection);
    let error = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap_err();
    assert_eq!(error.kind(), PersistenceErrorKind::Migration);
    assert!(error.to_string().contains("without a migration history"));
}

#[test]
fn active_writer_is_detected_without_waiting() {
    let fixture = TestDatabase::new("writer");
    let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    validate_no_active_writer(&fixture.path).unwrap();
    persistence
        .connection
        .execute_batch("BEGIN IMMEDIATE")
        .unwrap();
    let started = std::time::Instant::now();
    let error = validate_no_active_writer(&fixture.path).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(error.kind(), PersistenceErrorKind::Database);
    persistence.connection.execute_batch("ROLLBACK").unwrap();
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "single acceptance history proves business commit, projection conflict, restoration, and retry"
)]
fn projection_export_tracks_pending_failure_conflict_and_success_without_rolling_back_business() {
    let fixture = TestDatabase::new("projection-export");
    let workspace_root = TestWorkspace::new("projection-export");
    let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    persistence
        .transact(|repositories| {
            repositories.insert_workspace(&WorkspaceRecord {
                workspace_id: WorkspaceId::new("w-projection"),
                workspace_kind: WorkspaceKind::Local,
                display_name: "Projection".to_owned(),
                local_root: workspace_root.path.to_string_lossy().into_owned(),
                local_root_identity: None,
                local_status: LocalWorkspaceStatus::Ready,
                state_revision: 0,
                last_local_scan_at: None,
                created_at: 1,
                updated_at: 1,
            })?;
            repositories.initialize_workspace_export(&WorkspaceExportRecord {
                workspace_id: WorkspaceId::new("w-projection"),
                desired_revision: 1,
                exported_revision: 0,
                updated_at: 1,
            })
        })
        .unwrap();
    let mut writer = WorkspaceProjectionWriter;
    assert_eq!(
        export_workspace_projection(
            &mut persistence,
            &mut writer,
            &WorkspaceId::new("w-projection"),
            2,
        )
        .unwrap(),
        contexthub_application::ProjectionExportOutcome::Exported
    );
    let projection_directory = workspace_root.path.join(".cilai");
    let original_workspace = fs::read(projection_directory.join("workspace.json")).unwrap();
    let first_state: contexthub_manifest::ExportState =
        serde_json::from_slice(&fs::read(projection_directory.join("export-state.json")).unwrap())
            .unwrap();
    assert_eq!(first_state.database_export_revision, 1);

    // 模拟升级前的目录；数据库版本和清单摘要不变也必须完成目录迁移。
    let legacy_directory = workspace_root.path.join(".contexthub");
    fs::rename(&projection_directory, &legacy_directory).unwrap();
    assert_eq!(
        export_workspace_projection(
            &mut persistence,
            &mut writer,
            &WorkspaceId::new("w-projection"),
            2,
        )
        .unwrap(),
        contexthub_application::ProjectionExportOutcome::AlreadyCurrent
    );
    assert!(!legacy_directory.exists());
    assert_eq!(
        fs::read(projection_directory.join("workspace.json")).unwrap(),
        original_workspace
    );

    persistence
        .transact(|repositories| {
            insert_historical_account(&repositories.transaction, "a-projection", "Account", 3)?;
            insert_historical_platform_binding(
                &repositories.transaction,
                &WorkspacePlatformBindingRecord {
                    workspace_id: WorkspaceId::new("w-projection"),
                    account_id: AccountId::new("a-projection"),
                    organization_id: Some("org".to_owned()),
                    platform_workspace_id: Some("platform-workspace".to_owned()),
                    project_id: "project".to_owned(),
                    platform_root: "/docs".to_owned(),
                    binding_state: WorkspacePlatformBindingState::Bound,
                    platform_snapshot_revision: None,
                    last_platform_check_at: None,
                    team_space_id: None,
                    managed_root: None,
                    device_id: None,
                    baseline_revision: None,
                    change_cursor: None,
                    permission_role: None,
                    permission_revision: None,
                    created_at: 3,
                    updated_at: 3,
                },
            )
        })
        .unwrap();
    fs::write(projection_directory.join("workspace.json"), b"{}\n").unwrap();
    assert!(matches!(
        export_workspace_projection(
            &mut persistence,
            &mut writer,
            &WorkspaceId::new("w-projection"),
            4,
        ),
        Err(contexthub_application::ExportWorkspaceProjectionError::Writer(_))
    ));
    let after_failure: (i64, i64, String, i64) = persistence
        .connection
        .query_row(
            "SELECT e.desired_revision, e.exported_revision, e.export_state, (SELECT count(*) FROM workspace_platform_bindings WHERE workspace_id=e.workspace_id) FROM workspace_exports e WHERE e.workspace_id='w-projection'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(after_failure, (2, 1, "conflict".to_owned(), 1));

    fs::write(
        projection_directory.join("workspace.json"),
        original_workspace,
    )
    .unwrap();
    export_workspace_projection(
        &mut persistence,
        &mut writer,
        &WorkspaceId::new("w-projection"),
        5,
    )
    .unwrap();
    let final_state: (i64, i64, String, Option<String>) = persistence
        .connection
        .query_row(
            "SELECT desired_revision, exported_revision, export_state, last_error_reason FROM workspace_exports WHERE workspace_id='w-projection'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(final_state, (2, 2, "current".to_owned(), None));
    let workspace: contexthub_manifest::WorkspaceManifest =
        serde_json::from_slice(&fs::read(projection_directory.join("workspace.json")).unwrap())
            .unwrap();
    assert_eq!(workspace.project_id.as_deref(), Some("project"));
    assert_eq!(workspace.database_export_revision, 2);
}

#[test]
fn user_skill_mutations_are_revisioned_and_idempotent() {
    let database = TestDatabase::new("user-skills");
    let mut persistence = SqlitePersistence::open(&database.path, "test").unwrap();
    let save = UserSkillSaveCommand {
        client_operation_id: "save-skill-1".to_owned(),
        request_fingerprint: "a".repeat(64),
        skill_id: "skill_0123456789abcdef0123456789abcdef".to_owned(),
        name: "周报整理".to_owned(),
        normalized_name: "周报整理".to_owned(),
        description: "整理周报".to_owned(),
        managed_relative_path: "user/skill_0123456789abcdef0123456789abcdef/SKILL.md".to_owned(),
        enabled: true,
        automatic: false,
        expected_revision: None,
        content_digest: "b".repeat(64),
        occurred_at: 10,
    };
    let created = persistence.save_user_skill(&save).unwrap();
    assert_eq!(created.state_revision, 1);
    assert!(created.enabled);
    assert!(!created.automatic);

    let repeated = persistence.save_user_skill(&save).unwrap();
    assert_eq!(repeated, created);

    let updated = persistence
        .set_user_skill_state(
            &created.skill_id,
            true,
            true,
            created.state_revision,
            "set-state-1",
            &"c".repeat(64),
            11,
        )
        .unwrap();
    assert_eq!(updated.state_revision, 2);
    assert!(updated.automatic);

    let conflict = persistence
        .set_user_skill_state(
            &created.skill_id,
            false,
            false,
            created.state_revision,
            "set-state-stale",
            &"d".repeat(64),
            12,
        )
        .unwrap_err();
    assert_eq!(conflict.reason(), Some("skill_revision_conflict"));

    assert!(
        persistence
            .delete_user_skill(
                &created.skill_id,
                updated.state_revision,
                "delete-skill-1",
                &"e".repeat(64),
                13,
            )
            .unwrap()
    );
    assert!(persistence.user_skill(&created.skill_id).unwrap().is_none());
    assert!(
        persistence
            .delete_user_skill(
                &created.skill_id,
                updated.state_revision,
                "delete-skill-1",
                &"e".repeat(64),
                13,
            )
            .unwrap()
    );
    assert_eq!(
        persistence
            .delete_user_skill(
                "skill_ffffffffffffffffffffffffffffffff",
                updated.state_revision,
                "delete-skill-1",
                &"e".repeat(64),
                13,
            )
            .unwrap_err()
            .reason(),
        Some("operation_id_reused")
    );
}

#[derive(Debug)]
struct TestDatabase {
    root: PathBuf,
    path: PathBuf,
}

struct TestWorkspace {
    path: PathBuf,
}

impl TestWorkspace {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ch-workspace-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self { path }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

impl TestDatabase {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("ch-sqlite-{label}-{}-{nanos}", std::process::id()));
        fs::create_dir(&root).unwrap();
        Self {
            path: root.join("contexthub.db"),
            root,
        }
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        for suffix in ["-shm", "-wal", ""] {
            let path = PathBuf::from(format!("{}{suffix}", self.path.display()));
            if path.exists() {
                fs::remove_file(path).unwrap();
            }
        }
        let backups = self.root.join("schema-backups");
        if backups.exists() {
            fs::remove_dir_all(backups).unwrap();
        }
        fs::remove_dir(&self.root).unwrap();
    }
}
