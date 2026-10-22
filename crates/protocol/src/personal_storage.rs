//! 当前个人资料的占用、受管清理预览及真实结果，不授予任意路径删除能力。
use crate::PersonalDataQuiescence;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalStorageCategory {
    Cache,
    Logs,
    Attachments,
    ApplicationData,
    Skills,
    Applications,
    NativeHistory,
    NativeMemory,
    RecoveryCopies,
    BusinessDatabase,
    Configuration,
    ProjectOriginals,
    OtherProtected,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalStorageCoverageState {
    Covered,
    NotScanned,
    Unavailable,
    ExternalReference,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageCoverage {
    pub category: PersonalStorageCategory,
    pub scope: String,
    pub state: PersonalStorageCoverageState,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageReference {
    pub kind: String,
    pub object_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageItem {
    pub item_id: String,
    pub category: PersonalStorageCategory,
    pub display_path: String,
    pub managed_relative_path: Option<String>,
    #[ts(type = "number | null")]
    pub logical_bytes: Option<u64>,
    pub file_count: Option<u32>,
    pub snapshot_digest: Option<String>,
    pub cleanup_allowed: bool,
    pub reasons: Vec<String>,
    pub references: Vec<PersonalStorageReference>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageInventory {
    pub profile_id: String,
    pub profile_path: String,
    #[ts(type = "number")]
    pub scanned_at: i64,
    #[ts(type = "number")]
    pub known_logical_bytes: u64,
    pub size_complete: bool,
    pub active_writer_count: u32,
    pub coverage: Vec<PersonalStorageCoverage>,
    pub items: Vec<PersonalStorageItem>,
    pub issues: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalStorageOperationState {
    Prepared,
    Applying,
    Completed,
    Incomplete,
    Cancelled,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalStorageOutcome {
    Deleted,
    Retained,
    Unconfirmed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageItemResult {
    pub item_id: String,
    pub outcome: PersonalStorageOutcome,
    #[ts(type = "number | null")]
    pub deleted_logical_bytes: Option<u64>,
    pub deleted_file_count: Option<u32>,
    pub retained_relative_path: Option<String>,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageResult {
    pub items: Vec<PersonalStorageItemResult>,
    #[ts(type = "number | null")]
    pub deleted_logical_bytes: Option<u64>,
    pub deleted_file_count: Option<u32>,
    pub physical_space_released: Option<bool>,
    pub issues: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageOperation {
    pub operation_id: String,
    pub profile_id: String,
    pub state: PersonalStorageOperationState,
    pub preview_digest: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub selected_items: Vec<PersonalStorageItem>,
    #[ts(type = "number")]
    pub selected_logical_bytes: u64,
    pub requires_quiescence: bool,
    pub blockers: Vec<String>,
    pub result: Option<PersonalStorageResult>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageInventoryParams {}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStoragePrepareParams {
    pub item_ids: Vec<String>,
    pub client_operation_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageExecuteParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub client_operation_id: String,
    pub quiescence: Option<PersonalDataQuiescence>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageParams {
    pub operation_id: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalStorageConfirmParams {
    pub operation_id: String,
    pub preview_digest: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMaintenanceOperationStatus {
    pub operation_id: String,
    pub profile_id: String,
    pub kind: String,
    pub state: String,
    pub preview_digest: String,
}
