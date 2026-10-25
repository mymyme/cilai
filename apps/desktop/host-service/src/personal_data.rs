use contexthub_protocol::{
    PersonalBackupArtifact, PersonalBackupCaptureParams, PersonalDataOperation,
    PersonalDataOperationState, PersonalRestoreApplyPrepareParams,
    PersonalRestoreWorkspacePlanParams,
};
use contexthub_workbench_runtime::{
    ai_runtime::AiRuntimeManager,
    supervisor::{self, AppServerConnectionMode, AppServerSupervisor},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

static RESTORE_STATE: AtomicU8 = AtomicU8::new(0);

pub fn state() -> &'static str {
    match RESTORE_STATE.load(Ordering::SeqCst) {
        0 => "idle",
        1 => "applying",
        _ => "restart_required",
    }
}

pub fn command_allowed(command: &str) -> bool {
    state() == "idle"
        || matches!(
            command,
            "app_server_status"
                | "host_platform"
                | "test_runtime_diagnostics"
                | "personal_data_reconcile"
                | "personal_data_get"
                | "personal_data_list"
        )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CaptureRequest {
    operation_id: String,
    preview_digest: String,
    client_operation_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplyRequest {
    operation_id: String,
    preview_digest: String,
    expected_current_profile_id: String,
    client_operation_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MaintenanceRequest {
    fence_id: String,
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "personal_data_request_invalid".into())
}

fn envelope(
    operation: &PersonalDataOperation,
    maintenance_error: Option<&str>,
    restart_required: bool,
) -> Value {
    json!({"operation":operation,"maintenanceError":maintenance_error,"restartRequired":restart_required})
}

struct DataContext<'a> {
    profile_id: &'a str,
    profile: &'a Path,
    business: &'a Path,
    manager: &'a AiRuntimeManager,
    server: &'a AppServerSupervisor,
}

pub fn handle(
    command: &str,
    args: &Value,
    profile_id: &str,
    profile: &Path,
    business: &Path,
    manager: &AiRuntimeManager,
    server: &AppServerSupervisor,
) -> Result<Value, String> {
    let request = args.get("request").cloned().unwrap_or(Value::Null);
    let context = DataContext {
        profile_id,
        profile,
        business,
        manager,
        server,
    };
    match command {
        "personal_maintenance_status" => serde_json::to_value(
            contexthub_workbench_runtime::ai_runtime::personal_maintenance_status(server)?,
        )
        .map_err(|_| "personal_maintenance_response_invalid".into()),
        "personal_maintenance_recover" => {
            let request: MaintenanceRequest = decode(request)?;
            serde_json::to_value(
                contexthub_workbench_runtime::ai_runtime::recover_personal_maintenance(
                    server,
                    &request.fence_id,
                )?,
            )
            .map_err(|_| "personal_maintenance_response_invalid".into())
        }
        "personal_maintenance_cancel" => {
            let request: MaintenanceRequest = decode(request)?;
            serde_json::to_value(manager.cancel_profile_maintenance(profile, &request.fence_id)?)
                .map_err(|_| "personal_maintenance_response_invalid".into())
        }
        "personal_data_backup_capture" => capture(decode(request)?, &context),
        "personal_data_restore_apply" => apply_restore(decode(request)?, &context),
        "personal_data_backup_export" => export_backup(args, &context),
        "personal_data_restore_workspace_plan" => {
            let request: PersonalRestoreWorkspacePlanParams = decode(request)?;
            server.request_result(
                "personalData/restoreWorkspacePlan",
                &serde_json::to_value(request).map_err(|_| "personal_data_request_invalid")?,
            )
        }
        _ => {
            let method = match command {
                "personal_data_backup_prepare" => "personalData/backupPrepare",
                "personal_data_restore_prepare" => "personalData/restorePrepare",
                "personal_data_get" => "personalData/get",
                "personal_data_list" => "personalData/list",
                "personal_data_cancel" => "personalData/cancel",
                "personal_data_reconcile" => "personalData/reconcile",
                "personal_data_resolve_interrupted" => "personalData/resolveInterrupted",
                _ => return Err("personal_data_command_not_allowed".into()),
            };
            server.request_result(method, &request)
        }
    }
}

fn capture(request: CaptureRequest, context: &DataContext<'_>) -> Result<Value, String> {
    let mut params = PersonalBackupCaptureParams {
        operation_id: request.operation_id,
        preview_digest: request.preview_digest,
        client_operation_id: request.client_operation_id,
        quiescence: None,
    };
    let current: PersonalDataOperation = decode(context.server.request_result(
        "personalData/get",
        &json!({"operationId":params.operation_id}),
    )?)?;
    if current.preview_digest != params.preview_digest {
        return Err("personal_data_preview_changed".into());
    }
    if current.state != PersonalDataOperationState::Prepared {
        if !matches!(
            current.state,
            PersonalDataOperationState::Completed
                | PersonalDataOperationState::Incomplete
                | PersonalDataOperationState::Cancelled
        ) {
            return Err("personal_data_operation_unconfirmed".into());
        }
        let result = decode(context.server.request_result(
            "personalData/backupCapture",
            &serde_json::to_value(params).map_err(|_| "personal_data_request_invalid")?,
        )?)?;
        return Ok(envelope(&result, None, false));
    }
    let guard = match context.manager.begin_profile_maintenance(
        context.profile,
        &params.operation_id,
        &params.preview_digest,
    ) {
        Ok(guard) => guard,
        Err(reason) => {
            let operation = decode(context.server.request_result(
                "personalData/backupCapture",
                &serde_json::to_value(params).map_err(|_| "personal_data_request_invalid")?,
            )?)?;
            return Ok(envelope(&operation, Some(&reason), false));
        }
    };
    params.quiescence = Some(guard.quiescence().clone());
    let response = context.server.request_result(
        "personalData/backupCapture",
        &serde_json::to_value(params).map_err(|_| "personal_data_request_invalid")?,
    );
    let released = guard.finish();
    finish_capture_response(response, released)
}

fn finish_capture_response(
    response: Result<Value, String>,
    released: Result<(), String>,
) -> Result<Value, String> {
    let operation: PersonalDataOperation = match response.and_then(decode) {
        Ok(operation) => operation,
        Err(reason) => {
            return Err(if released.is_ok() {
                reason
            } else {
                "personal_data_maintenance_release_unconfirmed".into()
            });
        }
    };
    Ok(envelope(
        &operation,
        released
            .err()
            .as_ref()
            .map(|_| "personal_data_maintenance_release_unconfirmed"),
        false,
    ))
}

fn apply_restore(request: ApplyRequest, context: &DataContext<'_>) -> Result<Value, String> {
    if request.expected_current_profile_id != context.profile_id {
        return Err("local_profile_changed".into());
    }
    if context.server.status().connection_mode != Some(AppServerConnectionMode::Owned) {
        return Err("personal_data_restore_requires_owned_service".into());
    }
    let mut params = PersonalRestoreApplyPrepareParams {
        operation_id: request.operation_id,
        preview_digest: request.preview_digest,
        expected_current_profile_id: request.expected_current_profile_id,
        client_operation_id: request.client_operation_id,
        quiescence: None,
    };
    let guard = match context.manager.begin_profile_maintenance(
        context.profile,
        &params.operation_id,
        &params.preview_digest,
    ) {
        Ok(guard) => guard,
        Err(reason) => {
            let operation = decode(context.server.request_result(
                "personalData/restoreApplyPrepare",
                &serde_json::to_value(&params).map_err(|_| "personal_data_request_invalid")?,
            )?)?;
            return Ok(envelope(&operation, Some(&reason), false));
        }
    };
    params.quiescence = Some(guard.quiescence().clone());
    let response = context.server.request_result(
        "personalData/restoreApplyPrepare",
        &serde_json::to_value(&params).map_err(|_| "personal_data_request_invalid")?,
    );
    let operation: PersonalDataOperation = match response.and_then(decode) {
        Ok(operation) => operation,
        Err(reason) => {
            return if guard.finish().is_ok() {
                Err(reason)
            } else {
                Err("personal_data_maintenance_release_unconfirmed".into())
            };
        }
    };
    if operation.state != PersonalDataOperationState::ReadyToApply {
        let released = guard.finish();
        return Ok(envelope(
            &operation,
            released
                .err()
                .as_ref()
                .map(|_| "personal_data_maintenance_release_unconfirmed"),
            false,
        ));
    }
    let receipt = guard.consume_for_offline_restore()?;
    RESTORE_STATE.store(1, Ordering::SeqCst);
    if let Err(reason) = context.server.shutdown_exclusive() {
        if reason == "app_server_other_clients_active" {
            return if let Ok(operation) = receipt.abort_before_helper() {
                RESTORE_STATE.store(0, Ordering::SeqCst);
                Ok(envelope(&operation, Some(&reason), false))
            } else {
                RESTORE_STATE.store(2, Ordering::SeqCst);
                Err("personal_data_maintenance_release_unconfirmed".into())
            };
        }
        RESTORE_STATE.store(2, Ordering::SeqCst);
        return Err("personal_data_owned_service_exit_unconfirmed".into());
    }
    let _quiescence = receipt.quiescence();
    let result = run_restore_helper(
        context.business,
        &params.operation_id,
        &params.preview_digest,
    );
    RESTORE_STATE.store(2, Ordering::SeqCst);
    result.map(|operation| envelope(&operation, None, true))
}

fn export_backup(args: &Value, context: &DataContext<'_>) -> Result<Value, String> {
    let operation_id = args
        .get("operationId")
        .and_then(Value::as_str)
        .ok_or("personal_data_request_invalid")?;
    let destination = args
        .get("destinationPath")
        .and_then(Value::as_str)
        .ok_or("personal_data_request_invalid")?;
    let artifact: PersonalBackupArtifact = decode(context.server.request_result(
        "personalData/backupArtifact",
        &json!({"operationId":operation_id}),
    )?)?;
    if artifact.operation_id != operation_id {
        return Err("personal_data_artifact_invalid".into());
    }
    copy_backup_artifact(
        context.profile,
        context.business,
        &artifact,
        Path::new(destination),
    )?;
    Ok(json!({"saved":true,"bytes":artifact.bytes,"digest":artifact.digest}))
}

fn regular_owned_source(profile: &Path, source: &Path) -> Result<(), String> {
    if !source.is_absolute()
        || !source.starts_with(profile)
        || source.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("personal_data_artifact_invalid".into());
    }
    for path in source.ancestors() {
        let metadata =
            std::fs::symlink_metadata(path).map_err(|_| "personal_data_artifact_unavailable")?;
        if metadata.file_type().is_symlink()
            || (path != source && !metadata.is_dir())
            || (path == source && !metadata.is_file())
        {
            return Err("personal_data_artifact_invalid".into());
        }
        if path == profile {
            return Ok(());
        }
    }
    Err("personal_data_artifact_invalid".into())
}

fn copy_backup_artifact(
    profile: &Path,
    business: &Path,
    artifact: &PersonalBackupArtifact,
    destination: &Path,
) -> Result<(), String> {
    let source = Path::new(&artifact.path);
    regular_owned_source(profile, source)?;
    let parent = destination
        .parent()
        .ok_or("personal_data_export_destination_invalid")?;
    if !destination.is_absolute()
        || parent
            .canonicalize()
            .map_err(|_| "personal_data_export_destination_invalid")?
            .starts_with(
                business
                    .canonicalize()
                    .map_err(|_| "personal_data_export_destination_invalid")?,
            )
    {
        return Err("personal_data_export_destination_invalid".into());
    }
    let temporary = parent.join(format!(
        ".cilai-backup-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut output = contexthub_platform_adapter::security::create_new_restricted_file(&temporary)
        .map_err(|_| "personal_data_export_failed")?;
    let result = (|| {
        let mut input =
            contexthub_platform_adapter::filesystem::open_regular_file_no_follow(source, false)
                .map_err(|_| "personal_data_artifact_unavailable")?;
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        loop {
            let count = input
                .read(&mut buffer)
                .map_err(|_| "personal_data_artifact_unavailable")?;
            if count == 0 {
                break;
            }
            size = size
                .checked_add(count as u64)
                .ok_or("personal_data_artifact_invalid")?;
            if size > artifact.bytes {
                return Err("personal_data_artifact_changed");
            }
            hasher.update(&buffer[..count]);
            output
                .write_all(&buffer[..count])
                .map_err(|_| "personal_data_export_failed")?;
        }
        if size != artifact.bytes || format!("sha256:{:x}", hasher.finalize()) != artifact.digest {
            return Err("personal_data_artifact_changed");
        }
        output
            .sync_all()
            .map_err(|_| "personal_data_export_failed")?;
        Ok(())
    })();
    drop(output);
    if let Err(reason) = result {
        let _cleanup = std::fs::remove_file(&temporary);
        return Err(reason.into());
    }
    if std::fs::rename(&temporary, destination).is_err() {
        let _cleanup = std::fs::remove_file(&temporary);
        return Err("personal_data_export_failed".into());
    }
    Ok(())
}

fn run_restore_helper(
    business: &Path,
    operation_id: &str,
    preview_digest: &str,
) -> Result<PersonalDataOperation, String> {
    let executable = supervisor::bundled_app_server_path()
        .map_err(|_| "personal_data_restore_helper_unavailable")?;
    let mut command = std::process::Command::new(executable);
    command
        .args(["personal-data-restore", "--business-root"])
        .arg(business)
        .args([
            "--operation-id",
            operation_id,
            "--preview-digest",
            preview_digest,
        ]);
    command.env_clear();
    contexthub_platform_adapter::process::retain_required_system_environment(&mut command)
        .map_err(|_| "personal_data_restore_helper_environment_failed")?;
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    contexthub_platform_adapter::process::prepare_background_command_for_supervision(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| "personal_data_restore_helper_launch_failed")?;
    contexthub_platform_adapter::process::supervise_spawned_child(&mut child)
        .map_err(|_| "personal_data_restore_helper_ownership_unconfirmed")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("personal_data_restore_result_unconfirmed")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("personal_data_restore_result_unconfirmed")?;
    let reader = std::thread::spawn(move || read_capped(stdout, 8 * 1024 * 1024));
    let diagnostics = std::thread::spawn(move || read_capped(stderr, 0));
    let status = wait_restore_helper(&mut child);
    let output = reader
        .join()
        .map_err(|_| "personal_data_restore_result_unconfirmed")??;
    let _diagnostics = diagnostics
        .join()
        .map_err(|_| "personal_data_restore_result_unconfirmed")?;
    if !status?.success() {
        return Err("personal_data_restore_result_unconfirmed".into());
    }
    let operation: PersonalDataOperation =
        serde_json::from_slice(&output).map_err(|_| "personal_data_restore_result_unconfirmed")?;
    if operation.operation_id != operation_id {
        return Err("personal_data_restore_result_unconfirmed".into());
    }
    Ok(operation)
}

fn read_capped(mut input: impl std::io::Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut saved = Vec::new();
    let mut overflow = false;
    let mut buffer = [0_u8; 4096];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|_| "personal_data_restore_result_unconfirmed")?;
        if count == 0 {
            break;
        }
        if limit == 0 {
            continue;
        }
        let retained = count.min(limit.saturating_sub(saved.len()));
        saved.extend_from_slice(&buffer[..retained]);
        overflow |= retained != count;
    }
    if overflow {
        Err("personal_data_restore_result_unconfirmed".into())
    } else {
        Ok(saved)
    }
}

fn wait_restore_helper(
    child: &mut std::process::Child,
) -> Result<std::process::ExitStatus, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_mins(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Ok(None) | Err(_) => {
                let _kill = child.kill();
                let _wait = child.wait();
                return Err("personal_data_restore_helper_exit_unconfirmed".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            Self(crate::test_support::fixture_root(
                "verification/personal-data-host",
            ))
        }
        fn artifact(
            &self,
            content: &[u8],
        ) -> (
            std::path::PathBuf,
            std::path::PathBuf,
            PersonalBackupArtifact,
        ) {
            let business = self.0.join("business");
            let profile = business.join("profiles/personal");
            std::fs::create_dir_all(&profile).unwrap();
            let source = profile.join("backup.zip");
            std::fs::write(&source, content).unwrap();
            let artifact = PersonalBackupArtifact {
                operation_id: "test".into(),
                path: source.to_string_lossy().into_owned(),
                digest: format!("sha256:{:x}", Sha256::digest(content)),
                bytes: content.len() as u64,
            };
            (business, profile, artifact)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _cleanup = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn export_verifies_digest_and_preserves_existing_destination_on_mismatch() {
        let fixture = Fixture::new();
        let (business, profile, artifact) = fixture.artifact(b"complete snapshot");
        let target = fixture.0.join("export.zip");
        std::fs::write(&target, b"previous backup").unwrap();
        std::fs::write(&artifact.path, b"changed snapshot").unwrap();
        assert_eq!(
            copy_backup_artifact(&profile, &business, &artifact, &target).unwrap_err(),
            "personal_data_artifact_changed"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"previous backup");
        std::fs::write(&artifact.path, b"complete snapshot").unwrap();
        copy_backup_artifact(&profile, &business, &artifact, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"complete snapshot");
    }

    #[test]
    fn export_refuses_business_destination_and_parent_traversal_source() {
        let fixture = Fixture::new();
        let (business, profile, mut artifact) = fixture.artifact(b"snapshot");
        assert_eq!(
            copy_backup_artifact(&profile, &business, &artifact, &business.join("export.zip"))
                .unwrap_err(),
            "personal_data_export_destination_invalid"
        );
        artifact.path = profile
            .join("../personal/backup.zip")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            copy_backup_artifact(
                &profile,
                &business,
                &artifact,
                &fixture.0.join("outside.zip")
            )
            .unwrap_err(),
            "personal_data_artifact_invalid"
        );
    }

    #[test]
    fn renderer_cannot_supply_maintenance_proof_or_owner() {
        for extra in ["quiescence", "owner", "pid"] {
            let mut request =
                json!({"operationId":"op","previewDigest":"digest","clientOperationId":"capture"});
            request[extra] = json!({});
            assert!(decode::<CaptureRequest>(request).is_err());
        }
    }

    #[test]
    fn workspace_plan_decodes_only_the_exact_native_directory_contract() {
        let request = json!({"operationId":"op","previewDigest":"digest","clientOperationId":"choose","destinationParent":"/native-choice"});
        assert!(decode::<PersonalRestoreWorkspacePlanParams>(request.clone()).is_ok());
        for extra in ["targetPath", "quiescence", "owner", "pid"] {
            let mut invalid = request.clone();
            invalid[extra] = json!({});
            assert!(decode::<PersonalRestoreWorkspacePlanParams>(invalid).is_err());
        }
    }

    #[test]
    fn failed_capture_does_not_hide_unconfirmed_maintenance_release() {
        assert_eq!(
            finish_capture_response(Err("request_failed".into()), Err("release_failed".into()))
                .unwrap_err(),
            "personal_data_maintenance_release_unconfirmed"
        );
        assert_eq!(
            finish_capture_response(Err("request_failed".into()), Ok(())).unwrap_err(),
            "request_failed"
        );
    }

    #[test]
    fn restore_output_is_bounded_and_diagnostics_are_discarded() {
        assert_eq!(read_capped(&b"json"[..], 4).unwrap(), b"json");
        assert!(read_capped(&b"oversized"[..], 4).is_err());
        assert!(
            read_capped(&b"private helper diagnostics"[..], 0)
                .unwrap()
                .is_empty()
        );
    }
}
