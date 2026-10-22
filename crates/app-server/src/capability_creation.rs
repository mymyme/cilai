mod data;
mod files;
mod imports;
mod lifecycle;
mod pages;
use super::{AppServerServices, ServiceError};
use contexthub_persistence_sqlite::{
    CapabilityCreationBuildRecord, CapabilityCreationCreateCommand,
    CapabilityCreationInstallCommand, CapabilityCreationProjectRecord,
    PersonalCapabilityManageCommand,
};
#[cfg(test)]
use contexthub_protocol::CapabilityCreationInstallResult;
use contexthub_protocol::{
    CapabilityCreationAcceptanceExample, CapabilityCreationCreateParams,
    CapabilityCreationCreateResult, CapabilityCreationGetParams, CapabilityCreationGetResult,
    CapabilityCreationInstallParams, CapabilityCreationListParams, CapabilityCreationListResult,
    CapabilityCreationPreviewParams, CapabilityCreationPreviewResult, CapabilityCreationProject,
    CapabilityCreationSourceFile, PersonalApplicationOpenParams, PersonalApplicationOpenResult,
};
pub(super) use files::{PersonalFileAccessParams, PersonalFileSelectParams};
pub(super) use lifecycle::{PersonalBuildParams, PersonalUpgradeParams, PersonalVersionParams};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{Cursor, Read as _, Write as _};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PersonalCapabilityManageParams {
    creation_project_id: String,
    package_digest: String,
    expected_revision: i64,
    action: String,
    client_operation_id: String,
}

const ALLOWED_TARGET_TYPES: &[&str] = &["local_mcp", "web_application", "web_mcp_application"];
const ALLOWED_PERMISSIONS: &[&str] = &[
    "personal_file.read",
    "personal_file.write_confirmed",
    "workbench_artifact.create",
    "page_ai.context",
    "page_ai.ui_actions_confirmed",
    "page_ai.model",
    "page_ai.conversation_handoff",
    "page_ai.tools.read",
    "page_ai.tools.write_confirmed",
];
const MAX_SOURCE_FILES: usize = 64;
const MAX_SOURCE_FILE_BYTES: usize = 256 * 1024;
const MAX_SOURCE_TOTAL_BYTES: usize = 768 * 1024;

pub(super) fn capability_creation_request_fingerprint(
    params: &CapabilityCreationCreateParams,
) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            json!({
                "conversationId": params.conversation_id,
                "title": params.title,
                "businessGoal": params.business_goal,
                "targetType": params.target_type,
                "requestedPermissions": params.requested_permissions,
                "acceptanceExamples": params.acceptance_examples,
                "sourceFiles": params.source_files,
            })
            .to_string()
        )
    )
}

#[cfg(test)]
fn capability_install_request_fingerprint(params: &CapabilityCreationInstallParams) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            json!({
                "creationProjectId": params.creation_project_id,
                "expectedRevision": params.expected_revision,
                "packageDigest": params.package_digest,
            })
            .to_string()
        )
    )
}

struct MaterializedBuild {
    source_digest: String,
    package_digest: String,
    package_relative_path: String,
    source_file_count: i64,
    total_bytes: i64,
}

struct MaterializedPersonalInstall {
    final_directory: PathBuf,
    relative_location: String,
}

impl AppServerServices {
    fn ensure_personal_capability(&self, project_id: &str) -> Result<(), ServiceError> {
        if self
            .persistence()?
            .enterprise_capability_binding(project_id)
            .map_err(persistence_error)?
            .is_some()
        {
            return Err(ServiceError::new("enterprise_capability_not_personal"));
        }
        Ok(())
    }

    fn replayed_capability_creation(
        &mut self,
        params: &CapabilityCreationCreateParams,
    ) -> Result<Option<CapabilityCreationCreateResult>, ServiceError> {
        let Some(operation) = self
            .persistence()?
            .capability_creation_operation(&params.client_operation_id)
            .map_err(persistence_error)?
        else {
            return Ok(None);
        };
        if operation.operation_kind != "create"
            || operation.request_fingerprint != params.request_fingerprint
        {
            return Err(ServiceError::new("operation_id_reused"));
        }
        let project = self
            .persistence()?
            .capability_creation_project(&operation.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        Ok(Some(CapabilityCreationCreateResult {
            project: project_from_record(project)?,
            replayed: true,
        }))
    }

    pub(super) fn capability_creation_create(
        &mut self,
        params: &CapabilityCreationCreateParams,
        now: i64,
    ) -> Result<CapabilityCreationCreateResult, ServiceError> {
        validate_create(params)?;
        if let Some(result) = self.replayed_capability_creation(params)? {
            return Ok(result);
        }

        let creation_project_id = format!("capability_{}", Uuid::new_v4().simple());
        let managed_relative_path = format!("capability-projects/{creation_project_id}");
        let title = params.title.trim().to_owned();
        let business_goal = params.business_goal.trim().to_owned();

        let root = self.capability_projects_root()?;
        fs::create_dir_all(&root)
            .map_err(|_| ServiceError::new("capability_creation_project_path_failed"))?;
        reject_symlink(&root)?;
        let staging = root.join(format!(".{}.staging", Uuid::new_v4().simple()));
        let final_path = root.join(&creation_project_id);
        fs::create_dir(&staging)
            .map_err(|_| ServiceError::new("capability_creation_project_path_failed"))?;
        let build = if params.source_files.is_empty() {
            None
        } else {
            match materialize_source_package(
                &staging,
                &creation_project_id,
                &title,
                &business_goal,
                &params.target_type,
                &params.requested_permissions,
                &params.source_files,
                "0.1.0",
            ) {
                Ok(build) => Some(build),
                Err(error) => {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(error);
                }
            }
        };
        let status = if build.is_some() {
            "packaged"
        } else {
            "planned"
        };
        let metadata = json!({
            "schemaVersion": "1.0",
            "creationProjectId": creation_project_id,
            "title": title,
            "businessGoal": business_goal,
            "targetType": params.target_type,
            "conversationId": params.conversation_id,
            "requestedPermissions": params.requested_permissions,
            "acceptanceExamples": params.acceptance_examples,
            "status": status,
            "sourceDigest": build.as_ref().map(|value| value.source_digest.as_str()),
            "sourceFileCount": build.as_ref().map_or(0, |value| value.source_file_count),
            "packageDigest": build.as_ref().map(|value| value.package_digest.as_str()),
            "stateRevision": 1,
            "createdAt": now,
        });
        if let Err(error) = write_project_metadata(&staging, &metadata) {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        if fs::rename(&staging, &final_path).is_err() {
            let _ = fs::remove_dir_all(&staging);
            return Err(ServiceError::new("capability_creation_project_path_failed"));
        }

        let command = create_command(
            params,
            creation_project_id.clone(),
            managed_relative_path,
            title,
            business_goal,
            build.as_ref(),
            now,
        )?;

        let created = self
            .persistence_mut()?
            .create_capability_creation_project(&command);
        match created {
            Ok(project) => Ok(CapabilityCreationCreateResult {
                project: project_from_record(project)?,
                replayed: false,
            }),
            Err(error) => {
                let _ = fs::remove_dir_all(&final_path);
                Err(persistence_error(error))
            }
        }
    }

    pub(super) fn capability_creation_list(
        &mut self,
        _: &CapabilityCreationListParams,
        _: i64,
    ) -> Result<CapabilityCreationListResult, ServiceError> {
        let records = self
            .persistence()?
            .list_capability_creation_projects()
            .map_err(persistence_error)?;
        let mut projects = Vec::new();
        for record in records {
            if self
                .persistence()?
                .enterprise_capability_binding(&record.creation_project_id)
                .map_err(persistence_error)?
                .is_none()
            {
                projects.push(project_from_record(record)?);
            }
        }
        Ok(CapabilityCreationListResult { projects })
    }

    pub(super) fn capability_creation_get(
        &mut self,
        params: &CapabilityCreationGetParams,
        _: i64,
    ) -> Result<CapabilityCreationGetResult, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        let project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        Ok(CapabilityCreationGetResult {
            project: project_from_record(project)?,
        })
    }

    pub(super) fn capability_creation_preview(
        &mut self,
        params: &CapabilityCreationPreviewParams,
        _: i64,
    ) -> Result<CapabilityCreationPreviewResult, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        let project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        if !matches!(
            project.target_type.as_str(),
            "web_application" | "web_mcp_application"
        ) || !matches!(project.status.as_str(), "packaged" | "installed")
        {
            return Err(ServiceError::new("capability_creation_preview_unavailable"));
        }
        let expected_digest = project
            .source_digest
            .as_deref()
            .ok_or_else(|| ServiceError::new("capability_creation_preview_unavailable"))?;
        let project_root = self.personal_source_root(&project)?;
        let files = read_source_files(&project_root.join("source"))?;
        let inputs = files
            .iter()
            .map(|(path, bytes)| {
                Ok(CapabilityCreationSourceFile {
                    path: path.clone(),
                    content: String::from_utf8(bytes.clone())
                        .map_err(|_| ServiceError::new("capability_creation_source_invalid"))?,
                })
            })
            .collect::<Result<Vec<_>, ServiceError>>()?;
        validate_source_files(&project.target_type, &inputs)?;
        let actual_digest = format!("sha256:{}", digest_files(&files));
        if actual_digest != expected_digest {
            return Err(ServiceError::new(
                "capability_creation_source_digest_mismatch",
            ));
        }
        let html = inputs
            .into_iter()
            .find(|file| file.path == "web/index.html")
            .map(|file| file.content)
            .ok_or_else(|| ServiceError::new("capability_creation_source_entry_missing"))?;
        let content_digest = format!("sha256:{:x}", Sha256::digest(html.as_bytes()));
        Ok(CapabilityCreationPreviewResult {
            creation_project_id: project.creation_project_id,
            title: project.title,
            html,
            content_digest,
        })
    }

    #[cfg(test)]
    fn replayed_capability_install(
        &mut self,
        params: &CapabilityCreationInstallParams,
    ) -> Result<Option<CapabilityCreationInstallResult>, ServiceError> {
        let Some(operation) = self
            .persistence()?
            .capability_creation_operation(&params.client_operation_id)
            .map_err(persistence_error)?
        else {
            return Ok(None);
        };
        if operation.operation_kind != "install"
            || operation.request_fingerprint != params.request_fingerprint
            || operation.creation_project_id != params.creation_project_id
        {
            return Err(ServiceError::new("operation_id_reused"));
        }
        let project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        Ok(Some(CapabilityCreationInstallResult {
            project: project_from_record(project)?,
            replayed: true,
        }))
    }

    #[cfg(test)]
    fn verified_personal_package(
        &mut self,
        params: &CapabilityCreationInstallParams,
    ) -> Result<(CapabilityCreationProjectRecord, Vec<u8>), ServiceError> {
        let project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        if !ALLOWED_TARGET_TYPES.contains(&project.target_type.as_str())
            || project.status != "packaged"
            || project.state_revision != params.expected_revision
            || project.package_digest.as_deref() != Some(params.package_digest.as_str())
        {
            return Err(ServiceError::new(
                "capability_creation_install_state_conflict",
            ));
        }
        let build = self
            .persistence()?
            .capability_creation_build(&params.creation_project_id, &params.package_digest)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_build_not_found"))?;
        if build.source_digest != project.source_digest.as_deref().unwrap_or_default() {
            return Err(ServiceError::new("capability_creation_build_invalid"));
        }
        let package_path = self
            .capability_projects_root()?
            .join(&params.creation_project_id)
            .join(&build.package_relative_path);
        reject_symlink(&package_path)?;
        let package_bytes = fs::read(&package_path)
            .map_err(|_| ServiceError::new("capability_creation_package_unavailable"))?;
        let actual_package_digest = format!("sha256:{:x}", Sha256::digest(&package_bytes));
        if actual_package_digest != params.package_digest {
            return Err(ServiceError::new(
                "capability_creation_package_digest_mismatch",
            ));
        }
        Ok((project, package_bytes))
    }

    #[cfg(test)]
    fn capability_creation_install(
        &mut self,
        params: &CapabilityCreationInstallParams,
        now: i64,
    ) -> Result<CapabilityCreationInstallResult, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        self.ensure_personal_capability(&params.creation_project_id)?;
        validate_digest(
            &params.package_digest,
            "capability_creation_package_digest_invalid",
        )?;
        validate_text(
            &params.client_operation_id,
            160,
            "client_operation_id_invalid",
        )?;
        validate_fingerprint(&params.request_fingerprint)?;
        if params.request_fingerprint != capability_install_request_fingerprint(params) {
            return Err(ServiceError::new("request_fingerprint_invalid"));
        }
        if params.expected_revision < 1 {
            return Err(ServiceError::new(
                "capability_creation_expected_revision_invalid",
            ));
        }
        if let Some(result) = self.replayed_capability_install(params)? {
            return Ok(result);
        }
        let (project, package_bytes) = self.verified_personal_package(params)?;
        let install = materialize_personal_web_install(
            &self.personal_capabilities_root()?,
            params,
            &project,
            &package_bytes,
            now,
        )?;
        let command = CapabilityCreationInstallCommand {
            client_operation_id: params.client_operation_id.clone(),
            request_fingerprint: params.request_fingerprint.clone(),
            creation_project_id: params.creation_project_id.clone(),
            expected_revision: params.expected_revision,
            package_digest: params.package_digest.clone(),
            installed_relative_path: install.relative_location,
            occurred_at: now,
        };
        match self
            .persistence_mut()?
            .install_capability_creation_project(&command)
        {
            Ok((record, replayed)) => Ok(CapabilityCreationInstallResult {
                project: project_from_record(record)?,
                replayed,
            }),
            Err(error) => Err(persistence_error(error)),
        }
    }

    pub(super) fn personal_application_open(
        &mut self,
        params: &PersonalApplicationOpenParams,
        _now: i64,
    ) -> Result<PersonalApplicationOpenResult, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        self.ensure_personal_capability(&params.creation_project_id)?;
        let project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        let installation = self
            .persistence()?
            .personal_capability_installation(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("personal_capability_not_installed"))?;
        if project.status != "installed"
            || !matches!(
                project.target_type.as_str(),
                "web_application" | "web_mcp_application"
            )
            || !installation.enabled
            || project.package_digest.as_deref() != Some(installation.package_digest.as_str())
        {
            return Err(ServiceError::new("personal_capability_not_ready"));
        }
        let expected_relative = format!(
            "personal-capabilities/{}/{}",
            params.creation_project_id,
            installation.package_digest.trim_start_matches("sha256:")
        );
        if installation.installed_relative_path != expected_relative {
            return Err(ServiceError::new(
                "personal_capability_install_path_invalid",
            ));
        }
        let install_path =
            self.personal_install_path(&params.creation_project_id, &installation.package_digest)?;
        validate_existing_install(
            &install_path,
            &params.creation_project_id,
            &installation.package_digest,
            project.source_digest.as_deref().unwrap_or_default(),
        )?;
        let files = validate_installed_source(
            &install_path,
            &project.target_type,
            project.source_digest.as_deref().unwrap_or_default(),
        )?;
        let html = String::from_utf8(
            files
                .get("web/index.html")
                .cloned()
                .ok_or_else(|| ServiceError::new("capability_creation_source_entry_missing"))?,
        )
        .map_err(|_| ServiceError::new("capability_creation_source_invalid"))?;
        let content_digest = format!("sha256:{:x}", Sha256::digest(html.as_bytes()));
        Ok(PersonalApplicationOpenResult {
            creation_project_id: project.creation_project_id,
            title: project.title,
            html,
            content_digest,
            installation_revision: installation.state_revision,
        })
    }

    pub(super) fn personal_capability_status(
        &mut self,
        params: &PersonalApplicationOpenParams,
        _: i64,
    ) -> Result<serde_json::Value, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        let project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        let installation = self
            .persistence()?
            .personal_capability_installation(&params.creation_project_id)
            .map_err(persistence_error)?;
        Ok(
            json!({"project":project_from_record(project)?,"enabled":installation.map(|value|value.enabled)}),
        )
    }

    pub(super) fn personal_capability_manage(
        &mut self,
        params: &PersonalCapabilityManageParams,
        now: i64,
    ) -> Result<serde_json::Value, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        validate_digest(
            &params.package_digest,
            "capability_creation_package_digest_invalid",
        )?;
        validate_text(
            &params.client_operation_id,
            160,
            "client_operation_id_invalid",
        )?;
        if params.expected_revision < 1
            || !matches!(params.action.as_str(), "enable" | "disable" | "uninstall")
        {
            return Err(ServiceError::new("personal_capability_action_invalid"));
        }
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(params)
                    .map_err(|_| ServiceError::new("personal_capability_action_invalid"))?
            )
        );
        let replayed = self
            .persistence_mut()?
            .manage_personal_capability(&PersonalCapabilityManageCommand {
                client_operation_id: params.client_operation_id.clone(),
                request_fingerprint: fingerprint,
                creation_project_id: params.creation_project_id.clone(),
                expected_revision: params.expected_revision,
                package_digest: params.package_digest.clone(),
                action: params.action.clone(),
                occurred_at: now,
            })
            .map_err(persistence_error)?;
        let mut result = self.personal_capability_status(
            &PersonalApplicationOpenParams {
                creation_project_id: params.creation_project_id.clone(),
            },
            now,
        )?;
        result["replayed"] = json!(replayed);
        Ok(result)
    }

    pub(super) fn personal_package_export(
        &mut self,
        params: &PersonalApplicationOpenParams,
        _: i64,
    ) -> Result<serde_json::Value, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        if self
            .persistence()?
            .enterprise_capability_binding(&params.creation_project_id)
            .map_err(persistence_error)?
            .is_some()
        {
            return Err(ServiceError::new("enterprise_capability_not_personal"));
        }
        let project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        let (project, version) = self
            .persistence()?
            .personal_capability_versions(&params.creation_project_id)
            .map_err(persistence_error)?
            .into_iter()
            .next()
            .map_or((project, "0.1.0".to_owned()), |version| {
                (version.project, version.version)
            });
        if !matches!(project.status.as_str(), "packaged" | "installed") {
            return Err(ServiceError::new("capability_creation_package_unavailable"));
        }
        let package_digest = project
            .package_digest
            .as_deref()
            .ok_or_else(|| ServiceError::new("capability_creation_package_unavailable"))?;
        let build = self
            .persistence()?
            .capability_creation_build(&params.creation_project_id, package_digest)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_build_not_found"))?;
        let root = self
            .capability_projects_root()?
            .join(&params.creation_project_id)
            .canonicalize()
            .map_err(|_| ServiceError::new("capability_creation_package_unavailable"))?;
        let path = root
            .join(&build.package_relative_path)
            .canonicalize()
            .map_err(|_| ServiceError::new("capability_creation_package_unavailable"))?;
        if !path.starts_with(root) {
            return Err(ServiceError::new(
                "capability_creation_package_scope_invalid",
            ));
        }
        let bytes = fs::read(path)
            .map_err(|_| ServiceError::new("capability_creation_package_unavailable"))?;
        if format!("sha256:{:x}", Sha256::digest(&bytes)) != package_digest {
            return Err(ServiceError::new(
                "capability_creation_package_digest_mismatch",
            ));
        }
        validate_personal_export(&bytes, &project, &version)?;
        Ok(
            json!({"bytes":bytes,"packageDigest":package_digest,"format":"cilai-personal-package","formatVersion":1}),
        )
    }

    pub(super) fn personal_capability_runtime(
        &mut self,
        params: &PersonalApplicationOpenParams,
        now: i64,
    ) -> Result<serde_json::Value, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        self.ensure_personal_capability(&params.creation_project_id)?;
        let project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        let installation = self
            .persistence()?
            .personal_capability_installation(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("personal_capability_not_installed"))?;
        if project.status != "installed"
            || !installation.enabled
            || !matches!(
                project.target_type.as_str(),
                "local_mcp" | "web_mcp_application"
            )
            || project.package_digest.as_deref() != Some(&installation.package_digest)
        {
            return Err(ServiceError::new("personal_capability_not_ready"));
        }
        let relative = format!(
            "personal-capabilities/{}/{}",
            params.creation_project_id,
            installation.package_digest.trim_start_matches("sha256:")
        );
        if installation.installed_relative_path != relative {
            return Err(ServiceError::new(
                "personal_capability_install_path_invalid",
            ));
        }
        let profile = self
            .data_dir
            .as_ref()
            .ok_or_else(|| ServiceError::new("data_dir_unavailable"))?;
        let package =
            self.personal_install_path(&params.creation_project_id, &installation.package_digest)?;
        validate_existing_install(
            &package,
            &params.creation_project_id,
            &installation.package_digest,
            project.source_digest.as_deref().unwrap_or_default(),
        )?;
        validate_installed_source(
            &package,
            &project.target_type,
            project.source_digest.as_deref().unwrap_or_default(),
        )?;
        let harness_home = profile.join("ai-runtime/cilai-harness");
        let data = self.personal_application_data_runtime_root(
            &params.creation_project_id,
            &installation.package_digest,
            now,
        )?;
        Ok(
            json!({"packageRoot":package,"dataRoot":data,"harnessHome":harness_home,"packageDigest":installation.package_digest,"revision":installation.state_revision}),
        )
    }

    fn personal_install_path(
        &self,
        project_id: &str,
        package_digest: &str,
    ) -> Result<PathBuf, ServiceError> {
        let root = self.personal_capabilities_root()?;
        let project = root.join(project_id);
        reject_symlink(&project)?;
        let package = project.join(package_digest.trim_start_matches("sha256:"));
        reject_symlink(&package)?;
        Ok(package)
    }

    fn capability_projects_root(&self) -> Result<PathBuf, ServiceError> {
        let root = self
            .data_dir
            .as_ref()
            .map(|path| path.join("capability-projects"))
            .ok_or_else(|| ServiceError::new("data_dir_unavailable"))?;
        if fs::symlink_metadata(&root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(ServiceError::new(
                "capability_creation_project_path_invalid",
            ));
        }
        Ok(root)
    }

    fn personal_capabilities_root(&self) -> Result<PathBuf, ServiceError> {
        let root = self
            .data_dir
            .as_ref()
            .map(|path| path.join("personal-capabilities"))
            .ok_or_else(|| ServiceError::new("data_dir_unavailable"))?;
        if fs::symlink_metadata(&root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(ServiceError::new(
                "personal_capability_install_path_invalid",
            ));
        }
        Ok(root)
    }
}

fn create_command(
    params: &CapabilityCreationCreateParams,
    creation_project_id: String,
    managed_relative_path: String,
    title: String,
    business_goal: String,
    build: Option<&MaterializedBuild>,
    now: i64,
) -> Result<CapabilityCreationCreateCommand, ServiceError> {
    let requested_permissions_json = serde_json::to_string(&params.requested_permissions)
        .map_err(|_| ServiceError::new("capability_creation_project_invalid"))?;
    let acceptance_examples_json = serde_json::to_string(&params.acceptance_examples)
        .map_err(|_| ServiceError::new("capability_creation_project_invalid"))?;
    let build_record = build.map(|value| CapabilityCreationBuildRecord {
        build_id: format!("build_{}", Uuid::new_v4().simple()),
        creation_project_id: creation_project_id.clone(),
        source_digest: value.source_digest.clone(),
        package_digest: value.package_digest.clone(),
        package_relative_path: value.package_relative_path.clone(),
        file_count: value.source_file_count,
        total_bytes: value.total_bytes,
        created_at: now,
    });
    Ok(CapabilityCreationCreateCommand {
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        creation_project_id,
        title,
        business_goal,
        target_type: params.target_type.clone(),
        conversation_id: params.conversation_id.clone(),
        managed_relative_path,
        requested_permissions_json,
        acceptance_examples_json,
        initial_status: if build.is_some() {
            "packaged".to_owned()
        } else {
            "planned".to_owned()
        },
        source_digest: build.map(|value| value.source_digest.clone()),
        source_file_count: build.map_or(0, |value| value.source_file_count),
        package_digest: build.map(|value| value.package_digest.clone()),
        build: build_record,
        occurred_at: now,
    })
}

fn materialize_personal_web_install(
    install_root: &Path,
    params: &CapabilityCreationInstallParams,
    project: &CapabilityCreationProjectRecord,
    package_bytes: &[u8],
    now: i64,
) -> Result<MaterializedPersonalInstall, ServiceError> {
    fs::create_dir_all(install_root)
        .map_err(|_| ServiceError::new("personal_capability_install_path_failed"))?;
    reject_symlink(install_root)?;
    let project_install_root = install_root.join(&params.creation_project_id);
    fs::create_dir_all(&project_install_root)
        .map_err(|_| ServiceError::new("personal_capability_install_path_failed"))?;
    reject_symlink(&project_install_root)?;
    let digest_segment = params.package_digest.trim_start_matches("sha256:");
    let final_path = project_install_root.join(digest_segment);
    if final_path.exists() {
        validate_existing_install(
            &final_path,
            &params.creation_project_id,
            &params.package_digest,
            project.source_digest.as_deref().unwrap_or_default(),
        )?;
        return Ok(MaterializedPersonalInstall {
            final_directory: final_path,
            relative_location: format!(
                "personal-capabilities/{}/{digest_segment}",
                params.creation_project_id
            ),
        });
    }

    let staging = project_install_root.join(format!(
        ".{digest_segment}.staging-{}",
        Uuid::new_v4().simple()
    ));
    fs::create_dir(&staging)
        .map_err(|_| ServiceError::new("personal_capability_install_path_failed"))?;
    let installed = unpack_personal_package(package_bytes, &staging).and_then(|()| {
        validate_installed_source(
            &staging,
            &project.target_type,
            project.source_digest.as_deref().unwrap_or_default(),
        )?;
        let metadata = json!({
            "schemaVersion": "1.0",
            "creationProjectId": params.creation_project_id,
            "packageDigest": params.package_digest,
            "sourceDigest": project.source_digest,
            "installedAt": now,
        });
        write_new_file(
            &staging.join(".cilai-personal-install.json"),
            &encode_json(&metadata)?,
            "personal_capability_install_metadata_failed",
        )?;
        fs::rename(&staging, &final_path)
            .map_err(|_| ServiceError::new("personal_capability_install_commit_failed"))
    });
    if let Err(error) = installed {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    Ok(MaterializedPersonalInstall {
        final_directory: final_path,
        relative_location: format!(
            "personal-capabilities/{}/{digest_segment}",
            params.creation_project_id
        ),
    })
}

fn validate_create(params: &CapabilityCreationCreateParams) -> Result<(), ServiceError> {
    validate_text(&params.title, 80, "capability_creation_title_invalid")?;
    validate_text(
        &params.business_goal,
        2_000,
        "capability_creation_business_goal_invalid",
    )?;
    if !ALLOWED_TARGET_TYPES.contains(&params.target_type.as_str()) {
        return Err(ServiceError::new("capability_creation_target_type_invalid"));
    }
    if params
        .conversation_id
        .as_ref()
        .is_some_and(|value| value.trim().is_empty() || value.len() > 160)
    {
        return Err(ServiceError::new(
            "capability_creation_conversation_invalid",
        ));
    }
    pages::validate_permissions(&params.target_type, &params.requested_permissions)?;
    if !params.source_files.is_empty() {
        pages::from_files(
            &params.target_type,
            &params.requested_permissions,
            &params.source_files,
        )?;
    }
    if params.acceptance_examples.is_empty() || params.acceptance_examples.len() > 20 {
        return Err(ServiceError::new(
            "capability_creation_acceptance_examples_invalid",
        ));
    }
    for example in &params.acceptance_examples {
        validate_acceptance_example(example)?;
    }
    validate_text(
        &params.client_operation_id,
        160,
        "client_operation_id_invalid",
    )?;
    validate_fingerprint(&params.request_fingerprint)?;
    validate_source_files(&params.target_type, &params.source_files)?;
    if params.request_fingerprint != capability_creation_request_fingerprint(params) {
        return Err(ServiceError::new("request_fingerprint_invalid"));
    }
    Ok(())
}

fn validate_fingerprint(value: &str) -> Result<(), ServiceError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ServiceError::new("request_fingerprint_invalid"));
    }
    Ok(())
}

fn validate_digest(value: &str, reason: &'static str) -> Result<(), ServiceError> {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return Err(ServiceError::new(reason));
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ServiceError::new(reason));
    }
    Ok(())
}

fn validate_source_files(
    target_type: &str,
    files: &[CapabilityCreationSourceFile],
) -> Result<(), ServiceError> {
    if files.is_empty() {
        return Ok(());
    }
    if files.len() > MAX_SOURCE_FILES {
        return Err(ServiceError::new("capability_creation_source_too_large"));
    }
    let mut paths = HashSet::new();
    let mut folded_paths = HashSet::new();
    let mut total_bytes = 0_usize;
    for file in files {
        validate_source_path(&file.path)?;
        if !paths.insert(file.path.as_str()) || !folded_paths.insert(file.path.to_ascii_lowercase())
        {
            return Err(ServiceError::new(
                "capability_creation_source_path_conflict",
            ));
        }
        let size = file.content.len();
        if size > MAX_SOURCE_FILE_BYTES || file.content.contains('\0') {
            return Err(ServiceError::new("capability_creation_source_too_large"));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .filter(|value| *value <= MAX_SOURCE_TOTAL_BYTES)
            .ok_or_else(|| ServiceError::new("capability_creation_source_too_large"))?;
        validate_source_kind(target_type, file)?;
    }

    let has_web = paths.contains("web/index.html");
    let has_mcp = paths.contains("mcp/server.mjs");
    let required_entries_present = match target_type {
        "web_application" => has_web && !has_mcp,
        "local_mcp" => has_mcp && !has_web,
        "web_mcp_application" => has_web && has_mcp,
        _ => false,
    };
    if !required_entries_present {
        return Err(ServiceError::new(
            "capability_creation_source_entry_missing",
        ));
    }
    Ok(())
}

fn validate_source_path(path: &str) -> Result<(), ServiceError> {
    if path.is_empty()
        || path.len() > 240
        || path.starts_with('/')
        || path.starts_with('.')
        || path.contains('\\')
        || path.contains("//")
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-'))
    {
        return Err(ServiceError::new("capability_creation_source_path_invalid"));
    }
    Ok(())
}

fn validate_source_kind(
    target_type: &str,
    file: &CapabilityCreationSourceFile,
) -> Result<(), ServiceError> {
    let path = file.path.as_str();
    let allowed = if path == "README.md" {
        true
    } else if let Some(relative) = path.strip_prefix("web/") {
        target_type != "local_mcp"
            && ["html", "css", "js", "mjs", "json", "svg", "txt", "md"]
                .iter()
                .any(|extension| relative.ends_with(&format!(".{extension}")))
    } else if let Some(relative) = path.strip_prefix("mcp/") {
        target_type != "web_application"
            && ["js", "mjs", "json", "md"]
                .iter()
                .any(|extension| relative.ends_with(&format!(".{extension}")))
    } else {
        false
    };
    if !allowed {
        return Err(ServiceError::new("capability_creation_source_path_invalid"));
    }
    if path.starts_with("mcp/") {
        let source = file.content.to_ascii_lowercase();
        for forbidden in [
            "node:child_process",
            "node:cluster",
            "node:dgram",
            "node:dns",
            "node:http",
            "node:https",
            "node:net",
            "node:tls",
            "node:worker_threads",
            "process.binding",
            "process.dlopen",
        ] {
            if source.contains(forbidden) {
                return Err(ServiceError::new(
                    "capability_creation_source_runtime_forbidden",
                ));
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_personal_manifests(
    creation_project_id: &str,
    title: &str,
    business_goal: &str,
    target_type: &str,
    requested_permissions: &[String],
    source_digest: &str,
    version: &str,
    page_ai: Option<&contexthub_protocol::PersonalPageAiManifest>,
) -> Result<BTreeMap<String, Vec<u8>>, ServiceError> {
    let plugin_id = format!(
        "personal.{}",
        creation_project_id.trim_start_matches("capability_")
    );
    let app_id = format!("{plugin_id}.app");
    let has_web = target_type != "local_mcp";
    let has_mcp = target_type != "web_application";
    let provided_capabilities = personal_provided_capabilities(
        creation_project_id,
        target_type,
        requested_permissions,
        version,
    );
    let summary = business_goal.chars().take(200).collect::<String>();
    let mut application_manifest = json!({
        "schemaVersion": "personal-1.0",
        "appId": app_id,
        "name": title,
        "summary": summary,
        "entrypoint": has_web.then_some("source/web/index.html"),
        "mcpComponentId": has_mcp.then_some("primary"),
        "requestedPermissions": requested_permissions,
    });
    if let Some(declaration) = page_ai {
        application_manifest["pageAi"] = serde_json::to_value(declaration)
            .map_err(|_| ServiceError::new("personal_page_manifest_invalid"))?;
    }
    let mut manifests = BTreeMap::new();
    manifests.insert(
        "generated/personal-application.json".to_owned(),
        encode_json(&application_manifest)?,
    );
    if has_mcp {
        let runtime_manifest = json!({
            "schemaVersion": "1.0",
            "componentId": "primary",
            "componentType": "stdio_mcp",
            "runtime": "managed_node",
            "entrypoint": "source/mcp/server.mjs",
            "arguments": [],
            "workingDirectory": "component",
            "transport": "mcp_stdio",
            "startPolicy": if has_web { "application_open" } else { "on_demand" },
            "readiness": { "kind": "mcp_initialize", "timeoutSeconds": 10 },
            "shutdown": { "kind": "protocol_then_terminate", "graceSeconds": 3 },
            "permissions": {
                "fileAccess": if requested_permissions.iter().any(|value| value.starts_with("personal_file.")) { "workbench_brokered" } else { "none" },
                "networkAccess": false,
                "subprocess": false
            },
            "limits": { "memoryMb": 128, "maxProcesses": 1, "idleTimeoutSeconds": 300 },
            "runtimeDigest": source_digest,
        });
        manifests.insert(
            "generated/local-runtime.json".to_owned(),
            encode_json(&runtime_manifest)?,
        );
    }
    let mut plugin_manifest = json!({
        "schemaVersion": "1.1",
        "pluginId": plugin_id,
        "version": version,
        "publisherId": "personal.local",
        "name": title,
        "summary": summary,
        "category": "personal_productivity",
        "distributionScope": "personal",
        "minimumWorkbenchVersion": "0.1.0",
        "applications": if has_web { vec![json!({ "appId": app_id, "manifest": application_manifest })] } else { Vec::new() },
        "runtimeComponents": if has_mcp { vec![json!({ "componentId": "primary", "runtimeManifest": "generated/local-runtime.json", "required": true })] } else { Vec::new() },
        "provides": provided_capabilities,
        "dependencies": [],
        "dataResponsibility": { "owns": [], "references": [] },
        "uninstall": { "remove": ["runtime_cache"], "retain": ["user_data"] },
        "sourceDigest": source_digest,
    });
    let manifest_digest = format!(
        "sha256:{:x}",
        Sha256::digest(
            serde_json::to_vec(&plugin_manifest)
                .map_err(|_| ServiceError::new("capability_creation_manifest_invalid"))?
        )
    );
    plugin_manifest["manifestDigest"] = serde_json::Value::String(manifest_digest);
    manifests.insert(
        "generated/plugin-manifest.json".to_owned(),
        encode_json(&plugin_manifest)?,
    );
    Ok(manifests)
}

fn personal_provided_capabilities(
    creation_project_id: &str,
    target_type: &str,
    requested_permissions: &[String],
    version: &str,
) -> Vec<serde_json::Value> {
    let base = format!(
        "personal.{}",
        creation_project_id.trim_start_matches("capability_")
    );
    let capability = |code: String, kind: &str| {
        json!({
            "capabilityCode": code,
            "kind": kind,
            "version": version,
            "permissions": requested_permissions,
        })
    };
    match target_type {
        "web_mcp_application" => vec![
            capability(format!("{base}.web"), "web"),
            capability(format!("{base}.mcp"), "mcp"),
        ],
        "web_application" => vec![capability(base, "web")],
        _ => vec![capability(base, "mcp")],
    }
}

#[allow(clippy::too_many_arguments)]
fn materialize_source_package(
    staging: &Path,
    creation_project_id: &str,
    title: &str,
    business_goal: &str,
    target_type: &str,
    requested_permissions: &[String],
    files: &[CapabilityCreationSourceFile],
    version: &str,
) -> Result<MaterializedBuild, ServiceError> {
    validate_source_files(target_type, files)?;
    let mut source_files = BTreeMap::new();
    for file in files {
        source_files.insert(file.path.clone(), file.content.as_bytes().to_vec());
    }
    let source_digest = format!("sha256:{}", digest_files(&source_files));
    for (path, bytes) in &source_files {
        write_managed_file(&staging.join("source"), path, bytes)?;
    }
    let generated = generate_personal_manifests(
        creation_project_id,
        title,
        business_goal,
        target_type,
        requested_permissions,
        &source_digest,
        version,
        pages::from_files(target_type, requested_permissions, files)?.as_ref(),
    )?;
    for (path, bytes) in &generated {
        let relative = path
            .strip_prefix("generated/")
            .ok_or_else(|| ServiceError::new("capability_creation_manifest_invalid"))?;
        write_managed_file(&staging.join("generated"), relative, bytes)?;
    }

    let mut package_files = BTreeMap::new();
    for (path, bytes) in source_files {
        package_files.insert(format!("source/{path}"), bytes);
    }
    package_files.extend(generated);
    let package_bytes = encode_package(&package_files)?;
    let package_digest = format!("sha256:{:x}", Sha256::digest(&package_bytes));
    let artifacts = staging.join("artifacts");
    fs::create_dir(&artifacts)
        .map_err(|_| ServiceError::new("capability_creation_package_write_failed"))?;
    let package_name = format!("{}.zip", package_digest.trim_start_matches("sha256:"));
    let package_path = artifacts.join(&package_name);
    write_new_file(
        &package_path,
        &package_bytes,
        "capability_creation_package_write_failed",
    )?;
    Ok(MaterializedBuild {
        source_digest,
        package_digest,
        package_relative_path: format!("artifacts/{package_name}"),
        source_file_count: i64::try_from(files.len())
            .map_err(|_| ServiceError::new("capability_creation_source_too_large"))?,
        total_bytes: i64::try_from(package_bytes.len())
            .map_err(|_| ServiceError::new("capability_creation_package_too_large"))?,
    })
}

fn digest_files(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut digest = Sha256::new();
    for (path, bytes) in files {
        digest.update(path.as_bytes());
        digest.update([0]);
        digest.update(bytes);
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn encode_json(value: &serde_json::Value) -> Result<Vec<u8>, ServiceError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| ServiceError::new("capability_creation_manifest_invalid"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn encode_package(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, ServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut archive = ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o600);
        for (path, bytes) in files {
            archive
                .start_file(path, options)
                .and_then(|()| archive.write_all(bytes).map_err(zip::result::ZipError::Io))
                .map_err(|_| ServiceError::new("capability_creation_package_write_failed"))?;
        }
        archive
            .finish()
            .map_err(|_| ServiceError::new("capability_creation_package_write_failed"))?;
    }
    Ok(cursor.into_inner())
}

fn write_managed_file(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), ServiceError> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| ServiceError::new("capability_creation_source_write_failed"))?;
    }
    write_new_file(&path, bytes, "capability_creation_source_write_failed")
}

fn read_source_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, ServiceError> {
    reject_symlink(root)?;
    if !root.is_dir() {
        return Err(ServiceError::new("capability_creation_source_unavailable"));
    }
    let mut files = BTreeMap::new();
    read_source_directory(root, root, &mut files)?;
    Ok(files)
}

fn validate_personal_export(
    bytes: &[u8],
    project: &CapabilityCreationProjectRecord,
    version: &str,
) -> Result<(), ServiceError> {
    let invalid = || ServiceError::new("personal_package_export_invalid");
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| invalid())?;
    if archive.is_empty() || archive.len() > MAX_SOURCE_FILES + 3 {
        return Err(invalid());
    }
    let mut files = BTreeMap::new();
    let mut names = HashSet::new();
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| invalid())?;
        if entry.is_dir()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
            || entry.size() > MAX_SOURCE_FILE_BYTES as u64
        {
            return Err(invalid());
        }
        total = total
            .checked_add(entry.size())
            .filter(|size| *size <= 2 * 1024 * 1024)
            .ok_or_else(invalid)?;
        let name = entry
            .enclosed_name()
            .ok_or_else(invalid)?
            .to_string_lossy()
            .replace('\\', "/");
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(invalid());
        }
        let mut content = Vec::new();
        entry
            .take(MAX_SOURCE_FILE_BYTES as u64 + 1)
            .read_to_end(&mut content)
            .map_err(|_| invalid())?;
        if content.len() > MAX_SOURCE_FILE_BYTES {
            return Err(invalid());
        }
        files.insert(name, content);
    }
    let source = files
        .iter()
        .filter_map(|(name, content)| {
            name.strip_prefix("source/")
                .map(|path| (path.to_owned(), content.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let inputs = source
        .iter()
        .map(|(path, content)| {
            Ok(CapabilityCreationSourceFile {
                path: path.clone(),
                content: String::from_utf8(content.clone()).map_err(|_| invalid())?,
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    validate_source_files(&project.target_type, &inputs)?;
    let source_digest = format!("sha256:{}", digest_files(&source));
    if project.source_digest.as_deref() != Some(source_digest.as_str()) {
        return Err(invalid());
    }
    let permissions: Vec<String> =
        serde_json::from_str(&project.requested_permissions_json).map_err(|_| invalid())?;
    let mut expected = generate_personal_manifests(
        &project.creation_project_id,
        &project.title,
        &project.business_goal,
        &project.target_type,
        &permissions,
        &source_digest,
        version,
        pages::declaration(
            &project.target_type,
            &permissions,
            source.get(pages::SOURCE_PATH).map(Vec::as_slice),
        )?
        .as_ref(),
    )?;
    expected.extend(
        source
            .into_iter()
            .map(|(path, content)| (format!("source/{path}"), content)),
    );
    if files != expected {
        return Err(invalid());
    }
    Ok(())
}

fn unpack_personal_package(bytes: &[u8], destination: &Path) -> Result<(), ServiceError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| ServiceError::new("capability_creation_package_invalid"))?;
    if archive.len() > MAX_SOURCE_FILES + 8 {
        return Err(ServiceError::new("capability_creation_package_too_large"));
    }
    let mut total = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| ServiceError::new("capability_creation_package_invalid"))?;
        if entry.is_dir()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
        {
            return Err(ServiceError::new("capability_creation_package_invalid"));
        }
        total = total
            .checked_add(entry.size())
            .filter(|value| *value <= 2 * 1024 * 1024)
            .ok_or_else(|| ServiceError::new("capability_creation_package_too_large"))?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| ServiceError::new("capability_creation_package_invalid"))?;
        let relative_text = relative.to_string_lossy().replace('\\', "/");
        if !(relative_text.starts_with("source/") || relative_text.starts_with("generated/")) {
            return Err(ServiceError::new("capability_creation_package_invalid"));
        }
        let target = destination.join(relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|_| ServiceError::new("personal_capability_install_path_failed"))?;
        }
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(target)
            .map_err(|_| ServiceError::new("personal_capability_install_path_failed"))?;
        std::io::copy(&mut entry, &mut output)
            .and_then(|_| output.sync_all())
            .map_err(|_| ServiceError::new("personal_capability_install_path_failed"))?;
    }
    let manifest_path = destination.join("generated/plugin-manifest.json");
    reject_symlink(&manifest_path)?;
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(manifest_path)
            .map_err(|_| ServiceError::new("capability_creation_manifest_invalid"))?,
    )
    .map_err(|_| ServiceError::new("capability_creation_manifest_invalid"))?;
    if manifest["schemaVersion"] != "1.1" || manifest["distributionScope"] != "personal" {
        return Err(ServiceError::new("capability_creation_manifest_invalid"));
    }
    Ok(())
}

fn validate_installed_source(
    install_path: &Path,
    target_type: &str,
    expected_source_digest: &str,
) -> Result<BTreeMap<String, Vec<u8>>, ServiceError> {
    let files = read_source_files(&install_path.join("source"))?;
    let inputs = files
        .iter()
        .map(|(path, bytes)| {
            Ok(CapabilityCreationSourceFile {
                path: path.clone(),
                content: String::from_utf8(bytes.clone())
                    .map_err(|_| ServiceError::new("capability_creation_source_invalid"))?,
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    validate_source_files(target_type, &inputs)?;
    let digest = format!("sha256:{}", digest_files(&files));
    if digest != expected_source_digest {
        return Err(ServiceError::new(
            "capability_creation_source_digest_mismatch",
        ));
    }
    Ok(files)
}

fn validate_existing_install(
    install_path: &Path,
    creation_project_id: &str,
    package_digest: &str,
    source_digest: &str,
) -> Result<(), ServiceError> {
    reject_symlink(install_path)?;
    if !install_path.is_dir() {
        return Err(ServiceError::new(
            "personal_capability_install_path_invalid",
        ));
    }
    let metadata_path = install_path.join(".cilai-personal-install.json");
    reject_symlink(&metadata_path)?;
    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(metadata_path)
            .map_err(|_| ServiceError::new("personal_capability_install_metadata_invalid"))?,
    )
    .map_err(|_| ServiceError::new("personal_capability_install_metadata_invalid"))?;
    if metadata["schemaVersion"] != "1.0"
        || metadata["creationProjectId"] != creation_project_id
        || metadata["packageDigest"] != package_digest
        || metadata["sourceDigest"] != source_digest
    {
        return Err(ServiceError::new(
            "personal_capability_install_metadata_invalid",
        ));
    }
    Ok(())
}

fn read_source_directory(
    root: &Path,
    directory: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), ServiceError> {
    for entry in fs::read_dir(directory)
        .map_err(|_| ServiceError::new("capability_creation_source_unavailable"))?
    {
        let entry =
            entry.map_err(|_| ServiceError::new("capability_creation_source_unavailable"))?;
        let file_type = entry
            .file_type()
            .map_err(|_| ServiceError::new("capability_creation_source_unavailable"))?;
        if file_type.is_symlink() {
            return Err(ServiceError::new(
                "capability_creation_project_path_invalid",
            ));
        }
        if file_type.is_dir() {
            read_source_directory(root, &entry.path(), files)?;
            continue;
        }
        if !file_type.is_file() || files.len() >= MAX_SOURCE_FILES {
            return Err(ServiceError::new("capability_creation_source_invalid"));
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| ServiceError::new("capability_creation_project_path_invalid"))?
            .to_string_lossy()
            .replace('\\', "/");
        validate_source_path(&relative)?;
        let bytes = fs::read(path)
            .map_err(|_| ServiceError::new("capability_creation_source_unavailable"))?;
        if bytes.len() > MAX_SOURCE_FILE_BYTES {
            return Err(ServiceError::new("capability_creation_source_too_large"));
        }
        files.insert(relative, bytes);
    }
    let total = files
        .values()
        .try_fold(0_usize, |total, bytes| total.checked_add(bytes.len()))
        .ok_or_else(|| ServiceError::new("capability_creation_source_too_large"))?;
    if total > MAX_SOURCE_TOTAL_BYTES {
        return Err(ServiceError::new("capability_creation_source_too_large"));
    }
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8], reason: &'static str) -> Result<(), ServiceError> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| ServiceError::new(reason))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| ServiceError::new(reason))
}

fn validate_acceptance_example(
    example: &CapabilityCreationAcceptanceExample,
) -> Result<(), ServiceError> {
    validate_text(
        &example.input,
        1_000,
        "capability_creation_acceptance_examples_invalid",
    )?;
    validate_text(
        &example.expected,
        1_000,
        "capability_creation_acceptance_examples_invalid",
    )
}

fn validate_text(value: &str, max_length: usize, reason: &'static str) -> Result<(), ServiceError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > max_length || trimmed.contains('\0') {
        return Err(ServiceError::new(reason));
    }
    Ok(())
}

fn validate_project_id(project_id: &str) -> Result<(), ServiceError> {
    if !project_id.starts_with("capability_")
        || project_id.len() != "capability_".len() + 32
        || !project_id["capability_".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ServiceError::new("capability_creation_project_id_invalid"));
    }
    Ok(())
}

fn project_from_record(
    record: CapabilityCreationProjectRecord,
) -> Result<CapabilityCreationProject, ServiceError> {
    let requested_permissions = serde_json::from_str(&record.requested_permissions_json)
        .map_err(|_| ServiceError::new("capability_creation_project_invalid"))?;
    let acceptance_examples = serde_json::from_str(&record.acceptance_examples_json)
        .map_err(|_| ServiceError::new("capability_creation_project_invalid"))?;
    Ok(CapabilityCreationProject {
        creation_project_id: record.creation_project_id,
        title: record.title,
        business_goal: record.business_goal,
        target_type: record.target_type,
        status: record.status,
        conversation_id: record.conversation_id,
        managed_relative_path: record.managed_relative_path,
        requested_permissions,
        acceptance_examples,
        source_digest: record.source_digest,
        source_file_count: record.source_file_count,
        state_revision: record.state_revision,
        package_digest: record.package_digest,
        error_reason: record.error_reason,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn write_project_metadata(
    directory: &Path,
    metadata: &serde_json::Value,
) -> Result<(), ServiceError> {
    let path = directory.join("project.json");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| ServiceError::new("capability_creation_project_path_failed"))?;
    let mut encoded = serde_json::to_vec_pretty(metadata)
        .map_err(|_| ServiceError::new("capability_creation_project_invalid"))?;
    encoded.push(b'\n');
    file.write_all(&encoded)
        .and_then(|()| file.sync_all())
        .map_err(|_| ServiceError::new("capability_creation_project_path_failed"))
}

fn reject_symlink(path: &Path) -> Result<(), ServiceError> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(ServiceError::new(
            "capability_creation_project_path_invalid",
        ));
    }
    Ok(())
}

fn persistence_error(error: contexthub_persistence_sqlite::PersistenceError) -> ServiceError {
    let reason = error
        .reason()
        .unwrap_or("capability_creation_persistence_failed");
    drop(error);
    ServiceError::new(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_persistence_sqlite::SqlitePersistence;

    fn params(operation: &str, _: char) -> CapabilityCreationCreateParams {
        let mut params = CapabilityCreationCreateParams {
            title: "本地文件检查".to_owned(),
            business_goal: "读取员工选择的目录并返回问题清单".to_owned(),
            target_type: "local_mcp".to_owned(),
            conversation_id: Some("conversation-1".to_owned()),
            requested_permissions: vec!["personal_file.read".to_owned()],
            acceptance_examples: vec![CapabilityCreationAcceptanceExample {
                input: "选择包含待检查文件的目录".to_owned(),
                expected: "返回问题文件，不修改原文件".to_owned(),
            }],
            source_files: Vec::new(),
            client_operation_id: operation.to_owned(),
            request_fingerprint: String::new(),
        };
        params.request_fingerprint = capability_creation_request_fingerprint(&params);
        params
    }

    fn refresh_fingerprint(params: &mut CapabilityCreationCreateParams) {
        params.request_fingerprint = capability_creation_request_fingerprint(params);
    }

    fn install_params(
        project: &CapabilityCreationProject,
        client_operation_id: &str,
    ) -> CapabilityCreationInstallParams {
        let mut params = CapabilityCreationInstallParams {
            creation_project_id: project.creation_project_id.clone(),
            expected_revision: project.state_revision,
            package_digest: project.package_digest.clone().unwrap(),
            client_operation_id: client_operation_id.to_owned(),
            request_fingerprint: String::new(),
        };
        params.request_fingerprint = capability_install_request_fingerprint(&params);
        params
    }

    #[test]
    fn create_lists_reads_and_replays_project() {
        let root = std::env::temp_dir().join(format!(
            "cilai-capability-creation-service-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let persistence = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let request = params("create-project", 'a');
        let created = services.capability_creation_create(&request, 10).unwrap();
        assert!(!created.replayed);
        assert_eq!(created.project.status, "planned");
        assert!(root.join(&created.project.managed_relative_path).is_dir());
        assert!(
            root.join(&created.project.managed_relative_path)
                .join("project.json")
                .is_file()
        );
        let replayed = services.capability_creation_create(&request, 11).unwrap();
        assert!(replayed.replayed);
        assert_eq!(replayed.project, created.project);
        assert_eq!(
            services
                .capability_creation_list(&CapabilityCreationListParams {}, 11)
                .unwrap()
                .projects,
            vec![created.project.clone()]
        );
        assert_eq!(
            services
                .capability_creation_get(
                    &CapabilityCreationGetParams {
                        creation_project_id: created.project.creation_project_id.clone(),
                    },
                    11,
                )
                .unwrap()
                .project,
            created.project
        );
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unsupported_permission_and_operation_reuse() {
        let root = std::env::temp_dir().join(format!(
            "cilai-capability-creation-validation-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let persistence = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let mut invalid = params("invalid-project", 'b');
        invalid.requested_permissions = vec!["shell.execute".to_owned()];
        assert_eq!(
            services
                .capability_creation_create(&invalid, 10)
                .unwrap_err()
                .reason,
            "capability_creation_permissions_invalid"
        );

        let mut forged = params("forged-fingerprint", 'b');
        forged.request_fingerprint = "b".repeat(64);
        assert_eq!(
            services
                .capability_creation_create(&forged, 10)
                .unwrap_err()
                .reason,
            "request_fingerprint_invalid"
        );

        let original = params("reused-operation", 'c');
        services.capability_creation_create(&original, 10).unwrap();
        let mut conflicting = original;
        conflicting.business_goal.push_str("，并生成摘要");
        refresh_fingerprint(&mut conflicting);
        assert_eq!(
            services
                .capability_creation_create(&conflicting, 11)
                .unwrap_err()
                .reason,
            "operation_id_reused"
        );
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn projects_are_isolated_by_profile_storage() {
        let base = std::env::temp_dir().join(format!(
            "cilai-capability-creation-profiles-{}",
            Uuid::new_v4()
        ));
        let first_root = base.join("first");
        let second_root = base.join("second");
        fs::create_dir_all(&first_root).unwrap();
        fs::create_dir_all(&second_root).unwrap();
        let first_persistence =
            SqlitePersistence::open(&first_root.join("contexthub.db"), "test").unwrap();
        let second_persistence =
            SqlitePersistence::open(&second_root.join("contexthub.db"), "test").unwrap();
        let mut first =
            AppServerServices::with_data_dir(Some(first_persistence), Some(first_root.clone()));
        let mut second =
            AppServerServices::with_data_dir(Some(second_persistence), Some(second_root.clone()));
        let mut request = params("profile-one", 'e');
        request.target_type = "web_application".to_owned();
        request.requested_permissions.clear();
        request.source_files = vec![CapabilityCreationSourceFile {
            path: "web/index.html".to_owned(),
            content: "<!doctype html><h1>Profile One</h1>".to_owned(),
        }];
        refresh_fingerprint(&mut request);
        let created = first.capability_creation_create(&request, 10).unwrap();
        let install = install_params(&created.project, "profile-one-install");
        first.capability_creation_install(&install, 11).unwrap();
        assert_eq!(
            first
                .capability_creation_list(&CapabilityCreationListParams {}, 10)
                .unwrap()
                .projects
                .len(),
            1
        );
        assert!(
            second
                .capability_creation_list(&CapabilityCreationListParams {}, 10)
                .unwrap()
                .projects
                .is_empty()
        );
        assert_eq!(
            second
                .personal_application_open(
                    &PersonalApplicationOpenParams {
                        creation_project_id: created.project.creation_project_id,
                    },
                    12,
                )
                .unwrap_err()
                .reason,
            "capability_creation_project_not_found"
        );
        assert!(first_root.join("personal-capabilities").is_dir());
        assert!(!second_root.join("personal-capabilities").exists());
        drop(first);
        drop(second);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn source_files_are_validated_and_packaged_as_immutable_artifact() {
        let root = std::env::temp_dir().join(format!(
            "cilai-capability-creation-package-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let persistence = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let mut request = params("package-project", 'f');
        request.source_files = vec![
            CapabilityCreationSourceFile {
                path: "mcp/server.mjs".to_owned(),
                content: "console.log(JSON.stringify({jsonrpc:'2.0'}));".to_owned(),
            },
            CapabilityCreationSourceFile {
                path: "README.md".to_owned(),
                content: "# 本地文件检查".to_owned(),
            },
        ];
        refresh_fingerprint(&mut request);
        let created = services.capability_creation_create(&request, 10).unwrap();
        assert_eq!(created.project.status, "packaged");
        assert_eq!(created.project.source_file_count, 2);
        assert!(
            created
                .project
                .source_digest
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        let digest = created.project.package_digest.as_deref().unwrap();
        assert!(digest.starts_with("sha256:"));
        let project = root.join(&created.project.managed_relative_path);
        assert!(project.join("source/mcp/server.mjs").is_file());
        assert!(project.join("generated/plugin-manifest.json").is_file());
        assert!(
            project
                .join(format!(
                    "artifacts/{}.zip",
                    digest.trim_start_matches("sha256:")
                ))
                .is_file()
        );
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generated_web_mcp_manifests_declare_both_capabilities_and_managed_runtime() {
        let source_digest = format!("sha256:{}", "a".repeat(64));
        let manifests = generate_personal_manifests(
            "capability_1234abcd",
            "组合检查",
            "展示检查结果并提供只读文件工具",
            "web_mcp_application",
            &["personal_file.read".to_owned()],
            &source_digest,
            "0.1.0",
            None,
        )
        .unwrap();
        let plugin: serde_json::Value =
            serde_json::from_slice(&manifests["generated/plugin-manifest.json"]).unwrap();
        assert_eq!(plugin["schemaVersion"], "1.1");
        assert_eq!(plugin["distributionScope"], "personal");
        assert_eq!(plugin["applications"].as_array().unwrap().len(), 1);
        assert_eq!(plugin["runtimeComponents"].as_array().unwrap().len(), 1);
        assert_eq!(plugin["provides"].as_array().unwrap().len(), 2);
        assert_eq!(plugin["provides"][0]["kind"], "web");
        assert_eq!(plugin["provides"][1]["kind"], "mcp");
        assert_eq!(plugin["sourceDigest"], source_digest);
        assert!(
            plugin["manifestDigest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        let runtime: serde_json::Value =
            serde_json::from_slice(&manifests["generated/local-runtime.json"]).unwrap();
        assert_eq!(runtime["runtime"], "managed_node");
        assert_eq!(runtime["transport"], "mcp_stdio");
        assert_eq!(runtime["startPolicy"], "application_open");
        assert_eq!(runtime["permissions"]["networkAccess"], false);
        assert_eq!(runtime["permissions"]["subprocess"], false);
    }

    #[test]
    fn install_fingerprint_matches_the_desktop_canonical_request() {
        let params = CapabilityCreationInstallParams {
            creation_project_id: "capability_web".to_owned(),
            expected_revision: 1,
            package_digest: format!("sha256:{}", "b".repeat(64)),
            client_operation_id: "ignored-by-fingerprint".to_owned(),
            request_fingerprint: String::new(),
        };
        assert_eq!(
            capability_install_request_fingerprint(&params),
            "c380969c59728d4d7daa2c36a791f8cf543e206ebe28a86cd3ae29508f7851bd"
        );
    }

    #[test]
    fn source_package_rejects_missing_entry_and_forbidden_runtime() {
        let mut request = params("invalid-source", '9');
        request.source_files = vec![CapabilityCreationSourceFile {
            path: "mcp/helper.mjs".to_owned(),
            content: "export const value = 1;".to_owned(),
        }];
        assert_eq!(
            validate_create(&request).unwrap_err().reason,
            "capability_creation_source_entry_missing"
        );
        request.source_files = vec![CapabilityCreationSourceFile {
            path: "mcp/server.mjs".to_owned(),
            content: "import { spawn } from 'node:child_process';".to_owned(),
        }];
        assert_eq!(
            validate_create(&request).unwrap_err().reason,
            "capability_creation_source_runtime_forbidden"
        );
    }

    #[test]
    fn web_preview_rechecks_profile_source_digest() {
        let root = std::env::temp_dir().join(format!(
            "cilai-capability-creation-preview-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let persistence = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let mut request = params("preview-project", '8');
        request.target_type = "web_application".to_owned();
        request.requested_permissions.clear();
        request.source_files = vec![CapabilityCreationSourceFile {
            path: "web/index.html".to_owned(),
            content: "<!doctype html><h1>个人检查页</h1>".to_owned(),
        }];
        refresh_fingerprint(&mut request);
        let created = services.capability_creation_create(&request, 10).unwrap();
        let preview = services
            .capability_creation_preview(
                &CapabilityCreationPreviewParams {
                    creation_project_id: created.project.creation_project_id.clone(),
                },
                11,
            )
            .unwrap();
        assert!(preview.html.contains("个人检查页"));
        assert!(preview.content_digest.starts_with("sha256:"));
        fs::write(
            root.join(created.project.managed_relative_path)
                .join("source/web/index.html"),
            "tampered",
        )
        .unwrap();
        assert_eq!(
            services
                .capability_creation_preview(
                    &CapabilityCreationPreviewParams {
                        creation_project_id: created.project.creation_project_id,
                    },
                    12,
                )
                .unwrap_err()
                .reason,
            "capability_creation_source_digest_mismatch"
        );
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn personal_web_install_is_idempotent_and_open_uses_installed_bytes() {
        let root = std::env::temp_dir().join(format!(
            "cilai-capability-personal-install-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let persistence = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let mut request = params("install-project", '7');
        request.target_type = "web_application".to_owned();
        request.requested_permissions.clear();
        request.source_files = vec![CapabilityCreationSourceFile {
            path: "web/index.html".to_owned(),
            content: "<!doctype html><h1>已安装应用</h1>".to_owned(),
        }];
        refresh_fingerprint(&mut request);
        let created = services.capability_creation_create(&request, 10).unwrap();
        let install = install_params(&created.project, "install-personal-web");
        let installed = services.capability_creation_install(&install, 11).unwrap();
        assert!(!installed.replayed);
        assert_eq!(installed.project.status, "installed");
        assert_eq!(installed.project.state_revision, 2);
        assert!(
            root.join("personal-capabilities")
                .join(&created.project.creation_project_id)
                .join(install.package_digest.trim_start_matches("sha256:"))
                .is_dir()
        );
        let replayed = services.capability_creation_install(&install, 12).unwrap();
        assert!(replayed.replayed);
        assert_eq!(replayed.project, installed.project);
        let mut forged_replay = install.clone();
        forged_replay.expected_revision += 1;
        assert_eq!(
            services
                .capability_creation_install(&forged_replay, 12)
                .unwrap_err()
                .reason,
            "request_fingerprint_invalid"
        );
        let opened = services
            .personal_application_open(
                &PersonalApplicationOpenParams {
                    creation_project_id: created.project.creation_project_id,
                },
                13,
            )
            .unwrap();
        assert!(opened.html.contains("已安装应用"));
        assert_eq!(opened.installation_revision, 1);
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    fn installed_personal_fixture(
        root: &Path,
        target: &str,
    ) -> (
        AppServerServices,
        CapabilityCreationInstallParams,
        PersonalApplicationOpenParams,
        PathBuf,
    ) {
        fs::create_dir_all(root).unwrap();
        let persistence =
            super::data::test_support::ready_db(root, "contexthub.db", "legacy-application-test");
        let mut services =
            AppServerServices::with_data_dir(Some(persistence), Some(root.to_owned()));
        let mut create = params("manage-project", '5');
        create.target_type = target.to_owned();
        create.requested_permissions.clear();
        create.source_files = vec![CapabilityCreationSourceFile {
            path: if target == "local_mcp" {
                "mcp/server.mjs"
            } else {
                "web/index.html"
            }
            .to_owned(),
            content: if target == "local_mcp" {
                "console.log('ready');"
            } else {
                "<h1>应用</h1>"
            }
            .to_owned(),
        }];
        refresh_fingerprint(&mut create);
        let created = services.capability_creation_create(&create, 10).unwrap();
        let install = install_params(&created.project, "install-managed");
        services.capability_creation_install(&install, 11).unwrap();
        let open = PersonalApplicationOpenParams {
            creation_project_id: created.project.creation_project_id.clone(),
        };
        let data = root
            .join("personal-capability-data")
            .join(&open.creation_project_id);
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("saved.txt"), "保留内容").unwrap();
        (services, install, open, data)
    }

    #[test]
    fn personal_management_revokes_calls_preserves_data_and_replays_without_reapplying() {
        for target in ["web_application", "local_mcp"] {
            let root = super::data::test_support::profile_root("cilai-personal-management");
            let (mut services, install, open, data) = installed_personal_fixture(&root, target);
            let mut command = PersonalCapabilityManageParams {
                creation_project_id: open.creation_project_id.clone(),
                package_digest: install.package_digest.clone(),
                expected_revision: 2,
                action: "disable".to_owned(),
                client_operation_id: "disable-1".to_owned(),
            };
            let disabled = services.personal_capability_manage(&command, 12).unwrap();
            assert_eq!(disabled["enabled"], false);
            assert_eq!(disabled["project"]["stateRevision"], 3);
            assert!(services.personal_application_open(&open, 13).is_err());
            assert!(services.personal_capability_runtime(&open, 13).is_err());
            assert_eq!(
                services.personal_capability_manage(&command, 13).unwrap()["replayed"],
                true
            );
            command.action = "enable".to_owned();
            assert_eq!(
                services
                    .personal_capability_manage(&command, 14)
                    .unwrap_err()
                    .reason,
                "operation_id_reused"
            );
            command.client_operation_id = "enable-1".to_owned();
            assert_eq!(
                services
                    .personal_capability_manage(&command, 14)
                    .unwrap_err()
                    .reason,
                "personal_capability_state_conflict"
            );
            command.expected_revision = 3;
            command.package_digest = format!("sha256:{}", "a".repeat(64));
            assert_eq!(
                services
                    .personal_capability_manage(&command, 14)
                    .unwrap_err()
                    .reason,
                "personal_capability_state_conflict"
            );
            command.package_digest.clone_from(&install.package_digest);
            assert_eq!(
                services.personal_capability_manage(&command, 14).unwrap()["enabled"],
                true
            );
            if target == "local_mcp" {
                services.personal_capability_runtime(&open, 15).unwrap();
            } else {
                services.personal_application_open(&open, 15).unwrap();
            }
            command.action = "uninstall".to_owned();
            command.expected_revision = 4;
            command.client_operation_id = "uninstall-1".to_owned();
            let removed = services.personal_capability_manage(&command, 16).unwrap();
            assert!(removed["enabled"].is_null());
            assert_eq!(removed["project"]["status"], "packaged");
            assert!(services.personal_application_open(&open, 17).is_err());
            assert!(services.personal_capability_runtime(&open, 17).is_err());
            assert_eq!(
                fs::read_to_string(data.join("saved.txt")).unwrap(),
                "保留内容"
            );
            services.personal_package_export(&open, 17).unwrap();
            let project: CapabilityCreationProject =
                serde_json::from_value(removed["project"].clone()).unwrap();
            services
                .capability_creation_install(&install_params(&project, "reinstall-managed"), 18)
                .unwrap();
            drop(services);
            let persistence = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
            let mut services =
                AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
            let replay = services.personal_capability_manage(&command, 19).unwrap();
            assert_eq!(replay["replayed"], true);
            assert_eq!(replay["project"]["status"], "installed");
            assert_eq!(replay["project"]["stateRevision"], 6);
            assert_eq!(replay["enabled"], true);
            assert_eq!(
                fs::read_to_string(data.join("saved.txt")).unwrap(),
                "保留内容"
            );
            drop(services);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn historical_enterprise_binding_is_never_executed_or_consumed_as_a_personal_package() {
        let root = super::data::test_support::profile_root("cilai-personal-legacy-binding");
        fs::create_dir_all(&root).unwrap();
        let db_path = root.join("contexthub.db");
        let persistence =
            super::data::test_support::ready_db(&root, "contexthub.db", "legacy-application-test");
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let mut request = params("legacy-binding-fixture", '1');
        request.target_type = "web_mcp_application".into();
        request.source_files = vec![
            CapabilityCreationSourceFile {
                path: "web/index.html".into(),
                content: "<h1>原应用页面</h1>".into(),
            },
            CapabilityCreationSourceFile {
                path: "mcp/server.mjs".into(),
                content: "console.log(JSON.stringify({jsonrpc:'2.0'}));".into(),
            },
        ];
        refresh_fingerprint(&mut request);
        let project = services
            .capability_creation_create(&request, 1)
            .unwrap()
            .project;
        let install = install_params(&project, "install-before-legacy-marker");
        services.capability_creation_install(&install, 2).unwrap();
        let open = PersonalApplicationOpenParams {
            creation_project_id: project.creation_project_id.clone(),
        };
        assert_eq!(
            services.personal_application_open(&open, 3).unwrap().html,
            "<h1>原应用页面</h1>"
        );
        let runtime = services.personal_capability_runtime(&open, 4).unwrap();
        assert!(
            services.personal_package_export(&open, 5).unwrap()["bytes"]
                .as_array()
                .is_some()
        );
        let data = Path::new(runtime["dataRoot"].as_str().unwrap());
        fs::write(data.join("saved.txt"), "保留本地数据").unwrap();
        let connection = rusqlite::Connection::open(&db_path).unwrap();
        connection.execute("INSERT INTO enterprise_capability_bindings(creation_project_id,tenant_id,installation_id,package_digest,release_json,remaining_uses,authorized_until) VALUES(?1,'old-tenant','old-install',?2,?3,1,9223372036854775807)", rusqlite::params![project.creation_project_id,project.package_digest,json!({"original":true}).to_string()]).unwrap();
        let original = legacy_binding_row(&connection, &project.creation_project_id);
        for _ in 0..2 {
            assert_legacy_execution_rejected(&mut services, &open, &install);
            assert_eq!(
                legacy_binding_row(&connection, &project.creation_project_id),
                original
            );
        }
        assert!(
            services
                .capability_creation_list(&CapabilityCreationListParams {}, 7)
                .unwrap()
                .projects
                .is_empty()
        );
        drop(services);
        let persistence = SqlitePersistence::open(&db_path, "test").unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        assert_eq!(
            services
                .personal_application_open(&open, 8)
                .unwrap_err()
                .reason,
            "enterprise_capability_not_personal"
        );
        assert_eq!(
            legacy_binding_row(&connection, &project.creation_project_id),
            original
        );
        assert_eq!(
            fs::read_to_string(data.join("saved.txt")).unwrap(),
            "保留本地数据"
        );
        drop(services);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    fn assert_legacy_execution_rejected(
        services: &mut AppServerServices,
        open: &PersonalApplicationOpenParams,
        install: &CapabilityCreationInstallParams,
    ) {
        assert_eq!(
            services
                .personal_application_open(open, 6)
                .unwrap_err()
                .reason,
            "enterprise_capability_not_personal"
        );
        assert_eq!(
            services
                .personal_capability_runtime(open, 6)
                .unwrap_err()
                .reason,
            "enterprise_capability_not_personal"
        );
        assert_eq!(
            services
                .personal_package_export(open, 6)
                .unwrap_err()
                .reason,
            "enterprise_capability_not_personal"
        );
        assert_eq!(
            services
                .capability_creation_install(install, 6)
                .unwrap_err()
                .reason,
            "enterprise_capability_not_personal"
        );
    }

    fn legacy_binding_row(connection: &rusqlite::Connection, project: &str) -> (i64, i64, String) {
        connection.query_row("SELECT remaining_uses,authorized_until,release_json FROM enterprise_capability_bindings WHERE creation_project_id=?1", [project], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
    }

    fn exported_package_files(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut package_files = BTreeMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents).unwrap();
            package_files.insert(entry.name().unwrap().into_owned(), contents);
        }
        package_files
    }

    #[test]
    fn local_mcp_installation_requires_verified_source_before_runtime() {
        let root = super::data::test_support::profile_root("cilai-capability-mcp-install-reject");
        fs::create_dir_all(&root).unwrap();
        let persistence =
            super::data::test_support::ready_db(&root, "contexthub.db", "legacy-application-test");
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let mut request = params("mcp-install-project", '5');
        request.source_files = vec![CapabilityCreationSourceFile {
            path: "mcp/server.mjs".to_owned(),
            content: "console.log(JSON.stringify({jsonrpc:'2.0'}));".to_owned(),
        }];
        refresh_fingerprint(&mut request);
        let created = services.capability_creation_create(&request, 10).unwrap();
        let exported = services
            .personal_package_export(
                &PersonalApplicationOpenParams {
                    creation_project_id: created.project.creation_project_id.clone(),
                },
                11,
            )
            .unwrap();
        let bytes: Vec<u8> = serde_json::from_value(exported["bytes"].clone()).unwrap();
        assert_eq!(
            exported["packageDigest"],
            format!("sha256:{:x}", Sha256::digest(&bytes))
        );
        assert_eq!(exported["format"], "cilai-personal-package");
        let mut package_files = exported_package_files(&bytes);
        assert_eq!(
            package_files.keys().map(String::as_str).collect::<Vec<_>>(),
            vec![
                "generated/local-runtime.json",
                "generated/personal-application.json",
                "generated/plugin-manifest.json",
                "source/mcp/server.mjs",
            ]
        );
        let record = services
            .persistence()
            .unwrap()
            .capability_creation_project(&created.project.creation_project_id)
            .unwrap()
            .unwrap();
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&package_files["generated/plugin-manifest.json"]).unwrap();
        assert_eq!(manifest["distributionScope"], "personal");
        manifest["enterpriseReview"] = json!({ "state": "approved" });
        package_files.insert(
            "generated/plugin-manifest.json".into(),
            encode_json(&manifest).unwrap(),
        );
        assert!(
            validate_personal_export(&encode_package(&package_files).unwrap(), &record, "0.1.0")
                .is_err()
        );
        let install = install_params(&created.project, "reject-mcp-install");
        services.capability_creation_install(&install, 11).unwrap();
        let params = PersonalApplicationOpenParams {
            creation_project_id: created.project.creation_project_id,
        };
        let runtime = services.personal_capability_runtime(&params, 12).unwrap();
        #[cfg(unix)]
        {
            let external = root.join("outside");
            fs::create_dir_all(&external).unwrap();
            let data_root = root.join("personal-capability-data");
            fs::remove_dir_all(&data_root).unwrap();
            std::os::unix::fs::symlink(&external, &data_root).unwrap();
            assert!(services.personal_capability_runtime(&params, 12).is_err());
            assert!(!external.join(&params.creation_project_id).exists());
            fs::remove_file(&data_root).unwrap();
            let installed_root = root.join("personal-capabilities");
            let moved = root.join("moved-packages");
            fs::rename(&installed_root, &moved).unwrap();
            std::os::unix::fs::symlink(&moved, &installed_root).unwrap();
            assert!(services.personal_capability_runtime(&params, 12).is_err());
            fs::remove_file(&installed_root).unwrap();
            fs::rename(&moved, &installed_root).unwrap();
        }

        fs::write(
            std::path::Path::new(runtime["packageRoot"].as_str().unwrap())
                .join("source/mcp/server.mjs"),
            "tampered",
        )
        .unwrap();
        assert_eq!(
            services
                .personal_capability_runtime(&params, 13)
                .unwrap_err()
                .reason,
            "capability_creation_source_digest_mismatch"
        );
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }
}
