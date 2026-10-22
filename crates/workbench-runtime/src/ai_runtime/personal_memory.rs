use super::{
    AiRuntimeManager, data_dir_for_state, ensure_profile_admission, lock, persist_locked,
    restore_after_save_rejection,
};
use crate::memory::{
    MemoryEnableConfirmation, MemoryPreferences, MemoryServiceDisclosure, MemorySettings,
    MemorySettingsSaveRequest,
};
use contexthub_protocol::{
    ModelConnection, ModelExecutionPrepareParams, ModelPurpose, ModelRuntimeKind,
};
use serde_json::json;

impl AiRuntimeManager {
    pub(super) fn personal_memory_service(&self) -> Result<MemoryServiceDisclosure, String> {
        let snapshot = self.personal_model_resolve(ModelExecutionPrepareParams {
            launch_protocol: 1,
            owner: None,
            purpose: ModelPurpose::Memory,
            binding_id: None,
            selection: None,
            conversation_id: None,
        })?;
        if snapshot.runtime_kind != ModelRuntimeKind::Workbench {
            return Err("model_runtime_kind_mismatch".into());
        }
        let connection: ModelConnection =
            serde_json::from_value(self.personal_model_server()?.request_result(
                "modelConnection/get",
                &json!({"connectionId":snapshot.connection_id}),
            )?)
            .map_err(|_| "model_connection_response_invalid")?;
        let binding_id = snapshot.binding_id.ok_or("model_memory_binding_required")?;
        let binding_revision = snapshot
            .binding_revision
            .ok_or("model_memory_binding_required")?;
        let history_scope =
            "当前连接作用域内符合官方 Codex 记忆规则的历史会话，排除含外部工具上下文的会话"
                .to_owned();
        let tool_chats_history_scope =
            "当前连接作用域内符合官方 Codex 记忆规则的历史会话，包括外部工具上下文".to_owned();
        let service_name = connection.config.name.clone();
        let consent_digest = crate::persistent_threads::digest(&json!({
            "version":1,"profileId":snapshot.profile_id,"bindingId":binding_id,"bindingRevision":binding_revision,
            "connectionId":snapshot.connection_id,"configRevision":snapshot.config_revision,"authGeneration":snapshot.auth_generation,
            "modelId":snapshot.model_id,"runtimeHomeScope":snapshot.runtime_home_scope,"serviceName":service_name,
            "baseUrl":snapshot.base_url,"historyScope":history_scope,"toolChatsHistoryScope":tool_chats_history_scope,"mayIncurCost":true
        }));
        Ok(MemoryServiceDisclosure {
            binding_id,
            binding_revision,
            connection_id: snapshot.connection_id,
            connection_name: connection.config.name,
            config_revision: snapshot.config_revision,
            auth_generation: snapshot.auth_generation,
            model_id: snapshot.model_id,
            service_name,
            base_url: snapshot.base_url,
            history_scope,
            tool_chats_history_scope,
            may_incur_cost: true,
            consent_digest,
        })
    }

    pub(super) fn personal_memory_settings_read(&self) -> Result<MemorySettings, String> {
        self.refresh_personal_memory_authorization()?;
        let (preferences, confirmation, data_dir, running, last_error) = {
            let state = lock(&self.inner);
            (
                state.persisted.memory_preferences,
                state.persisted.memory_enable_confirmation.clone(),
                data_dir_for_state(&state)?,
                state.memory_resetting
                    || state.local_operations > 0
                    || state.persisted.runs.iter().any(|run| {
                        run.thread_writer_active
                            || matches!(
                                run.status.as_str(),
                                "queued"
                                    | "running"
                                    | "paused"
                                    | "waiting_permission"
                                    | "terminating"
                            )
                    }),
                state.memory_last_error.clone(),
            )
        };
        let root = data_dir.parent().ok_or("ai_memory_path_denied")?;
        let homes = super::maintenance::native_homes(root)?;
        let mut has_memory = false;
        let mut last_updated_at = None;
        let mut native_scope_count = 0;
        for home in homes.iter().filter(|home| {
            home.relative_path.ends_with("workbench/official-runtime")
                || home.relative_path == "ai-runtime/cilai-harness/official-runtime"
        }) {
            let memory = crate::memory::native_files(&root.join(&home.relative_path))?;
            native_scope_count += 1;
            has_memory |= memory.has_memory;
            last_updated_at = last_updated_at.max(memory.last_updated_at);
        }
        let (service, service_error) = match self.personal_memory_service() {
            Ok(service) => {
                let error = preferences
                    .enabled
                    .then(|| {
                        confirmation
                            .as_ref()
                            .ok_or_else(|| "ai_memory_enable_consent_required".to_owned())
                            .and_then(|confirmation| {
                                validate_confirmation(
                                    &service,
                                    preferences.from_tool_chats,
                                    confirmation,
                                )
                            })
                    })
                    .transpose()
                    .err();
                (Some(service), error)
            }
            Err(error) => (None, Some(error)),
        };
        let model_label = service.as_ref().map(|service| service.model_id.clone());
        Ok(MemorySettings {
            lifecycle_ready: true,
            enabled: preferences.enabled,
            from_tool_chats: preferences.from_tool_chats,
            running,
            has_memory,
            model_label,
            last_error,
            phase: if !preferences.enabled {
                "disabled"
            } else if service_error.is_some() {
                "blocked"
            } else if has_memory {
                "available"
            } else {
                "empty"
            }
            .into(),
            last_updated_at,
            service,
            service_error,
            native_scope_count,
        })
    }

    /// # Errors
    /// 启用记忆时必须确认当前用途、模型服务、历史范围与费用；变更后旧确认失效。
    pub fn save_personal_memory_settings(
        &self,
        request: MemorySettingsSaveRequest,
    ) -> Result<MemorySettings, String> {
        self.refresh_personal_memory_authorization()?;
        let enabled = request.enabled;
        let from_tool_chats = request.from_tool_chats;
        let confirmation = self.memory_confirmation_for_save(request)?;
        let mut state = lock(&self.inner);
        ensure_profile_admission(&state)?;
        let previous = state.persisted.memory_preferences;
        let previous_confirmation = state.persisted.memory_enable_confirmation.clone();
        state.persisted.memory_preferences = MemoryPreferences {
            enabled,
            from_tool_chats,
        };
        state.persisted.memory_enable_confirmation = confirmation;
        if let Err(error) = persist_locked(&mut state) {
            restore_after_save_rejection(&mut state, |persisted| {
                persisted.memory_preferences = previous;
                persisted.memory_enable_confirmation = previous_confirmation;
            });
            return Err(error);
        }
        if enabled {
            state.memory_last_error = None;
        }
        drop(state);
        if previous
            != (MemoryPreferences {
                enabled,
                from_tool_chats,
            })
        {
            self.retire_idle_harnesses()?;
        }
        self.personal_memory_settings_read()
    }

    fn memory_confirmation_for_save(
        &self,
        request: MemorySettingsSaveRequest,
    ) -> Result<Option<MemoryEnableConfirmation>, String> {
        if !request.enabled {
            return Ok(None);
        }
        let service = self.personal_memory_service()?;
        let (previous, previous_confirmation) = {
            let state = lock(&self.inner);
            (
                state.persisted.memory_preferences,
                state.persisted.memory_enable_confirmation.clone(),
            )
        };
        let confirmation = if let Some(confirmation) = request.confirmation {
            confirmation
        } else {
            let mut previous_confirmation =
                previous_confirmation.ok_or("ai_memory_enable_consent_required")?;
            if !previous.enabled || (request.from_tool_chats && !previous.from_tool_chats) {
                return Err("ai_memory_enable_consent_required".into());
            }
            validate_confirmation(&service, previous.from_tool_chats, &previous_confirmation)?;
            previous_confirmation.from_tool_chats = request.from_tool_chats;
            previous_confirmation
        };
        validate_confirmation(&service, request.from_tool_chats, &confirmation)?;
        let snapshot = self.personal_model_resolve(ModelExecutionPrepareParams {
            launch_protocol: 1,
            owner: None,
            purpose: ModelPurpose::Memory,
            binding_id: Some(service.binding_id.clone()),
            selection: None,
            conversation_id: None,
        })?;
        self.personal_model_credential_preflight(&snapshot)?;
        if self.personal_memory_service()?.consent_digest != service.consent_digest {
            return Err("ai_memory_enable_consent_stale".into());
        }
        Ok(Some(confirmation))
    }

    pub(crate) fn personal_memory_preferences(&self) -> MemoryPreferences {
        lock(&self.inner).persisted.memory_preferences
    }

    pub(crate) fn validate_personal_memory_scope(
        &self,
        expected: MemoryPreferences,
    ) -> Result<(), String> {
        let authorization = self.personal_memory_authorization()?;
        if authorization.0 != expected {
            return Err("ai_memory_preferences_changed".into());
        }
        validate_confirmation(
            &self.personal_memory_service()?,
            expected.from_tool_chats,
            authorization
                .1
                .as_ref()
                .ok_or("ai_memory_enable_consent_required")?,
        )
    }

    fn personal_memory_authorization(
        &self,
    ) -> Result<(MemoryPreferences, Option<MemoryEnableConfirmation>), String> {
        let result: contexthub_protocol::PersonalMemoryAuthorization = serde_json::from_value(
            self.personal_model_server()?
                .request_result("personalMemory/authorization", &json!({}))?,
        )
        .map_err(|_| "personal_memory_authorization_invalid")?;
        Ok((
            MemoryPreferences {
                enabled: result.preferences.enabled,
                from_tool_chats: result.preferences.from_tool_chats,
            },
            result.confirmation.map(|c| MemoryEnableConfirmation {
                consent_digest: c.consent_digest,
                from_tool_chats: c.from_tool_chats,
                confirmed_at: c.confirmed_at,
            }),
        ))
    }

    pub(super) fn refresh_personal_memory_authorization(&self) -> Result<(), String> {
        let authorization = self.personal_memory_authorization()?;
        let mut state = lock(&self.inner);
        if state.persisted.memory_preferences == authorization.0
            && state.persisted.memory_enable_confirmation == authorization.1
        {
            return Ok(());
        }
        super::ensure_persistence_ready(&state)?;
        let server = state.server.clone().ok_or("ai_runtime_not_initialized")?;
        refresh_authorization_from_ledger(&mut state, &server)
    }

    pub(super) fn validate_personal_memory_confirmation(&self) -> Result<(), String> {
        let (preferences, confirmation) = {
            let state = lock(&self.inner);
            (
                state.persisted.memory_preferences,
                state.persisted.memory_enable_confirmation.clone(),
            )
        };
        if !preferences.enabled {
            return Err("ai_memory_disabled".into());
        }
        validate_confirmation(
            &self.personal_memory_service()?,
            preferences.from_tool_chats,
            confirmation
                .as_ref()
                .ok_or("ai_memory_enable_consent_required")?,
        )
    }
}

fn validate_confirmation(
    service: &MemoryServiceDisclosure,
    from_tool_chats: bool,
    confirmation: &MemoryEnableConfirmation,
) -> Result<(), String> {
    if confirmation.consent_digest != service.consent_digest
        || confirmation.from_tool_chats != from_tool_chats
        || confirmation.confirmed_at <= 0
    {
        return Err("ai_memory_enable_consent_stale".into());
    }
    Ok(())
}

pub(super) fn refresh_authorization_from_ledger(
    state: &mut super::ManagerState,
    server: &super::AppServerSupervisor,
) -> Result<(), String> {
    super::ensure_persistence_ready(state)?;
    if super::persistence_outcome_unknown(state) {
        return Err("personal_memory_authorization_unconfirmed".into());
    }
    let result = server.request_result("aiLedger/get", &json!({}))?;
    let (baseline, metadata) = super::ledger::Baseline::read(&result["ledger"])?;
    let preferences = match metadata.get("memoryPreferences") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| "personal_memory_authorization_invalid")?,
        None => MemoryPreferences::default(),
    };
    let confirmation = match metadata.get("memoryEnableConfirmation") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|_| "personal_memory_authorization_invalid")?,
        None => None,
    };
    state.persisted.memory_preferences = preferences;
    state.persisted.memory_enable_confirmation = confirmation;
    state.ledger_baseline.refresh_memory_entries(&baseline);
    Ok(())
}
