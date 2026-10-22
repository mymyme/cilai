use super::*;
use serde_json::json;

#[test]
#[ignore = "requires an explicitly supplied read-only database snapshot"]
fn migrates_database_snapshot_in_a_fresh_native_home() {
    let source =
        PathBuf::from(std::env::var("CILAI_MIGRATION_SNAPSHOT").expect("snapshot path required"));
    let root = model_test_fixture::fixture_root("history-rehearsal");
    let data = model_test_fixture::profile_data_root(&root);
    fs::copy(&source, data.join("contexthub.db")).unwrap();
    contexthub_platform_adapter::security::restrict_existing_path(&data.join("contexthub.db"))
        .unwrap();
    let server = AppServerSupervisor::new();
    server
        .start(
            &crate::supervisor::app_server_test_binary(),
            "test",
            &data,
            &data.join("run"),
        )
        .unwrap_or_else(|error| panic!("{error}: {:?}", server.status()));
    let previous = server.request_result("aiState/get", &json!({})).unwrap();
    let original: PersistedAiState = serde_json::from_value(previous["state"].clone()).unwrap();
    let ai_dir = data.join("ai-runtime");
    fs::create_dir_all(&ai_dir).unwrap();
    let service = codex_thread_service(&ai_dir, &server).unwrap();
    let started = Instant::now();
    let (baseline, restored) = history_migration::load(&server, &service, &ai_dir)
        .unwrap_or_else(|error| panic!("{error}; isolated data: {}", data.display()));
    assert_eq!(
        persistent_threads::digest(&serde_json::to_value(&restored).unwrap()),
        persistent_threads::digest(&serde_json::to_value(&original).unwrap())
    );
    let migration_ms = started.elapsed().as_millis();
    let started = Instant::now();
    for _ in 0..20 {
        let request = baseline
            .prepare(&ledger_projection::state(&restored))
            .unwrap();
        assert!(request.request["records"].as_array().unwrap().is_empty());
    }
    let no_change_save_ms = started.elapsed().as_secs_f64() * 50.0;
    let started = Instant::now();
    for _ in 0..100 {
        let run = restored.runs.last().unwrap();
        let request = baseline.prepare_run(run, restored.runs.len() - 1).unwrap();
        assert!(request.request["records"].as_array().unwrap().is_empty());
    }
    let run_progress_save_ms = started.elapsed().as_secs_f64() * 10.0;
    service.shutdown_checked().unwrap();
    let (_, reopened) = history_migration::load(&server, &service, &ai_dir).unwrap();
    assert_eq!(
        persistent_threads::digest(&serde_json::to_value(&reopened).unwrap()),
        persistent_threads::digest(&serde_json::to_value(&original).unwrap())
    );
    let summary = json!({"conversations":restored.conversations.len(),"messages":restored.messages.len(),"runs":restored.runs.len(),"migrationMs":migration_ms,"averageUnchangedSaveMs":no_change_save_ms,"averageRunProgressSaveMs":run_progress_save_ms,"equalAfterMigration":true,"equalAfterReopen":true,"isolatedData":data});
    println!("{summary}");
    fs::write(
        root.join("summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    service.shutdown_checked().unwrap();
    server.shutdown().unwrap();
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "完整迁移、重启与搜索在同一隔离生命周期内验收"
)]
fn migrates_long_history_and_local_activity_then_reopens_and_searches_native_threads() {
    let _guard = crate::supervisor::app_server_test_guard();
    let root = model_test_fixture::fixture_root("history-migration");
    let data = model_test_fixture::profile_data_root(&root);
    let server = AppServerSupervisor::new();
    server
        .start(
            &crate::supervisor::app_server_test_binary(),
            "test",
            &data,
            &data.join("run"),
        )
        .unwrap_or_else(|error| panic!("{error}: {:?}", server.status()));
    let mut conversation = tests::fixture_conversation("agent_contexthub_orchestrator");
    conversation.context.workspace_id = None;
    conversation.context.local_file_id = None;
    conversation.context.relative_path = None;
    conversation.context.file_revision = None;
    conversation.context.workspace_root = root.to_string_lossy().into_owned();
    conversation.title = "原生迁移核对".into();
    let mut empty = conversation.clone();
    empty.conversation_id = "empty-conversation".into();
    empty.title = "空对话".into();
    empty.archived = true;
    let user = ConversationMessage {
        content: "中文原始材料".repeat(8000),
        run_id: Some("old-run".into()),
        ..tests::fixture_message(&conversation.conversation_id)
    };
    let assistant = ConversationMessage {
        message_id: "old-reply".into(),
        role: "assistant".into(),
        content: "交付回复".into(),
        ..user.clone()
    };
    let run:AgentRun=serde_json::from_value(json!({"runId":"old-run","conversationId":conversation.conversation_id,"agentId":"agent_contexthub_orchestrator","inputMessageId":user.message_id,"provider":"claude","executionTarget":"local","status":"completed","startedAt":1000,"completedAt":2000,"output":assistant.content,"events":[{"sequence":1,"eventType":"item.completed","summary":"历史工具调用","createdAt":1500}],"logTail":["历史活动"]})).unwrap();
    let saved = PersistedAiState {
        agents: vec![default_agent(0)],
        conversations: vec![conversation.clone(), empty],
        messages: vec![user.clone(), assistant.clone()],
        runs: vec![run.clone()],
        ..Default::default()
    };
    let previous = server.request_result("aiState/get", &json!({})).unwrap();
    server
        .request_result(
            "aiState/replace",
            &json!({"expectedRevision":previous["stateRevision"],"state":saved}),
        )
        .unwrap();
    let ai_dir = data.join("ai-runtime");
    fs::create_dir_all(&ai_dir).unwrap();
    let service = codex_thread_service(&ai_dir, &server).unwrap();
    let plan = history_migration::plan(&saved, &conversation.conversation_id, &ai_dir).unwrap();
    let thread = service
        .request(
            "thread/start",
            &json!({"cwd":root,"approvalPolicy":"never","sandbox":"read-only","ephemeral":false}),
        )
        .unwrap()["thread"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    server.request_result("aiHistory/execute",&json!({"action":"record","payload":{"conversationId":conversation.conversation_id,"sourceDigest":persistent_threads::digest(&plan),"nativeThreadId":thread,"status":"importing","messageCount":2,"receipt":{}}})).unwrap();
    service
        .import_history(&json!({"threadId":thread,"name":conversation.title,"turns":plan["turns"]}))
        .unwrap();
    let imported_boundary = service
        .imported_context_before(&thread, plan["turns"][0]["id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert!(imported_boundary.synthetic);
    assert!(imported_boundary.context.is_empty());
    assert!(imported_boundary.display.is_empty());
    let native_boundary = service
        .imported_context_before(&thread, "future-native-turn")
        .unwrap()
        .unwrap();
    assert!(!native_boundary.synthetic);
    assert!(native_boundary.context.is_empty());
    assert_eq!(
        native_boundary.display,
        plan["turns"].as_array().unwrap().clone()
    );
    service.shutdown_checked().unwrap();
    let manager = AiRuntimeManager::new();
    manager.initialize(&data, &server).unwrap();
    let snapshot = manager.snapshot().unwrap();
    assert_eq!(snapshot.messages, vec![user, assistant]);
    assert_eq!(snapshot.runs[0].events, run.events);
    assert_eq!(
        lock(&manager.inner).persisted.runs[0].log_tail,
        run.log_tail
    );
    assert_eq!(snapshot.runs[0].output, run.output);
    assert_eq!(snapshot.conversations.len(), 2);
    let matches = manager
        .search_conversations("原始材料", false, None, 20)
        .unwrap();
    assert_eq!(matches.items.len(), 1);
    let ledger = server.request_result("aiLedger/get", &json!({})).unwrap();
    let stored = ledger["ledger"]["messages"].to_string();
    assert!(!stored.contains("中文原始材料"));
    assert!(ledger["ledger"]["outbox"].as_array().unwrap().is_empty());
    assert!(
        server
            .request_result("aiState/get", &json!({}))
            .unwrap_err()
            .contains("ai_legacy_storage_retired")
    );
    manager.shutdown_checked().unwrap();
    let reopened = AiRuntimeManager::new();
    reopened.initialize(&data, &server).unwrap();
    assert_eq!(reopened.snapshot().unwrap().messages, snapshot.messages);
    assert_eq!(reopened.snapshot().unwrap().conversations.len(), 2);
    reopened.shutdown_checked().unwrap();
    server.shutdown().unwrap();
    fs::remove_dir_all(root).unwrap();
}
