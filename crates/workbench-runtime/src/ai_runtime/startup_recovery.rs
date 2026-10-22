use super::{AppServerSupervisor, PersistedAiState};
use contexthub_protocol::ModelExecutionLease;
use std::collections::HashSet;

pub(super) fn retained_model_runs(
    server: &AppServerSupervisor,
    persisted: &PersistedAiState,
) -> Result<HashSet<String>, String> {
    let leases: Vec<ModelExecutionLease> = serde_json::from_value(
        server.request_result("modelExecution/list", &serde_json::json!({}))?,
    )
    .map_err(|_| "model_execution_response_invalid")?;
    Ok(persisted
        .runs
        .iter()
        .filter(|run| {
            run.model_execution.as_ref().is_some_and(|snapshot| {
                leases
                    .iter()
                    .any(|lease| !lease.released && lease.snapshot == *snapshot)
            })
        })
        .map(|run| run.run_id.clone())
        .collect())
}
