use super::{AiRuntimeManager, AppServerSupervisor, Path, PathBuf, lock};
use contexthub_protocol::{
    PersonalMemoryResetConfirmParams, PersonalMemoryResetFinishParams,
    PersonalMemoryResetOperation, PersonalMemoryResetPermit,
    PersonalMemoryResetScopeCompleteParams, PersonalMemoryResetStartParams,
    PersonalMemoryResetStartResult, PersonalMemoryResetState, PersonalMemoryResetWriterParams,
    PersonalWriterLease,
};
use serde_json::{Value, json};

impl AiRuntimeManager {
    /// # Errors
    /// 只在当前预览、全资料维护和一次性作用域许可均可核对时调用官方记忆删除接口。
    pub fn personal_memory_reset_confirm(
        &self,
        expected_profile_root: &Path,
        request: PersonalMemoryResetConfirmParams,
    ) -> Result<PersonalMemoryResetOperation, String> {
        let server = {
            let state = lock(&self.inner);
            super::ensure_profile_admission(&state)?;
            state.server.clone().ok_or("ai_runtime_not_initialized")?
        };
        let Ok(guard) = self.begin_profile_maintenance(
            expected_profile_root,
            &request.operation_id,
            &request.preview_digest,
        ) else {
            return start(&server, &request, None).map(|result| result.operation);
        };
        let result = start(&server, &request, Some(guard.quiescence().clone()))?;
        if result.operation.state != PersonalMemoryResetState::Applying {
            guard.finish()?;
            return Ok(result.operation);
        }
        self.synchronize_reset_memory_authorization(&server)?;
        let mut failed = false;
        for permit in result.permits {
            let receipt = if failed {
                ScopeReceipt {
                    lease_id: None,
                    response: None,
                    error: Some("personal_memory_reset_not_executed".into()),
                }
            } else {
                reset_scope(&server, expected_profile_root, &permit)
            };
            failed |= receipt.error.is_some();
            let params = PersonalMemoryResetScopeCompleteParams {
                operation_id: request.operation_id.clone(),
                preview_digest: request.preview_digest.clone(),
                scope_id: permit.scope_id,
                writer_lease_id: receipt.lease_id,
                owner: permit.owner,
                official_response: receipt.response,
                failure_reason: receipt.error,
            };
            server.request_result(
                "personalMemoryReset/scopeComplete",
                &serde_json::to_value(params)
                    .map_err(|_| "personal_memory_reset_request_invalid")?,
            )?;
        }
        let operation: PersonalMemoryResetOperation = serde_json::from_value(
            server.request_result(
                "personalMemoryReset/finish",
                &serde_json::to_value(PersonalMemoryResetFinishParams {
                    operation_id: request.operation_id,
                    preview_digest: request.preview_digest,
                    owner: guard.quiescence().owner.clone(),
                })
                .map_err(|_| "personal_memory_reset_request_invalid")?,
            )?,
        )
        .map_err(|_| "personal_memory_reset_response_invalid")?;
        if !matches!(
            operation.state,
            PersonalMemoryResetState::Completed | PersonalMemoryResetState::Incomplete
        ) {
            return Err("personal_memory_reset_operation_unconfirmed".into());
        }
        guard.finish()?;
        Ok(operation)
    }

    fn synchronize_reset_memory_authorization(
        &self,
        server: &AppServerSupervisor,
    ) -> Result<(), String> {
        let mut state = lock(&self.inner);
        if state.maintenance_fence.is_none() {
            return Err("personal_maintenance_owner_changed".into());
        }
        super::personal_memory::refresh_authorization_from_ledger(&mut state, server)
    }
}

fn start(
    server: &AppServerSupervisor,
    request: &PersonalMemoryResetConfirmParams,
    quiescence: Option<contexthub_protocol::PersonalDataQuiescence>,
) -> Result<PersonalMemoryResetStartResult, String> {
    serde_json::from_value(
        server.request_result(
            "personalMemoryReset/start",
            &serde_json::to_value(PersonalMemoryResetStartParams {
                operation_id: request.operation_id.clone(),
                preview_digest: request.preview_digest.clone(),
                quiescence,
            })
            .map_err(|_| "personal_memory_reset_request_invalid")?,
        )?,
    )
    .map_err(|_| "personal_memory_reset_response_invalid".into())
}

struct ScopeReceipt {
    lease_id: Option<String>,
    response: Option<Value>,
    error: Option<String>,
}

fn reset_scope(
    server: &AppServerSupervisor,
    root: &Path,
    permit: &PersonalMemoryResetPermit,
) -> ScopeReceipt {
    let mut receipt = ScopeReceipt {
        lease_id: None,
        response: None,
        error: None,
    };
    let result = (|| {
        let home = reset_home(root, permit)?;
        let lease: PersonalWriterLease = serde_json::from_value(
            server.request_result(
                "personalMemoryReset/prepareWriter",
                &serde_json::to_value(PersonalMemoryResetWriterParams {
                    permit: permit.clone(),
                    client_operation_id: format!("memory-reset-{}", uuid::Uuid::new_v4().simple()),
                })
                .map_err(|_| "personal_memory_reset_request_invalid")?,
            )?,
        )
        .map_err(|_| "personal_memory_reset_response_invalid")?;
        receipt.lease_id = Some(lease.lease_id.clone());
        if lease.owner != permit.owner
            || lease.resource_id != permit.scope_id
            || lease.resource_revision != permit.preview_digest
        {
            return Err("personal_memory_reset_writer_invalid".into());
        }
        reset_home(root, permit)?;
        let service = crate::codex_threads::CodexThreadService::new(
            PathBuf::from(super::bundled_codex_harness_executable()?),
            home,
        )
        .with_memory_reset_writer(server.clone(), lease)?;
        let response = service.request("memory/reset", &json!({}));
        let stopped = service.shutdown_checked();
        if let Ok(value) = &response
            && value.as_object().is_some_and(serde_json::Map::is_empty)
        {
            receipt.response = Some(value.clone());
        }
        stopped.map_err(|_| "personal_memory_reset_exit_unconfirmed")?;
        let value = response.map_err(|_| "personal_memory_reset_response_unconfirmed")?;
        if !value.as_object().is_some_and(serde_json::Map::is_empty) {
            return Err("personal_memory_reset_response_unconfirmed".into());
        }
        Ok(())
    })();
    receipt.error = result.err();
    receipt
}

fn reset_home(root: &Path, permit: &PersonalMemoryResetPermit) -> Result<PathBuf, String> {
    let relative = Path::new(&permit.native_home_relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || permit.owner != crate::model_connections::execution_owner()?
    {
        return Err("personal_memory_reset_scope_denied".into());
    }
    let root = root
        .canonicalize()
        .map_err(|_| "personal_memory_reset_scope_unavailable")?;
    if !super::maintenance::native_homes(&root)?
        .iter()
        .any(|home| home.relative_path == permit.native_home_relative_path)
    {
        return Err("personal_memory_reset_scope_changed".into());
    }
    let home = root.join(relative);
    let metadata =
        contexthub_platform_adapter::filesystem::symlink_metadata_with_stable_identity(&home)
            .map_err(|_| "personal_memory_reset_scope_unavailable")?;
    if metadata.stable_identity != permit.native_home_fingerprint
        || !metadata.metadata.is_dir()
        || metadata.metadata.file_type().is_symlink()
        || contexthub_platform_adapter::filesystem::is_cloud_placeholder_file(&metadata.metadata)
    {
        return Err("personal_memory_reset_scope_changed".into());
    }
    Ok(home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_runtime::model_test_fixture::{fixture_root, profile_data_root};

    #[test]
    fn official_profile_memory_reset_covers_all_managed_scopes_and_preserves_conversations() {
        let _guard = crate::supervisor::app_server_test_guard();
        let owner = fixture_root("profile-memory-reset");
        let root = profile_data_root(&owner);
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
        let conversation=manager.create_conversation(serde_json::from_value(json!({"title":"保留本地会话","conversationType":"direct","mode":"discuss","context":{"workspaceRoot":""}})).unwrap()).unwrap();
        let connection = root
            .join("ai-runtime/model-connections")
            .join("a".repeat(64));
        for kind in [
            super::super::ConversationKind::Workbench,
            super::super::ConversationKind::ApplicationPage,
        ] {
            std::fs::create_dir_all(kind.harness_home(&root.join("ai-runtime"))).unwrap();
        }
        for scope in ["workbench/official-runtime", "application/official-runtime"] {
            std::fs::create_dir_all(connection.join(scope)).unwrap();
        }
        let homes = super::super::maintenance::native_homes(&root).unwrap();
        assert!(homes.len() >= 4);
        for home in &homes {
            seed_scope(&root.join(&home.relative_path));
        }
        assert!(manager.memory_settings().unwrap().has_memory);
        let operation: PersonalMemoryResetOperation = serde_json::from_value(
            server
                .request_result(
                    "personalMemoryReset/prepare",
                    &json!({"clientOperationId":uuid::Uuid::new_v4().simple().to_string()}),
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(operation.scopes.len(), homes.len());
        let result = manager
            .personal_memory_reset_confirm(
                &root,
                PersonalMemoryResetConfirmParams {
                    operation_id: operation.operation_id,
                    preview_digest: operation.preview_digest,
                },
            )
            .unwrap();
        assert_eq!(
            result.state,
            PersonalMemoryResetState::Completed,
            "{result:?}"
        );
        assert!(result.scope_results.iter().all(|scope| scope.outcome
            == contexthub_protocol::PersonalMemoryResetOutcome::Reset
            && scope.official_response_received
            && scope.process_exit_confirmed));
        for home in &homes {
            verify_scope(&root.join(&home.relative_path));
        }
        let snapshot = serde_json::to_value(manager.snapshot().unwrap()).unwrap();
        assert!(
            snapshot["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["conversationId"] == conversation.conversation_id
                    && c["title"] == "保留本地会话")
        );
        let settings = manager.memory_settings().unwrap();
        assert!(!settings.enabled);
        assert!(!settings.has_memory);
        assert!(lock(&manager.inner).maintenance_fence.is_none());
        manager.shutdown();
        server.shutdown().unwrap();
        std::fs::remove_dir_all(owner).unwrap();
    }

    fn seed_scope(home: &Path) {
        for directory in [
            "memories",
            "memories_v2",
            "memories_extensions",
            "memory-publication",
        ] {
            std::fs::create_dir_all(home.join(directory)).unwrap();
            std::fs::write(
                home.join(directory).join("scope-canary.txt"),
                "仅本机验收用的记忆范围标记",
            )
            .unwrap();
        }
        std::fs::write(home.join("memories/MEMORY.md"), "本机验收记忆").unwrap();
        std::fs::write(home.join("memories/memory_summary.md"), "本机验收摘要").unwrap();
        std::fs::write(
            home.join("business-preserve-marker.txt"),
            "本机验收非记忆资料",
        )
        .unwrap();
    }

    fn verify_scope(home: &Path) {
        for directory in ["memories", "memories_v2", "memories_extensions"] {
            assert!(
                std::fs::read_dir(home.join(directory))
                    .unwrap()
                    .next()
                    .is_none()
            );
        }
        assert!(home.join("memory-publication/scope-canary.txt").is_file());
        assert!(home.join("business-preserve-marker.txt").is_file());
    }
}
