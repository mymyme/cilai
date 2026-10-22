use contexthub_protocol::{
    PersonalStorageExecuteParams, PersonalStorageOperation, PersonalStorageOperationState,
};
use contexthub_workbench_runtime::{ai_runtime::AiRuntimeManager, supervisor::AppServerSupervisor};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExecuteRequest {
    operation_id: String,
    preview_digest: String,
    client_operation_id: String,
}

fn operation(value: Value) -> Result<PersonalStorageOperation, String> {
    serde_json::from_value(value).map_err(|_| "personal_storage_response_invalid".into())
}

pub fn handle(
    command: &str,
    args: &Value,
    profile: &Path,
    manager: &AiRuntimeManager,
    server: &AppServerSupervisor,
) -> Result<Value, String> {
    let request = args.get("request").cloned().unwrap_or(Value::Null);
    if command != "personal_storage_execute" {
        let method = match command {
            "personal_storage_inventory" => "personalStorage/inventory",
            "personal_storage_prepare" => "personalStorage/prepare",
            "personal_storage_get" => "personalStorage/get",
            "personal_storage_list" => "personalStorage/list",
            "personal_storage_reconcile" => "personalStorage/reconcile",
            "personal_storage_cancel" => "personalStorage/cancel",
            "personal_storage_resolve_interrupted" => "personalStorage/resolveInterrupted",
            _ => return Err("personal_storage_command_not_allowed".into()),
        };
        return server.request_result(method, &request);
    }
    let request: ExecuteRequest =
        serde_json::from_value(request).map_err(|_| "personal_storage_request_invalid")?;
    let mut params = PersonalStorageExecuteParams {
        operation_id: request.operation_id,
        preview_digest: request.preview_digest,
        client_operation_id: request.client_operation_id,
        quiescence: None,
    };
    let current = operation(server.request_result(
        "personalStorage/get",
        &json!({"operationId":params.operation_id}),
    )?)?;
    if current.preview_digest != params.preview_digest {
        return Err("personal_storage_preview_changed".into());
    }
    if current.state == PersonalStorageOperationState::Prepared && !current.blockers.is_empty() {
        return Ok(json!({"operation":current,"maintenanceError":null}));
    }
    if current.state != PersonalStorageOperationState::Prepared {
        if current.state == PersonalStorageOperationState::Applying {
            return Err("personal_storage_operation_unconfirmed".into());
        }
        let result = operation(server.request_result(
            "personalStorage/execute",
            &serde_json::to_value(params).map_err(|_| "personal_storage_request_invalid")?,
        )?)?;
        return Ok(json!({"operation":result,"maintenanceError":null}));
    }
    let guard = match manager.begin_profile_maintenance(
        profile,
        &params.operation_id,
        &params.preview_digest,
    ) {
        Ok(guard) => guard,
        Err(reason) => {
            let result = operation(server.request_result(
                "personalStorage/execute",
                &serde_json::to_value(params).map_err(|_| "personal_storage_request_invalid")?,
            )?)?;
            return Ok(json!({"operation":result,"maintenanceError":reason}));
        }
    };
    params.quiescence = Some(guard.quiescence().clone());
    let result = server
        .request_result(
            "personalStorage/execute",
            &serde_json::to_value(params).map_err(|_| "personal_storage_request_invalid")?,
        )
        .and_then(operation);
    let released = guard.finish();
    match result {
        Ok(result) => Ok(
            json!({"operation":result,"maintenanceError":released.err().as_ref().map(|_|"personal_maintenance_release_unconfirmed")}),
        ),
        Err(reason) => Err(if released.is_ok() {
            reason
        } else {
            "personal_maintenance_release_unconfirmed".into()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renderer_cannot_provide_quiescence_or_ownership() {
        for key in ["quiescence", "owner", "pid"] {
            let mut value =
                json!({"operationId":"op","previewDigest":"digest","clientOperationId":"execute"});
            value[key] = json!({});
            assert!(serde_json::from_value::<ExecuteRequest>(value).is_err());
        }
    }
}
