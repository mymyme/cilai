use contexthub_protocol::{
    ModelExecutionOwner, ModelExecutionProcess, PersonalNamespaceActionAckParams,
    PersonalNamespaceActionParams, PersonalNamespaceBrowserSnapshot, PersonalNamespaceLogSnapshot,
    PersonalNamespaceStorageConfirmParams, PersonalNamespaceStorageKind,
    PersonalNamespaceStorageOperation, PersonalNamespaceStoragePrepareParams,
    PersonalNamespaceStorageStartParams, PersonalNamespaceStorageStartResult,
    PersonalStorageOperationState,
};
use contexthub_workbench_runtime::{
    ai_runtime::{AiRuntimeManager, ProfileMaintenanceGuard},
    supervisor::AppServerSupervisor,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

type Callback<'a> = &'a dyn Fn(&str, &Value) -> Result<Value, String>;

fn guards() -> &'static Mutex<HashMap<String, ProfileMaintenanceGuard>> {
    static GUARDS: OnceLock<Mutex<HashMap<String, ProfileMaintenanceGuard>>> = OnceLock::new();
    GUARDS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PrepareRequest {
    client_operation_id: String,
    item_kinds: Vec<PersonalNamespaceStorageKind>,
    shell_pid: u32,
    shell_instance_id: String,
    namespace_path: String,
    log_segment_id: Option<String>,
    log_files: Vec<PersonalNamespaceLogSnapshot>,
    browser_session: Option<PersonalNamespaceBrowserSnapshot>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BeginRequest {
    operation_id: String,
    action_id: String,
    permit_token: String,
    shell_owner: ModelExecutionOwner,
    execution_pids: Vec<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SegmentRequest {
    segment_id: String,
    shell_pid: u32,
    shell_instance_id: String,
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "personal_namespace_request_invalid".into())
}

fn shell_owner(pid: u32, instance_id: &str) -> Result<ModelExecutionOwner, String> {
    let parent = contexthub_platform_adapter::instance::current_parent_process()
        .map_err(|_| "personal_namespace_shell_owner_unconfirmed")?;
    if parent.pid != pid
        || uuid::Uuid::parse_str(instance_id).is_err()
        || parent.start_fingerprint.is_empty()
    {
        return Err("personal_namespace_shell_owner_unconfirmed".into());
    }
    Ok(ModelExecutionOwner {
        instance_id: instance_id.into(),
        pid,
        start_fingerprint: parent.start_fingerprint,
    })
}

fn verify_owner(owner: &ModelExecutionOwner) -> Result<(), String> {
    if shell_owner(owner.pid, &owner.instance_id)? != *owner {
        return Err("personal_namespace_shell_owner_unconfirmed".into());
    }
    Ok(())
}

pub fn handle(
    command: &str,
    args: &Value,
    profile: &Path,
    manager: &AiRuntimeManager,
    server: &AppServerSupervisor,
    callback: Callback<'_>,
) -> Result<Value, String> {
    let request = args.get("request").cloned().unwrap_or(Value::Null);
    match command {
        "personal_namespace_storage_segment_create_internal"
        | "personal_namespace_storage_segment_sync_internal" => {
            segment(command, &decode(request)?, profile)
        }
        "personal_namespace_storage_action_begin_internal" => {
            let request: BeginRequest = decode(request)?;
            verify_owner(&request.shell_owner)?;
            if request.execution_pids.is_empty()
                || request.execution_pids.len() > 256
                || !request.execution_pids.contains(&request.shell_owner.pid)
            {
                return Err("personal_namespace_execution_processes_unconfirmed".into());
            }
            let mut processes = vec![];
            for pid in request.execution_pids {
                if processes
                    .iter()
                    .any(|p: &ModelExecutionProcess| p.pid == pid)
                {
                    return Err("personal_namespace_execution_processes_unconfirmed".into());
                }
                let instance =
                    contexthub_platform_adapter::instance::process_start_fingerprint(pid)
                        .map_err(|_| "personal_namespace_execution_processes_unconfirmed")?
                        .ok_or("personal_namespace_execution_processes_unconfirmed")?;
                if !contexthub_platform_adapter::instance::process_is_descendant(
                    pid,
                    request.shell_owner.pid,
                )
                .map_err(|_| "personal_namespace_execution_processes_unconfirmed")?
                {
                    return Err("personal_namespace_execution_processes_unconfirmed".into());
                }
                processes.push(ModelExecutionProcess {
                    pid,
                    start_fingerprint: instance,
                });
            }
            let params = PersonalNamespaceActionParams {
                operation_id: request.operation_id,
                action_id: request.action_id,
                permit_token: request.permit_token,
                shell_owner: request.shell_owner,
                execution_processes: processes,
            };
            server.request_result(
                "personalNamespaceStorage/beginAction",
                &serde_json::to_value(params).map_err(|_| "personal_namespace_request_invalid")?,
            )
        }
        "personal_namespace_storage_action_ack_internal" => {
            let params: PersonalNamespaceActionAckParams = decode(request)?;
            verify_owner(&params.shell_owner)?;
            server.request_result(
                "personalNamespaceStorage/ackAction",
                &serde_json::to_value(params).map_err(|_| "personal_namespace_request_invalid")?,
            )
        }
        "personal_namespace_storage_prepare_internal" => {
            let request: PrepareRequest = decode(request)?;
            let params = PersonalNamespaceStoragePrepareParams {
                client_operation_id: request.client_operation_id,
                item_kinds: request.item_kinds,
                shell_owner: shell_owner(request.shell_pid, &request.shell_instance_id)?,
                namespace_path: request.namespace_path,
                log_segment_id: request.log_segment_id,
                log_files: request.log_files,
                browser_session: request.browser_session,
            };
            server.request_result(
                "personalNamespaceStorage/prepare",
                &serde_json::to_value(params).map_err(|_| "personal_namespace_request_invalid")?,
            )
        }
        "personal_namespace_storage_execute_internal" => {
            execute(&decode(request)?, profile, manager, server, callback)
        }
        "personal_namespace_storage_finish" => finish(&decode(request)?, server),
        _ => {
            let method = match command {
                "personal_namespace_storage_get" => "personalNamespaceStorage/get",
                "personal_namespace_storage_list" => "personalNamespaceStorage/list",
                "personal_namespace_storage_cancel" => "personalNamespaceStorage/cancel",
                "personal_namespace_storage_reconcile" => "personalNamespaceStorage/reconcile",
                "personal_namespace_storage_resolve_interrupted" => {
                    "personalNamespaceStorage/resolveInterrupted"
                }
                _ => return Err("personal_namespace_command_not_allowed".into()),
            };
            server.request_result(method, &request)
        }
    }
}

fn execute(
    params: &PersonalNamespaceStorageConfirmParams,
    profile: &Path,
    manager: &AiRuntimeManager,
    server: &AppServerSupervisor,
    callback: Callback<'_>,
) -> Result<Value, String> {
    let current: PersonalNamespaceStorageOperation = decode(server.request_result(
        "personalNamespaceStorage/get",
        &json!({"operationId":params.operation_id}),
    )?)?;
    if current.preview_digest != params.preview_digest {
        return Err("personal_namespace_preview_changed".into());
    }
    verify_owner(&current.shell_owner)?;
    if current.state != PersonalStorageOperationState::Prepared {
        return Err("personal_namespace_operation_unconfirmed".into());
    }
    if !current.blockers.is_empty() {
        return Ok(json!({"operation":current,"maintenanceError":null}));
    }
    let guard = match manager.begin_profile_maintenance(
        profile,
        &params.operation_id,
        &params.preview_digest,
    ) {
        Ok(guard) => guard,
        Err(reason) => return settle_without_callback(params, server, None, &reason),
    };
    let start = PersonalNamespaceStorageStartParams {
        operation_id: params.operation_id.clone(),
        preview_digest: params.preview_digest.clone(),
        quiescence: guard.quiescence().clone(),
    };
    let started: PersonalNamespaceStorageStartResult = match server
        .request_result(
            "personalNamespaceStorage/start",
            &serde_json::to_value(start).map_err(|_| "personal_namespace_request_invalid")?,
        )
        .and_then(decode)
    {
        Ok(started) => started,
        Err(reason) => {
            return settle_without_callback(params, server, Some(guard), &reason);
        }
    };
    if started.operation.operation_id != params.operation_id
        || started.operation.preview_digest != params.preview_digest
        || started.operation.shell_owner != current.shell_owner
    {
        return Err("personal_namespace_operation_unconfirmed".into());
    }
    guards()
        .lock()
        .map_err(|_| "personal_namespace_maintenance_unconfirmed")?
        .insert(params.operation_id.clone(), guard);
    callback(
        "personal_namespace_storage_execute",
        &serde_json::to_value(started).map_err(|_| "personal_namespace_response_invalid")?,
    )
    .map_err(|_| "personal_namespace_action_unconfirmed")?;
    finish(params, server)
}

fn settle_without_callback(
    params: &PersonalNamespaceStorageConfirmParams,
    server: &AppServerSupervisor,
    guard: Option<ProfileMaintenanceGuard>,
    reason: &str,
) -> Result<Value, String> {
    let settlement = (|| {
        let current: PersonalNamespaceStorageOperation = decode(server.request_result(
            "personalNamespaceStorage/get",
            &json!({"operationId":params.operation_id}),
        )?)?;
        if current.operation_id != params.operation_id
            || current.preview_digest != params.preview_digest
            || current.actions.iter().any(|action| {
                action.state != contexthub_protocol::PersonalNamespaceActionState::Planned
            })
        {
            return Err("personal_namespace_action_unconfirmed".to_owned());
        }
        let operation: PersonalNamespaceStorageOperation = decode(server.request_result(
            "personalNamespaceStorage/resolveInterrupted",
            &serde_json::to_value(params).map_err(|_| "personal_namespace_request_invalid")?,
        )?)?;
        if operation.operation_id != params.operation_id
            || operation.preview_digest != params.preview_digest
            || operation.state != PersonalStorageOperationState::Incomplete
            || operation.actions.iter().any(|action| {
                action.state == contexthub_protocol::PersonalNamespaceActionState::Started
            })
        {
            return Err("personal_namespace_action_unconfirmed".into());
        }
        Ok(operation)
    })();
    if let Ok(operation) = settlement {
        let released = guard.map_or(Ok(()), ProfileMaintenanceGuard::finish);
        Ok(
            json!({"operation":operation,"maintenanceError":if released.is_ok(){reason}else{"personal_namespace_maintenance_release_unconfirmed"}}),
        )
    } else {
        if let Some(guard) = guard {
            guards()
                .lock()
                .map_err(|_| "personal_namespace_maintenance_unconfirmed")?
                .insert(params.operation_id.clone(), guard);
        }
        Err("personal_namespace_action_unconfirmed".into())
    }
}

fn finish(
    params: &PersonalNamespaceStorageConfirmParams,
    server: &AppServerSupervisor,
) -> Result<Value, String> {
    let operation: PersonalNamespaceStorageOperation = decode(server.request_result(
        "personalNamespaceStorage/finish",
        &serde_json::to_value(params).map_err(|_| "personal_namespace_request_invalid")?,
    )?)?;
    if operation.operation_id != params.operation_id
        || operation.preview_digest != params.preview_digest
        || operation.state == PersonalStorageOperationState::Applying
        || operation.actions.iter().any(|action| {
            action.state == contexthub_protocol::PersonalNamespaceActionState::Started
        })
    {
        return Err("personal_namespace_action_unconfirmed".into());
    }
    let guard = guards()
        .lock()
        .map_err(|_| "personal_namespace_maintenance_unconfirmed")?
        .remove(&params.operation_id);
    let error = if let Some(guard) = guard {
        guard
            .finish()
            .err()
            .map(|_| "personal_namespace_maintenance_release_unconfirmed")
    } else {
        let fences = contexthub_workbench_runtime::ai_runtime::personal_maintenance_status(server)?;
        fences
            .iter()
            .any(|fence| {
                fence.operation_id == params.operation_id
                    && fence.state != contexthub_protocol::PersonalMaintenanceState::Released
            })
            .then_some("personal_namespace_maintenance_release_unconfirmed")
    };
    Ok(json!({"operation":operation,"maintenanceError":error}))
}

fn segment(command: &str, request: &SegmentRequest, profile: &Path) -> Result<Value, String> {
    shell_owner(request.shell_pid, &request.shell_instance_id)?;
    if request.segment_id.len() != 32
        || !request
            .segment_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("personal_namespace_request_invalid".into());
    }
    let root = profile
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "profiles"))
        .and_then(Path::parent)
        .filter(|p| p.file_name().is_some_and(|n| n == "business"))
        .and_then(Path::parent)
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n == "cn.cilai.personal.workbench")
        })
        .ok_or("personal_namespace_path_invalid")?;
    verify_directory(root)?;
    let segments = root.join("personal-storage-log-segments");
    let target = segments.join(&request.segment_id);
    if command == "personal_namespace_storage_segment_create_internal" {
        contexthub_platform_adapter::security::ensure_restricted_directory(&segments)
            .map_err(|_| "personal_namespace_segment_create_failed")?;
        verify_directory(&segments)?;
        std::fs::create_dir(&target).map_err(|_| "personal_namespace_segment_create_failed")?;
        contexthub_platform_adapter::security::ensure_restricted_directory(&target)
            .map_err(|_| "personal_namespace_segment_create_failed")?;
        let mut file = contexthub_platform_adapter::security::create_new_restricted_file(
            &target.join("segment.json"),
        )
        .map_err(|_| "personal_namespace_segment_create_failed")?;
        let bytes = serde_json::to_vec(&json!({"schemaVersion":1,"segmentId":request.segment_id,
            "profileId":profile.file_name().and_then(|s| s.to_str()).ok_or("personal_namespace_path_invalid")?,
            "createdAt":chrono::Utc::now().timestamp_millis()})).map_err(|_| "personal_namespace_request_invalid")?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| "personal_namespace_segment_create_failed")?;
    }
    verify_directory(&target)?;
    for name in [
        "host-service.log",
        "host-service.log.1",
        "events.log",
        "events.log.1",
    ] {
        let path = target.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                contexthub_platform_adapter::filesystem::open_regular_file_no_follow(&path, true)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| "personal_namespace_segment_sync_failed")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err("personal_namespace_segment_invalid".into()),
        }
    }
    for directory in [&target, &segments, root] {
        contexthub_platform_adapter::filesystem::sync_directory(directory)
            .map_err(|_| "personal_namespace_segment_sync_failed")?;
    }
    Ok(json!({"segmentId":request.segment_id,"synced":true}))
}

fn verify_directory(path: &Path) -> Result<(), String> {
    for ancestor in path.ancestors() {
        let metadata =
            std::fs::symlink_metadata(ancestor).map_err(|_| "personal_namespace_path_invalid")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("personal_namespace_path_invalid".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_ui_requests_cannot_supply_execution_proof() {
        for field in ["shellOwner", "quiescence", "namespacePath", "permits"] {
            let mut request = json!({"operationId":"op","previewDigest":"digest"});
            request[field] = json!({});
            assert!(decode::<PersonalNamespaceStorageConfirmParams>(request).is_err());
        }
    }
}
