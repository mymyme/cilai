use super::*;

fn official_rollout(fixture: &Fixture, journal: &Value) -> PathBuf {
    let profile = fixture.manager.gateway_data_dir().unwrap();
    let service = crate::codex_threads::CodexThreadService::new(
        crate::codex_harness::bundled_codex_harness_executable()
            .unwrap()
            .into(),
        crate::conversation_kernel::ConversationKind::Workbench.harness_home(&profile),
    )
    .with_profile_dir(profile)
    .with_history_server(fixture.server.clone());
    let scoped = service
        .for_runtime_scope(&journal["binding"]["runtimeScope"])
        .unwrap();
    let thread = scoped
        .request(
            "thread/read",
            &json!({"threadId":journal["binding"]["nativeThreadId"],"includeTurns":true}),
        )
        .unwrap();
    let path: PathBuf = thread["thread"]["path"].as_str().unwrap().into();
    scoped.shutdown_checked().unwrap();
    service.shutdown_checked().unwrap();
    assert!(path.canonicalize().unwrap().starts_with(&fixture.profile));
    path
}

#[test]
fn a_new_thread_is_acknowledged_and_a_missing_existing_rollout_never_starts_a_replacement() {
    let _guard = crate::supervisor::app_server_test_guard();
    let fixture = Fixture::new();
    fixture.validate_tools();
    let started = fixture.start();
    completed_run(&fixture.manager, &started.run.run_id);
    fixture.assert_released();
    let params = json!({"conversationId":started.conversation.conversation_id});
    let journal = fixture
        .server
        .request_result("aiThread/get", &params)
        .unwrap();
    assert!(journal["binding"]["nativeThreadId"].is_string());
    assert_eq!(journal["binding"]["status"], "ready");
    assert!(
        journal["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| {
                operation["request"]["kind"] == "create" && operation["state"] == "acknowledged"
            })
    );
    let rollout = official_rollout(&fixture, &journal);
    let retained = fixture.owner.join("preserved-native-rollout.jsonl");
    fs::rename(&rollout, &retained).unwrap();
    let baseline = fixture.count.load(Ordering::Acquire);
    let before = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    let request = serde_json::from_value(json!({
        "conversationId":started.conversation.conversation_id,
        "clientMessageId":"missing-existing-thread-input", "content":"不可转交新线程的输入",
        "executionTarget":"model","permissionMode":"request","planning":false,
        "modelSelection":{"connectionId":fixture.connection.connection_id,
            "modelId":"isolated-official-fixture","reasoningEffort":null},
        "modelOverride":"isolated-official-fixture"
    }))
    .unwrap();
    let result = fixture
        .manager
        .send_message_with_application_context(request, None, None, None);
    let run = result.unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
        let current = snapshot["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|current| current["runId"] == run.run_id)
            .unwrap();
        if current["status"] == "failed" && current["threadWriterActive"] == false {
            break;
        }
        assert_eq!(fixture.count.load(Ordering::Acquire), baseline);
        assert!(Instant::now() < deadline, "{current}");
        std::thread::sleep(Duration::from_millis(20));
    }
    fs::rename(&retained, &rollout).unwrap();
    assert_eq!(fixture.count.load(Ordering::Acquire), baseline);
    let after = serde_json::to_value(fixture.manager.live_snapshot()).unwrap();
    let previous = before["messages"].as_array().unwrap();
    let messages = after["messages"].as_array().unwrap();
    assert_eq!(&messages[..previous.len()], previous);
    assert!(
        messages
            .iter()
            .any(|message| message["content"] == "不可转交新线程的输入")
    );
    let current = fixture
        .server
        .request_result("aiThread/get", &params)
        .unwrap();
    assert_eq!(
        current["binding"]["nativeThreadId"],
        journal["binding"]["nativeThreadId"]
    );
    assert_eq!(
        current["binding"]["runtimeScope"],
        journal["binding"]["runtimeScope"]
    );
    fixture.manager.shutdown_checked().unwrap();
}
