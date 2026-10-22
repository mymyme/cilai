use super::*;
use codex_core::StartIfIdleSubmission;
use codex_core::TurnInputRequest;

/// Feed one concrete missing-summary error back to the same worker. Its cumulative
/// usage and job ownership remain unchanged; this never publishes an invalid draft.
pub(super) async fn run(
    db: Arc<StateRuntime>,
    token: String,
    thread_id: ThreadId,
    thread: &codex_core::CodexThread,
    root: &Path,
    budget: ConsolidationBudget,
) -> AgentStatus {
    let status =
        super::agent::loop_agent(db.clone(), token.clone(), thread_id, thread, budget).await;
    if !matches!(status, AgentStatus::Completed(_))
        || !missing_summary_with_existing_index(root).await
    {
        return status;
    }
    match db
        .memories()
        .heartbeat_global_phase2_job(&token, crate::stage_two::JOB_LEASE_SECONDS)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return AgentStatus::Errored(
                "lost global phase-2 ownership before summary repair".to_owned(),
            );
        }
        Err(error) => {
            return AgentStatus::Errored(format!(
                "summary repair ownership check failed: {error:#}"
            ));
        }
    }
    tracing::info!(%thread_id, "memory summary missing; requesting one bounded repair turn");
    let prompt = vec![UserInput::Text {
        text: "Host validation failed: the required memory_summary.md file is missing, so memory has NOT been published. Continue this same task using the existing MEMORY.md and already checked sources. Write memory_summary.md in the current memory workspace with the exact first line v1 and the required summary sections. A completion message without writing this file cannot succeed. Preserve the supported index and raw sources. This is the only repair turn and uses the remaining cumulative budget; do not restart exploration or create optional artifacts. Return after the file write succeeds.".to_owned(),
        text_elements: vec![],
    }];
    match thread
        .start_turn_if_idle(TurnInputRequest::user_input(prompt))
        .await
    {
        Ok(StartIfIdleSubmission::Started { .. }) => {
            super::agent::loop_agent(db, token, thread_id, thread, budget).await
        }
        Ok(submission) => {
            AgentStatus::Errored(format!("summary repair was not started: {submission:?}"))
        }
        Err(error) => AgentStatus::Errored(format!("summary repair input failed: {error:#}")),
    }
}

async fn missing_summary_with_existing_index(root: &Path) -> bool {
    let index = tokio::fs::symlink_metadata(root.join("MEMORY.md")).await;
    index.is_ok_and(|metadata| {
        metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() > 0
    }) && tokio::fs::symlink_metadata(root.join("memory_summary.md"))
        .await
        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}
