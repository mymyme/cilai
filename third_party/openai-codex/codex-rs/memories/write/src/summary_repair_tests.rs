use super::*;
use pretty_assertions::assert_eq;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn missing_summary_gets_one_feedback_turn_and_only_valid_results_publish()
-> anyhow::Result<()> {
    for write_summary in [true, false] {
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
            "summary-repair",
        )
        .await?;
        tokio::fs::create_dir_all(home.path().join("memory-publication")).await?;
        let root = home.path().join("memories");
        seed_required_memory_artifacts(&root).await?;
        tokio::fs::remove_file(root.join("memory_summary.md")).await?;
        let count = Arc::new(AtomicUsize::new(0));
        let response_count = Arc::clone(&count);
        let summary = root.join("memory_summary.md");
        wiremock::Mock::given(wiremock::matchers::path("/v1/responses"))
            .respond_with(move |_request: &wiremock::Request| {
                let attempt = response_count.fetch_add(1, Ordering::SeqCst);
                if attempt == 1 && write_summary {
                    std::fs::write(&summary, "v1\n\nsummary\n").unwrap();
                }
                wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse(vec![
                        ev_assistant_message(&format!("msg-{attempt}"), "done"),
                        ev_completed(&format!("response-{attempt}")),
                    ]))
            })
            .mount(&server)
            .await;
        let provider = Arc::new(MockMemoryModelProvider::new(
            test.config.model_provider.clone(),
            Some(test.thread_manager.auth_manager()),
        ));
        let (context, config) = memory_startup_context_with_provider(&test, provider).await;
        let permissions = config.permissions.effective_permission_profile();
        let outcome = tokio::time::timeout(
            Duration::from_secs(20),
            phase2::run(context, config, permissions),
        )
        .await?;
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "repair must be bounded to one turn"
        );
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[1].body)?;
        assert!(body.to_string().contains("Host validation failed"));
        assert_eq!(
            tokio::fs::read_to_string(root.join("MEMORY.md")).await?,
            "memory\n"
        );
        if write_summary {
            assert_eq!(outcome, phase2::Outcome::Succeeded);
            let revision =
                tokio::fs::read_to_string(home.path().join("memory-publication/current")).await?;
            assert!(
                home.path()
                    .join("memory-publication/versions")
                    .join(revision)
                    .join("memories/memory_summary.md")
                    .is_file()
            );
        } else {
            assert_eq!(outcome, phase2::Outcome::Failed);
            assert!(!home.path().join("memory-publication/current").exists());
            assert!(
                crate::publication::last_error(home.path())
                    .await?
                    .unwrap()
                    .contains("MEMORY-failed_invalid_artifacts")
            );
        }
        shutdown_test_codex(&test).await?;
    }
    Ok(())
}
