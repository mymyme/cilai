use crate::ModelPurpose;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum ModelUsageState {
    Started,
    Completed,
    Failed,
    Unconfirmed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsageTokens {
    #[ts(type = "number | null")]
    pub input_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub cached_input_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub cache_write_input_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub output_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub reasoning_output_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub total_tokens: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsageCost {
    pub amount: String,
    pub currency: String,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsageBeginParams {
    pub request_id: String,
    pub lease_id: String,
    pub snapshot_digest: String,
    pub scope_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsageFinishParams {
    pub request_id: String,
    pub lease_id: String,
    pub snapshot_digest: String,
    pub state: ModelUsageState,
    pub response_id_sha256: Option<String>,
    pub tokens: Option<ModelUsageTokens>,
    pub cost: Option<ModelUsageCost>,
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsageParams {
    pub request_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsageListParams {
    #[ts(type = "number | null")]
    pub before_sequence: Option<i64>,
    #[ts(type = "number | null")]
    pub through_sequence: Option<i64>,
    pub limit: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsageRecord {
    pub request_id: String,
    pub source_profile_id: String,
    pub lease_id: String,
    pub snapshot_digest: String,
    pub connection_id: String,
    pub config_revision: u32,
    pub auth_generation: u32,
    pub model_id: String,
    pub purpose: ModelPurpose,
    pub scope_id: String,
    pub state: ModelUsageState,
    pub response_id_sha256: Option<String>,
    pub tokens: Option<ModelUsageTokens>,
    pub cost: Option<ModelUsageCost>,
    pub reason_code: Option<String>,
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number | null")]
    pub finished_at: Option<i64>,
    pub read_only_after_restore: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelUsageListResult {
    pub records: Vec<ModelUsageRecord>,
    #[ts(type = "number")]
    pub snapshot_sequence: i64,
    #[ts(type = "number | null")]
    pub next_before_sequence: Option<i64>,
}
