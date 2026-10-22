use super::ManagerState;
use contexthub_protocol::{ModelExecutionLease, ModelExecutionSnapshot, ModelPurpose};

pub(super) fn verify_foreground_process(
    state: &ManagerState,
    run: &str,
    pid: u32,
    snapshot: Option<&ModelExecutionSnapshot>,
) -> Result<(), String> {
    if !state.harness_controls.contains_key(run)
        || state.harness_processes.get(run) != Some(&Some(pid))
    {
        return Err("agent_run_owner_required".into());
    }
    let expected = snapshot.ok_or("agent_run_process_unconfirmed")?;
    verify_lease(state, pid, |lease| lease.snapshot == *expected)
        .map_err(|_| "agent_run_process_unconfirmed".into())
}

pub(super) fn verify_application_process(
    state: &ManagerState,
    run: &str,
    pid: u32,
    model: &str,
) -> Result<(), String> {
    if !state.harness_controls.contains_key(run) {
        return Err("application_model_owner_required".into());
    }
    verify_lease(state, pid, |lease| {
        lease.snapshot.purpose == ModelPurpose::PageAi && lease.snapshot.model_id == model
    })
    .map_err(|_| "application_model_process_unconfirmed".into())
}

fn verify_lease(
    state: &ManagerState,
    pid: u32,
    matches: impl Fn(&ModelExecutionLease) -> bool,
) -> Result<(), String> {
    let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
        .map_err(|_| "model_execution_process_unconfirmed")?
        .ok_or("model_execution_process_unconfirmed")?;
    let owner = crate::model_connections::execution_owner()?;
    let server = state.server.as_ref().ok_or("ai_runtime_not_initialized")?;
    let leases: Vec<ModelExecutionLease> = serde_json::from_value(
        server.request_result("modelExecution/list", &serde_json::json!({}))?,
    )
    .map_err(|_| "model_execution_response_invalid")?;
    if !leases.iter().any(|lease| {
        !lease.released
            && lease.owner == owner
            && matches(lease)
            && lease
                .processes
                .iter()
                .any(|process| process.pid == pid && process.start_fingerprint == fingerprint)
    }) {
        return Err("model_execution_process_unconfirmed".into());
    }
    if contexthub_platform_adapter::instance::process_start_fingerprint(pid)
        .map_err(|_| "model_execution_process_unconfirmed")?
        .as_deref()
        != Some(fingerprint.as_str())
    {
        return Err("model_execution_process_unconfirmed".into());
    }
    Ok(())
}
