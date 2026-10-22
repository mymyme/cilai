use codex_core::config::Config;
use serde_json::json;
use std::path::Path;
use tokio::fs;

pub(crate) fn configuration_key(config: &Config) -> anyhow::Result<String> {
    let value = json!({
        "worker": concat!(env!("CARGO_PKG_VERSION"), ":local-memory-tools-v1"),
        "extractModel": config.memories.extract_model,
        "consolidationModel": config.memories.consolidation_model,
        "catalog": config.model_catalog,
    });
    Ok(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &serde_json::to_vec(&value)?).to_string())
}

pub(crate) async fn blocked(home: &Path, key: &str) -> anyhow::Result<Option<String>> {
    for name in ["quota-error.json", "configuration-error.json"] {
        let bytes = match fs::read(home.join("memory-publication").join(name)).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if value["configurationKey"] == key {
            return Ok(value["error"].as_str().map(str::to_owned));
        }
    }
    Ok(None)
}

pub(crate) fn is_quota_error(error: &str) -> bool {
    error.contains(&codex_protocol::error::CodexErr::QuotaExceeded.to_string())
        || error
            .find('{')
            .and_then(|start| serde_json::from_str::<serde_json::Value>(&error[start..]).ok())
            .is_some_and(|value| {
                value["error"]["code"] == "AI-QUOTA-001"
                    || value["error"]["retryPolicy"] == "add_quota"
            })
}

pub(crate) async fn record(home: &Path, key: &str, error: Option<&str>) -> anyhow::Result<()> {
    if let Some(error) = error {
        let diagnostic = error
            .find('{')
            .and_then(|start| serde_json::from_str::<serde_json::Value>(&error[start..]).ok());
        let quota_error = is_quota_error(error);
        let configuration_error = diagnostic
            .as_ref()
            .is_some_and(|value| value["error"]["retryPolicy"] == "fix_model_configuration");
        if quota_error || configuration_error {
            fs::create_dir_all(home.join("memory-publication")).await?;
            crate::publication::atomic_write(
                &home.join("memory-publication"),
                if quota_error {
                    "quota-error.json"
                } else {
                    "configuration-error.json"
                },
                &serde_json::to_vec(&json!({"configurationKey":key,"error":error}))?,
            )
            .await?;
        }
    } else {
        for name in ["quota-error.json", "configuration-error.json"] {
            match fs::remove_file(home.join("memory-publication").join(name)).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

pub(crate) async fn prepare_resource_policy(
    home: &Path,
    db: &codex_state::StateRuntime,
    previous_error: Option<&str>,
) -> anyhow::Result<()> {
    let directory = home.join("memory-publication");
    if !directory.is_dir() {
        return Ok(());
    }
    let path = directory.join("resource-policy.json");
    let previous = match fs::read(&path).await {
        Ok(bytes) => Some(serde_json::from_slice::<serde_json::Value>(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if previous
        .as_ref()
        .is_some_and(|value| value["version"] == crate::budget::RESOURCE_POLICY)
    {
        return Ok(());
    }
    if previous_error.is_some_and(|error| error.contains("consolidation token budget exceeded")) {
        db.memories()
            .enqueue_global_consolidation(chrono::Utc::now().timestamp())
            .await?;
    }
    crate::publication::atomic_write(
        &directory,
        "resource-policy.json",
        &serde_json::to_vec(&json!({"version": crate::budget::RESOURCE_POLICY}))?,
    )
    .await
}

pub(crate) async fn prepare_extraction_recovery(
    home: &Path,
    db: &codex_state::StateRuntime,
    configuration_key: &str,
) -> anyhow::Result<()> {
    let directory = home.join("memory-publication");
    if !directory.is_dir() {
        return Ok(());
    }
    let expected =
        json!({"policy":"stage1-configuration-recovery-v1","configurationKey":configuration_key});
    match fs::read(directory.join("extraction-recovery.json")).await {
        Ok(bytes) if serde_json::from_slice::<serde_json::Value>(&bytes)? == expected => {
            return Ok(());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let count = db.memories().rearm_exhausted_stage1_jobs().await?;
    crate::publication::atomic_write(
        &directory,
        "extraction-recovery.json",
        &serde_json::to_vec(&expected)?,
    )
    .await?;
    tracing::info!(rearmed_jobs = count, "memory extraction recovery prepared");
    Ok(())
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;

/// A validated bundled runtime upgrade may recover one exhausted baseline or
/// host-access job after the Windows runtime-directory permissions are repaired,
/// or an incomplete-summary job after bounded artifact feedback is available.
/// The marker is written after enqueueing, so an interrupted recovery can be retried safely.
pub(crate) async fn prepare_runtime_policy(
    home: &Path,
    db: &codex_state::StateRuntime,
    previous_error: Option<&str>,
    runtime: Option<&str>,
) -> anyhow::Result<()> {
    let Some(runtime) = runtime.filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let directory = home.join("memory-publication");
    fs::create_dir_all(&directory).await?;
    let policy = format!("git-baseline-recovery-v3:{runtime}");
    let path = directory.join("runtime-policy.json");
    let previous = match fs::read(&path).await {
        Ok(bytes) => Some(serde_json::from_slice::<serde_json::Value>(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if previous
        .as_ref()
        .is_some_and(|value| value["version"] == policy)
    {
        return Ok(());
    }
    if previous_error.is_some_and(|error| {
        [
            "执行超时、超出预算或产物校验失败",
            "write git baseline index from HEAD",
            "MEMORY-failed_prepare_workspace",
            "MEMORY-failed_empty_publication",
            "MEMORY-failed_workspace_commit",
        ]
        .iter()
        .any(|reason| error.contains(reason))
            || (error.contains("MEMORY-failed_workspace_status") && error.contains("os error 5"))
            || (error.contains("MEMORY-failed_invalid_artifacts")
                && error.contains("read memory summary artifact")
                && error.contains("os error 2"))
    }) {
        db.memories()
            .enqueue_global_consolidation(chrono::Utc::now().timestamp())
            .await?;
    }
    crate::publication::atomic_write(
        &directory,
        "runtime-policy.json",
        &serde_json::to_vec(&json!({"version": policy}))?,
    )
    .await
}
