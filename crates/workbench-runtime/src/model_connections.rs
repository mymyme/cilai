//! 个人模型连接的执行租约、独立运行目录与官方授权边界。
pub(crate) mod native_proxy;
mod network_certificates;
mod network_credentials;
mod system_proxy;
pub use network_certificates::validate_model_ca_certificate;
pub(crate) use system_proxy::{
    ResolvedNetwork, direct_network as direct_native_network,
    resolve_without_credentials as resolve_network_without_credentials,
};

use crate::ai_runtime::AiRuntimeManager;
use crate::codex_threads::CodexThreadService;
use crate::supervisor::AppServerSupervisor;
use contexthub_protocol::{
    MODEL_CREDENTIAL_SERVICE, ModelAuthMode, ModelConnection, ModelConnectionKind,
    ModelExecutionLease, ModelExecutionLeaseParams, ModelExecutionPrepareParams,
    ModelExecutionSnapshot, ModelNetworkConfig,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const OFFICIAL_RUNTIME_VERSION: &str = "0.163.0";
pub(crate) const LOOPBACK_KEY_ENV: &str = "CILAI_PERSONAL_MODEL_LOOPBACK_TOKEN";
pub(crate) const LOOPBACK_PROVIDER: &str = "cilai_personal_responses";
pub(crate) const CHATGPT_PROVIDER: &str = "cilai_personal_chatgpt";

pub use contexthub_protocol::ModelRuntimeKind;
mod auth;
pub(crate) mod isolation;
mod transport;
mod validation;
pub(crate) use transport::ExecutionTransport;
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthTarget {
    pub connection_id: String,
    pub runtime_kind: ModelRuntimeKind,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthCancel {
    pub connection_id: String,
    pub runtime_kind: ModelRuntimeKind,
    pub login_id: Option<String>,
    pub operation_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelAuthOperation {
    pub connection_id: String,
    pub operation_id: String,
}

#[derive(Default)]
pub(crate) struct ConnectionRuntime {
    services: Mutex<HashMap<PathBuf, Arc<CodexThreadService>>>,
    logins: Mutex<HashMap<PathBuf, PendingAuth>>,
}

struct PendingAuth {
    login_id: String,
    epoch: u64,
    operation_id: String,
}

/// 活跃执行及空闲记忆进程持有同一不可变快照；进程退出后才释放后端租约。
pub(crate) struct ExecutionScope {
    pub lease: ModelExecutionLease,
    pub data_dir: PathBuf,
    pub server: AppServerSupervisor,
    manager: AiRuntimeManager,
    memory_preferences: Option<crate::memory::MemoryPreferences>,
    launch_attempted: std::sync::atomic::AtomicBool,
    released: std::sync::atomic::AtomicBool,
    transports: Arc<transport::TransportLifecycle>,
    network_route: std::sync::OnceLock<ResolvedNetwork>,
    native_proxy: Mutex<Option<Arc<native_proxy::NativeProxy>>>,
    home_isolation: std::sync::OnceLock<
        Option<contexthub_platform_adapter::macos_runtime_isolation::ChatgptIsolationSnapshot>,
    >,
}

impl Drop for ExecutionScope {
    fn drop(&mut self) {
        if self.finish_release().is_err() {
            eprintln!("personal_model_execution_release_unconfirmed");
        }
    }
}

pub(crate) fn ensure_runtime_managed_auth_supported(data_dir: &Path) -> Result<(), String> {
    if !data_dir.is_absolute() || data_dir.file_name().is_none_or(|name| name != "ai-runtime") {
        return Err("model_credential_vault_root_required".into());
    }
    for ancestor in data_dir.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor)
            .map_err(|_| "model_credential_vault_root_required")?;
        if !metadata.is_dir()
            || contexthub_platform_adapter::filesystem::is_link_or_reparse_point(&metadata)
            || contexthub_platform_adapter::filesystem::is_cloud_placeholder_file(&metadata)
        {
            return Err("model_credential_vault_root_required".into());
        }
    }
    contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(
        data_dir
            .parent()
            .ok_or("model_credential_vault_root_required")?
            .join("credentials"),
    )
    .map_err(|_| "model_credential_vault_root_required")?;
    Ok(())
}

impl ExecutionScope {
    pub fn finish_release(&self) -> Result<(), String> {
        if self.released.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        if let Some(proxy) = self
            .native_proxy
            .lock()
            .map_err(|_| "model_native_proxy_state_unavailable")?
            .as_ref()
        {
            proxy.shutdown_checked()?;
        }
        self.transports.retire_and_wait(Duration::from_secs(5))?;
        let launched = self
            .launch_attempted
            .load(std::sync::atomic::Ordering::Acquire);
        let method = if launched {
            "modelExecution/release"
        } else {
            "modelExecution/abortUnlaunched"
        };
        let mut params =
            json!({"leaseId":self.lease.lease_id,"snapshotDigest":self.lease.snapshot_digest});
        if !launched {
            params["owner"] = json!(self.lease.owner);
        }
        self.server.request_result(method, &params)?;
        self.released
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }
    pub fn prepare(
        manager: &AiRuntimeManager,
        mut params: ModelExecutionPrepareParams,
    ) -> Result<Arc<Self>, String> {
        params.launch_protocol = 1;
        params.owner = Some(execution_owner()?);
        let server = manager.personal_model_server()?;
        let data_dir = manager.gateway_data_dir()?;
        let result = server.request_result(
            "modelExecution/prepare",
            &serde_json::to_value(params).map_err(|_| "model_execution_request_invalid")?,
        )?;
        let lease: ModelExecutionLease =
            serde_json::from_value(result).map_err(|_| "model_execution_response_invalid")?;
        Self::from_lease(manager, server, data_dir, lease)
    }

    pub(crate) fn from_lease(
        manager: &AiRuntimeManager,
        server: AppServerSupervisor,
        data_dir: PathBuf,
        lease: ModelExecutionLease,
    ) -> Result<Arc<Self>, String> {
        let scope = Arc::new(Self {
            memory_preferences: (lease.snapshot.purpose
                == contexthub_protocol::ModelPurpose::Memory)
                .then(|| manager.personal_memory_preferences()),
            lease,
            data_dir,
            server,
            manager: manager.clone(),
            launch_attempted: std::sync::atomic::AtomicBool::new(false),
            released: std::sync::atomic::AtomicBool::new(false),
            transports: Arc::new(transport::TransportLifecycle::default()),
            network_route: std::sync::OnceLock::new(),
            native_proxy: Mutex::new(None),
            home_isolation: std::sync::OnceLock::new(),
        });
        scope.validate()?;
        validate_secret_ref(&scope.lease.snapshot)?;
        if let Some(reference) = &scope.lease.snapshot.network.proxy_secret_ref {
            contexthub_protocol::model_network_reference_generation(
                &scope.lease.snapshot.profile_id,
                &scope.lease.snapshot.connection_id,
                scope.lease.snapshot.config_revision,
                reference,
            )?;
        }
        scope
            .network_route
            .set(resolve_network(
                &scope.lease.snapshot.network,
                &scope.manager.credential_vault_root()?,
            )?)
            .map_err(|_| "model_network_snapshot_changed")?;
        let isolation = if scope.lease.snapshot.kind == ModelConnectionKind::Chatgpt {
            ensure_runtime_managed_auth_supported(&scope.data_dir)?;
            contexthub_platform_adapter::macos_runtime_isolation::ChatgptIsolationSnapshot::capture(
            )
            .map_err(|_| "model_runtime_home_isolation_unavailable")?
        } else {
            None
        };
        scope
            .home_isolation
            .set(isolation)
            .map_err(|_| "model_runtime_home_isolation_changed")?;
        Ok(scope)
    }

    pub fn home_isolation(
        &self,
    ) -> Option<&contexthub_platform_adapter::macos_runtime_isolation::ChatgptIsolationSnapshot>
    {
        self.home_isolation.get().and_then(Option::as_ref)
    }

    pub fn network(&self) -> Result<&ResolvedNetwork, String> {
        self.network_route
            .get()
            .ok_or_else(|| "model_network_snapshot_unavailable".into())
    }

    pub fn apply_native_network(&self, command: &mut std::process::Command) -> Result<(), String> {
        self.validate()?;
        let route = self.network()?;
        route.validate_credentials()?;
        if !route.requires_native_bridge() {
            route.apply_native(command);
            return Ok(());
        }
        let mut bridge = self
            .native_proxy
            .lock()
            .map_err(|_| "model_native_proxy_state_unavailable")?;
        if bridge.is_none() {
            *bridge = Some(Arc::new(native_proxy::NativeProxy::start(
                route.clone(),
                native_proxy::allowed_targets(
                    &self.lease.snapshot.kind,
                    self.lease.snapshot.base_url.as_deref(),
                )?,
                self.native_proxy_admission(),
            )?));
        }
        bridge
            .as_ref()
            .ok_or("model_native_proxy_start_failed")?
            .apply(route, command);
        Ok(())
    }

    fn native_proxy_admission(&self) -> native_proxy::Admission {
        let server = self.server.clone();
        let lease = self.lease.clone();
        let transports = Arc::clone(&self.transports);
        let manager = self.manager.clone();
        let directory = self.data_dir.clone();
        let memory_preferences = self.memory_preferences;
        Arc::new(move || {
            if manager.gateway_data_dir()? != directory {
                return Err("model_execution_profile_changed".into());
            }
            let transport = transports.enter()?;
            let fresh: ModelExecutionLease = serde_json::from_value(server.request_result(
                "modelExecution/validateForRequest",
                &json!({"leaseId":lease.lease_id,"snapshotDigest":lease.snapshot_digest}),
            )?)
            .map_err(|_| "model_execution_response_invalid")?;
            if fresh.released
                || fresh.snapshot != lease.snapshot
                || fresh.snapshot_digest != lease.snapshot_digest
            {
                return Err("model_execution_snapshot_changed".into());
            }
            if let Some(preferences) = memory_preferences {
                manager.validate_personal_memory_scope(preferences)?;
            }
            Ok(Box::new(transport) as Box<dyn Send>)
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.manager.gateway_data_dir()? != self.data_dir {
            return Err("model_execution_profile_changed".into());
        }
        let params = ModelExecutionLeaseParams {
            lease_id: self.lease.lease_id.clone(),
            snapshot_digest: self.lease.snapshot_digest.clone(),
        };
        let value = self.server.request_result(
            "modelExecution/validate",
            &serde_json::to_value(params).map_err(|_| "model_execution_request_invalid")?,
        )?;
        let current: ModelExecutionLease =
            serde_json::from_value(value).map_err(|_| "model_execution_response_invalid")?;
        if current.released
            || current.snapshot != self.lease.snapshot
            || current.snapshot_digest != self.lease.snapshot_digest
        {
            return Err("model_execution_snapshot_changed".into());
        }
        Ok(())
    }

    pub fn register_process(&self, pid: u32) -> Result<(), String> {
        self.validate()?;
        let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
            .map_err(|_| "model_execution_process_unavailable")?
            .ok_or("model_execution_process_unavailable")?;
        self.server.request_result("modelExecution/registerProcess",&json!({"leaseId":self.lease.lease_id,"snapshotDigest":self.lease.snapshot_digest,"process":{"pid":pid,"startFingerprint":fingerprint}}))?;
        Ok(())
    }

    pub fn mark_launch_attempted(&self) {
        self.launch_attempted
            .store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn mark_spawn_failed(&self) {
        self.launch_attempted
            .store(false, std::sync::atomic::Ordering::Release);
    }

    pub fn validate_for_request(&self) -> Result<(), String> {
        if self.transports.is_retiring() {
            return Err("model_execution_retiring".into());
        }
        self.validate()?;
        if let Some(preferences) = self.memory_preferences {
            self.manager.validate_personal_memory_scope(preferences)?;
        }
        self.server.request_result(
            "modelExecution/validateForRequest",
            &json!({"leaseId":self.lease.lease_id,"snapshotDigest":self.lease.snapshot_digest}),
        )?;
        self.network()?.validate_credentials()?;
        Ok(())
    }

    pub fn credential(&self) -> Result<String, String> {
        self.validate()?;
        let snapshot = &self.lease.snapshot;
        validate_secret_ref(snapshot)?;
        let secret = match snapshot.auth_mode {
            ModelAuthMode::None => return Ok(String::new()),
            ModelAuthMode::RuntimeManagedChatgpt => return Err("model_auth_runtime_managed".into()),
            ModelAuthMode::ApiKey => {
                let reference = snapshot.secret_ref.as_ref().ok_or("model_auth_required")?;
                contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(
                    self.manager.credential_vault_root()?,
                )
                .map_err(|_| "model_credential_access_failed")?
                .read(&reference.service, &reference.account)
                .map_err(|_| "model_credential_access_failed")?
                .ok_or("model_auth_required")?
            }
        };
        let value = String::from_utf8(secret).map_err(|_| "model_credential_invalid")?;
        if value.trim().is_empty() || value.contains(['\r', '\n', '\0']) {
            return Err("model_credential_invalid".into());
        }
        self.validate()?;
        Ok(value)
    }

    pub fn home(&self, kind: ModelRuntimeKind) -> PathBuf {
        scoped_home(
            &self.data_dir,
            &self.lease.snapshot.runtime_home_scope,
            kind,
        )
    }
}

pub(crate) fn scoped_home(data_dir: &Path, scope: &str, kind: ModelRuntimeKind) -> PathBuf {
    let digest = crate::persistent_threads::digest(&json!({"scope":scope}));
    data_dir
        .join("model-connections")
        .join(digest)
        .join(match kind {
            ModelRuntimeKind::Workbench => "workbench",
            ModelRuntimeKind::Application => "application",
        })
        .join("official-runtime")
}

fn validate_secret_ref(snapshot: &ModelExecutionSnapshot) -> Result<(), String> {
    if snapshot
        .secret_ref
        .as_ref()
        .is_some_and(|reference| reference.service == "cn.cilai.personal.workbench.model")
        || snapshot
            .network
            .proxy_secret_ref
            .as_ref()
            .is_some_and(|reference| reference.service == "cn.cilai.personal.workbench.proxy")
    {
        return Err("model_credential_reentry_required".into());
    }
    match snapshot.auth_mode {
        ModelAuthMode::ApiKey => {
            let reference = snapshot.secret_ref.as_ref().ok_or("model_auth_required")?;
            let account = format!(
                "{}:{}:{}",
                snapshot.profile_id, snapshot.connection_id, snapshot.auth_generation
            );
            if reference.service != MODEL_CREDENTIAL_SERVICE || reference.account != account {
                return Err("model_credential_reference_invalid".into());
            }
        }
        ModelAuthMode::RuntimeManagedChatgpt => {
            if snapshot.kind != ModelConnectionKind::Chatgpt || snapshot.secret_ref.is_some() {
                return Err("model_auth_configuration_invalid".into());
            }
        }
        ModelAuthMode::None => {
            if !matches!(
                snapshot.kind,
                ModelConnectionKind::Local | ModelConnectionKind::CustomResponses
            ) || snapshot.secret_ref.is_some()
            {
                return Err("model_auth_configuration_invalid".into());
            }
        }
    }
    Ok(())
}

pub(crate) fn resolve_network(
    network: &ModelNetworkConfig,
    vault_root: &Path,
) -> Result<ResolvedNetwork, String> {
    system_proxy::resolve(network, vault_root)
}

pub(crate) fn connection_client(
    network: &ModelNetworkConfig,
    vault_root: &Path,
) -> Result<reqwest::Client, String> {
    connection_client_with_route(&resolve_network(network, vault_root)?)
}

pub(crate) fn connection_client_with_route(
    route: &ResolvedNetwork,
) -> Result<reqwest::Client, String> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never());
    route
        .apply_http(builder)?
        .build()
        .map_err(|_| "model_network_client_failed".into())
}

pub(crate) fn network_failure_reason(
    error: &reqwest::Error,
    fallback: &'static str,
) -> &'static str {
    use std::error::Error as _;
    let mut source = error.source();
    while let Some(error) = source {
        if error.to_string() == "tunnel error: proxy authorization required" {
            return "model_proxy_authentication_rejected";
        }
        source = error.source();
    }
    fallback
}

impl ConnectionRuntime {
    pub(crate) fn shutdown(&self) -> Result<(), String> {
        let services = std::mem::take(
            &mut *self
                .services
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let mut failed = false;
        for (home, service) in services {
            if service.shutdown_checked().is_err() {
                failed = true;
                self.services
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(home, service);
            }
        }
        if failed {
            return Err("model_auth_process_exit_unconfirmed".into());
        }
        self.logins
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        Ok(())
    }
}

pub(crate) fn execution_owner() -> Result<contexthub_protocol::ModelExecutionOwner, String> {
    let pid = std::process::id();
    let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
        .map_err(|_| "model_execution_owner_unavailable")?
        .ok_or("model_execution_owner_unavailable")?;
    Ok(contexthub_protocol::ModelExecutionOwner {
        instance_id: crate::persistent_threads::owner_instance_id(),
        pid,
        start_fingerprint: fingerprint,
    })
}

fn connection(manager: &AiRuntimeManager, id: &str) -> Result<ModelConnection, String> {
    let connection: ModelConnection = serde_json::from_value(
        manager
            .personal_model_server()?
            .request_result("modelConnection/get", &json!({"connectionId":id}))?,
    )
    .map_err(|_| "model_connection_response_invalid")?;
    if connection
        .secret_ref
        .as_ref()
        .is_some_and(|reference| reference.service == "cn.cilai.personal.workbench.model")
        || connection
            .config
            .network
            .proxy_secret_ref
            .as_ref()
            .is_some_and(|reference| reference.service == "cn.cilai.personal.workbench.proxy")
    {
        return Err("model_credential_reentry_required".into());
    }
    if let Some(reference) = &connection.config.network.proxy_secret_ref {
        contexthub_protocol::model_network_reference_generation(
            &connection.profile_id,
            &connection.connection_id,
            connection.config_revision,
            reference,
        )?;
    }
    Ok(connection)
}

fn connection_credential(
    manager: &AiRuntimeManager,
    current: &ModelConnection,
) -> Result<String, String> {
    if &connection(manager, &current.connection_id)? != current
        || current.deleted
        || !current.config.enabled
    {
        return Err("model_connection_changed".into());
    }
    match current.config.auth_mode {
        ModelAuthMode::None
            if matches!(
                current.config.kind,
                ModelConnectionKind::Local | ModelConnectionKind::CustomResponses
            ) =>
        {
            Ok(String::new())
        }
        ModelAuthMode::ApiKey => {
            let reference = current.secret_ref.as_ref().ok_or("model_auth_required")?;
            let expected = contexthub_protocol::model_runtime_home_scope(
                &current.profile_id,
                &current.connection_id,
                current.auth_generation,
            );
            if reference.service != MODEL_CREDENTIAL_SERVICE || reference.account != expected {
                return Err("model_credential_reference_invalid".into());
            }
            let bytes = contexthub_platform_adapter::credential_vault::LocalCredentialVault::new(
                manager.credential_vault_root()?,
            )
            .map_err(|_| "model_credential_access_failed")?
            .read(MODEL_CREDENTIAL_SERVICE, &expected)
            .map_err(|_| "model_credential_access_failed")?
            .ok_or("model_auth_required")?;
            let secret = String::from_utf8(bytes).map_err(|_| "model_credential_invalid")?;
            if secret.trim().is_empty() || secret.contains(['\r', '\n', '\0']) {
                return Err("model_credential_invalid".into());
            }
            Ok(secret)
        }
        _ => Err("model_auth_runtime_managed".into()),
    }
}

fn endpoint(current: &ModelConnection, suffix: &str) -> Result<reqwest::Url, String> {
    let base = match current.config.kind {
        ModelConnectionKind::OpenaiKey => "https://api.openai.com/v1",
        _ => current
            .config
            .base_url
            .as_deref()
            .ok_or("model_endpoint_required")?,
    };
    let mut url = reqwest::Url::parse(base).map_err(|_| "model_endpoint_invalid")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("model_endpoint_invalid".into());
    }
    url.set_path(&format!("{}/{}", url.path().trim_end_matches('/'), suffix));
    Ok(url)
}

impl AiRuntimeManager {
    pub(crate) fn credential_vault_root(&self) -> Result<PathBuf, String> {
        let data_dir = self.gateway_data_dir()?;
        ensure_runtime_managed_auth_supported(&data_dir)?;
        Ok(data_dir
            .parent()
            .ok_or("model_credential_vault_root_required")?
            .join("credentials"))
    }
    pub(crate) fn personal_model_credential_preflight(
        &self,
        snapshot: &ModelExecutionSnapshot,
    ) -> Result<(), String> {
        validate_secret_ref(snapshot)?;
        let current = connection(self, &snapshot.connection_id)?;
        if current.profile_id != snapshot.profile_id
            || current.config_revision != snapshot.config_revision
            || contexthub_protocol::model_active_auth_generation(&current, snapshot.runtime_kind)
                != Some(snapshot.auth_generation)
            || current.secret_ref != snapshot.secret_ref
            || current.config.network != snapshot.network
        {
            return Err("model_connection_changed".into());
        }
        if snapshot.auth_mode == ModelAuthMode::RuntimeManagedChatgpt {
            let account = self.personal_model_auth_read(&ModelAuthTarget {
                connection_id: snapshot.connection_id.clone(),
                runtime_kind: snapshot.runtime_kind,
            })?;
            if account["account"].is_null() {
                return Err("model_auth_required".into());
            }
        } else {
            connection_credential(self, &current)?;
        }
        resolve_network(&snapshot.network, &self.credential_vault_root()?)?;
        Ok(())
    }

    /// # Errors
    /// 连接、用途绑定或模型能力未满足执行要求时返回错误。
    pub fn personal_model_resolve(
        &self,
        request: ModelExecutionPrepareParams,
    ) -> Result<ModelExecutionSnapshot, String> {
        serde_json::from_value(self.personal_model_server()?.request_result(
            "modelExecution/resolve",
            &serde_json::to_value(request).map_err(|_| "model_execution_request_invalid")?,
        )?)
        .map_err(|_| "model_execution_response_invalid".into())
    }

    /// # Errors
    /// 连接已变更、目录不可访问或运行时来源无法核对时返回错误。
    pub fn personal_model_catalog_refresh(
        &self,
        connection_id: &str,
    ) -> Result<Vec<contexthub_protocol::ModelDescriptor>, String> {
        use contexthub_protocol::{ModelDescriptorObserveParams, ModelEvidenceSource};
        let current = connection(self, connection_id)?;
        if current.deleted || !current.config.enabled {
            return Err("model_connection_disabled".into());
        }
        let runtime = current.config.kind == ModelConnectionKind::Chatgpt;
        let entries = if runtime {
            self.personal_models.read_catalog(self, &current)?
        } else {
            let token = connection_credential(self, &current)?;
            let route = resolve_network(&current.config.network, &self.credential_vault_root()?)?;
            let client = connection_client_with_route(&route)?;
            let url = endpoint(&current, "models")?;
            let value = crate::async_support::block_on(async {
                let mut request = client.get(url).timeout(Duration::from_secs(30));
                if !token.is_empty() {
                    request = request.bearer_auth(&token);
                }
                request = route.decorate(request, &token)?;
                let response = request.send().await.map_err(|error| {
                    network_failure_reason(&error, "model_catalog_network_failed")
                })?;
                if response.status() == reqwest::StatusCode::PROXY_AUTHENTICATION_REQUIRED {
                    return Err("model_proxy_authentication_rejected".to_owned());
                }
                if !response.status().is_success() {
                    return Err("model_catalog_access_failed".to_owned());
                }
                response
                    .json::<Value>()
                    .await
                    .map_err(|_| "model_catalog_response_invalid".to_owned())
            })?;
            value["data"]
                .as_array()
                .ok_or("model_catalog_response_invalid")?
                .clone()
        };
        if connection(self, connection_id)? != current {
            return Err("model_connection_changed".into());
        }
        let source = if runtime {
            ModelEvidenceSource::RuntimeCatalog
        } else {
            ModelEvidenceSource::ServiceDeclared
        };
        let models = entries
            .iter()
            .map(|entry| catalog_observation(entry, &source, runtime))
            .collect::<Result<Vec<_>, String>>()?;
        let params = ModelDescriptorObserveParams {
            connection_id: connection_id.into(),
            expected_config_revision: current.config_revision,
            expected_auth_generation: current.auth_generation,
            models,
        };
        let result = serde_json::from_value(self.personal_model_server()?.request_result(
            "modelDescriptor/observe",
            &serde_json::to_value(params).map_err(|_| "model_catalog_request_invalid")?,
        )?)
        .map_err(|_| "model_catalog_response_invalid")?;
        self.refresh_model_catalog()?;
        Ok(result)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelHistoryTransferConsent {
    pub connection_id: String,
    pub config_revision: u32,
    pub auth_generation: u32,
    pub model_id: String,
    pub runtime_kind: ModelRuntimeKind,
    pub snapshot_digest: String,
    pub history_digest: String,
    pub confirmed_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelHistoryTransferPreview {
    pub connection_id: String,
    pub config_revision: u32,
    pub auth_generation: u32,
    pub model_id: String,
    pub runtime_kind: ModelRuntimeKind,
    pub snapshot_digest: String,
    pub history_digest: String,
    pub message_count: usize,
    pub attachment_count: usize,
    pub service_name: String,
    pub base_url: Option<String>,
    pub history_scope: String,
    pub may_incur_cost: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelExecutionPreflight {
    pub snapshot: ModelExecutionSnapshot,
    pub snapshot_digest: String,
    pub history_transfer: Option<ModelHistoryTransferPreview>,
}

impl ModelHistoryTransferPreview {
    pub(crate) fn validate(
        &self,
        consent: Option<&ModelHistoryTransferConsent>,
    ) -> Result<(), String> {
        let consent = consent.ok_or("model_thread_history_consent_required")?;
        if consent.connection_id != self.connection_id
            || consent.config_revision != self.config_revision
            || consent.auth_generation != self.auth_generation
            || consent.model_id != self.model_id
            || consent.runtime_kind != self.runtime_kind
            || consent.snapshot_digest != self.snapshot_digest
            || consent.history_digest != self.history_digest
            || consent.confirmed_at <= 0
        {
            return Err("model_history_consent_stale".into());
        }
        Ok(())
    }
}

pub(crate) fn snapshot_digest(snapshot: &ModelExecutionSnapshot) -> Result<String, String> {
    Ok(crate::persistent_threads::digest(
        &serde_json::to_value(snapshot).map_err(|_| "model_execution_response_invalid")?,
    ))
}

pub(crate) fn runtime_scope(snapshot: &ModelExecutionSnapshot, kind: ModelRuntimeKind) -> Value {
    json!({"profileId":snapshot.profile_id,"connectionId":snapshot.connection_id,"configRevision":snapshot.config_revision,"authGeneration":snapshot.auth_generation,"runtimeHomeScope":snapshot.runtime_home_scope,"runtimeKind":kind})
}

#[derive(Clone)]
pub(crate) struct ExecutionBundle {
    pub foreground: Arc<ExecutionScope>,
    pub review: Option<Arc<ExecutionScope>>,
    pub memory: Option<Arc<ExecutionScope>>,
}

impl ExecutionBundle {
    pub(crate) fn is_retiring(&self) -> bool {
        self.foreground.transports.is_retiring()
            || self
                .review
                .iter()
                .chain(self.memory.iter())
                .any(|scope| scope.transports.is_retiring())
    }
    pub(crate) fn begin_transport(&self, purpose: &str) -> Result<ExecutionTransport, String> {
        self.validate_request(purpose)?.transports.enter()
    }
    pub fn validate_lifecycle(&self) -> Result<(), String> {
        self.foreground.validate_for_request()?;
        for scope in self.review.iter().chain(self.memory.iter()) {
            scope.validate_for_request()?;
        }
        Ok(())
    }
    pub fn finish_release(&self) -> Result<(), String> {
        self.foreground.finish_release()?;
        for scope in self.review.iter().chain(self.memory.iter()) {
            scope.finish_release()?;
        }
        Ok(())
    }
    pub fn mark_launch_attempted(&self) {
        self.foreground.mark_launch_attempted();
        for scope in self.review.iter().chain(self.memory.iter()) {
            scope.mark_launch_attempted();
        }
    }
    pub fn mark_spawn_failed(&self) {
        self.foreground.mark_spawn_failed();
        for scope in self.review.iter().chain(self.memory.iter()) {
            scope.mark_spawn_failed();
        }
    }
    pub fn register_process(&self, pid: u32) -> Result<(), String> {
        self.foreground.register_process(pid)?;
        for scope in self.review.iter().chain(self.memory.iter()) {
            scope.register_process(pid)?;
        }
        Ok(())
    }
    pub fn validate_request(&self, purpose: &str) -> Result<&ExecutionScope, String> {
        let scope = match purpose {
            "memory" => self
                .memory
                .as_deref()
                .ok_or("model_memory_purpose_not_enabled")?,
            "review" => self
                .review
                .as_deref()
                .ok_or("model_review_purpose_not_enabled")?,
            _ => &self.foreground,
        };
        scope.validate_for_request()?;
        Ok(scope)
    }
}

pub(crate) fn isolate_runtime_environment(
    command: &mut std::process::Command,
    home: &Path,
) -> Result<(), String> {
    let process_home = home.join("process-home");
    contexthub_platform_adapter::security::ensure_owner_writable_runtime_tree(&process_home)
        .map_err(|_| "model_runtime_home_failed")?;
    let process_tmp = home.join("process-tmp");
    contexthub_platform_adapter::security::ensure_owner_writable_runtime_tree(&process_tmp)
        .map_err(|_| "model_runtime_home_failed")?;
    command.env_clear();
    contexthub_platform_adapter::process::retain_required_system_environment(command)
        .map_err(|_| "model_runtime_environment_failed")?;
    for name in ["PATH", "TMPDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let tool_environment = native_tool_environment(home, &process_home)?;
    let tool_environment = tool_environment
        .iter()
        .map(|(name, value)| serde_json::to_string(value).map(|value| format!("{name}={value}")))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "model_runtime_environment_failed")?
        .join(",");
    command.args([
        "-c",
        "features.shell_snapshot=false",
        "-c",
        "features.shell_snapshot_v2=false",
        "-c",
        "shell_environment_policy.inherit=\"none\"",
        "-c",
        &format!("shell_environment_policy.set={{{tool_environment}}}"),
    ]);
    command
        .env("HOME", &process_home)
        .env("USERPROFILE", &process_home)
        .env("XDG_CONFIG_HOME", process_home.join("config"))
        .env("XDG_DATA_HOME", process_home.join("data"))
        .env("TMPDIR", &process_tmp)
        .env("TEMP", &process_tmp)
        .env("TMP", &process_tmp)
        .env("NO_PROXY", "*");
    Ok(())
}

fn native_tool_environment(
    home: &Path,
    process_home: &Path,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let mut environment = std::collections::BTreeMap::new();
    for name in [
        "PATH",
        "TMPDIR",
        "TEMP",
        "TMP",
        "SystemRoot",
        "WINDIR",
        "PATHEXT",
        "COMSPEC",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TZ",
    ] {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(
                name.into(),
                value
                    .into_string()
                    .map_err(|_| "model_runtime_environment_failed")?,
            );
        }
    }
    for (name, path) in [
        ("HOME", process_home.to_path_buf()),
        ("USERPROFILE", process_home.to_path_buf()),
        ("XDG_CONFIG_HOME", process_home.join("config")),
        ("XDG_DATA_HOME", process_home.join("data")),
        ("XDG_CACHE_HOME", process_home.join("cache")),
        ("CODEX_HOME", home.to_path_buf()),
        ("CODEX_SQLITE_HOME", home.to_path_buf()),
        ("TMPDIR", home.join("process-tmp")),
        ("TEMP", home.join("process-tmp")),
        ("TMP", home.join("process-tmp")),
    ] {
        environment.insert(
            name.into(),
            path.into_os_string()
                .into_string()
                .map_err(|_| "model_runtime_environment_failed")?,
        );
    }
    Ok(environment)
}

pub(crate) fn launch_gate_executable() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        let path = crate::supervisor::app_server_test_binary();
        contexthub_platform_adapter::executable_search::validate_executable_file(&path)
            .map_err(|_| "model_launch_gate_unavailable")?;
        Ok(path)
    }
    #[cfg(not(test))]
    {
        crate::supervisor::bundled_app_server_path()
            .map_err(|_| "model_launch_gate_unavailable".into())
    }
}

fn catalog_observation(
    entry: &Value,
    source: &contexthub_protocol::ModelEvidenceSource,
    runtime: bool,
) -> Result<contexthub_protocol::ModelDescriptorObservation, String> {
    let id = entry
        .get("model")
        .or_else(|| entry.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or("model_catalog_response_invalid")?;
    let strings = |name: &str| {
        entry[name]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok(contexthub_protocol::ModelDescriptorObservation {
        model_id: id.into(),
        display_name: entry["displayName"].as_str().unwrap_or(id).into(),
        source: source.clone(),
        capabilities: std::collections::BTreeMap::new(),
        context_window_tokens: entry["contextWindow"]
            .as_u64()
            .or_else(|| entry["context_window"].as_u64()),
        input_modalities: strings("inputModalities"),
        default_reasoning_effort: entry["defaultReasoningEffort"].as_str().map(str::to_owned),
        supported_reasoning_efforts: entry["supportedReasoningEfforts"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| {
                        v["reasoningEffort"]
                            .as_str()
                            .or_else(|| v.as_str())
                            .map(str::to_owned)
                    })
                    .collect()
            })
            .unwrap_or_default(),
        runtime_version: runtime.then(|| OFFICIAL_RUNTIME_VERSION.into()),
        service_version: None,
    })
}

#[cfg(test)]
mod runtime_integration_tests;
