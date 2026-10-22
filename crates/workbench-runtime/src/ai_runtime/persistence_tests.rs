use super::*;

struct Fixture {
    root: PathBuf,
    server: AppServerSupervisor,
    manager: AiRuntimeManager,
}

impl Fixture {
    fn new() -> Self {
        let root = model_test_fixture::fixture_root("ai-persistence");
        let data = model_test_fixture::profile_data_root(&root);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&data, &server).unwrap();
        Self {
            root,
            server,
            manager,
        }
    }

    fn running_reply(&self) -> PersistedAiState {
        let mut state = lock(&self.manager.inner);
        let conversation = tests::fixture_conversation("agent");
        state.native_catalog.insert(
            conversation.conversation_id.clone(),
            ("isolated-thread".into(), conversation.title.clone(), false),
        );
        state.persisted.conversations.push(conversation);
        state.persisted.runs.push(
            serde_json::from_value(serde_json::json!({
                "runId":"run","conversationId":"conversation_1","agentId":"agent",
                "inputMessageId":"message_1","provider":"codex","status":"running","startedAt":1
            }))
            .unwrap(),
        );
        persist_locked(&mut state).unwrap();
        let previous = state.persisted.clone();
        state.persisted.runs[0].status = "completed".into();
        state.persisted.runs[0].output = Some("已完成回复".into());
        let mut reply = tests::fixture_message("conversation_1");
        reply.message_id = "reply".into();
        reply.role = "assistant".into();
        reply.run_id = Some("run".into());
        reply.content = "已完成回复".into();
        state.persisted.messages.push(reply);
        previous
    }

    fn metadata(&self) -> Value {
        let loaded = self
            .server
            .request_result("aiLedger/get", &serde_json::json!({}))
            .unwrap();
        ledger::Baseline::read(&loaded["ledger"]).unwrap().1
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.manager.shutdown();
        self.server.shutdown().unwrap();
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn parked_same_thread_saves_without_reloading_the_native_catalog() {
    let _guard = crate::supervisor::app_server_test_guard();
    for (event_type, session_id, retained) in [
        ("harness.thread_parked", "isolated-thread", true),
        ("harness.thread_parked", "replacement-thread", false),
        ("harness.thread_released", "isolated-thread", false),
    ] {
        let fixture = Fixture::new();
        fixture.running_reply();
        {
            let mut state = lock(&fixture.manager.inner);
            state.persisted.runs[0].session_id = Some(session_id.into());
            state.persisted.runs[0].thread_writer_active = true;
        }
        fixture.manager.record_harness_progress(
            "run",
            CodexHarnessEvent {
                event_type: event_type.into(),
                item_id: None,
                phase: "lifecycle".into(),
                kind: "status".into(),
                status: "completed".into(),
                summary: "线程已空闲".into(),
                duration_ms: None,
                detail_available: false,
                streamed_output: None,
                steered_input: None,
                context_usage: None,
                read_paths: Vec::new(),
                created_at: 3,
            },
        );
        let mut state = lock(&fixture.manager.inner);
        assert!(!state.persisted.runs[0].thread_writer_active);
        assert_eq!(
            state.native_catalog.contains_key("conversation_1"),
            retained
        );
        if retained {
            // 夹具的线程不在官方索引中；保存成功证明无需原生目录重新发现。
            persist_locked(&mut state).unwrap();
        }
    }
}

#[test]
fn uncertain_reply_preserves_body_and_replays_only_the_original_command() {
    let _guard = crate::supervisor::app_server_test_guard();
    for already_committed in [false, true] {
        let fixture = Fixture::new();
        let previous = fixture.running_reply();
        let mut state = lock(&fixture.manager.inner);
        let pending = state
            .ledger_baseline
            .prepare(&ledger_projection::state(&state.persisted))
            .unwrap();
        let command = pending.request.clone();
        if already_committed {
            fixture
                .server
                .request_result("aiLedger/apply", &command)
                .unwrap();
        }
        state.pending_ledger_write = Some(pending);
        // 连续未收到回执时，完成回调不得把未知结果判成保存失败。
        fail_reply_completion(
            &mut state,
            previous,
            0,
            2,
            "App Server request timed out: aiLedger/apply".into(),
        );
        assert_eq!(state.persisted.runs[0].status, "completed");
        assert!(
            state.persisted.runs[0]
                .events
                .iter()
                .any(|event| event.event_type == "persistence.pending")
        );
        assert_eq!(state.persisted.messages[0].content, "已完成回复");
        assert_eq!(
            state.pending_ledger_write.as_ref().unwrap().request,
            command
        );
        state.persisted.runs[0].thread_writer_active = false;
        state
            .persisted
            .inbox_read_item_ids
            .push("later-change".into());
        persist_locked(&mut state).unwrap();
        assert!(!persistence_outcome_unknown(&state));
        assert!(
            state.persisted.runs[0]
                .events
                .iter()
                .any(|event| event.event_type == "persistence.confirmed")
        );
        persist_locked(&mut state).unwrap();
        drop(state);
        let stored = fixture.metadata();
        assert_eq!(stored["runs"][0]["status"], "completed");
        assert_eq!(stored["messages"].as_array().unwrap().len(), 1);
        assert_eq!(
            stored["messages"][0]["contentDigest"],
            sha256_hex("已完成回复")
        );
        assert_eq!(
            stored["inboxReadItemIds"],
            serde_json::json!(["later-change"])
        );
        // 重复确认仍返回同一回执，不产生重复回复。
        let first = fixture
            .server
            .request_result("aiLedger/apply", &command)
            .unwrap();
        assert_eq!(
            first,
            fixture
                .server
                .request_result("aiLedger/apply", &command)
                .unwrap()
        );
    }
}

#[test]
fn unavailable_server_keeps_pending_command_and_does_not_restore_old_snapshot() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    let mut state = lock(&fixture.manager.inner);
    let previous = state.persisted.clone();
    state.persisted.inbox_read_item_ids.push("saved".into());
    let pending = state
        .ledger_baseline
        .prepare(&ledger_projection::state(&state.persisted))
        .unwrap();
    let command = pending.request.clone();
    fixture
        .server
        .request_result("aiLedger/apply", &command)
        .unwrap();
    state.pending_ledger_write = Some(pending);
    state.server = None;
    for _ in 0..2 {
        assert!(persist_locked_or_restore(&mut state, previous.clone()).is_err());
        assert_eq!(state.persisted.inbox_read_item_ids, ["saved"]);
        assert_eq!(
            state.pending_ledger_write.as_ref().unwrap().request,
            command
        );
    }
    state.server = Some(fixture.server.clone());
    persist_locked(&mut state).unwrap();
    assert!(!persistence_outcome_unknown(&state));
    drop(state);
    assert_eq!(
        fixture.metadata()["inboxReadItemIds"],
        serde_json::json!(["saved"])
    );
}

#[test]
fn definite_rejection_still_restores_reply_and_records_failure() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    let previous = fixture.running_reply();
    let mut state = lock(&fixture.manager.inner);
    let mut pending = state
        .ledger_baseline
        .prepare(&ledger_projection::state(&state.persisted))
        .unwrap();
    pending.request["records"][0]["expectedDigest"] = serde_json::json!("stale");
    state.pending_ledger_write = Some(pending);
    let error = flush_pending_ledger_write(&mut state).unwrap_err();
    assert!(error.contains("ai_ledger_record_conflict"));
    assert!(!persistence_outcome_unknown(&state));
    fail_reply_completion(&mut state, previous, 0, 2, error);
    assert_eq!(state.persisted.runs[0].status, "failed");
    assert!(state.persisted.messages.is_empty());
    drop(state);
    let stored = fixture.metadata();
    assert_eq!(stored["runs"][0]["status"], "failed");
    assert!(stored["messages"].as_array().unwrap().is_empty());
}

#[test]
fn first_submission_creates_one_durable_conversation_before_background_preparation() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    let agent_id = lock(&fixture.manager.inner).persisted.agents[0]
        .agent_id
        .clone();
    let mut context = tests::fixture_conversation(&agent_id).context;
    context.workspace_id = None;
    context.workspace_root.clear();
    context.workspace_roots.clear();
    context.local_file_id = None;
    context.relative_path = None;
    context.file_revision = None;
    let selection = model_test_fixture::seed_simulated_connection(&fixture.server, "test-model");
    let request: ConversationStartRequest = serde_json::from_value(serde_json::json!({
        "modelSelection":selection,
        "title":"第一条受理", "conversationType":"direct", "mode":"discuss",
        "agentId":agent_id, "context":context, "content":"原文", "clientMessageId":"first-submission",
        "executionTarget":"model", "modelOverride":"test-model", "permissionMode":"request"
    })).unwrap();
    let (conversation, message, accepted) = fixture
        .manager
        .accept_conversation_submission(request.clone())
        .unwrap();
    assert_eq!(message.conversation_id, conversation.conversation_id);
    assert_eq!(conversation.approval_mode.as_deref(), Some("request"));
    assert_eq!(
        fixture.metadata()["conversations"][0]["approvalMode"],
        "request"
    );
    assert_eq!(
        fixture.metadata()["conversations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(lock(&fixture.manager.inner).persisted.runs.is_empty());
    assert_eq!(
        fixture
            .manager
            .accept_conversation_submission(request)
            .err()
            .unwrap(),
        "message_submission_in_progress"
    );
    assert_eq!(
        lock(&fixture.manager.inner).persisted.conversations.len(),
        1
    );
    fixture
        .manager
        .finish_message_submission("first-submission", Some("model_connection_unavailable"));
    drop(accepted);
    assert_eq!(
        AiRuntimeManager::message_submissions_for_ui(&lock(&fixture.manager.inner))[0].status,
        "failed"
    );
}

#[test]
fn unavailable_server_does_not_drop_an_uncertain_business_commit() {
    let mut state = ManagerState::default();
    let pending = state
        .ledger_baseline
        .prepare(&ledger_projection::state(&state.persisted))
        .unwrap();
    let command = pending.request.clone();
    state.pending_turn_finalization =
        Some((pending, serde_json::json!({"ledger":command,"runId":"run"})));
    for _ in 0..2 {
        assert!(flush_pending_turn_finalization(&mut state).is_err());
        assert_eq!(
            state.pending_turn_finalization.as_ref().unwrap().0.request,
            command
        );
    }
}
