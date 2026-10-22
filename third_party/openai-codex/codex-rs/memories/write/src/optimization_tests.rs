use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn empty_memory_pipeline_does_not_call_the_model() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let test = build_test_codex(&server, Arc::clone(&home)).await?;
    let db = test.codex.state_db().unwrap();
    let config_snapshot = test.codex.config_snapshot().await;
    let mut config = test.config.clone();
    config.features.enable(Feature::MemoryTool)?;
    let permissions = config.permissions.effective_permission_profile();
    start_memories_startup_task(
        Arc::clone(&test.thread_manager),
        test.thread_manager.auth_manager(),
        test.session_configured.thread_id,
        Arc::clone(&test.codex),
        Arc::new(config),
        permissions,
        &config_snapshot.session_source,
    )
    .unwrap()
    .await?
    .map_err(anyhow::Error::msg)?;
    assert_eq!(
        wait_for_phase2_job_to_finish(db.as_ref()).await?,
        Phase2JobClaimOutcome::SkippedCooldown
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| !request.url.path().ends_with("/responses"))
    );
    assert!(!home.path().join("memories/MEMORY.md").exists());
    shutdown_test_codex(&test).await?;
    Ok(())
}

#[tokio::test]
async fn slow_extraction_finishes_and_keeps_its_claim() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let test = build_test_codex(&server, Arc::clone(&home)).await?;
    let db = test.codex.state_db().unwrap();
    let updated = chrono::Utc::now() - chrono::Duration::hours(2);
    let candidate =
        seed_stage1_candidate(db.as_ref(), home.path(), updated, "slow-extraction").await?;
    wiremock::Mock::given(wiremock::matchers::path("/v1/responses"))
        .respond_with(wiremock::ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(sse(vec![ev_assistant_message("msg-slow", r#"{"raw_memory":"durable preference","rollout_summary":"user preference","rollout_slug":"slow"}"#), ev_completed("resp-slow")]))
            .set_delay(Duration::from_millis(500)))
        .mount(&server).await;
    let provider = Arc::new(MockMemoryModelProvider::new(
        test.config.model_provider.clone(),
        Some(test.thread_manager.auth_manager()),
    ));
    let (context, config) = memory_startup_context_with_provider(&test, provider).await;
    let handle = tokio::spawn(phase1::run(Arc::clone(&context), config));
    wait_for_memory_request(&server).await?;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(151)).await;
    tokio::time::resume();
    assert!(!handle.is_finished());
    assert_eq!(
        db.memories()
            .try_claim_stage1_job(candidate, ThreadId::new(), updated.timestamp(), 1, 1)
            .await?,
        codex_state::Stage1JobClaimOutcome::SkippedRunning
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::time::resume();
    let result = tokio::time::timeout(Duration::from_secs(10), handle).await???;
    assert_eq!(result.failed, 0);
    assert_eq!(result.succeeded_with_output, 1);
    let memories = db.memories().get_phase2_input_selection(1, 1).await?;
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].raw_memory, "durable preference");
    shutdown_test_codex(&test).await?;
    Ok(())
}

#[tokio::test]
async fn slow_consolidation_publishes_and_closes_its_agent() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let test = build_test_codex(&server, Arc::clone(&home)).await?;
    let db = test.codex.state_db().unwrap();
    seed_stage1_output(
        db.as_ref(),
        home.path(),
        chrono::Utc::now(),
        "raw memory",
        "rollout summary",
        "slow-consolidation",
    )
    .await?;
    tokio::fs::create_dir_all(home.path().join("memory-publication")).await?;
    let root = home.path().join("memories");
    seed_extension_instructions(&root).await?;
    seed_required_memory_artifacts(&root).await?;
    wiremock::Mock::given(wiremock::matchers::path("/v1/responses"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse(vec![
                    ev_assistant_message("msg-slow", "done"),
                    ev_completed("resp-slow"),
                ]))
                .set_delay(Duration::from_millis(500)),
        )
        .mount(&server)
        .await;
    let provider = Arc::new(MockMemoryModelProvider::new(
        test.config.model_provider.clone(),
        Some(test.thread_manager.auth_manager()),
    ));
    let (context, config) = memory_startup_context_with_provider(&test, provider).await;
    let mut config = (*config).clone();
    config.web_search_mode =
        codex_config::Constrained::allow_only(codex_protocol::config_types::WebSearchMode::Live);
    config.model_context_window = Some(1234);
    let config = Arc::new(config);
    test.codex.shutdown_and_wait().await?;
    let permissions = config.permissions.effective_permission_profile();
    let handle = tokio::spawn(phase2::run(context, config, permissions));
    wait_for_memory_request(&server).await?;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(151)).await;
    tokio::time::resume();
    assert!(!handle.is_finished());
    assert_eq!(
        db.memories()
            .try_claim_global_phase2_job(ThreadId::new(), 1)
            .await?,
        Phase2JobClaimOutcome::SkippedRunning
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::time::resume();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(10), handle).await??,
        phase2::Outcome::Succeeded
    );
    let revision =
        tokio::fs::read_to_string(home.path().join("memory-publication/current")).await?;
    assert!(!revision.is_empty());
    assert!(
        home.path()
            .join("memory-publication/versions")
            .join(revision)
            .join("memories/MEMORY.md")
            .is_file()
    );
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body)?;
    assert!(body["tools"].as_array().unwrap().iter().all(|tool| {
        !matches!(
            tool["type"].as_str(),
            Some("web_search" | "web_search_preview")
        )
    }));
    let agent = ThreadId::from_string(body["client_metadata"]["thread_id"].as_str().unwrap())?;
    assert!(test.thread_manager.remove_thread(&agent).await.is_none());
    Ok(())
}

#[tokio::test]
async fn mixed_extraction_results_publish_successes_and_report_partial_failure()
-> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let test = build_test_codex(&server, Arc::clone(&home)).await?;
    let db = test.codex.state_db().unwrap();
    let updated = chrono::Utc::now() - chrono::Duration::hours(2);
    seed_stage1_candidate(db.as_ref(), home.path(), updated, "valid-candidate").await?;
    let failed =
        seed_stage1_candidate(db.as_ref(), home.path(), updated, "invalid-candidate").await?;
    tokio::fs::create_dir_all(home.path().join("memory-publication")).await?;
    let root = home.path().join("memories");
    seed_extension_instructions(&root).await?;
    seed_required_memory_artifacts(&root).await?;
    wiremock::Mock::given(wiremock::matchers::path("/v1/responses"))
        .respond_with(|request: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let text = if body["text"]["format"]["type"] == "json_schema" {
                if body.to_string().contains("invalid-candidate") { "invalid json" }
                else { r#"{"raw_memory":"valid durable preference","rollout_summary":"valid summary","rollout_slug":"valid"}"# }
            } else { "done" };
            wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream")
                .set_body_string(sse(vec![ev_assistant_message("msg", text), ev_completed("resp")]))
        }).mount(&server).await;
    let snapshot = test.codex.config_snapshot().await;
    let mut config = test.config.clone();
    config.features.enable(Feature::MemoryTool)?;
    let permissions = config.permissions.effective_permission_profile();
    let result = start_memories_startup_task(
        Arc::clone(&test.thread_manager),
        test.thread_manager.auth_manager(),
        test.session_configured.thread_id,
        Arc::clone(&test.codex),
        Arc::new(config),
        permissions,
        &snapshot.session_source,
    )
    .unwrap()
    .await?;
    assert!(result.unwrap_err().contains("成功提炼 1 条，失败 1 条"));
    let status: serde_json::Value = serde_json::from_slice(
        &tokio::fs::read(home.path().join("memory-publication/status.json")).await?,
    )?;
    assert_eq!(status["phase"], "partial");
    let revision =
        tokio::fs::read_to_string(home.path().join("memory-publication/current")).await?;
    let published = home
        .path()
        .join("memory-publication/versions")
        .join(revision);
    let raw = tokio::fs::read_to_string(published.join("memories/raw_memories.md")).await?;
    assert!(raw.contains("valid durable preference"));
    assert_eq!(
        db.memories()
            .try_claim_stage1_job(failed, ThreadId::new(), updated.timestamp(), 1, 1)
            .await?,
        codex_state::Stage1JobClaimOutcome::SkippedRetryBackoff
    );
    shutdown_test_codex(&test).await?;
    Ok(())
}

#[tokio::test]
async fn stalled_extraction_stream_fails_and_releases_its_claim() -> anyhow::Result<()> {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let test = build_test_codex(&server, Arc::clone(&home)).await?;
    let db = test.codex.state_db().unwrap();
    let updated = chrono::Utc::now() - chrono::Duration::hours(2);
    let candidate =
        seed_stage1_candidate(db.as_ref(), home.path(), updated, "stalled-stream").await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let len = socket.read(&mut buffer).await.unwrap();
            if len == 0 {
                return;
            }
            request.extend_from_slice(&buffer[..len]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").await.unwrap();
        let body = sse(vec![ev_response_created("stalled")]);
        socket
            .write_all(format!("{:x}\r\n{body}\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        std::future::pending::<()>().await;
    });
    let mut info = test.config.model_provider.clone();
    info.base_url = Some(format!("http://{address}/v1"));
    info.stream_idle_timeout_ms = Some(100);
    info.stream_max_retries = Some(0);
    info.request_max_retries = Some(0);
    let provider = Arc::new(MockMemoryModelProvider::new(
        info.clone(),
        Some(test.thread_manager.auth_manager()),
    ));
    let (context, config) = memory_startup_context_with_provider(&test, provider).await;
    let mut config = (*config).clone();
    config.model_provider = info;
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        phase1::run(context, Arc::new(config)),
    )
    .await??;
    task.abort();
    assert_eq!(result.failed, 1);
    assert_eq!(result.succeeded_with_output, 0);
    let error = result.first_error.unwrap();
    assert!(error.contains("idle timeout"), "{error}");
    assert_eq!(
        db.memories()
            .try_claim_stage1_job(candidate, ThreadId::new(), updated.timestamp(), 1, 1)
            .await?,
        codex_state::Stage1JobClaimOutcome::SkippedRetryBackoff
    );
    assert!(
        db.memories()
            .get_phase2_input_selection(1, 1)
            .await?
            .is_empty()
    );
    shutdown_test_codex(&test).await?;
    Ok(())
}

async fn wait_for_memory_request(server: &wiremock::MockServer) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|request| request.url.path() == "/v1/responses")
        {
            return Ok(());
        }
        anyhow::ensure!(Instant::now() < deadline, "memory request not started");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn deferred_consolidation_preserves_failure_until_a_successful_retry() -> anyhow::Result<()> {
    for delay in [3_600, 0] {
        let server = start_mock_server().await;
        let home = Arc::new(TempDir::new()?);
        let test = build_test_codex(&server, Arc::clone(&home)).await?;
        let db = test.codex.state_db().unwrap();
        let publication = home.path().join("memory-publication");
        tokio::fs::create_dir_all(&publication).await?;
        crate::publication::status(home.path(), "failed", Some("previous consolidation failed"))
            .await?;
        let claim = db
            .memories()
            .try_claim_global_phase2_job(ThreadId::new(), 3600)
            .await?;
        let Phase2JobClaimOutcome::Claimed {
            ownership_token, ..
        } = claim
        else {
            panic!("expected a claimed consolidation job");
        };
        assert!(
            db.memories()
                .mark_global_phase2_job_failed(&ownership_token, "failed_agent", delay)
                .await?
        );
        let snapshot = test.codex.config_snapshot().await;
        let mut config = test.config.clone();
        config.features.enable(Feature::MemoryTool)?;
        let permissions = config.permissions.effective_permission_profile();
        let result = start_memories_startup_task(
            Arc::clone(&test.thread_manager),
            test.thread_manager.auth_manager(),
            test.session_configured.thread_id,
            Arc::clone(&test.codex),
            Arc::new(config),
            permissions,
            &snapshot.session_source,
        )
        .unwrap()
        .await?;
        let status: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(publication.join("status.json")).await?)?;
        if delay > 0 {
            assert_eq!(result, Err("previous consolidation failed".to_owned()));
            assert_eq!(status["phase"], "failed");
            assert_eq!(status["error"], "previous consolidation failed");
            assert!(!publication.join("current").exists());
            assert_eq!(
                db.memories()
                    .try_claim_global_phase2_job(ThreadId::new(), 3600)
                    .await?,
                Phase2JobClaimOutcome::SkippedRetryUnavailable
            );
        } else {
            assert_eq!(result, Ok(()));
            assert_eq!(status["phase"], "idle");
            assert_eq!(status["error"], serde_json::Value::Null);
            assert_eq!(
                tokio::fs::read_to_string(publication.join("current")).await?,
                ""
            );
        }
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| !request.url.path().ends_with("/responses"))
        );
        shutdown_test_codex(&test).await?;
    }
    Ok(())
}

#[tokio::test]
async fn large_history_uses_workload_budget_and_still_preserves_publication_on_overrun()
-> anyhow::Result<()> {
    for (used_tokens, expected) in [
        (1_300_000, phase2::Outcome::Succeeded),
        (2_500_000, phase2::Outcome::Failed),
    ] {
        let server = start_mock_server().await;
        let home = Arc::new(TempDir::new()?);
        let test = build_test_codex_with_memories_config(
            &server,
            Arc::clone(&home),
            MemoriesConfig {
                max_raw_memories_for_consolidation: 28,
                ..startup_test_memories_config()
            },
        )
        .await?;
        let db = test.codex.state_db().unwrap();
        for index in 0..28 {
            seed_stage1_output(
                db.as_ref(),
                home.path(),
                chrono::Utc::now(),
                &"source evidence ".repeat(256),
                &"rollout evidence ".repeat(256),
                &format!("history-{index}"),
            )
            .await?;
        }
        let root = home.path().join("memories");
        tokio::fs::create_dir_all(home.path().join("memory-publication")).await?;
        seed_extension_instructions(&root).await?;
        seed_required_memory_artifacts(&root).await?;
        crate::publication::publish(&root, 1).await?;
        let current = home.path().join("memory-publication/current");
        let previous = tokio::fs::read_to_string(&current).await?;
        mount_sse_once(
            &server,
            sse(vec![
                ev_response_created("budget-response"),
                ev_assistant_message("budget-message", "done"),
                core_test_support::responses::ev_completed_with_tokens(
                    "budget-response",
                    used_tokens,
                ),
            ]),
        )
        .await;
        let provider = Arc::new(MockMemoryModelProvider::new(
            test.config.model_provider.clone(),
            Some(test.thread_manager.auth_manager()),
        ));
        let (context, config) = memory_startup_context_with_provider(&test, provider).await;
        let permissions = config.permissions.effective_permission_profile();
        test.codex.shutdown_and_wait().await?;
        assert_eq!(phase2::run(context, config, permissions).await, expected);
        let revision = tokio::fs::read_to_string(&current).await?;
        if expected == phase2::Outcome::Succeeded {
            let manifest: serde_json::Value = serde_json::from_slice(
                &tokio::fs::read(
                    home.path()
                        .join("memory-publication/versions")
                        .join(&revision)
                        .join("manifest.json"),
                )
                .await?,
            )?;
            assert_eq!(manifest["sourceCount"], 28);
        } else {
            assert_eq!(revision, previous);
            let status =
                tokio::fs::read_to_string(home.path().join("memory-publication/status.json"))
                    .await?;
            assert!(status.contains("token budget exceeded"));
        }
    }
    Ok(())
}

#[tokio::test]
async fn resource_policy_upgrade_requeues_budget_failure_only_once() -> anyhow::Result<()> {
    for error in [
        "记忆整理失败：consolidation token budget exceeded",
        "upstream timeout",
    ] {
        let home = Arc::new(TempDir::new()?);
        tokio::fs::create_dir(home.path().join("memory-publication")).await?;
        let db = init_state_db(&home).await?;
        let Phase2JobClaimOutcome::Claimed {
            ownership_token, ..
        } = db
            .memories()
            .try_claim_global_phase2_job(ThreadId::new(), 60)
            .await?
        else {
            panic!("claim expected")
        };
        db.memories()
            .mark_global_phase2_job_failed(&ownership_token, error, 3600)
            .await?;
        crate::retry::prepare_resource_policy(home.path(), &db, Some(error)).await?;
        let claim = db
            .memories()
            .try_claim_global_phase2_job(ThreadId::new(), 60)
            .await?;
        if error.contains("token budget") {
            let Phase2JobClaimOutcome::Claimed {
                ownership_token, ..
            } = claim
            else {
                panic!("updated resource policy should retry budget failure")
            };
            db.memories()
                .mark_global_phase2_job_failed(&ownership_token, error, 3600)
                .await?;
        } else {
            assert_eq!(claim, Phase2JobClaimOutcome::SkippedRetryUnavailable);
        }
        crate::retry::prepare_resource_policy(home.path(), &db, Some(error)).await?;
        assert_eq!(
            db.memories()
                .try_claim_global_phase2_job(ThreadId::new(), 60)
                .await?,
            Phase2JobClaimOutcome::SkippedRetryUnavailable
        );
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn recovery_supplies_file_metadata_and_publishes_model_completed_summary()
-> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let test = build_test_codex(&server, Arc::clone(&home)).await?;
    let db = test.codex.state_db().unwrap();
    seed_stage1_output(
        db.as_ref(),
        home.path(),
        chrono::Utc::now(),
        "verified preference",
        "supporting evidence",
        "recovery",
    )
    .await?;
    let root = home.path().join("memories");
    tokio::fs::create_dir_all(home.path().join("memory-publication")).await?;
    seed_extension_instructions(&root).await?;
    tokio::fs::write(root.join("MEMORY.md"), "# Existing supported index\n").await?;
    let worker_root = root.clone();
    let response = mount_sse_once_match(
        &server,
        move |_request: &wiremock::Request| {
            std::fs::write(
                worker_root.join("memory_summary.md"),
                "v1\nVerified user preference",
            )
            .unwrap();
            true
        },
        sse(vec![
            ev_assistant_message("recovery-message", "done"),
            ev_completed("recovery-response"),
        ]),
    )
    .await;
    let provider = Arc::new(MockMemoryModelProvider::new(
        test.config.model_provider.clone(),
        Some(test.thread_manager.auth_manager()),
    ));
    let (context, config) = memory_startup_context_with_provider(&test, provider).await;
    let permissions = config.permissions.effective_permission_profile();
    test.codex.shutdown_and_wait().await?;
    assert_eq!(
        phase2::run(context, config, permissions).await,
        phase2::Outcome::Succeeded
    );
    let request = response.single_request().body_json().to_string();
    assert!(request.contains("Verified file metadata for this run"));
    assert!(request.contains("summaryVersionValid"));
    assert_eq!(
        tokio::fs::read_to_string(root.join("MEMORY.md")).await?,
        "# Existing supported index\n"
    );
    let revision =
        tokio::fs::read_to_string(home.path().join("memory-publication/current")).await?;
    let published = home
        .path()
        .join("memory-publication/versions")
        .join(revision)
        .join("memories");
    assert_eq!(
        tokio::fs::read_to_string(published.join("memory_summary.md")).await?,
        "v1\nVerified user preference"
    );
    Ok(())
}
