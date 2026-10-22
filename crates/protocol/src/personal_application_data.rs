//! 个人应用升级的数据副本、明确切换与回退，不接受外部数据路径。
use crate::PersonalDataQuiescence;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalApplicationDataAction {
    Upgrade,
    ImportUpgrade,
    Rollback,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalApplicationDataState {
    Prepared,
    Applying,
    Completed,
    Incomplete,
    Cancelled,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalApplicationDataPrepareParams {
    pub creation_project_id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub package_digest: String,
    pub client_operation_id: String,
    pub action: PersonalApplicationDataAction,
    pub target_operation_id: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalApplicationDataParams {
    pub operation_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalApplicationDataConfirmParams {
    pub operation_id: String,
    pub preview_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalApplicationDataExecuteParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub quiescence: Option<PersonalDataQuiescence>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalApplicationDataOperation {
    #[serde(default)]
    pub read_only_after_restore: bool,
    #[serde(default)]
    pub source_profile_id: Option<String>,
    pub operation_id: String,
    pub profile_id: String,
    pub creation_project_id: String,
    pub client_operation_id: String,
    #[ts(type = "number")]
    pub expected_revision: i64,
    pub package_digest: String,
    pub source_package_digest: Option<String>,
    pub source_slot_id: Option<String>,
    pub target_slot_id: String,
    pub action: PersonalApplicationDataAction,
    pub state: PersonalApplicationDataState,
    pub preview_digest: String,
    pub snapshot_digest: Option<String>,
    pub reason: Option<String>,
    #[ts(type = "number | null")]
    pub activated_revision: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}
