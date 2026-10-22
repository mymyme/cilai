use contexthub_protocol::PersonalMemoryResetConfirmParams;
use contexthub_workbench_runtime::{ai_runtime::AiRuntimeManager, supervisor::AppServerSupervisor};
use serde_json::Value;
use std::path::Path;

pub fn handle(
    command: &str,
    args: &Value,
    profile: &Path,
    manager: &AiRuntimeManager,
    server: &AppServerSupervisor,
) -> Result<Value, String> {
    let request = args.get("request").cloned().unwrap_or(Value::Null);
    if command == "personal_memory_reset_confirm" {
        let request: PersonalMemoryResetConfirmParams =
            serde_json::from_value(request).map_err(|_| "personal_memory_reset_request_invalid")?;
        return serde_json::to_value(manager.personal_memory_reset_confirm(profile, request)?)
            .map_err(|_| "personal_memory_reset_response_invalid".into());
    }
    let method = match command {
        "personal_memory_reset_prepare" => "personalMemoryReset/prepare",
        "personal_memory_reset_get" => "personalMemoryReset/get",
        "personal_memory_reset_list" => "personalMemoryReset/list",
        "personal_memory_reset_cancel" => "personalMemoryReset/cancel",
        "personal_memory_reset_reconcile" => "personalMemoryReset/reconcile",
        "personal_memory_reset_resolve_interrupted" => "personalMemoryReset/resolveInterrupted",
        _ => return Err("personal_memory_reset_command_not_allowed".into()),
    };
    server.request_result(method, &request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renderer_cannot_provide_memory_reset_permit_or_writer_identity() {
        for key in [
            "quiescence",
            "owner",
            "pid",
            "permit",
            "scopeId",
            "nativeHomeRelativePath",
        ] {
            let mut request = json!({"operationId":"op","previewDigest":"digest"});
            request[key] = json!({});
            assert!(serde_json::from_value::<PersonalMemoryResetConfirmParams>(request).is_err());
        }
    }
}
