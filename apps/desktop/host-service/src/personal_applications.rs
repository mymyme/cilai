use contexthub_protocol::{
    PersonalApplicationDataAction, PersonalApplicationDataExecuteParams,
    PersonalApplicationDataOperation, PersonalApplicationDataPrepareParams,
    PersonalApplicationDataState, PersonalPackageImportPreview,
};
use contexthub_workbench_runtime::{ai_runtime::AiRuntimeManager, supervisor::AppServerSupervisor};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpgradeRequest {
    creation_project_id: String,
    expected_revision: i64,
    package_digest: String,
    confirmed_permissions: Vec<String>,
    client_operation_id: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportRequest {
    import_id: String,
    preview_digest: String,
    confirmed_permissions: Vec<String>,
    client_operation_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfirmRequest {
    operation_id: String,
    preview_digest: String,
}

fn operation(value: Value) -> Result<PersonalApplicationDataOperation, String> {
    serde_json::from_value(value).map_err(|_| "personal_application_data_response_invalid".into())
}
fn encode(value: &impl Serialize) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|_| "personal_application_data_request_invalid".into())
}

fn guarded(
    mut request: Value,
    data: &PersonalApplicationDataOperation,
    method: &str,
    profile: &Path,
    manager: &AiRuntimeManager,
    server: &AppServerSupervisor,
) -> Result<Value, String> {
    if data.read_only_after_restore {
        return Err("personal_application_data_restored_read_only".into());
    }
    let mut execution = PersonalApplicationDataExecuteParams {
        operation_id: data.operation_id.clone(),
        preview_digest: data.preview_digest.clone(),
        quiescence: None,
    };
    let rollback = method == "personalApplicationData/rollback";
    if data.state == PersonalApplicationDataState::Completed {
        if rollback {
            return server.request_result(method, &encode(&execution)?);
        }
        request["dataChange"] = encode(&execution)?;
        return server.request_result(method, &request);
    }
    if data.state != PersonalApplicationDataState::Prepared {
        return Err("personal_application_data_operation_unconfirmed".into());
    }
    let guard =
        manager.begin_profile_maintenance(profile, &data.operation_id, &data.preview_digest)?;
    execution.quiescence = Some(guard.quiescence().clone());
    let result = if rollback {
        server.request_result(method, &encode(&execution)?)
    } else {
        request["dataChange"] = encode(&execution)?;
        server.request_result(method, &request)
    };
    if guard.finish().is_err() {
        return Err("personal_maintenance_release_unconfirmed".into());
    }
    result
}

pub fn handle(
    command: &str,
    args: &Value,
    profile: &Path,
    manager: &AiRuntimeManager,
    server: &AppServerSupervisor,
) -> Result<Value, String> {
    let request = args.get("request").cloned().unwrap_or(Value::Null);
    if command == "personal_capability_upgrade" {
        let request: UpgradeRequest = serde_json::from_value(request)
            .map_err(|_| "personal_application_data_request_invalid")?;
        let prepared = PersonalApplicationDataPrepareParams {
            creation_project_id: request.creation_project_id.clone(),
            expected_revision: request.expected_revision,
            package_digest: request.package_digest.clone(),
            client_operation_id: request.client_operation_id.clone(),
            action: PersonalApplicationDataAction::Upgrade,
            target_operation_id: None,
        };
        let data = operation(
            server.request_result("personalApplicationData/prepare", &encode(&prepared)?)?,
        )?;
        return guarded(
            encode(&request)?,
            &data,
            "personalCapability/upgrade",
            profile,
            manager,
            server,
        );
    }
    if command == "personal_package_import_commit" {
        let request: ImportRequest = serde_json::from_value(request)
            .map_err(|_| "personal_application_data_request_invalid")?;
        let preview: PersonalPackageImportPreview = serde_json::from_value(server.request_result(
            "personalPackage/importGet",
            &json!({"importId":request.import_id}),
        )?)
        .map_err(|_| "personal_package_import_response_invalid")?;
        if preview.preview_digest != request.preview_digest {
            return Err("personal_package_import_preview_changed".into());
        }
        let prepared = PersonalApplicationDataPrepareParams {
            creation_project_id: preview.local_creation_project_id,
            expected_revision: preview.expected_revision.unwrap_or(1),
            package_digest: preview.local_package_digest,
            client_operation_id: request.client_operation_id.clone(),
            action: PersonalApplicationDataAction::ImportUpgrade,
            target_operation_id: None,
        };
        let data = operation(
            server.request_result("personalApplicationData/prepare", &encode(&prepared)?)?,
        )?;
        return guarded(
            encode(&request)?,
            &data,
            "personalPackage/importCommit",
            profile,
            manager,
            server,
        );
    }
    if command == "personal_application_data_rollback" {
        let request: ConfirmRequest = serde_json::from_value(request)
            .map_err(|_| "personal_application_data_request_invalid")?;
        let data = operation(server.request_result(
            "personalApplicationData/get",
            &json!({"operationId":request.operation_id}),
        )?)?;
        if data.preview_digest != request.preview_digest
            || data.action != PersonalApplicationDataAction::Rollback
        {
            return Err("personal_application_data_preview_changed".into());
        }
        return guarded(
            Value::Null,
            &data,
            "personalApplicationData/rollback",
            profile,
            manager,
            server,
        );
    }
    let method = match command {
        "personal_application_data_prepare" => "personalApplicationData/prepare",
        "personal_application_data_get" => "personalApplicationData/get",
        "personal_application_data_list" => "personalApplicationData/list",
        "personal_application_data_cancel" => "personalApplicationData/cancel",
        "personal_application_data_resolve_interrupted" => {
            "personalApplicationData/resolveInterrupted"
        }
        _ => return Err("personal_application_data_command_not_allowed".into()),
    };
    server.request_result(method, &request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renderer_cannot_provide_application_data_proof() {
        let upgrade = json!({"creationProjectId":"app","expectedRevision":1,"packageDigest":"digest","confirmedPermissions":[],"clientOperationId":"once"});
        let import = json!({"importId":"import","previewDigest":"digest","confirmedPermissions":[],"clientOperationId":"once"});
        for key in [
            "dataChange",
            "quiescence",
            "owner",
            "sourceSlotId",
            "targetSlotId",
        ] {
            let mut value = upgrade.clone();
            value[key] = json!({});
            assert!(serde_json::from_value::<UpgradeRequest>(value).is_err());
            let mut value = import.clone();
            value[key] = json!({});
            assert!(serde_json::from_value::<ImportRequest>(value).is_err());
        }
        assert!(
            serde_json::from_value::<ConfirmRequest>(
                json!({"operationId":"op","previewDigest":"digest","quiescence":{}})
            )
            .is_err()
        );
    }
}
