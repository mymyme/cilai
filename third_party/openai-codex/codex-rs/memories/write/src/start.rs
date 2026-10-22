use crate::ensure_layout;
use crate::extensions::seed_extension_instructions;
use crate::guard;
use crate::memory_root;
use crate::phase1;
use crate::phase2;
use crate::runtime::MemoryStartupContext;
use codex_core::CodexThread;
use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_protocol::ThreadId;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::SessionSource;
use std::sync::Arc;

/// Starts the asynchronous startup memory pipeline for an eligible root session.
///
/// The pipeline is skipped for ephemeral sessions, disabled feature flags, and
/// subagent sessions.
pub fn start_memories_startup_task(
    thread_manager: Arc<ThreadManager>,
    auth_manager: Arc<AuthManager>,
    thread_id: ThreadId,
    thread: Arc<CodexThread>,
    config: Arc<Config>,
    parent_permission_profile: PermissionProfile,
    source: &SessionSource,
) -> Option<tokio::task::JoinHandle<Result<(), String>>> {
    if config.ephemeral
        || !config.features.enabled(Feature::MemoryTool)
        || source.is_non_root_agent()
    {
        return None;
    }

    let context = Arc::new(MemoryStartupContext::new(
        thread_manager,
        Arc::clone(&auth_manager),
        thread_id,
        thread,
        config.as_ref(),
        source.clone(),
    ));

    if context.state_db().is_none() {
        tracing::warn!("state db unavailable for memories startup pipeline; skipping");
        return None;
    }

    Some(tokio::spawn(async move {
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(config.codex_home.join("memory-worker.lock"))
            .map_err(|error| error.to_string())?;
        match lock_file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        }
        let configuration_key =
            crate::retry::configuration_key(&config).map_err(|error| error.to_string())?;
        if let Some(error) = crate::retry::blocked(&config.codex_home, &configuration_key)
            .await
            .map_err(|error| error.to_string())?
        {
            return Err(error);
        }
        let previous_error = crate::publication::last_error(&config.codex_home)
            .await
            .map_err(|error| error.to_string())?;
        if let Some(db) = context.state_db() {
            crate::retry::prepare_extraction_recovery(&config.codex_home, &db, &configuration_key)
                .await
                .map_err(|error| error.to_string())?;
            crate::retry::prepare_resource_policy(
                &config.codex_home,
                &db,
                previous_error.as_deref(),
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        let started = std::time::Instant::now();
        let mut partial = false;
        let result = async {
            let root = memory_root(&config.codex_home);
            ensure_layout(&root).await?;
            if let Some(db) = context.state_db() {
                crate::retry::prepare_runtime_policy(
                    &config.codex_home,
                    &db,
                    previous_error.as_deref(),
                    std::env::var("CILAI_MEMORY_GIT_RUNTIME").ok().as_deref(),
                )
                .await?;
            }
            seed_extension_instructions(&root).await?;
            phase1::prune(context.as_ref(), &config).await;
            if !guard::rate_limits_ok(&auth_manager, &config).await {
                return Ok::<phase2::Outcome, anyhow::Error>(phase2::Outcome::Deferred);
            }
            crate::publication::status(&config.codex_home, "extracting", None).await?;
            let extraction = phase1::run(Arc::clone(&context), Arc::clone(&config)).await?;
            if let Some(error) = extraction
                .first_error
                .as_deref()
                .filter(|error| crate::retry::is_quota_error(error))
            {
                anyhow::bail!("{error}");
            }
            crate::publication::status(&config.codex_home, "consolidating", None).await?;
            let outcome = phase2::run(
                Arc::clone(&context),
                Arc::clone(&config),
                parent_permission_profile,
            )
            .await;
            if outcome == phase2::Outcome::Failed {
                let detail = crate::publication::last_error(&config.codex_home)
                    .await?
                    .unwrap_or_else(|| {
                        "记忆整理失败：未能读取失败详情，请查看本次整理日志".to_owned()
                    });
                anyhow::bail!("{detail}");
            }
            if extraction.failed > 0 {
                partial = extraction.succeeded_with_output > 0;
                let detail = extraction
                    .first_error
                    .as_deref()
                    .unwrap_or("任务提交失败，请查看本次提炼日志");
                anyhow::bail!(
                    "本次成功提炼 {} 条，失败 {} 条：{detail}",
                    extraction.succeeded_with_output,
                    extraction.failed
                );
            }
            Ok(outcome)
        }
        .await;
        let deferred = matches!(result, Ok(phase2::Outcome::Deferred));
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            succeeded = matches!(result, Ok(phase2::Outcome::Succeeded)),
            deferred,
            "memory pipeline finished"
        );
        context.counter(
            crate::metrics::MEMORY_STARTUP,
            1,
            &[(
                "status",
                if partial {
                    "partial"
                } else if deferred {
                    "deferred"
                } else if result.is_ok() {
                    "succeeded"
                } else {
                    "failed"
                },
            )],
        );
        let mut error = result.as_ref().err().map(ToString::to_string);
        if deferred {
            error = previous_error;
        }
        if result.is_err()
            && let Ok(bytes) =
                tokio::fs::read(config.codex_home.join("memory-publication/status.json")).await
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && value["phase"] == "failed"
            && let Some(detail) = value["error"].as_str()
        {
            error = Some(detail.to_owned());
        }
        if error.as_deref().is_some_and(crate::retry::is_quota_error) {
            error = Some(format!(
                "记忆任务暂停：{}",
                serde_json::json!({"error": {
                    "code": "AI-QUOTA-001", "title": "记忆任务已暂停",
                    "message": "模型额度不足。补充额度后重新启用记忆，或选择可用的后台模型。",
                    "retryPolicy": "add_quota"
                }})
            ));
        }
        if !deferred {
            crate::retry::record(&config.codex_home, &configuration_key, error.as_deref())
                .await
                .map_err(|error| error.to_string())?;
        }
        crate::publication::status(
            &config.codex_home,
            if partial {
                "partial"
            } else if error.is_none() {
                "idle"
            } else {
                "failed"
            },
            error.as_deref(),
        )
        .await
        .map_err(|error| error.to_string())?;
        error.map_or(Ok(()), Err)
    }))
}
