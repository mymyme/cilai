//! 个人资料备份与恢复；秘密不进入归档，静止和恢复回执分别记录。
use crate::{ModelExecutionOwner, PersonalProfileManifest};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PERSONAL_BACKUP_FORMAT: &str = "cilai-personal-backup";
pub const PERSONAL_BACKUP_FORMAT_VERSION: u32 = 2;
pub const PERSONAL_DATA_MAINTENANCE_PROTOCOL: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalDataCategory {
    BusinessDatabase,
    NativeHistory,
    NativeMemory,
    Skills,
    Applications,
    ApplicationData,
    Attachments,
    Configuration,
    WorkspaceOriginals,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalDataOperationKind {
    Backup,
    Restore,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalDataOperationState {
    Prepared,
    Capturing,
    Incomplete,
    ReadyToApply,
    Applying,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalDataIssue {
    pub code: String,
    pub category: Option<PersonalDataCategory>,
    pub object_id: Option<String>,
    pub relative_path: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalBackupSelection {
    #[serde(default)]
    pub include_workspace_files: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalBackupWorkspace {
    pub workspace_id: String,
    pub display_name: String,
    pub original_path: String,
    pub original_files_included: bool,
    pub archive_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub content_snapshot: Option<PersonalWorkspaceContentSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalWorkspaceContentEntry {
    pub local_file_id: String,
    pub relative_path: String,
    pub file_type: String,
    pub file_identity: String,
    #[ts(type = "number")]
    pub state_revision: i64,
    #[ts(type = "number | null")]
    pub bytes: Option<u64>,
    #[ts(type = "number | null")]
    pub modified_at: Option<i64>,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalWorkspaceContentSnapshot {
    pub protocol_version: u32,
    pub workspace_id: String,
    pub root_identity: String,
    pub root_path: String,
    #[ts(type = "number")]
    pub workspace_revision: i64,
    #[ts(type = "number")]
    pub latest_generation: i64,
    pub full_snapshot_id: String,
    #[ts(type = "number")]
    pub full_generation: i64,
    pub digest: String,
    pub entries: Vec<PersonalWorkspaceContentEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalRestoreWorkspaceDestination {
    pub parent_path: String,
    pub parent_identity: String,
    pub target_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalRestoredWorkspace {
    pub workspace_id: String,
    pub source_path: String,
    pub restored_path: String,
    pub content_snapshot_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalBackupNativeHome {
    pub relative_path: String,
    pub runtime_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalBackupFile {
    pub archive_path: String,
    pub category: PersonalDataCategory,
    pub digest: String,
    #[ts(type = "number")]
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalBackupManifest {
    pub format: String,
    pub format_version: u32,
    pub profile: PersonalProfileManifest,
    pub source_profile_path: String,
    pub schema_migration_version: u32,
    pub app_server_version: String,
    #[ts(type = "number")]
    pub captured_at: i64,
    pub complete: bool,
    pub selection: PersonalBackupSelection,
    pub files: Vec<PersonalBackupFile>,
    pub native_homes: Vec<PersonalBackupNativeHome>,
    pub workspaces: Vec<PersonalBackupWorkspace>,
    pub exclusions: Vec<PersonalDataIssue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalRestorePreview {
    pub source_profile: PersonalProfileManifest,
    pub current_profile_id: String,
    pub target_profile_id: String,
    pub archive_digest: String,
    pub manifest_digest: String,
    pub replaces_current_profile_directory: bool,
    pub preserves_current_copy: bool,
    pub file_count: u32,
    #[ts(type = "number")]
    pub total_bytes: u64,
    pub workspaces: Vec<PersonalBackupWorkspace>,
    pub warnings: Vec<PersonalDataIssue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_destination: Option<PersonalRestoreWorkspaceDestination>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalDataResult {
    pub artifact_relative_path: Option<String>,
    pub artifact_digest: Option<String>,
    pub restored_profile_id: Option<String>,
    pub retained_current_relative_path: Option<String>,
    pub verified_categories: Vec<PersonalDataCategory>,
    pub pending_categories: Vec<PersonalDataCategory>,
    pub issues: Vec<PersonalDataIssue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub restored_workspaces: Option<Vec<PersonalRestoredWorkspace>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalDataOperation {
    pub operation_id: String,
    pub profile_id: String,
    pub kind: PersonalDataOperationKind,
    pub state: PersonalDataOperationState,
    pub preview_digest: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub selection: Option<PersonalBackupSelection>,
    pub backup_manifest: Option<PersonalBackupManifest>,
    pub restore_preview: Option<PersonalRestorePreview>,
    pub blockers: Vec<PersonalDataIssue>,
    pub result: Option<PersonalDataResult>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalBackupPrepareParams {
    pub selection: PersonalBackupSelection,
    pub client_operation_id: String,
}

/// 只由持有 Runtime 不透明维护 guard 的 Host 构造，不接受普通桌面透传。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalDataQuiescence {
    pub protocol_version: u32,
    pub profile_id: String,
    pub owner: ModelExecutionOwner,
    pub token: String,
    pub writer_registry_digest: String,
    pub covered_writers: Vec<String>,
    pub native_homes: Vec<PersonalBackupNativeHome>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalBackupCaptureParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub client_operation_id: String,
    pub quiescence: Option<PersonalDataQuiescence>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalRestorePrepareParams {
    pub archive_path: String,
    pub client_operation_id: String,
}

/// 目录路径由宿主原生选择器取得，普通页面不能提供恢复写入目标。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalRestoreWorkspacePlanParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub destination_parent: String,
    pub client_operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalRestoreApplyPrepareParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub expected_current_profile_id: String,
    pub client_operation_id: String,
    pub quiescence: Option<PersonalDataQuiescence>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalRestoreAbortBeforeHelperParams {
    pub operation_id: String,
    pub preview_digest: String,
    pub quiescence: PersonalDataQuiescence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalDataParams {
    pub operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalDataListParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalDataCancelParams {
    pub operation_id: String,
    pub preview_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalBackupArtifact {
    pub operation_id: String,
    pub path: String,
    pub digest: String,
    #[ts(type = "number")]
    pub bytes: u64,
}
