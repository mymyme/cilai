use super::{
    AiRuntimeManager, ManagerState, ModelToolCapabilities, lock, model_info_from_descriptor,
    unix_epoch_millis,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub(super) connection_id: Option<String>,
    pub(super) connection_name: Option<String>,
    pub(super) config_revision: Option<u32>,
    pub(super) auth_generation: Option<u32>,
    pub(super) model_id: String,
    pub(super) label: String,
    pub(super) provider: String,
    pub(super) description: String,
    pub(super) source: String,
    pub(super) context_window_tokens: Option<u64>,
    pub(super) input_modalities: Vec<String>,
    pub(super) supported_reasoning_efforts: Vec<String>,
    pub(super) default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub(super) reasoning_control: Option<String>,
    #[serde(default)]
    pub(super) reasoning_enabled_efforts: Vec<String>,
    #[serde(default)]
    pub(super) tool_capabilities: Option<ModelToolCapabilities>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogStatus {
    pub(super) configured: bool,
    pub(super) models: Vec<ModelInfo>,
}

pub const MODEL_CATALOG_TTL_MS: i64 = 300_000;
const MODEL_CATALOG_PICKER_DEDUP_MS: i64 = 1_000;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelCatalogReadReason {
    Startup,
    Picker,
    Retry,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogSnapshot {
    pub(super) generation: u64,
    pub(super) revision: u64,
    pub(super) status: Option<ModelCatalogStatus>,
    pub(super) checked_at: Option<i64>,
    pub(super) revalidate_after: Option<i64>,
    pub(super) error: Option<String>,
}

pub(super) struct CatalogState {
    status: Option<ModelCatalogStatus>,
    checked_at: Option<i64>,
    generation: u64,
    revision: u64,
    error: Option<String>,
    changed: tokio::sync::watch::Sender<()>,
}

impl Default for CatalogState {
    fn default() -> Self {
        Self {
            status: None,
            checked_at: None,
            generation: 0,
            revision: 0,
            error: None,
            changed: tokio::sync::watch::channel(()).0,
        }
    }
}

impl CatalogState {
    pub(super) fn models(&self) -> &[ModelInfo] {
        self.status
            .as_ref()
            .map_or(&[], |status| status.models.as_slice())
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn subscribe(&self) -> tokio::sync::watch::Receiver<()> {
        self.changed.subscribe()
    }

    pub(super) fn snapshot(&self) -> ModelCatalogSnapshot {
        ModelCatalogSnapshot {
            generation: self.generation,
            revision: self.revision,
            status: self.status.clone(),
            checked_at: self.checked_at,
            revalidate_after: self
                .checked_at
                .map(|time| time.saturating_add(MODEL_CATALOG_TTL_MS)),
            error: self.error.clone(),
        }
    }

    pub(super) fn replace(&mut self, status: ModelCatalogStatus, now: i64) {
        let changed = self.status.as_ref() != Some(&status);
        self.status = Some(status);
        self.error = None;
        if changed {
            self.checked_at = Some(now);
            self.revision = self.revision.wrapping_add(1);
            self.changed.send_replace(());
        }
    }

    pub(super) fn reset(&mut self) {
        self.status = None;
        self.checked_at = None;
        self.error = None;
        self.generation = self.generation.wrapping_add(1);
        self.revision = 0;
        self.changed.send_replace(());
    }

    pub(super) fn invalidate(&mut self, generation: u64, reason: &str) {
        if generation != self.generation
            || ![
                "AI-AUTH-002",
                "AI-MODEL-001",
                "MODEL_ACCESS_DISABLED",
                "MEMBERSHIP_REMOVED",
                "model_not_authorized",
                "model_not_entitled",
                "model_explicitly_denied",
                "model_unavailable",
                "qwen_gateway_model_access_disabled",
            ]
            .iter()
            .any(|code| reason.contains(code))
        {
            return;
        }
        self.status = None;
        self.checked_at = None;
        self.error = Some("model_catalog_access_revoked".to_owned());
        self.generation = self.generation.wrapping_add(1);
        self.revision = self.revision.wrapping_add(1);
        self.changed.send_replace(());
    }
}

impl AiRuntimeManager {
    /// # Errors
    /// 企业凭据读取或模型网关状态检查失败时返回错误。
    pub fn model_catalog_status(&self) -> Result<ModelCatalogStatus, String> {
        self.gateway_catalog_with_max_age(MODEL_CATALOG_TTL_MS)
    }

    /// # Errors
    /// 身份变化或目录读取失败时返回错误，不使用历史目录。
    pub fn read_model_catalog(
        &self,
        reason: ModelCatalogReadReason,
    ) -> Result<ModelCatalogSnapshot, String> {
        let max_age = match reason {
            ModelCatalogReadReason::Startup => MODEL_CATALOG_TTL_MS,
            ModelCatalogReadReason::Picker => MODEL_CATALOG_PICKER_DEDUP_MS,
            ModelCatalogReadReason::Retry => 0,
        };
        let result = self.gateway_catalog_with_max_age(max_age);
        if result
            .as_ref()
            .is_err_and(|error| error == "ai_profile_changed")
        {
            return Err("ai_profile_changed".to_owned());
        }
        let snapshot = self.model_catalog_snapshot();
        if let Err(error) = result
            && snapshot.error.is_none()
        {
            return Err(error);
        }
        Ok(snapshot)
    }

    #[must_use]
    pub fn model_catalog_snapshot(&self) -> ModelCatalogSnapshot {
        let state = lock(&self.inner);
        state.model_catalog.snapshot()
    }

    /// # Errors
    /// 目录不属于指定企业 Profile 时返回错误。
    pub fn model_catalog_for_profile(
        &self,
        data_dir: &Path,
    ) -> Result<ModelCatalogSnapshot, String> {
        let state = lock(&self.inner);
        if state.data_dir.as_deref() != Some(data_dir.join("ai-runtime").as_path()) {
            return Err("ai_profile_changed".to_owned());
        }
        Ok(state.model_catalog.snapshot())
    }

    #[must_use]
    pub fn subscribe_model_catalog(&self) -> tokio::sync::watch::Receiver<()> {
        lock(&self.inner).model_catalog.subscribe()
    }

    fn gateway_catalog_with_max_age(&self, _max_age: i64) -> Result<ModelCatalogStatus, String> {
        let server = self.personal_model_server()?;
        let connections: Vec<contexthub_protocol::ModelConnection> = serde_json::from_value(
            server.request_result("modelConnection/list", &serde_json::json!({}))?,
        )
        .map_err(|_| "model_connection_response_invalid")?;
        let mut models = Vec::new();
        let mut configured = false;
        for connection in connections
            .iter()
            .filter(|connection| connection.config.enabled && !connection.deleted)
        {
            configured = true;
            let descriptors: Vec<contexthub_protocol::ModelDescriptor> =
                serde_json::from_value(server.request_result(
                    "modelCatalog/read",
                    &serde_json::json!({"connectionId":connection.connection_id}),
                )?)
                .map_err(|_| "model_catalog_response_invalid")?;
            models.extend(
                descriptors
                    .into_iter()
                    .filter(|descriptor| {
                        descriptor.config_revision == connection.config_revision
                            && (connection.config.kind
                                == contexthub_protocol::ModelConnectionKind::Chatgpt
                                || descriptor.auth_generation == connection.auth_generation)
                    })
                    .map(|descriptor| model_info_from_descriptor(connection, &descriptor)),
            );
        }
        let status = ModelCatalogStatus { configured, models };
        let mut state = lock(&self.inner);
        state
            .model_catalog
            .replace(status.clone(), unix_epoch_millis()?);
        Ok(status)
    }

    /// # Errors
    /// 模型网关认证或目录读取失败时返回错误。
    pub fn refresh_model_catalog(&self) -> Result<Vec<ModelInfo>, String> {
        Ok(self.model_catalog_status()?.models)
    }

    #[must_use]
    pub fn application_models(&self) -> Vec<ModelInfo> {
        lock(&self.inner).model_catalog.models().to_vec()
    }

    #[must_use]
    pub fn model_catalog_generation(&self) -> u64 {
        lock(&self.inner).model_catalog.generation()
    }

    pub fn invalidate_model_catalog_for_error(&self, generation: u64, reason: &str) {
        lock(&self.inner)
            .model_catalog
            .invalidate(generation, reason);
    }

    pub(super) fn invalidate_catalog_for_error(state: &mut ManagerState, reason: &str) {
        state
            .model_catalog
            .invalidate(state.model_catalog.generation(), reason);
    }
}
