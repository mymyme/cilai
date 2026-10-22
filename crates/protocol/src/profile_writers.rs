//! 资料文件写者与维护栅栏；执行结果与本机写入是否停止分别记录。
use crate::{ModelExecutionOwner, ModelExecutionProcess};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalWriterMode {
    LocalProcess,
    RemoteRequest,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalWriterState {
    Prepared,
    Active,
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalWriterPrepareParams {
    pub client_operation_id: String,
    pub kind: String,
    pub resource_id: String,
    pub resource_revision: String,
    pub owner: ModelExecutionOwner,
    pub launch_protocol: u32,
    pub process_mode: PersonalWriterMode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalWriterLease {
    pub lease_id: String,
    pub profile_id: String,
    pub instance_fence: String,
    pub kind: String,
    pub resource_id: String,
    pub resource_revision: String,
    pub owner: ModelExecutionOwner,
    pub launch_protocol: u32,
    pub process_mode: PersonalWriterMode,
    pub processes: Vec<ModelExecutionProcess>,
    pub state: PersonalWriterState,
    pub local_transport_finished: bool,
    pub remote_outcome_unconfirmed: bool,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalWriterRegisterParams {
    pub lease_id: String,
    pub process: ModelExecutionProcess,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalWriterLeaseParams {
    pub lease_id: String,
    pub owner: Option<ModelExecutionOwner>,
    #[serde(default)]
    pub local_transport_finished: bool,
    #[serde(default)]
    pub remote_outcome_unconfirmed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalWriterAbortParams {
    pub lease_id: String,
    pub owner: ModelExecutionOwner,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalWriterListParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalMaintenanceState {
    Draining,
    Released,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMaintenanceFence {
    pub fence_id: String,
    pub operation_id: String,
    pub profile_id: String,
    pub instance_fence: String,
    pub owner: ModelExecutionOwner,
    pub state: PersonalMaintenanceState,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMaintenanceAcquireParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub owner: ModelExecutionOwner,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMaintenanceParams {
    pub fence_id: String,
    pub owner: ModelExecutionOwner,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMaintenanceDrainStatus {
    pub fence: PersonalMaintenanceFence,
    pub active_model_leases: Vec<crate::ModelExecutionLease>,
    pub active_writers: Vec<PersonalWriterLease>,
    pub writer_registry_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMaintenanceListParams {}
