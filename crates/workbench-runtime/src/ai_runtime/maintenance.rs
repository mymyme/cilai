use super::{
    AiRuntimeManager, AppServerSupervisor, ConversationKind, Instant, ManagerState, Ordering, Path,
    PathBuf, fs, lock, persistence_outcome_unknown, unix_epoch_millis,
};
use contexthub_protocol::{
    PersonalBackupNativeHome, PersonalDataQuiescence, PersonalMaintenanceFence,
    PersonalMaintenanceState,
};
use sha2::{Digest as _, Sha256};

/// 持有期间资料只允许维护操作；丢弃不会把未知状态恢复为可写。
pub struct ProfileMaintenanceGuard {
    manager: AiRuntimeManager,
    server: AppServerSupervisor,
    profile_root: PathBuf,
    fence: PersonalMaintenanceFence,
    proof: PersonalDataQuiescence,
    preview_digest: String,
}

/// 将同一已排空资料的维护所有权交给离线恢复程序。
pub struct ProfileMaintenanceRestoreReceipt {
    guard: ProfileMaintenanceGuard,
}

/// # Errors
/// 只读取得当前资料的维护状态；不会释放占用或启动官方进程。
pub fn personal_maintenance_status(
    server: &AppServerSupervisor,
) -> Result<Vec<PersonalMaintenanceFence>, String> {
    serde_json::from_value(
        server.request_result("personalMaintenance/list", &serde_json::json!({}))?,
    )
    .map_err(|_| "personal_maintenance_response_invalid".into())
}

/// # Errors
/// 仅在用户明确核对后执行；旧拥有者未退出、资料状态未知时保留原栅栏和写者。
pub fn recover_personal_maintenance(
    server: &AppServerSupervisor,
    fence_id: &str,
) -> Result<PersonalMaintenanceFence, String> {
    if !fence_id.starts_with("maintenance_")
        || fence_id.len() != 44
        || !fence_id[12..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("personal_maintenance_request_invalid".into());
    }
    let owner = crate::model_connections::execution_owner()?;
    let result: PersonalMaintenanceFence = serde_json::from_value(server.request_result(
        "personalMaintenance/recover",
        &serde_json::json!({"fenceId":fence_id,"owner":owner}),
    )?)
    .map_err(|_| "personal_maintenance_response_invalid")?;
    if result.fence_id != fence_id || result.state != PersonalMaintenanceState::Released {
        return Err("personal_maintenance_release_unconfirmed".into());
    }
    Ok(result)
}

pub(super) struct InitializationWriter {
    server: AppServerSupervisor,
    lease: contexthub_protocol::PersonalWriterLease,
    released: bool,
}
impl InitializationWriter {
    pub(super) fn prepare(server: &AppServerSupervisor, root: &Path) -> Result<Self, String> {
        let owner = crate::model_connections::execution_owner()?;
        let result=server.request_result("profileWriter/prepare",&serde_json::json!({
            "clientOperationId":format!("runtime-init-{}",uuid::Uuid::new_v4().simple()),"kind":"local_operation",
            "resourceId":crate::persistent_threads::digest(&serde_json::json!(root)),"resourceRevision":env!("CARGO_PKG_VERSION"),
            "owner":owner,"launchProtocol":1,"processMode":"remote_request"
        }))?;
        Ok(Self {
            server: server.clone(),
            lease: serde_json::from_value(result)
                .map_err(|_| "personal_writer_response_invalid")?,
            released: false,
        })
    }
    pub(super) fn finish(&mut self) -> Result<(), String> {
        self.server.request_result("profileWriter/release",&serde_json::json!({"leaseId":self.lease.lease_id,"owner":self.lease.owner,"localTransportFinished":true,"remoteOutcomeUnconfirmed":false}))?;
        self.released = true;
        Ok(())
    }
}
impl Drop for InitializationWriter {
    fn drop(&mut self) {
        if !self.released {
            eprintln!("runtime_initialization_writer_release_unconfirmed");
        }
    }
}

impl ProfileMaintenanceRestoreReceipt {
    #[must_use]
    pub fn quiescence(&self) -> &PersonalDataQuiescence {
        self.guard.quiescence()
    }

    /// # Errors
    /// 仅已确认独占停止被其他客户端拒绝、服务仍归本宿主且离线助手尚未启动时可撤销移交。
    pub fn abort_before_helper(self) -> Result<contexthub_protocol::PersonalDataOperation, String> {
        let status = self.guard.server.status();
        if status.phase != crate::supervisor::AppServerPhase::Running
            || status.connection_mode != Some(crate::supervisor::AppServerConnectionMode::Owned)
            || status.server_instance_id.as_deref()
                != Some(self.guard.fence.instance_fence.as_str())
        {
            return Err("personal_data_restore_handoff_unconfirmed".into());
        }
        self.guard.validate_local_state()?;
        let operation: contexthub_protocol::PersonalDataOperation = serde_json::from_value(
            self.guard.server.request_result(
                "personalData/restoreAbortBeforeHelper",
                &serde_json::json!({"operationId":self.guard.fence.operation_id,"previewDigest":self.guard.preview_digest,"quiescence":self.guard.proof}),
            )?,
        ).map_err(|_| "personal_data_operation_unconfirmed")?;
        if operation.operation_id != self.guard.fence.operation_id
            || operation.preview_digest != self.guard.preview_digest
            || operation.profile_id != self.guard.fence.profile_id
            || operation.kind != contexthub_protocol::PersonalDataOperationKind::Restore
            || operation.state != contexthub_protocol::PersonalDataOperationState::Incomplete
        {
            return Err("personal_data_restore_handoff_unconfirmed".into());
        }
        self.guard.finish()?;
        Ok(operation)
    }
}

impl ProfileMaintenanceGuard {
    #[must_use]
    pub fn quiescence(&self) -> &PersonalDataQuiescence {
        &self.proof
    }

    /// # Errors
    /// 排空结果、资料归属或后端释放回执未知时保留维护栅栏。
    pub fn finish(self) -> Result<(), String> {
        self.validate_local_state()?;
        {
            let mut state = lock(&self.manager.inner);
            let now = unix_epoch_millis()?;
            if super::recover_missed_automations(
                &mut state.persisted.automation_rules,
                now,
                "资料维护期间错过本次任务，未补跑。",
            ) {
                persist_quiescent_ledger(&mut state)?;
            }
        }
        let value = self.server.request_result(
            "personalMaintenance/release",
            &serde_json::json!({
                "fenceId":self.fence.fence_id,"owner":self.fence.owner
            }),
        )?;
        let released: PersonalMaintenanceFence =
            serde_json::from_value(value).map_err(|_| "personal_maintenance_response_invalid")?;
        if released.fence_id != self.fence.fence_id
            || released.state != PersonalMaintenanceState::Released
        {
            return Err("personal_maintenance_release_unconfirmed".into());
        }
        lock(&self.manager.inner).maintenance_fence = None;
        self.manager.start_automation_scheduler()?;
        Ok(())
    }

    /// # Errors
    /// 资料发生变化或本机仍有写者时拒绝移交；成功后旧 Manager 持续封闭。
    pub fn consume_for_offline_restore(self) -> Result<ProfileMaintenanceRestoreReceipt, String> {
        self.validate_local_state()?;
        Ok(ProfileMaintenanceRestoreReceipt { guard: self })
    }

    fn validate_local_state(&self) -> Result<(), String> {
        let state = lock(&self.manager.inner);
        if state.data_dir.as_ref() != Some(&self.profile_root.join("ai-runtime"))
            || state.maintenance_fence.as_ref() != Some(&self.fence)
        {
            return Err("personal_maintenance_owner_changed".into());
        }
        ensure_drained(&state)
    }
}

impl AiRuntimeManager {
    /// # Errors
    /// 仅结束当前宿主自己的维护；捕获、恢复或回执未确认时保留栅栏，不改变写者结果。
    pub fn cancel_profile_maintenance(
        &self,
        expected_profile_root: &Path,
        fence_id: &str,
    ) -> Result<PersonalMaintenanceFence, String> {
        let expected = expected_profile_root
            .canonicalize()
            .map_err(|_| "personal_maintenance_profile_invalid")?;
        let (server, fence) = {
            let state = lock(&self.inner);
            let fence = state
                .maintenance_fence
                .as_ref()
                .ok_or("personal_maintenance_not_found")?;
            if state.data_dir.as_ref() != Some(&expected.join("ai-runtime"))
                || fence.fence_id != fence_id
                || fence.owner != crate::model_connections::execution_owner()?
            {
                return Err("personal_maintenance_owner_changed".into());
            }
            (
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
                fence.clone(),
            )
        };
        let operation: contexthub_protocol::PersonalMaintenanceOperationStatus =
            serde_json::from_value(server.request_result(
                "personalMaintenance/operationStatus",
                &serde_json::json!({"operationId":fence.operation_id}),
            )?)
            .map_err(|_| "personal_maintenance_response_invalid")?;
        if operation.profile_id != fence.profile_id
            || operation.operation_id != fence.operation_id
            || !matches!(
                operation.kind.as_str(),
                "backup"
                    | "restore"
                    | "storage_cleanup"
                    | "memory_reset"
                    | "personal_capability_data_change"
            )
            || !matches!(
                operation.state.as_str(),
                "prepared" | "incomplete" | "ready_to_apply" | "completed" | "cancelled"
            )
        {
            return Err("personal_data_operation_unconfirmed".into());
        }
        {
            let mut state = lock(&self.inner);
            if super::recover_missed_automations(
                &mut state.persisted.automation_rules,
                unix_epoch_millis()?,
                "资料维护期间错过本次任务，未补跑。",
            ) {
                persist_quiescent_ledger(&mut state)?;
            }
        }
        let released: PersonalMaintenanceFence = serde_json::from_value(server.request_result(
            "personalMaintenance/release",
            &serde_json::json!({"fenceId":fence.fence_id,"owner":fence.owner}),
        )?)
        .map_err(|_| "personal_maintenance_response_invalid")?;
        if released.fence_id != fence.fence_id
            || released.state != PersonalMaintenanceState::Released
        {
            return Err("personal_maintenance_release_unconfirmed".into());
        }
        let mut state = lock(&self.inner);
        if state.maintenance_fence.as_ref() != Some(&fence) {
            return Err("personal_maintenance_owner_changed".into());
        }
        state.maintenance_fence = None;
        drop(state);
        self.start_automation_scheduler()?;
        Ok(released)
    }

    /// 先持久关闭跨宿主写者准入，再排空本机拥有的运行、授权、附件与账本。
    /// # Errors
    /// 任一进程、回执或资料归属不能证明时返回未完成，并保留维护栅栏。
    pub fn begin_profile_maintenance(
        &self,
        expected_profile_root: &Path,
        operation_id: &str,
        preview_digest: &str,
    ) -> Result<ProfileMaintenanceGuard, String> {
        let root = expected_profile_root
            .canonicalize()
            .map_err(|_| "personal_maintenance_profile_invalid")?;
        let (server, owner) = {
            let state = lock(&self.inner);
            if state.data_dir.as_ref() != Some(&root.join("ai-runtime")) {
                return Err("personal_maintenance_profile_changed".into());
            }
            if state
                .maintenance_fence
                .as_ref()
                .is_some_and(|f| f.operation_id != operation_id)
            {
                return Err("personal_data_maintenance_active".into());
            }
            (
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
                crate::model_connections::execution_owner()?,
            )
        };
        let fence: PersonalMaintenanceFence = serde_json::from_value(server.request_result(
            "personalMaintenance/acquire",
            &serde_json::json!({
                "operationId":operation_id,"previewDigest":preview_digest,"owner":owner
            }),
        )?)
        .map_err(|_| "personal_maintenance_response_invalid")?;
        {
            let mut state = lock(&self.inner);
            if state.data_dir.as_ref() != Some(&root.join("ai-runtime"))
                || fence.owner != owner
                || fence.operation_id != operation_id
                || fence.state != PersonalMaintenanceState::Draining
            {
                return Err("personal_maintenance_owner_changed".into());
            }
            state.maintenance_fence = Some(fence.clone());
        }
        self.automation_generation.fetch_add(1, Ordering::AcqRel);
        self.drain_profile_for_maintenance()?;
        let status: contexthub_protocol::PersonalMaintenanceDrainStatus =
            serde_json::from_value(server.request_result(
                "personalMaintenance/drainStatus",
                &serde_json::json!({"fenceId":fence.fence_id,"owner":owner}),
            )?)
            .map_err(|_| "personal_maintenance_response_invalid")?;
        if status.fence != fence
            || !status.active_model_leases.is_empty()
            || !status.active_writers.is_empty()
        {
            return Err("personal_data_writer_active".into());
        }
        let digest = format!(
            "sha256:{:x}",
            Sha256::digest(
                serde_json::to_vec(&status.active_writers)
                    .map_err(|_| "personal_writer_response_invalid")?
            )
        );
        if status.writer_registry_digest != digest {
            return Err("personal_writer_registry_changed".into());
        }
        let proof = PersonalDataQuiescence {
            protocol_version: contexthub_protocol::PERSONAL_DATA_MAINTENANCE_PROTOCOL,
            profile_id: fence.profile_id.clone(),
            owner,
            token: fence.fence_id.clone(),
            writer_registry_digest: digest,
            covered_writers: vec![
                "foreground".into(),
                "automation".into(),
                "review".into(),
                "memory".into(),
                "page_ai".into(),
                "authentication".into(),
                "personal_package_mcp".into(),
                "user_mcp".into(),
                "attachments".into(),
                "local_operations".into(),
                "ledger".into(),
                "native_history".into(),
                "runtime_initialization".into(),
            ],
            native_homes: native_homes(&root)?,
        };
        Ok(ProfileMaintenanceGuard {
            manager: self.clone(),
            server,
            profile_root: root,
            fence,
            proof,
            preview_digest: preview_digest.to_owned(),
        })
    }

    fn drain_profile_for_maintenance(&self) -> Result<(), String> {
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        self.retire_idle_harnesses()?;
        self.personal_models.shutdown()?;
        self.interrupt_owned_runs_for_maintenance()?;
        loop {
            let state = lock(&self.inner);
            if let Some(error) = state.harness_retirement_error.as_ref() {
                return Err(error.clone());
            }
            let settled = state.harness_processes.is_empty()
                && state.harness_controls.is_empty()
                && state.local_operations == 0
                && !state.memory_resetting
                && !state.pending_attachments.values().any(|a| !a.complete)
                && !state.application_model_runs.values().any(|r| {
                    matches!(
                        r.snapshot.status.as_str(),
                        "queued"
                            | "running"
                            | "waiting_confirmation"
                            | "submitting"
                            | "terminating"
                            | "recovery_required"
                    )
                });
            drop(state);
            if settled {
                break;
            }
            if Instant::now() >= deadline {
                return Err("personal_maintenance_drain_unconfirmed".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.retire_idle_harnesses()?;
        self.personal_models.shutdown()?;
        let (workbench, application) = {
            let mut state = lock(&self.inner);
            persist_quiescent_ledger(&mut state)?;
            if persistence_outcome_unknown(&state) {
                return Err("personal_maintenance_ledger_unconfirmed".into());
            }
            (
                state.codex_threads.clone(),
                state.application_threads.clone(),
            )
        };
        if let Some(service) = workbench {
            service.shutdown_checked()?;
        }
        if let Some(service) = application {
            service.shutdown_checked()?;
        }
        ensure_drained(&lock(&self.inner))
    }
    fn interrupt_owned_runs_for_maintenance(&self) -> Result<(), String> {
        let applications = {
            let state = lock(&self.inner);
            state
                .application_model_runs
                .iter()
                .filter(|(_, r)| {
                    matches!(
                        r.snapshot.status.as_str(),
                        "queued" | "running" | "waiting_confirmation" | "submitting"
                    )
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
        };
        for id in applications {
            self.cancel_application_model_run(&id)?;
        }
        let controls = {
            let mut state = lock(&self.inner);
            let submitted = std::mem::take(&mut state.submitted_harnesses);
            for run in &mut state.persisted.runs {
                if submitted.contains_key(&run.run_id) {
                    run.status = "interrupted".into();
                    run.completed_at = unix_epoch_millis().ok();
                    run.thread_writer_active = false;
                    run.error_reason = Some("资料维护开始，运行未执行。".into());
                }
            }
            for (_, sender) in state.application_write_controls.drain() {
                let _ = sender.send(Err("personal_data_maintenance_active".into()));
            }
            for (_, sender) in state.application_model_write_controls.drain() {
                let _ = sender.send(Err("personal_data_maintenance_active".into()));
            }
            state.harness_controls.values().cloned().collect::<Vec<_>>()
        };
        for control in controls {
            control.close_input()?;
        }
        Ok(())
    }
}

pub(super) fn persist_quiescent_ledger(state: &mut ManagerState) -> Result<(), String> {
    super::application_handoff::flush(state)?;
    super::flush_pending_turn_finalization(state)?;
    super::flush_pending_ledger_write(state)?;
    super::ensure_persistence_ready(state)?;
    let projected = super::ledger_projection::state_without_messages(&state.persisted);
    let pending = state
        .ledger_baseline
        .prepare_cached(&projected, &state.persisted.messages)?;
    if pending.request["records"]
        .as_array()
        .is_some_and(Vec::is_empty)
    {
        return Ok(());
    }
    state.pending_ledger_write = Some(pending);
    super::flush_pending_ledger_write(state)
}

fn ensure_drained(state: &ManagerState) -> Result<(), String> {
    if state.maintenance_fence.is_none()
        || state.local_operations != 0
        || state.memory_resetting
        || !state.harness_processes.is_empty()
        || !state.harness_controls.is_empty()
        || state.harness_retirement_error.is_some()
        || persistence_outcome_unknown(state)
        || state.persisted.runs.iter().any(|run| {
            run.thread_writer_active
                || matches!(
                    run.status.as_str(),
                    "queued" | "running" | "paused" | "waiting_permission" | "terminating"
                )
        })
        || state.pending_attachments.values().any(|a| !a.complete)
        || state.application_model_runs.values().any(|r| {
            matches!(
                r.snapshot.status.as_str(),
                "queued"
                    | "running"
                    | "waiting_confirmation"
                    | "submitting"
                    | "terminating"
                    | "recovery_required"
            )
        })
    {
        return Err("personal_maintenance_drain_unconfirmed".into());
    }
    Ok(())
}

pub(super) fn native_homes(root: &Path) -> Result<Vec<PersonalBackupNativeHome>, String> {
    let ai = root.join("ai-runtime");
    let mut paths = vec![
        ConversationKind::Workbench.harness_home(&ai),
        ConversationKind::ApplicationPage.harness_home(&ai),
    ];
    let connections = ai.join("model-connections");
    if connections.exists() {
        if fs::symlink_metadata(&connections)
            .map_err(|_| "personal_maintenance_native_home_invalid")?
            .file_type()
            .is_symlink()
        {
            return Err("personal_maintenance_native_home_invalid".into());
        }
        for entry in
            fs::read_dir(&connections).map_err(|_| "personal_maintenance_native_home_invalid")?
        {
            let entry = entry.map_err(|_| "personal_maintenance_native_home_invalid")?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "personal_maintenance_native_home_invalid")?;
            if name.len() != 64
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err("personal_maintenance_native_home_invalid".into());
            }
            if !entry
                .file_type()
                .map_err(|_| "personal_maintenance_native_home_invalid")?
                .is_dir()
            {
                return Err("personal_maintenance_native_home_invalid".into());
            }
            paths.push(entry.path().join("workbench/official-runtime"));
            paths.push(entry.path().join("application/official-runtime"));
        }
    }
    let mut homes = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        if !fs::symlink_metadata(&path)
            .map_err(|_| "personal_maintenance_native_home_invalid")?
            .is_dir()
        {
            return Err("personal_maintenance_native_home_invalid".into());
        }
        for ancestor in path.ancestors().take_while(|p| *p != root) {
            if fs::symlink_metadata(ancestor)
                .map_err(|_| "personal_maintenance_native_home_invalid")?
                .file_type()
                .is_symlink()
            {
                return Err("personal_maintenance_native_home_invalid".into());
            }
        }
        let canonical = path
            .canonicalize()
            .map_err(|_| "personal_maintenance_native_home_invalid")?;
        let relative = canonical
            .strip_prefix(root)
            .map_err(|_| "personal_maintenance_native_home_invalid")?
            .to_str()
            .ok_or("personal_maintenance_native_home_invalid")?
            .replace('\\', "/");
        homes.push(PersonalBackupNativeHome {
            relative_path: relative,
            runtime_version: Some(crate::model_connections::OFFICIAL_RUNTIME_VERSION.into()),
        });
    }
    homes.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    homes.dedup_by(|a, b| a.relative_path == b.relative_path);
    Ok(homes)
}

#[cfg(test)]
mod tests {
    use super::{AiRuntimeManager, AppServerSupervisor, PathBuf, lock};
    use crate::ai_runtime::{ensure_profile_admission, model_test_fixture};
    use serde_json::{Value, json};

    struct Fixture {
        owner: PathBuf,
        root: PathBuf,
        server: AppServerSupervisor,
        manager: AiRuntimeManager,
    }
    impl Fixture {
        fn new() -> Self {
            let owner = model_test_fixture::fixture_root("maintenance-runtime");
            let root = model_test_fixture::profile_data_root(&owner);
            let server = AppServerSupervisor::new();
            server
                .start(
                    &crate::supervisor::app_server_test_binary(),
                    "test",
                    &root,
                    &root.join("run"),
                )
                .unwrap();
            let manager = AiRuntimeManager::new();
            manager.initialize(&root, &server).unwrap();
            Self {
                owner,
                root,
                server,
                manager,
            }
        }
        fn backup(&self) -> Value {
            self.server.request_result("personalData/backupPrepare",&json!({"selection":{"includeWorkspaceFiles":[]},"clientOperationId":format!("maintenance-test-{}",uuid::Uuid::new_v4().simple())})).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.manager.shutdown();
            self.server.shutdown().unwrap();
            if std::thread::panicking() {
                eprintln!("maintenance_fixture_failure={}", self.owner.display());
            } else {
                std::fs::remove_dir_all(&self.owner).unwrap();
            }
        }
    }

    #[test]
    fn maintenance_closes_local_and_cross_manager_admission_until_checked_release() {
        let _guard = crate::supervisor::app_server_test_guard();
        let f = Fixture::new();
        let op = f.backup();
        let held = f
            .manager
            .begin_profile_maintenance(
                &f.root,
                op["operationId"].as_str().unwrap(),
                op["previewDigest"].as_str().unwrap(),
            )
            .unwrap();
        assert!(f.manager.begin_capability_operation().is_err());
        assert!(f.manager.reset_for_profile_switch().is_err());
        assert!(f.manager.initialize(&f.root, &f.server).is_err());
        let owner = crate::model_connections::execution_owner().unwrap();
        let denied=f.server.request_result("profileWriter/prepare",&json!({"clientOperationId":"cross-manager-blocked","kind":"user_mcp","resourceId":"own-mcp","resourceRevision":"v1","owner":owner,"launchProtocol":1,"processMode":"remote_request"}));
        assert!(
            denied
                .unwrap_err()
                .contains("personal_data_maintenance_active")
        );
        let outcome=f.server.request_result("personalData/backupCapture",&json!({"operationId":op["operationId"],"previewDigest":op["previewDigest"],"clientOperationId":"maintenance-capture","quiescence":held.quiescence()})).unwrap();
        assert_eq!(outcome["state"], "completed", "{outcome}");
        assert_eq!(outcome["backupManifest"]["complete"], true);
        held.finish().unwrap();
        assert!(f.manager.begin_capability_operation().is_ok());
    }

    #[test]
    fn dropping_a_drained_guard_does_not_release_a_backup_fence() {
        let _guard = crate::supervisor::app_server_test_guard();
        let f = Fixture::new();
        let op = f.backup();
        let held = f
            .manager
            .begin_profile_maintenance(
                &f.root,
                op["operationId"].as_str().unwrap(),
                op["previewDigest"].as_str().unwrap(),
            )
            .unwrap();
        let proof = held.quiescence().clone();
        drop(held);
        assert!(ensure_profile_admission(&lock(&f.manager.inner)).is_err());
        let status = f
            .server
            .request_result(
                "personalMaintenance/drainStatus",
                &json!({"fenceId":proof.token,"owner":proof.owner}),
            )
            .unwrap();
        assert_eq!(status["fence"]["state"], "draining");
        let held = f
            .manager
            .begin_profile_maintenance(
                &f.root,
                op["operationId"].as_str().unwrap(),
                op["previewDigest"].as_str().unwrap(),
            )
            .unwrap();
        held.finish().unwrap();
    }

    #[test]
    fn registered_writer_keeps_maintenance_incomplete_and_is_never_auto_released() {
        let _guard = crate::supervisor::app_server_test_guard();
        let f = Fixture::new();
        let op = f.backup();
        let owner = crate::model_connections::execution_owner().unwrap();
        let writer=f.server.request_result("profileWriter/prepare",&json!({"clientOperationId":"other-writer","kind":"user_mcp","resourceId":"other-mcp","resourceRevision":"v1","owner":owner,"launchProtocol":1,"processMode":"remote_request"})).unwrap();
        let held = f.manager.begin_profile_maintenance(
            &f.root,
            op["operationId"].as_str().unwrap(),
            op["previewDigest"].as_str().unwrap(),
        );
        assert_eq!(held.err().as_deref(), Some("personal_data_writer_active"));
        let active = f
            .server
            .request_result("profileWriter/list", &json!({}))
            .unwrap();
        assert_eq!(active[0]["leaseId"], writer["leaseId"]);
        f.server.request_result("profileWriter/release",&json!({"leaseId":writer["leaseId"],"owner":owner,"localTransportFinished":true,"remoteOutcomeUnconfirmed":true})).unwrap();
        let held = f
            .manager
            .begin_profile_maintenance(
                &f.root,
                op["operationId"].as_str().unwrap(),
                op["previewDigest"].as_str().unwrap(),
            )
            .unwrap();
        held.finish().unwrap();
    }

    #[test]
    fn another_native_history_process_remains_a_writer_until_its_actual_exit() {
        let _guard = crate::supervisor::app_server_test_guard();
        let f = Fixture::new();
        let op = f.backup();
        let home = crate::conversation_kernel::ConversationKind::Workbench
            .harness_home(&f.root.join("ai-runtime"));
        let other = crate::codex_threads::CodexThreadService::new(
            crate::supervisor::codex_harness_test_binary(),
            home,
        )
        .with_profile_dir(f.root.join("ai-runtime"))
        .with_history_server(f.server.clone());
        other.catalog().unwrap();
        let result = f.manager.begin_profile_maintenance(
            &f.root,
            op["operationId"].as_str().unwrap(),
            op["previewDigest"].as_str().unwrap(),
        );
        assert_eq!(result.err().as_deref(), Some("personal_data_writer_active"));
        let active = f
            .server
            .request_result("profileWriter/list", &json!({}))
            .unwrap();
        assert!(
            active
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["kind"] == "native_history")
        );
        other.shutdown_checked().unwrap();
        let held = f
            .manager
            .begin_profile_maintenance(
                &f.root,
                op["operationId"].as_str().unwrap(),
                op["previewDigest"].as_str().unwrap(),
            )
            .unwrap();
        held.finish().unwrap();
    }

    #[test]
    fn offline_restore_handoff_preserves_the_closed_old_manager() {
        let _guard = crate::supervisor::app_server_test_guard();
        let f = Fixture::new();
        let op = f.backup();
        let held = f
            .manager
            .begin_profile_maintenance(
                &f.root,
                op["operationId"].as_str().unwrap(),
                op["previewDigest"].as_str().unwrap(),
            )
            .unwrap();
        let receipt = held.consume_for_offline_restore().unwrap();
        assert_eq!(
            receipt.quiescence().profile_id,
            f.root.file_name().unwrap().to_str().unwrap()
        );
        f.server.shutdown().unwrap();
        assert!(f.manager.begin_capability_operation().is_err());
        assert!(f.manager.initialize(&f.root, &f.server).is_err());
    }

    #[test]
    fn explicit_end_of_failed_maintenance_preserves_incomplete_operation_and_writer() {
        let _guard = crate::supervisor::app_server_test_guard();
        let f = Fixture::new();
        let op = f.backup();
        let owner = crate::model_connections::execution_owner().unwrap();
        let writer=f.server.request_result("profileWriter/prepare",&json!({"clientOperationId":"explicit-end-writer","kind":"user_mcp","resourceId":"own-mcp","resourceRevision":"v1","owner":owner,"launchProtocol":1,"processMode":"remote_request"})).unwrap();
        assert_eq!(
            f.manager
                .begin_profile_maintenance(
                    &f.root,
                    op["operationId"].as_str().unwrap(),
                    op["previewDigest"].as_str().unwrap()
                )
                .err()
                .as_deref(),
            Some("personal_data_writer_active")
        );
        let incomplete=f.server.request_result("personalData/backupCapture",&json!({"operationId":op["operationId"],"previewDigest":op["previewDigest"],"clientOperationId":"failed-maintenance-record","quiescence":null})).unwrap();
        assert_eq!(incomplete["state"], "incomplete");
        let fence = lock(&f.manager.inner).maintenance_fence.clone().unwrap();
        let listed = super::personal_maintenance_status(&f.server).unwrap();
        assert!(listed.contains(&fence));
        assert!(
            super::recover_personal_maintenance(&f.server, &fence.fence_id)
                .unwrap_err()
                .contains("personal_maintenance_owner_alive")
        );
        f.manager
            .cancel_profile_maintenance(&f.root, &fence.fence_id)
            .unwrap();
        assert!(f.manager.begin_capability_operation().is_ok());
        let still_incomplete = f
            .server
            .request_result(
                "personalData/get",
                &json!({"operationId":op["operationId"]}),
            )
            .unwrap();
        assert_eq!(still_incomplete["state"], "incomplete");
        let active = f
            .server
            .request_result("profileWriter/list", &json!({}))
            .unwrap();
        assert_eq!(active[0]["leaseId"], writer["leaseId"]);
        f.server.request_result("profileWriter/release",&json!({"leaseId":writer["leaseId"],"owner":owner,"localTransportFinished":true,"remoteOutcomeUnconfirmed":true})).unwrap();
    }
}
