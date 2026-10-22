use super::*;
use codex_protocol::openai_models::ReasoningEffortPreset;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn memory_requests_omit_effort_for_models_without_reasoning_controls() -> anyhow::Result<()> {
    verify_memory_model_efforts(Vec::new(), None, [None, None]).await
}

#[tokio::test]
async fn memory_requests_use_the_declared_default_when_stage_efforts_are_unsupported()
-> anyhow::Result<()> {
    verify_memory_model_efforts(
        vec![ReasoningEffort::High],
        Some(ReasoningEffort::High),
        [Some("high"), Some("high")],
    )
    .await
}

#[tokio::test]
async fn memory_requests_keep_supported_stage_efforts() -> anyhow::Result<()> {
    verify_memory_model_efforts(
        vec![
            ReasoningEffort::Low,
            ReasoningEffort::Medium,
            ReasoningEffort::High,
        ],
        Some(ReasoningEffort::High),
        [Some("low"), Some("medium")],
    )
    .await
}

async fn verify_memory_model_efforts(
    supported: Vec<ReasoningEffort>,
    default: Option<ReasoningEffort>,
    expected: [Option<&str>; 2],
) -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let model = "gpt-5.2";
    let test = test_codex()
        .with_home(Arc::clone(&home))
        .with_model_info_override(model, move |info| {
            info.supported_reasoning_levels = supported
                .into_iter()
                .map(|effort| ReasoningEffortPreset {
                    effort,
                    description: "test capability".to_owned(),
                })
                .collect();
            info.default_reasoning_level = default;
            info.supports_reasoning_summary_parameter = false;
        })
        .with_config(move |config| {
            config.features.enable(Feature::Sqlite).unwrap();
            config.memories = MemoriesConfig {
                extract_model: Some(model.to_owned()),
                consolidation_model: Some(model.to_owned()),
                ..startup_test_memories_config()
            };
        })
        .build(&server)
        .await?;
    let db = test.codex.state_db().unwrap();
    seed_stage1_candidate(
        db.as_ref(),
        home.path(),
        chrono::Utc::now() - chrono::Duration::hours(2),
        "model-capabilities",
    )
    .await?;
    tokio::fs::create_dir_all(home.path().join("memory-publication")).await?;
    let root = home.path().join("memories");
    seed_extension_instructions(&root).await?;
    seed_required_memory_artifacts(&root).await?;
    let expected = expected.map(|effort| effort.map(str::to_owned));
    let gateway_expected = expected.clone();
    wiremock::Mock::given(wiremock::matchers::path("/v1/responses"))
        .respond_with(move |request: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let extraction = body["text"]["format"]["type"] == "json_schema";
            let stage = if extraction { 0 } else { 1 };
            if body["reasoning"]["effort"].as_str() != gateway_expected[stage].as_deref() {
                return wiremock::ResponseTemplate::new(400).set_body_json(serde_json::json!({
                    "error": {"code":"AI-REQ-001", "message":"reasoning_effort_unsupported"}
                }));
            }
            let output = if extraction {
                r#"{"raw_memory":"durable preference","rollout_summary":"user preference","rollout_slug":"capabilities"}"#
            } else { "done" };
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse(vec![ev_assistant_message("msg", output), ev_completed("resp")]))
        }).mount(&server).await;
    let provider = Arc::new(MockMemoryModelProvider::new(
        test.config.model_provider.clone(),
        Some(test.thread_manager.auth_manager()),
    ));
    let (context, config) = memory_startup_context_with_provider(&test, provider).await;
    let result = phase1::run(Arc::clone(&context), Arc::clone(&config)).await?;
    assert_eq!(result.failed, 0, "{:?}", result.first_error);
    assert_eq!(result.succeeded_with_output, 1);
    let permissions = config.permissions.effective_permission_profile();
    assert_eq!(
        phase2::run(context, config, permissions).await,
        phase2::Outcome::Succeeded
    );
    let requests = server.received_requests().await.unwrap();
    let observed: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path() == "/v1/responses")
        .map(|request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            (
                body["model"].as_str().unwrap().to_owned(),
                body["reasoning"]["effort"].as_str().map(str::to_owned),
            )
        })
        .collect();
    assert_eq!(
        observed,
        vec![
            (model.to_owned(), expected[0].clone()),
            (model.to_owned(), expected[1].clone())
        ]
    );
    let revision =
        tokio::fs::read_to_string(home.path().join("memory-publication/current")).await?;
    assert!(
        home.path()
            .join("memory-publication/versions")
            .join(revision)
            .join("memories/MEMORY.md")
            .is_file()
    );
    shutdown_test_codex(&test).await?;
    Ok(())
}
