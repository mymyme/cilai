//! 个人应用包导入；外包中的身份仅作未认证来源，不作为本地归属。
use crate::CapabilityCreationProject;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PersonalPackageImportTarget {
    New {},
    Upgrade {
        creation_project_id: String,
        #[ts(type = "number")]
        expected_revision: i64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalPackageImportState {
    Prepared,
    Committed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPackageImportSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub page_ai: Option<crate::PersonalPageAiManifest>,
    pub authenticated: bool,
    pub format_version: u32,
    pub archive_digest: String,
    pub source_digest: String,
    pub plugin_id: String,
    pub version: String,
    pub title: String,
    pub summary: String,
    pub target_type: String,
    pub source_file_count: u32,
    #[ts(type = "number")]
    pub archive_bytes: u64,
    #[ts(type = "number")]
    pub source_bytes: u64,
    #[ts(type = "number")]
    pub unpacked_bytes: u64,
    pub full_business_goal_present: bool,
    pub acceptance_examples_present: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPackageImportPreview {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub installed_page_ai: Option<crate::PersonalPageAiManifest>,
    pub import_id: String,
    pub profile_id: String,
    pub state: PersonalPackageImportState,
    #[ts(type = "number")]
    pub prepared_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub preview_digest: String,
    pub source: PersonalPackageImportSource,
    pub target: PersonalPackageImportTarget,
    pub local_creation_project_id: String,
    #[ts(type = "number | null")]
    pub expected_revision: Option<i64>,
    pub installed_digest: Option<String>,
    pub local_version: String,
    pub local_package_digest: String,
    pub requested_permissions: Vec<String>,
    pub added_permissions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPackageImportPrepareParams {
    pub archive_path: String,
    pub target: PersonalPackageImportTarget,
    pub client_operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPackageImportCommitParams {
    #[serde(default)]
    pub data_change: Option<crate::PersonalApplicationDataExecuteParams>,
    pub import_id: String,
    pub preview_digest: String,
    pub confirmed_permissions: Vec<String>,
    pub client_operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPackageImportParams {
    pub import_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPackageImportCancelParams {
    pub import_id: String,
    pub preview_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPackageImportListParams {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalPackageImportCommitResult {
    pub import_id: String,
    pub local_package_digest: String,
    pub project: CapabilityCreationProject,
    pub replayed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn import_target_contract_keeps_tagged_camel_case_fields_and_rejects_unknown_fields() {
        let value = json!({"mode":"upgrade","creationProjectId":"capability_11111111111141118111111111111111","expectedRevision":2});
        let target: PersonalPackageImportTarget = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&target).unwrap(), value);
        for invalid in [
            json!({"mode":"new","tenantId":"untrusted"}),
            json!({"mode":"upgrade","creationProjectId":"x","expectedRevision":2,"archivePath":"external"}),
        ] {
            assert!(serde_json::from_value::<PersonalPackageImportTarget>(invalid).is_err());
        }
        let declaration = PersonalPackageImportTarget::decl(&ts_rs::Config::default());
        assert!(declaration.contains("creationProjectId: string"));
        assert!(declaration.contains("expectedRevision: number"));
        assert!(!declaration.contains("creation_project_id"));
        let schema =
            serde_json::to_value(schemars::schema_for!(PersonalPackageImportTarget)).unwrap();
        assert!(
            schema["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .all(|variant| variant["additionalProperties"] == false)
        );
    }
}
