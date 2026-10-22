use super::{
    ExecutionBundle, ExecutionScope, ModelRuntimeKind, OFFICIAL_RUNTIME_VERSION, connection,
    connection_client, connection_credential, endpoint,
};
use crate::ai_runtime::AiRuntimeManager;
use crate::codex_harness::{
    ApplicationMcpScope, CodexHarnessModel, CodexHarnessRequest, HarnessProcessState,
    ModelToolCapabilities, spawn_codex_harness,
};
use contexthub_protocol::{
    ModelAuthMode, ModelConnection, ModelConnectionKind, ModelExecutionLease, ModelExecutionOwner,
    ModelValidationExecutionParams, ModelValidationLayer, ModelValidationOutcome,
    ModelValidationPrepareParams, ModelValidationRecord, ModelValidationReportParams,
};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_mins(2);
const EXIT_TIMEOUT: Duration = Duration::from_secs(15);

struct Evidence {
    reason: &'static str,
    runtime_version: Option<String>,
}

impl Evidence {
    fn local(reason: &'static str) -> Self {
        Self {
            reason,
            runtime_version: None,
        }
    }
}

#[derive(Default)]
struct StreamObservation {
    updates: usize,
    last: String,
}

impl AiRuntimeManager {
    /// # Errors
    /// 验证受理、作用域核验、进程退出确认或结果保存失败时返回错误。
    pub fn personal_model_validate(
        &self,
        params: ModelValidationPrepareParams,
    ) -> Result<ModelValidationRecord, String> {
        let _operation = self.begin_capability_operation()?;
        let server = self.personal_model_server()?;
        let record: ModelValidationRecord = serde_json::from_value(server.request_result(
            "modelValidation/prepare",
            &serde_json::to_value(params).map_err(|_| "model_validation_request_invalid")?,
        )?)
        .map_err(|_| "model_validation_response_invalid")?;
        let current = connection(self, &record.connection_id)?;
        ensure_current(self, &current, &record)?;
        let result = validate_layer(self, &current, &record);
        if result
            .as_ref()
            .is_err_and(|error| error == "model_validation_cleanup_unconfirmed")
        {
            return Err("model_validation_cleanup_unconfirmed".into());
        }
        ensure_current(self, &current, &record)?;
        let (outcome, reason, runtime_version) = match result {
            Ok(evidence) => (
                ModelValidationOutcome::Passed,
                evidence.reason.to_owned(),
                evidence.runtime_version,
            ),
            Err(error) => (
                ModelValidationOutcome::Failed,
                failure_reason(&error).into(),
                None,
            ),
        };
        let report = ModelValidationReportParams {
            validation_id: record.validation_id,
            outcome,
            reason: Some(reason),
            runtime_version,
            service_version: None,
        };
        let reported = serde_json::from_value(server.request_result(
            "modelValidation/report",
            &serde_json::to_value(report).map_err(|_| "model_validation_request_invalid")?,
        )?)
        .map_err(|_| "model_validation_response_invalid")?;
        self.refresh_model_catalog()?;
        Ok(reported)
    }
}

fn ensure_current(
    manager: &AiRuntimeManager,
    expected: &ModelConnection,
    record: &ModelValidationRecord,
) -> Result<(), String> {
    let current = connection(manager, &record.connection_id)?;
    let generation = contexthub_protocol::model_validation_auth_generation(
        &current,
        record.runtime_kind,
        &record.layer,
    );
    if &current != expected
        || current.deleted
        || !current.config.enabled
        || current.config_revision != record.config_revision
        || generation != Some(record.auth_generation)
    {
        return Err("model_validation_stale".into());
    }
    Ok(())
}

fn validate_layer(
    manager: &AiRuntimeManager,
    current: &ModelConnection,
    record: &ModelValidationRecord,
) -> Result<Evidence, String> {
    match record.layer {
        ModelValidationLayer::Configuration => {
            validate_configuration(current, &manager.credential_vault_root()?)?;
            Ok(Evidence::local("configuration_saved_revision_verified"))
        }
        ModelValidationLayer::Network => network_evidence(models_status(
            current,
            "",
            false,
            &manager.credential_vault_root()?,
        )?),
        ModelValidationLayer::Authentication => {
            if current.config.kind == ModelConnectionKind::Chatgpt {
                validate_chatgpt_authentication(manager, current, record)
            } else {
                if current.config.auth_mode == ModelAuthMode::ApiKey
                    || current.config.network.service_auth
                        != contexthub_protocol::ModelNetworkAuthentication::None
                    || !current.config.network.header_names.is_empty()
                {
                    require_authentication_challenge(models_status(
                        current,
                        "",
                        false,
                        &manager.credential_vault_root()?,
                    )?)?;
                }
                let token = connection_credential(manager, current)?;
                let status =
                    models_status(current, &token, true, &manager.credential_vault_root()?)?;
                authentication_evidence(
                    status,
                    &current.config.auth_mode,
                    contexthub_protocol::model_network_requires_credentials(
                        &current.config.network,
                    ) && (current.config.network.service_auth
                        != contexthub_protocol::ModelNetworkAuthentication::None
                        || !current.config.network.header_names.is_empty()),
                )
            }
        }
        ModelValidationLayer::Catalog => {
            let models = manager
                .personal_model_catalog_refresh(&current.connection_id)
                .map_err(|_| "model_validation_catalog_failed")?;
            if record
                .model_id
                .as_ref()
                .is_some_and(|id| !models.iter().any(|m| &m.model_id == id))
            {
                return Err("model_validation_model_not_in_catalog".into());
            }
            Ok(Evidence::local(if models.is_empty() {
                "catalog_read_empty"
            } else {
                "catalog_read_completed"
            }))
        }
        ModelValidationLayer::Protocol
        | ModelValidationLayer::Inference
        | ModelValidationLayer::Streaming
        | ModelValidationLayer::Tools => run_probe(manager, current, record),
    }
}

fn validate_configuration(current: &ModelConnection, vault_root: &Path) -> Result<(), String> {
    let compatible = match current.config.kind {
        ModelConnectionKind::OpenaiKey => current.config.auth_mode == ModelAuthMode::ApiKey,
        ModelConnectionKind::Chatgpt => {
            current.config.auth_mode == ModelAuthMode::RuntimeManagedChatgpt
        }
        ModelConnectionKind::CustomResponses | ModelConnectionKind::Local => {
            matches!(
                current.config.auth_mode,
                ModelAuthMode::None | ModelAuthMode::ApiKey
            )
        }
    };
    if !compatible
        || (current.config.kind == ModelConnectionKind::Local
            && current.config.local_runtime.is_none())
    {
        return Err("model_validation_configuration_invalid".into());
    }
    if current.config.kind != ModelConnectionKind::Chatgpt {
        endpoint(current, "models")?;
    }
    connection_client(&current.config.network, vault_root)?;
    if let Some(reference) = &current.secret_ref {
        let account = contexthub_protocol::model_runtime_home_scope(
            &current.profile_id,
            &current.connection_id,
            current.auth_generation,
        );
        if current.config.auth_mode != ModelAuthMode::ApiKey
            || reference.service != contexthub_protocol::MODEL_CREDENTIAL_SERVICE
            || reference.account != account
        {
            return Err("model_credential_reference_invalid".into());
        }
    }
    Ok(())
}

fn network_evidence(status: reqwest::StatusCode) -> Result<Evidence, String> {
    if status.is_success() || matches!(status.as_u16(), 401 | 403 | 404 | 405) {
        Ok(Evidence::local("network_http_endpoint_reached"))
    } else {
        Err("model_validation_network_http_rejected".into())
    }
}

fn require_authentication_challenge(status: reqwest::StatusCode) -> Result<(), String> {
    if matches!(status.as_u16(), 401 | 403) {
        Ok(())
    } else {
        Err("model_validation_authentication_not_established".into())
    }
}

fn authentication_evidence(
    status: reqwest::StatusCode,
    mode: &ModelAuthMode,
    service_authentication: bool,
) -> Result<Evidence, String> {
    if !status.is_success() {
        return Err("model_validation_authentication_failed".into());
    }
    Ok(Evidence::local(
        if *mode == ModelAuthMode::None && !service_authentication {
            "authentication_not_required_endpoint_accepted"
        } else {
            "authentication_models_endpoint_accepted"
        },
    ))
}

fn validate_chatgpt_authentication(
    manager: &AiRuntimeManager,
    current: &ModelConnection,
    record: &ModelValidationRecord,
) -> Result<Evidence, String> {
    let scope = prepare_probe_scope(manager, record)?;
    let service = crate::codex_threads::CodexThreadService::new(
        crate::codex_harness::bundled_codex_harness_executable()?.into(),
        scope.home(record.runtime_kind),
    )
    .with_network(current.config.network.clone())
    .with_execution_scope(Arc::clone(&scope));
    let result = service
        .request("account/read", &json!({"refreshToken":true}))
        .and_then(|account| {
            if account["account"].is_null() {
                return Err("model_auth_required".into());
            }
            service.request(
                "account/rateLimits/read",
                &json!({"excludeResetCreditDetails":true,"supportsLunaReserve":false}),
            )
        });
    service
        .shutdown_checked()
        .map_err(|_| "model_validation_cleanup_unconfirmed")?;
    drop(service);
    scope
        .finish_release()
        .map_err(|_| "model_validation_cleanup_unconfirmed")?;
    let limits = result.map_err(|_| "model_validation_authentication_failed")?;
    validate_rate_limits(&limits)?;
    Ok(Evidence::local(
        "authentication_runtime_remote_limits_accepted",
    ))
}

fn validate_rate_limits(value: &Value) -> Result<(), String> {
    let valid_snapshot = |snapshot: &Value| {
        snapshot.as_object().is_some_and(|snapshot| {
            ["primary", "secondary", "credits", "limitId", "planType"]
                .iter()
                .any(|key| snapshot.contains_key(*key))
        })
    };
    if !valid_snapshot(&value["rateLimits"]) {
        return Err("model_validation_authentication_response_invalid".into());
    }
    if !value["rateLimitsByLimitId"].is_null()
        && !value["rateLimitsByLimitId"]
            .as_object()
            .is_some_and(|limits| !limits.is_empty() && limits.values().all(valid_snapshot))
    {
        return Err("model_validation_authentication_response_invalid".into());
    }
    Ok(())
}

fn models_status(
    current: &ModelConnection,
    credential: &str,
    authenticate: bool,
    vault_root: &Path,
) -> Result<reqwest::StatusCode, String> {
    let url = if current.config.kind == ModelConnectionKind::Chatgpt {
        reqwest::Url::parse("https://chatgpt.com/backend-api/codex/models")
            .map_err(|_| "model_endpoint_invalid")?
    } else {
        endpoint(current, "models")?
    };
    let route = super::resolve_network(&current.config.network, vault_root)?;
    let client = super::connection_client_with_route(&route)?;
    crate::async_support::block_on(async {
        let mut request = client.get(url).timeout(Duration::from_secs(30));
        if !credential.is_empty() {
            request = request.bearer_auth(credential);
        }
        if authenticate {
            request = route.decorate(request, credential)?;
        }
        let response = request.send().await.map_err(|error| {
            super::network_failure_reason(&error, "model_validation_network_failed")
        })?;
        if response.status() == reqwest::StatusCode::PROXY_AUTHENTICATION_REQUIRED {
            return Err("model_proxy_authentication_rejected".to_owned());
        }
        Ok(response.status())
    })
}

fn prepare_probe_scope(
    manager: &AiRuntimeManager,
    record: &ModelValidationRecord,
) -> Result<Arc<ExecutionScope>, String> {
    let pid = std::process::id();
    let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
        .map_err(|_| "model_execution_owner_unavailable")?
        .ok_or("model_execution_owner_unavailable")?;
    let params = ModelValidationExecutionParams {
        validation_id: record.validation_id.clone(),
        owner: ModelExecutionOwner {
            instance_id: crate::persistent_threads::owner_instance_id(),
            pid,
            start_fingerprint: fingerprint,
        },
        launch_protocol: contexthub_protocol::MODEL_LAUNCH_PROTOCOL,
    };
    let server = manager.personal_model_server()?;
    let lease: ModelExecutionLease = serde_json::from_value(server.request_result(
        "modelValidation/prepareExecution",
        &serde_json::to_value(params).map_err(|_| "model_validation_request_invalid")?,
    )?)
    .map_err(|_| "model_execution_response_invalid")?;
    let scope = ExecutionScope::from_lease(manager, server, manager.gateway_data_dir()?, lease)?;
    validate_probe_lease(&scope.lease, record)?;
    Ok(scope)
}

fn validate_probe_lease(
    lease: &ModelExecutionLease,
    record: &ModelValidationRecord,
) -> Result<(), String> {
    let snapshot = &lease.snapshot;
    if snapshot.validation_id.as_deref() != Some(&record.validation_id)
        || snapshot.runtime_kind != record.runtime_kind
        || snapshot.connection_id != record.connection_id
        || snapshot.config_revision != record.config_revision
        || snapshot.auth_generation != record.auth_generation
        || Some(&snapshot.model_id) != record.model_id.as_ref()
        || snapshot.runtime_home_scope
            != contexthub_protocol::model_runtime_home_scope(
                &snapshot.profile_id,
                &snapshot.connection_id,
                snapshot.auth_generation,
            )
    {
        return Err("model_validation_snapshot_mismatch".into());
    }
    Ok(())
}

fn run_probe(
    manager: &AiRuntimeManager,
    current: &ModelConnection,
    record: &ModelValidationRecord,
) -> Result<Evidence, String> {
    let scope = prepare_probe_scope(manager, record)?;
    let result = run_probe_scoped(current, record, &scope);
    if result
        .as_ref()
        .is_err_and(|error| error == "model_validation_cleanup_unconfirmed")
    {
        return result;
    }
    scope
        .finish_release()
        .map_err(|_| "model_validation_cleanup_unconfirmed")?;
    result
}

fn run_probe_scoped(
    current: &ModelConnection,
    record: &ModelValidationRecord,
    scope: &Arc<ExecutionScope>,
) -> Result<Evidence, String> {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let workspace = scope
        .data_dir
        .join("model-validation")
        .join(&record.validation_id);
    contexthub_platform_adapter::security::ensure_owner_writable_runtime_tree(&workspace)
        .map_err(|_| "model_validation_workspace_failed")?;
    let event_log = workspace.join("protocol.jsonl");
    let stream = Arc::new(Mutex::new(StreamObservation::default()));
    let observed = Arc::clone(&stream);
    let progress = Arc::new(move |event: crate::codex_harness::CodexHarnessEvent| {
        if event.event_type == "item.agentMessage.delta"
            && let Some(text) = event.streamed_output
            && let Ok(mut observation) = observed.lock()
        {
            observation.updates += 1;
            observation.last = text;
        }
    });
    let snapshot = &scope.lease.snapshot;
    let bundle = ExecutionBundle {
        foreground: Arc::clone(scope),
        review: None,
        memory: None,
    };
    let upstream = if matches!(
        current.config.kind,
        ModelConnectionKind::OpenaiKey | ModelConnectionKind::Chatgpt
    ) {
        "https://api.openai.com/v1"
    } else {
        snapshot
            .base_url
            .as_deref()
            .ok_or("model_endpoint_required")?
    };
    let proxy = crate::model_session_proxy::start_model_session_proxy(
        bundle.clone(),
        upstream,
        &record.validation_id,
    )?;
    let process = Arc::new(HarnessProcessState::default());
    let (sender, receiver) = mpsc::sync_channel(1);
    let request = probe_request(scope, record, &nonce, &workspace, &proxy)?;
    let spawned = spawn_codex_harness(
        request,
        progress,
        Arc::new(|_| {}),
        Arc::new(|_| Err("model_validation_tool_denied".into())),
        move |result| {
            let _ = sender.send(result);
        },
        (proxy, Some(Arc::clone(&process))),
        None,
    )?;
    let result = if let Ok(result) = receiver.recv_timeout(PROBE_TIMEOUT) {
        result
    } else {
        let _ = spawned.control.close_input();
        Err("model_validation_timeout".into())
    };
    if !wait_for_exit(&process, EXIT_TIMEOUT) {
        let _ = spawned.control.close_input();
        if !wait_for_exit(&process, EXIT_TIMEOUT) {
            return Err("model_validation_cleanup_unconfirmed".into());
        }
    }
    let reply = result.map_err(|error| {
        match error.as_str() {
            "model_validation_timeout" => "model_validation_timeout",
            "codex_harness_empty_response" => "model_validation_empty_response",
            _ => "model_validation_runtime_failed",
        }
        .to_owned()
    })?;
    if !reply.file_changes.is_empty() {
        return Err("model_validation_side_effect_observed".into());
    }
    let wire = protocol_evidence(&event_log, &reply.thread_id)?;
    let observation = stream
        .lock()
        .map_err(|_| "model_validation_observation_failed")?;
    assess_probe(&record.layer, &nonce, &reply.output, &observation, &wire)?;
    Ok(Evidence {
        reason: match record.layer {
            ModelValidationLayer::Protocol => "protocol_official_adapter_turn_completed",
            ModelValidationLayer::Inference => "inference_nonce_returned",
            ModelValidationLayer::Streaming => "streaming_delta_and_completion_observed",
            ModelValidationLayer::Tools => "tools_safe_dynamic_result_accepted",
            _ => return Err("model_validation_layer_invalid".into()),
        },
        runtime_version: Some(wire.runtime_version),
    })
}

fn probe_request(
    scope: &Arc<ExecutionScope>,
    record: &ModelValidationRecord,
    nonce: &str,
    workspace: &Path,
    proxy: &crate::model_session_proxy::ModelSessionProxy,
) -> Result<CodexHarnessRequest, String> {
    let snapshot = &scope.lease.snapshot;
    let bundle = ExecutionBundle {
        foreground: Arc::clone(scope),
        review: None,
        memory: None,
    };
    let task = probe_task(record, nonce);
    let prompt = if let Some(task) = &task {
        format!(
            "只调用 cilai_submit_application_result 一次。taskId={}，inputDigest={}，result={{\"nonce\":\"{}\"}}。不要执行其他操作。",
            task.task_id, task.input_digest, nonce
        )
    } else {
        format!("只回复下面这串校验文本，不添加其他内容，不使用任何工具：{nonce}")
    };
    Ok(CodexHarnessRequest {
        turn_admission: None,
        validation_probe: true,
        execution: Some(bundle), memory_model: None,
        conversation_kind: match record.runtime_kind {
            ModelRuntimeKind::Workbench => crate::conversation_kernel::ConversationKind::Workbench,
            ModelRuntimeKind::Application => crate::conversation_kernel::ConversationKind::ApplicationTask,
        },
        planning: false,
        executable: crate::codex_harness::bundled_codex_harness_executable()?,
        harness_home: scope.home(record.runtime_kind), tool_snapshot_scope_id: record.validation_id.clone(),
        conversation_id: None, workspace_root: workspace.to_path_buf(), workspace_roots: vec![workspace.to_path_buf()],
        model: snapshot.model_id.clone(),
        model_provider: if snapshot.kind == ModelConnectionKind::Chatgpt { super::CHATGPT_PROVIDER } else { super::LOOPBACK_PROVIDER }.into(),
        provider_label: "个人连接验证".into(), base_url: proxy.base_url.clone(),
        api_key_env: super::LOOPBACK_KEY_ENV.into(), api_key: proxy.local_token.clone(),
        context_window: snapshot.context_window_tokens, reasoning_effort: None,
        model_catalog: vec![CodexHarnessModel {
            model_id: snapshot.model_id.clone(), display_name: snapshot.model_id.clone(),
            context_window: snapshot.context_window_tokens,
            supported_reasoning_efforts: snapshot.supported_reasoning_efforts.clone(),
            default_reasoning_effort: snapshot.default_reasoning_effort.clone(),
            tool_capabilities: Some(ModelToolCapabilities {
                version:1,profile:"responses-functions".into(),function_calling:task.is_some(),
                client_tool_search:false,custom_tools:false,web_search_mode:"disabled".into(),
            }),
            input_modalities:vec!["text".into()],
        }],
        developer_instructions: "这是用户明确发起的连接验证，仅处理当前随机校验文本；不读取文件，不执行命令，不访问浏览器，不产生业务修改。".into(),
        classification:None, report_result:None, application_result:task,
        user_input:prompt,turn_context:String::new(),conversation_text:String::new(),
        attachments:Vec::new(),skills:Vec::new(),user_mcp:None,personal_tools:None,application_action:None,
        application_write:None,
        application_mcp: (record.runtime_kind == ModelRuntimeKind::Application).then(|| ApplicationMcpScope {
            application_id:"cilai.personal.validation".into(),application_name:"连接验证".into(),
            application_ids:vec!["cilai.personal.validation".into()],restrict_to_applications:true,
            browser_origins:Vec::new(),report_review:false,tool_names:Vec::new(),read_tools:Vec::new(),write_tools:Vec::new(),
        }),
        resume_thread_id:None,persistent_thread:None,application_adapter:None,
        approval_mode:"request".into(),approval_review_model:None,
        memory_preferences:crate::memory::MemoryPreferences::disabled(),sandbox_mode:"read-only".into(),
        event_log_path:workspace.join("protocol.jsonl"),stderr_path:workspace.join("stderr.log"),
    })
}

fn probe_task(
    record: &ModelValidationRecord,
    nonce: &str,
) -> Option<crate::app_bridge::ApplicationModelTask> {
    (record.layer == ModelValidationLayer::Tools).then(|| crate::app_bridge::ApplicationModelTask {
        purpose: "personal_connection_validation".into(),
        task_id: uuid::Uuid::new_v4().to_string(),
        input_digest: crate::persistent_threads::digest(
            &json!({"validation":record.validation_id,"nonce":nonce}),
        ),
        result_schema: Some(json!({"type":"object","additionalProperties":false,
            "properties":{"nonce":{"type":"string","const":nonce}},"required":["nonce"]})),
    })
}

fn wait_for_exit(process: &HarnessProcessState, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while !process.exited.load(Ordering::Acquire) {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

struct ProtocolEvidence {
    runtime_version: String,
    completed: bool,
    result_tool_called: bool,
}

fn protocol_evidence(path: &Path, thread_id: &str) -> Result<ProtocolEvidence, String> {
    let raw = fs::read_to_string(path).map_err(|_| "model_validation_protocol_log_unavailable")?;
    if raw.len() > 8 * 1024 * 1024 {
        return Err("model_validation_protocol_log_too_large".into());
    }
    let mut evidence = ProtocolEvidence {
        runtime_version: String::new(),
        completed: false,
        result_tool_called: false,
    };
    for line in raw.lines() {
        let message: Value =
            serde_json::from_str(line).map_err(|_| "model_validation_protocol_log_invalid")?;
        if message["threadId"] != thread_id {
            continue;
        }
        if let Some(version) = message["runtimeVersion"].as_str() {
            evidence.runtime_version = version.into();
        }
        if message["method"] == "turn/completed" && message["completedStatus"] == "completed" {
            evidence.completed = true;
        }
        if message["applicationResultCall"] == true {
            evidence.result_tool_called = true;
        }
    }
    if evidence.runtime_version != OFFICIAL_RUNTIME_VERSION {
        return Err("model_validation_runtime_version_mismatch".into());
    }
    Ok(evidence)
}

fn assess_probe(
    layer: &ModelValidationLayer,
    nonce: &str,
    output: &str,
    stream: &StreamObservation,
    protocol: &ProtocolEvidence,
) -> Result<(), String> {
    if !protocol.completed {
        return Err("model_validation_completion_missing".into());
    }
    match layer {
        ModelValidationLayer::Protocol => Ok(()),
        ModelValidationLayer::Inference if output.trim() == nonce => Ok(()),
        ModelValidationLayer::Streaming
            if output.trim() == nonce && stream.updates > 0 && stream.last.trim() == nonce =>
        {
            Ok(())
        }
        ModelValidationLayer::Tools if protocol.result_tool_called => {
            let value: Value =
                serde_json::from_str(output).map_err(|_| "model_validation_tool_result_invalid")?;
            if value.as_object().is_some_and(|value| value.len() == 1) && value["nonce"] == nonce {
                Ok(())
            } else {
                Err("model_validation_tool_result_invalid".into())
            }
        }
        ModelValidationLayer::Inference => Err("model_validation_inference_nonce_mismatch".into()),
        ModelValidationLayer::Streaming => Err("model_validation_streaming_delta_missing".into()),
        ModelValidationLayer::Tools => Err("model_validation_tool_call_missing".into()),
        _ => Err("model_validation_layer_invalid".into()),
    }
}

fn failure_reason(error: &str) -> &'static str {
    const CODES: &[&str] = &[
        "model_validation_configuration_invalid",
        "model_descriptor_stale",
        "model_descriptor_not_found",
        "model_validation_model_required",
        "model_credential_reference_invalid",
        "model_auth_required",
        "model_credential_access_failed",
        "model_credential_invalid",
        "model_validation_authentication_failed",
        "model_validation_authentication_not_established",
        "model_validation_authentication_response_invalid",
        "model_validation_network_failed",
        "model_validation_network_http_rejected",
        "model_validation_catalog_failed",
        "model_validation_model_not_in_catalog",
        "model_validation_snapshot_mismatch",
        "model_validation_workspace_failed",
        "model_validation_timeout",
        "model_validation_runtime_failed",
        "model_validation_empty_response",
        "model_validation_side_effect_observed",
        "model_validation_protocol_log_unavailable",
        "model_validation_protocol_log_too_large",
        "model_validation_protocol_log_invalid",
        "model_validation_runtime_version_mismatch",
        "model_validation_completion_missing",
        "model_validation_tool_result_invalid",
        "model_validation_inference_nonce_mismatch",
        "model_validation_streaming_delta_missing",
        "model_validation_tool_call_missing",
        "model_proxy_authentication_rejected",
        "model_proxy_scheme_not_supported",
        "model_proxy_credentials_required",
        "model_proxy_url_credentials_forbidden",
        "model_network_credential_required",
        "model_network_credential_access_failed",
        "model_network_credential_invalid",
        "model_network_credentials_changed",
        "model_network_credential_scope_invalid",
        "model_service_auth_conflict",
        "model_network_header_invalid",
        "model_native_proxy_admission_required",
        "model_native_proxy_exit_unconfirmed",
        "model_system_proxy_read_failed",
        "model_system_proxy_snapshot_invalid",
        "model_system_proxy_platform_not_supported",
        "model_system_proxy_pac_not_supported",
        "model_system_proxy_auto_discovery_not_supported",
        "model_system_proxy_scoped_not_supported",
        "model_system_proxy_bypass_not_supported",
        "model_system_proxy_socks_not_supported",
        "model_proxy_url_required",
        "model_proxy_url_invalid",
        "model_endpoint_required",
        "model_endpoint_invalid",
        "model_validation_layer_invalid",
    ];
    CODES
        .iter()
        .copied()
        .find(|code| *code == error)
        .unwrap_or("model_validation_failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completed(tool: bool) -> ProtocolEvidence {
        ProtocolEvidence {
            runtime_version: OFFICIAL_RUNTIME_VERSION.into(),
            completed: true,
            result_tool_called: tool,
        }
    }

    #[test]
    fn text_inference_does_not_establish_streaming_or_tools() {
        let stream = StreamObservation::default();
        assert!(
            assess_probe(
                &ModelValidationLayer::Inference,
                "nonce",
                "nonce",
                &stream,
                &completed(false)
            )
            .is_ok()
        );
        assert!(
            assess_probe(
                &ModelValidationLayer::Streaming,
                "nonce",
                "nonce",
                &stream,
                &completed(false)
            )
            .is_err()
        );
        assert!(
            assess_probe(
                &ModelValidationLayer::Tools,
                "nonce",
                r#"{"nonce":"nonce"}"#,
                &stream,
                &completed(false)
            )
            .is_err()
        );
    }

    #[test]
    fn streaming_requires_observed_delta_and_matching_output() {
        let stream = StreamObservation {
            updates: 1,
            last: "nonce".into(),
        };
        assert!(
            assess_probe(
                &ModelValidationLayer::Streaming,
                "nonce",
                "nonce",
                &stream,
                &completed(false)
            )
            .is_ok()
        );
        assert!(
            assess_probe(
                &ModelValidationLayer::Streaming,
                "nonce",
                "other",
                &stream,
                &completed(false)
            )
            .is_err()
        );
        assert!(
            assess_probe(
                &ModelValidationLayer::Inference,
                "nonce",
                "description nonce",
                &stream,
                &completed(false)
            )
            .is_err()
        );
    }

    #[test]
    fn tools_require_native_call_and_exact_nonce_result() {
        let stream = StreamObservation::default();
        assert!(
            assess_probe(
                &ModelValidationLayer::Tools,
                "nonce",
                r#"{"nonce":"nonce"}"#,
                &stream,
                &completed(true)
            )
            .is_ok()
        );
        assert!(
            assess_probe(
                &ModelValidationLayer::Tools,
                "nonce",
                r#"{"nonce":"other"}"#,
                &stream,
                &completed(true)
            )
            .is_err()
        );
        assert!(
            assess_probe(
                &ModelValidationLayer::Tools,
                "nonce",
                r#"{"nonce":"nonce","extra":1}"#,
                &stream,
                &completed(true)
            )
            .is_err()
        );
    }

    #[test]
    fn response_without_completed_turn_never_passes() {
        let protocol = ProtocolEvidence {
            runtime_version: OFFICIAL_RUNTIME_VERSION.into(),
            completed: false,
            result_tool_called: true,
        };
        assert!(
            assess_probe(
                &ModelValidationLayer::Protocol,
                "nonce",
                "nonce",
                &StreamObservation::default(),
                &protocol
            )
            .is_err()
        );
    }

    #[test]
    fn provider_error_text_is_not_persisted_in_validation_reason() {
        assert_eq!(
            failure_reason("upstream echoed a credential"),
            "model_validation_failed"
        );
        assert_eq!(failure_reason("model_auth_required"), "model_auth_required");
    }

    #[test]
    fn model_selection_gate_errors_are_exact_and_never_preserve_attached_text() {
        for code in [
            "model_descriptor_stale",
            "model_descriptor_not_found",
            "model_validation_model_required",
        ] {
            assert_eq!(failure_reason(code), code);
            for untrusted in [
                format!("{code}: private-fixture-secret"),
                format!("private-fixture-secret {code}"),
                format!("{code}\nAuthorization: Bearer private-fixture-secret"),
            ] {
                assert_eq!(failure_reason(&untrusted), "model_validation_failed");
            }
        }
    }

    fn validation_record() -> ModelValidationRecord {
        ModelValidationRecord {
            runtime_kind: ModelRuntimeKind::Application,
            validation_id: "validation-fixture".into(),
            connection_id: "connection-fixture".into(),
            model_id: Some("model-a".into()),
            layer: ModelValidationLayer::Tools,
            config_revision: 3,
            auth_generation: 7,
            outcome: None,
            completion_revision: None,
            reason: None,
            runtime_version: None,
            service_version: None,
            updated_at: 1,
        }
    }

    fn validation_lease(record: &ModelValidationRecord) -> ModelExecutionLease {
        serde_json::from_value(json!({
            "leaseId":"lease-fixture","snapshotDigest":"digest-fixture","released":false,
            "owner":{"instanceId":"fixture","pid":1,"startFingerprint":"fixture"},
            "processes":[],"executionStarted":false,"launchProtocol":1,"createdAt":1,
            "snapshot":{
                "validationId":record.validation_id,"runtimeKind":record.runtime_kind,
                "profileId":"profile-fixture","instanceFence":"fence-fixture",
                "connectionId":record.connection_id,"kind":"local","configRevision":record.config_revision,
                "authGeneration":record.auth_generation,"purpose":"foreground","modelId":record.model_id,
                "localRuntime":"ollama","authMode":"none","network":{"mode":"direct"},
                "runtimeHomeScope":"profile-fixture:connection-fixture:7","descriptorRevision":1,"capabilities":{}
            }
        })).unwrap()
    }

    #[test]
    fn validation_lease_requires_exact_model_auth_generation_and_runtime_kind() {
        let record = validation_record();
        let original = validation_lease(&record);
        assert!(validate_probe_lease(&original, &record).is_ok());
        let mut changed = original.clone();
        changed.snapshot.model_id = "model-a-other".into();
        assert!(validate_probe_lease(&changed, &record).is_err());
        let mut changed = original.clone();
        changed.snapshot.runtime_kind = ModelRuntimeKind::Workbench;
        assert!(validate_probe_lease(&changed, &record).is_err());
        let mut changed = original.clone();
        changed.snapshot.auth_generation += 1;
        assert!(validate_probe_lease(&changed, &record).is_err());
        let mut changed = original;
        changed.snapshot.runtime_home_scope = "different-home".into();
        assert!(validate_probe_lease(&changed, &record).is_err());
    }

    fn http_fixture(
        status: &str,
        expected_authorization: Option<&str>,
    ) -> (ModelConnection, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let expected_authorization = expected_authorization.map(str::to_owned);
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                let size = socket.read(&mut chunk).unwrap();
                if size == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..size]);
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.starts_with("GET /v1/models HTTP/1.1\r\n"));
            if let Some(expected) = expected_authorization {
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains(&format!("authorization: bearer {expected}\r\n"))
                );
            } else {
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
            }
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                    )
                    .as_bytes(),
                )
                .unwrap();
        });
        let current=serde_json::from_value(json!({
            "profileId":"profile-fixture","connectionId":"connection-fixture","configRevision":1,"authGeneration":1,
            "config":{"name":"local fixture","kind":"local","baseUrl":format!("http://{address}/v1"),
                "localRuntime":"ollama","authMode":"none","enabled":true,"network":{"mode":"direct"}},
            "deleted":false,"createdAt":1,"updatedAt":1
        })).unwrap();
        (current, worker)
    }

    #[test]
    fn unauthenticated_http_401_is_network_evidence_only() {
        let (current, worker) = http_fixture("401 Unauthorized", None);
        let status = models_status(
            &current,
            "",
            false,
            &std::env::temp_dir().join("credentials"),
        )
        .unwrap();
        worker.join().unwrap();
        assert!(network_evidence(status).is_ok());
        assert!(authentication_evidence(status, &ModelAuthMode::ApiKey, false).is_err());
    }

    #[test]
    fn local_authentication_request_carries_only_explicit_test_credential() {
        let (current, worker) = http_fixture("200 OK", Some("fixture-only"));
        let status = models_status(
            &current,
            "fixture-only",
            true,
            &std::env::temp_dir().join("credentials"),
        )
        .unwrap();
        worker.join().unwrap();
        assert!(authentication_evidence(status, &ModelAuthMode::ApiKey, false).is_ok());
        assert!(
            assess_probe(
                &ModelValidationLayer::Tools,
                "nonce",
                r#"{"nonce":"nonce"}"#,
                &StreamObservation::default(),
                &completed(false)
            )
            .is_err()
        );
    }

    #[test]
    fn account_metadata_is_not_remote_authentication_evidence() {
        assert!(validate_rate_limits(&json!({"account":{"type":"chatgpt"}})).is_err());
        assert!(validate_rate_limits(&json!({"rateLimits":{}})).is_err());
        assert!(
            validate_rate_limits(
                &json!({"rateLimits":{"primary":{"usedPercent":0}},"rateLimitsByLimitId":null})
            )
            .is_ok()
        );
    }

    #[test]
    fn unconfirmed_process_exit_remains_unconfirmed_after_timeout() {
        let process = HarnessProcessState::default();
        assert!(!wait_for_exit(&process, Duration::from_millis(1)));
        process.exited.store(true, Ordering::Release);
        assert!(wait_for_exit(&process, Duration::from_millis(1)));
    }

    #[test]
    fn public_catalog_does_not_validate_an_api_key() {
        let (current, worker) = http_fixture("200 OK", None);
        let status = models_status(
            &current,
            "",
            false,
            &std::env::temp_dir().join("credentials"),
        )
        .unwrap();
        worker.join().unwrap();
        assert!(network_evidence(status).is_ok());
        assert!(require_authentication_challenge(status).is_err());
        assert!(require_authentication_challenge(reqwest::StatusCode::UNAUTHORIZED).is_ok());
    }
}
