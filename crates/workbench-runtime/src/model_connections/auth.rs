use super::{
    AiRuntimeManager, Arc, CodexThreadService, ConnectionRuntime, ModelAuthCancel,
    ModelAuthOperation, ModelAuthTarget, ModelConnection, ModelConnectionKind, ModelRuntimeKind,
    PathBuf, PendingAuth, Value, connection, execution_owner, scoped_home,
};
use contexthub_protocol::{
    ModelAuthChangeState, ModelAuthIntent, ModelAuthStepState, RootAuthChange,
};
use serde_json::json;

fn pending(change: &RootAuthChange) -> bool {
    matches!(
        change.state,
        ModelAuthChangeState::Prepared
            | ModelAuthChangeState::InProgress
            | ModelAuthChangeState::Uncertain
    )
}

fn changes(manager: &AiRuntimeManager, id: &str) -> Result<Vec<RootAuthChange>, String> {
    serde_json::from_value(
        manager
            .personal_model_server()?
            .request_result("modelAuth/list", &json!({"connectionId":id}))?,
    )
    .map_err(|_| "model_auth_response_invalid".into())
}

fn operation(
    manager: &AiRuntimeManager,
    params: &ModelAuthOperation,
) -> Result<RootAuthChange, String> {
    changes(manager, &params.connection_id)?
        .into_iter()
        .find(|op| op.operation_id == params.operation_id)
        .ok_or_else(|| "model_auth_operation_not_found".into())
}

fn step(
    manager: &AiRuntimeManager,
    change: &RootAuthChange,
    kind: ModelRuntimeKind,
    generation: u32,
    state: &str,
    login_id: Option<&str>,
) -> Result<(), String> {
    manager.personal_model_server()?.request_result(
        "modelAuth/step",
        &json!({
            "operationId":change.operation_id,"runtimeKind":kind,"authGeneration":generation,
            "state":state,"loginId":login_id,"reason":null,
        }),
    )?;
    Ok(())
}

fn finish(manager: &AiRuntimeManager, change: &RootAuthChange) -> Result<(), String> {
    manager.personal_model_server()?.request_result(
        "modelAuth/finish",
        &json!({"operationId":change.operation_id}),
    )?;
    manager.refresh_model_catalog()?;
    Ok(())
}

fn recover(manager: &AiRuntimeManager, change: &RootAuthChange) -> Result<RootAuthChange, String> {
    serde_json::from_value(manager.personal_model_server()?.request_result(
        "modelAuth/recover",
        &json!({
            "operationId":change.operation_id,"owner":execution_owner()?,"launchProtocol":1,
        }),
    )?)
    .map_err(|_| "model_auth_response_invalid".into())
}

fn abort_unlaunched(manager: &AiRuntimeManager, change: &RootAuthChange) -> Result<(), String> {
    manager.personal_model_server()?.request_result(
        "modelAuth/abortUnlaunched",
        &json!({
            "operationId":change.operation_id,"owner":execution_owner()?,"launchProtocol":1,
        }),
    )?;
    Ok(())
}

fn processes_exited(
    change: &RootAuthChange,
    kind: ModelRuntimeKind,
    generation: u32,
) -> Result<(), String> {
    for registered in change
        .processes
        .iter()
        .filter(|p| p.runtime_kind == kind && p.auth_generation == generation)
    {
        let current = contexthub_platform_adapter::instance::process_start_fingerprint(
            registered.process.pid,
        )
        .map_err(|_| "model_auth_process_unconfirmed")?;
        if current.as_deref() == Some(registered.process.start_fingerprint.as_str()) {
            return Err("model_auth_process_alive".into());
        }
    }
    Ok(())
}

impl ConnectionRuntime {
    fn auth_service(
        &self,
        manager: &AiRuntimeManager,
        current: &ModelConnection,
        change: &RootAuthChange,
        kind: ModelRuntimeKind,
        generation: u32,
    ) -> Result<Arc<CodexThreadService>, String> {
        let scope = contexthub_protocol::model_runtime_home_scope(
            &current.profile_id,
            &current.connection_id,
            generation,
        );
        let home = scoped_home(&manager.gateway_data_dir()?, &scope, kind);
        let mut services = self
            .services
            .lock()
            .map_err(|_| "model_auth_state_unavailable")?;
        if let Some(old) = services.remove(&home) {
            old.shutdown_checked()?;
        }
        processes_exited(change, kind, generation)?;
        let server = manager.personal_model_server()?;
        let operation_id = change.operation_id.clone();
        let registration = Arc::new(move |pid| {
            let start = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
                .map_err(|_| "model_auth_process_unconfirmed")?
                .ok_or("model_auth_process_unconfirmed")?;
            server.request_result(
                "modelAuth/registerProcess",
                &json!({
                    "operationId":operation_id,"runtimeKind":kind,"authGeneration":generation,
                    "process":{"pid":pid,"startFingerprint":start},
                }),
            )?;
            Ok(())
        });
        let network_guard = network_admission(manager, current, change);
        let service = Arc::new(
            CodexThreadService::new(
                PathBuf::from(crate::codex_harness::bundled_codex_harness_executable()?),
                home.clone(),
            )
            .with_network(current.config.network.clone())
            .with_credential_vault_root(
                manager
                    .gateway_data_dir()?
                    .parent()
                    .ok_or("model_credential_vault_root_required")?
                    .join("credentials"),
            )
            .with_network_guard(network_guard)
            .with_history_server(manager.personal_model_server()?)
            .with_runtime_managed_chatgpt()
            .with_launch_registration(registration),
        );
        services.insert(home, Arc::clone(&service));
        Ok(service)
    }

    fn local_login_service(
        &self,
        manager: &AiRuntimeManager,
        current: &ModelConnection,
        change: &RootAuthChange,
        kind: ModelRuntimeKind,
    ) -> Result<Option<Arc<CodexThreadService>>, String> {
        let scope = contexthub_protocol::model_runtime_home_scope(
            &current.profile_id,
            &current.connection_id,
            change.target_generation,
        );
        let home = scoped_home(&manager.gateway_data_dir()?, &scope, kind);
        let service = self
            .services
            .lock()
            .map_err(|_| "model_auth_state_unavailable")?
            .get(&home)
            .cloned();
        let Some(service) = service else {
            return Ok(None);
        };
        let logins = self
            .logins
            .lock()
            .map_err(|_| "model_auth_state_unavailable")?;
        let owned = logins.get(&home).is_some_and(|local| {
            local.operation_id == change.operation_id && local.epoch == service.connection_epoch()
        });
        if owned && service.has_running_connection()? {
            Ok(Some(service))
        } else {
            Ok(None)
        }
    }

    pub fn auth_read(
        &self,
        manager: &AiRuntimeManager,
        target: &ModelAuthTarget,
    ) -> Result<Value, String> {
        let current = connection(manager, &target.connection_id)?;
        if current.config.kind != ModelConnectionKind::Chatgpt {
            return Err("model_auth_target_invalid".into());
        }
        let change = changes(manager, &target.connection_id)?
            .into_iter()
            .find(|change| {
                pending(change)
                    && change.intent == ModelAuthIntent::ChatgptLogin
                    && change.runtime_kind == Some(target.runtime_kind)
            });
        if let Some(change) = change {
            return self.read_pending(manager, &current, target.runtime_kind, &change);
        }
        if contexthub_protocol::model_active_auth_generation(&current, target.runtime_kind)
            .is_none()
        {
            return Ok(
                json!({"account":null,"requiresOpenaiAuth":true,"runtimeKind":target.runtime_kind,"pendingLoginId":null,"loginStatus":"signed_out"}),
            );
        }
        let account = self.readonly_call(
            manager,
            &current,
            target.runtime_kind,
            "modelAuth/readPrepare",
            |service| service.request("account/read", &json!({"refreshToken":false})),
        )?;
        Ok(
            json!({"account":account["account"],"requiresOpenaiAuth":account["requiresOpenaiAuth"],"runtimeKind":target.runtime_kind,"pendingLoginId":null,"loginStatus":if account["account"].is_null() {"signed_out"} else {"authenticated"}}),
        )
    }

    fn read_pending(
        &self,
        manager: &AiRuntimeManager,
        current: &ModelConnection,
        kind: ModelRuntimeKind,
        change: &RootAuthChange,
    ) -> Result<Value, String> {
        let recorded_login = change
            .steps
            .iter()
            .find(|s| s.runtime_kind == kind && s.auth_generation == change.target_generation)
            .and_then(|s| s.login_id.clone());
        let Some(service) = self.local_login_service(manager, current, change, kind)? else {
            return Ok(
                json!({"account":null,"requiresOpenaiAuth":true,"runtimeKind":kind,"pendingLoginId":recorded_login,"loginStatus":"unconfirmed","operationId":change.operation_id}),
            );
        };
        let account = service.request("account/read", &json!({"refreshToken":false}))?;
        let status = if !account["account"].is_null() {
            service.shutdown_checked()?;
            step(
                manager,
                change,
                kind,
                change.target_generation,
                "confirmed",
                recorded_login.as_deref(),
            )?;
            finish(manager, change)?;
            self.logins
                .lock()
                .map_err(|_| "model_auth_state_unavailable")?
                .remove(service.home());
            "authenticated"
        } else if recorded_login
            .as_deref()
            .and_then(|id| service.login_completion(id))
            == Some(false)
        {
            step(
                manager,
                change,
                kind,
                change.target_generation,
                "failed",
                recorded_login.as_deref(),
            )?;
            "failed"
        } else {
            "pending"
        };
        Ok(
            json!({"account":account["account"],"requiresOpenaiAuth":account["requiresOpenaiAuth"],"runtimeKind":kind,"pendingLoginId":if status=="authenticated" {None} else {recorded_login},"loginStatus":status,"operationId":change.operation_id}),
        )
    }

    pub fn auth_start(
        &self,
        manager: &AiRuntimeManager,
        target: &ModelAuthTarget,
    ) -> Result<Value, String> {
        let current = connection(manager, &target.connection_id)?;
        if current.deleted
            || !current.config.enabled
            || current.config.kind != ModelConnectionKind::Chatgpt
        {
            return Err("model_auth_target_invalid".into());
        }
        super::ensure_runtime_managed_auth_supported(&manager.gateway_data_dir()?)?;
        if changes(manager, &target.connection_id)?
            .iter()
            .any(|change| {
                pending(change)
                    && (change.runtime_kind == Some(target.runtime_kind)
                        || change.intent == ModelAuthIntent::ChatgptLogout)
            })
        {
            return Err("model_auth_login_in_progress".into());
        }
        let operation = uuid::Uuid::new_v4().simple().to_string();
        let change: RootAuthChange = serde_json::from_value(manager.personal_model_server()?.request_result("modelAuth/startPrepare", &json!({
            "operationId":operation,"connectionId":target.connection_id,"expectedConfigRevision":current.config_revision,
            "runtimeKind":target.runtime_kind,"owner":execution_owner()?,"launchProtocol":1,
        }))?).map_err(|_| "model_auth_response_invalid")?;
        let service = match self.auth_service(
            manager,
            &current,
            &change,
            target.runtime_kind,
            change.target_generation,
        ) {
            Ok(service) => service,
            Err(error) => {
                abort_unlaunched(manager, &change)?;
                return Err(error);
            }
        };
        let Ok(response) =
            service.request("account/login/start", &json!({"type":"chatgptDeviceCode"}))
        else {
            if service.launch_attempted() {
                step(
                    manager,
                    &change,
                    target.runtime_kind,
                    change.target_generation,
                    "uncertain",
                    None,
                )?;
            } else {
                abort_unlaunched(manager, &change)?;
            }
            return Err(format!("model_auth_login_unconfirmed:{operation}"));
        };
        let login = response["loginId"]
            .as_str()
            .ok_or("model_auth_login_response_invalid")?;
        let url = response["verificationUrl"]
            .as_str()
            .filter(|url| *url == "https://auth.openai.com/codex/device")
            .ok_or("model_auth_login_response_invalid")?;
        let code = response["userCode"]
            .as_str()
            .filter(|code| !code.is_empty())
            .ok_or("model_auth_login_response_invalid")?;
        step(
            manager,
            &change,
            target.runtime_kind,
            change.target_generation,
            "started",
            Some(login),
        )?;
        self.logins
            .lock()
            .map_err(|_| "model_auth_state_unavailable")?
            .insert(
                service.home().to_path_buf(),
                PendingAuth {
                    login_id: login.into(),
                    epoch: service.connection_epoch(),
                    operation_id: operation.clone(),
                },
            );
        Ok(
            json!({"operationId":operation,"loginId":login,"runtimeKind":target.runtime_kind,"verificationUrl":url,"userCode":code}),
        )
    }

    pub fn auth_cancel(
        &self,
        manager: &AiRuntimeManager,
        params: &ModelAuthCancel,
    ) -> Result<Value, String> {
        let change = changes(manager, &params.connection_id)?
            .into_iter()
            .find(|change| {
                pending(change)
                    && change.intent == ModelAuthIntent::ChatgptLogin
                    && change.runtime_kind == Some(params.runtime_kind)
                    && params
                        .operation_id
                        .as_ref()
                        .is_none_or(|id| *id == change.operation_id)
            })
            .ok_or("model_auth_operation_not_found")?;
        let current = connection(manager, &params.connection_id)?;
        let service = if let Some(service) =
            self.local_login_service(manager, &current, &change, params.runtime_kind)?
        {
            let logins = self
                .logins
                .lock()
                .map_err(|_| "model_auth_state_unavailable")?;
            let local = logins
                .get(service.home())
                .ok_or("model_auth_lifecycle_unconfirmed")?;
            if params
                .login_id
                .as_ref()
                .is_some_and(|id| *id != local.login_id)
            {
                return Err("model_auth_lifecycle_unconfirmed".into());
            }
            let result =
                service.request("account/login/cancel", &json!({"loginId":local.login_id}))?;
            if result["status"] != "canceled" {
                return Err("model_auth_cancel_unconfirmed".into());
            }
            drop(logins);
            service
        } else {
            self.stop_owned_service(
                manager,
                &current,
                params.runtime_kind,
                change.target_generation,
            )?;
            let recovered: RootAuthChange = serde_json::from_value(manager.personal_model_server()?.request_result("modelAuth/recover", &json!({
                "operationId":change.operation_id,"owner":execution_owner()?,"launchProtocol":1,"allowUnstartedLoginCleanup":true,
            }))?).map_err(|_| "model_auth_response_invalid")?;
            self.auth_service(
                manager,
                &current,
                &recovered,
                params.runtime_kind,
                change.target_generation,
            )?
        };
        Self::sign_out_scope(
            manager,
            &change,
            params.runtime_kind,
            change.target_generation,
            &service,
        )?;
        manager.personal_model_server()?.request_result(
            "modelAuth/cancel",
            &json!({"operationId":change.operation_id}),
        )?;
        self.logins
            .lock()
            .map_err(|_| "model_auth_state_unavailable")?
            .remove(service.home());
        manager.refresh_model_catalog()?;
        Ok(
            json!({"status":"canceled","operationId":change.operation_id,"runtimeKind":params.runtime_kind}),
        )
    }

    fn stop_owned_service(
        &self,
        manager: &AiRuntimeManager,
        current: &ModelConnection,
        kind: ModelRuntimeKind,
        generation: u32,
    ) -> Result<(), String> {
        let scope = contexthub_protocol::model_runtime_home_scope(
            &current.profile_id,
            &current.connection_id,
            generation,
        );
        let home = scoped_home(&manager.gateway_data_dir()?, &scope, kind);
        if let Some(service) = self
            .services
            .lock()
            .map_err(|_| "model_auth_state_unavailable")?
            .remove(&home)
        {
            service.shutdown_checked()?;
        }
        Ok(())
    }

    fn sign_out_scope(
        manager: &AiRuntimeManager,
        change: &RootAuthChange,
        kind: ModelRuntimeKind,
        generation: u32,
        service: &CodexThreadService,
    ) -> Result<(), String> {
        if service.request("account/logout", &Value::Null).is_err() {
            step(manager, change, kind, generation, "uncertain", None)?;
            return Err(format!(
                "model_auth_logout_unconfirmed:{}",
                change.operation_id
            ));
        }
        let account = service.request("account/read", &json!({"refreshToken":false}))?;
        if !account["account"].is_null() {
            return Err("model_auth_logout_unconfirmed".into());
        }
        service.shutdown_checked()?;
        step(manager, change, kind, generation, "signed_out", None)
    }

    pub fn auth_logout(
        &self,
        manager: &AiRuntimeManager,
        target: &ModelAuthTarget,
    ) -> Result<Value, String> {
        let current = connection(manager, &target.connection_id)?;
        let operation = uuid::Uuid::new_v4().simple().to_string();
        let change: RootAuthChange = serde_json::from_value(manager.personal_model_server()?.request_result("modelAuth/logoutPrepare", &json!({
            "operationId":operation,"connectionId":current.connection_id,"expectedConfigRevision":current.config_revision,
            "owner":execution_owner()?,"launchProtocol":1,
        }))?).map_err(|_| "model_auth_response_invalid")?;
        self.finish_logout(manager, &current, &change)
    }

    pub fn auth_finish_logout(
        &self,
        manager: &AiRuntimeManager,
        params: &ModelAuthOperation,
    ) -> Result<Value, String> {
        let change = operation(manager, params)?;
        if !pending(&change) || change.intent != ModelAuthIntent::ChatgptLogout {
            return Err("model_auth_operation_invalid".into());
        }
        let current = connection(manager, &params.connection_id)?;
        for scope in &change.steps {
            self.stop_owned_service(manager, &current, scope.runtime_kind, scope.auth_generation)?;
        }
        let change = recover(manager, &change)?;
        self.finish_logout(manager, &current, &change)
    }

    fn finish_logout(
        &self,
        manager: &AiRuntimeManager,
        current: &ModelConnection,
        change: &RootAuthChange,
    ) -> Result<Value, String> {
        for scope in &change.steps {
            if scope.state == ModelAuthStepState::SignedOut {
                continue;
            }
            let service = self.auth_service(
                manager,
                current,
                change,
                scope.runtime_kind,
                scope.auth_generation,
            )?;
            Self::sign_out_scope(
                manager,
                change,
                scope.runtime_kind,
                scope.auth_generation,
                &service,
            )?;
            self.logins
                .lock()
                .map_err(|_| "model_auth_state_unavailable")?
                .remove(service.home());
        }
        finish(manager, change)?;
        Ok(
            json!({"status":"signed_out","operationId":change.operation_id,"affectsAllRuntimeKinds":true}),
        )
    }

    pub fn auth_resume(
        manager: &AiRuntimeManager,
        params: &ModelAuthOperation,
    ) -> Result<Value, String> {
        let change = operation(manager, params)?;
        Ok(json!({"operation":change,"status":"recorded","replayed":false}))
    }
}

fn network_admission(
    manager: &AiRuntimeManager,
    current: &ModelConnection,
    change: &RootAuthChange,
) -> super::native_proxy::Admission {
    let manager = manager.clone();
    let current = current.clone();
    let change = change.clone();
    Arc::new(move || {
        if connection(&manager, &current.connection_id)? != current {
            return Err("model_connection_changed".into());
        }
        let fresh = operation(
            &manager,
            &ModelAuthOperation {
                connection_id: current.connection_id.clone(),
                operation_id: change.operation_id.clone(),
            },
        )?;
        if !pending(&fresh) || fresh.owner != change.owner {
            return Err("model_auth_operation_changed".into());
        }
        Ok(Box::new(()) as Box<dyn Send>)
    })
}

impl ConnectionRuntime {
    fn readonly_call<T>(
        &self,
        manager: &AiRuntimeManager,
        current: &ModelConnection,
        kind: ModelRuntimeKind,
        method: &str,
        read: impl FnOnce(&CodexThreadService) -> Result<T, String>,
    ) -> Result<T, String> {
        let operation = uuid::Uuid::new_v4().simple().to_string();
        let change: RootAuthChange = serde_json::from_value(manager.personal_model_server()?.request_result(method, &json!({
            "operationId":operation,"connectionId":current.connection_id,"expectedConfigRevision":current.config_revision,
            "runtimeKind":kind,"owner":execution_owner()?,"launchProtocol":1,
        }))?).map_err(|_| "model_auth_response_invalid")?;
        let service =
            match self.auth_service(manager, current, &change, kind, change.target_generation) {
                Ok(service) => service,
                Err(error) => {
                    abort_unlaunched(manager, &change)?;
                    return Err(error);
                }
            };
        let result = read(&service);
        service.shutdown_checked()?;
        match result {
            Ok(value) => {
                step(
                    manager,
                    &change,
                    kind,
                    change.target_generation,
                    "confirmed",
                    None,
                )?;
                finish(manager, &change)?;
                Ok(value)
            }
            Err(error) => {
                if !service.launch_attempted() {
                    abort_unlaunched(manager, &change)?;
                    return Err(error);
                }
                step(
                    manager,
                    &change,
                    kind,
                    change.target_generation,
                    "failed",
                    None,
                )?;
                manager
                    .personal_model_server()?
                    .request_result("modelAuth/cancel", &json!({"operationId":operation}))?;
                Err(error)
            }
        }
    }

    pub(super) fn read_catalog(
        &self,
        manager: &AiRuntimeManager,
        current: &ModelConnection,
    ) -> Result<Vec<Value>, String> {
        self.readonly_call(
            manager,
            current,
            ModelRuntimeKind::Workbench,
            "modelAuth/catalogPrepare",
            |service| {
                let mut all = Vec::new();
                let mut cursor = Value::Null;
                let mut seen = std::collections::HashSet::new();
                loop {
                    let page = service.request(
                        "model/list",
                        &json!({"cursor":cursor,"limit":100,"includeHidden":false}),
                    )?;
                    all.extend(
                        page["data"]
                            .as_array()
                            .ok_or("model_catalog_response_invalid")?
                            .clone(),
                    );
                    cursor = page
                        .get("nextCursor")
                        .cloned()
                        .ok_or("model_catalog_response_invalid")?;
                    if cursor.is_null() {
                        break;
                    }
                    if !cursor.is_string() || !seen.insert(cursor.to_string()) {
                        return Err("model_catalog_cursor_invalid".into());
                    }
                }
                Ok(all)
            },
        )
    }

    pub fn auth_cancel_readonly(
        &self,
        manager: &AiRuntimeManager,
        params: &ModelAuthOperation,
    ) -> Result<Value, String> {
        let change = operation(manager, params)?;
        if !pending(&change)
            || !matches!(
                change.intent,
                ModelAuthIntent::ChatgptStatus | ModelAuthIntent::ModelCatalog
            )
        {
            return Err("model_auth_operation_invalid".into());
        }
        let current = connection(manager, &params.connection_id)?;
        for scope in &change.steps {
            self.stop_owned_service(manager, &current, scope.runtime_kind, scope.auth_generation)?;
        }
        let recovered: RootAuthChange = serde_json::from_value(manager.personal_model_server()?.request_result("modelAuth/recover", &json!({
            "operationId":change.operation_id,"owner":execution_owner()?,"launchProtocol":1,"allowUnstartedReadonlyCleanup":true,
        }))?).map_err(|_| "model_auth_response_invalid")?;
        if recovered.state == ModelAuthChangeState::Cancelled {
            return Ok(
                json!({"status":"canceled","operationId":change.operation_id,"replayed":false}),
            );
        }
        for scope in &recovered.steps {
            if scope.state != ModelAuthStepState::Confirmed
                && scope.state != ModelAuthStepState::Failed
            {
                processes_exited(&recovered, scope.runtime_kind, scope.auth_generation)?;
                step(
                    manager,
                    &recovered,
                    scope.runtime_kind,
                    scope.auth_generation,
                    "failed",
                    None,
                )?;
            }
        }
        manager.personal_model_server()?.request_result(
            "modelAuth/cancel",
            &json!({"operationId":change.operation_id}),
        )?;
        Ok(json!({"status":"canceled","operationId":change.operation_id,"replayed":false}))
    }
}

#[cfg(test)]
#[path = "auth_file_tests.rs"]
mod auth_file_tests;
