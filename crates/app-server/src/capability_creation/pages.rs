mod audit;
mod schema;
#[cfg(test)]
mod tests;

use super::*;
use contexthub_protocol::{
    PersonalPageAiManifest, PersonalPageBindingParams, PersonalPageBindingSnapshot,
    PersonalPageRuntimeBinding,
};

pub(super) const SOURCE_PATH: &str = "web/page-ai.json";
const PAGE_PERMISSIONS: [&str; 6] = [
    "page_ai.context",
    "page_ai.ui_actions_confirmed",
    "page_ai.model",
    "page_ai.conversation_handoff",
    "page_ai.tools.read",
    "page_ai.tools.write_confirmed",
];

fn invalid() -> ServiceError {
    ServiceError::new("personal_page_manifest_invalid")
}

fn path_string(path: &Path) -> Result<String, ServiceError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| ServiceError::new("personal_page_binding_invalid"))
}

pub(super) fn validate_permissions(
    target: &str,
    permissions: &[String],
) -> Result<(), ServiceError> {
    let unique: HashSet<_> = permissions.iter().collect();
    if permissions.len() > 16
        || unique.len() != permissions.len()
        || permissions
            .iter()
            .any(|p| !ALLOWED_PERMISSIONS.contains(&p.as_str()))
        || (target == "web_application"
            && permissions
                .iter()
                .any(|p| !PAGE_PERMISSIONS.contains(&p.as_str())))
        || (target == "local_mcp"
            && permissions
                .iter()
                .any(|p| PAGE_PERMISSIONS.contains(&p.as_str())))
    {
        return Err(ServiceError::new("capability_creation_permissions_invalid"));
    }
    Ok(())
}

pub(super) fn from_files(
    target: &str,
    permissions: &[String],
    files: &[CapabilityCreationSourceFile],
) -> Result<Option<PersonalPageAiManifest>, ServiceError> {
    declaration(
        target,
        permissions,
        files
            .iter()
            .find(|f| f.path == SOURCE_PATH)
            .map(|f| f.content.as_bytes()),
    )
}

pub(super) fn declaration(
    target: &str,
    permissions: &[String],
    content: Option<&[u8]>,
) -> Result<Option<PersonalPageAiManifest>, ServiceError> {
    validate_permissions(target, permissions)?;
    if !permissions
        .iter()
        .any(|p| PAGE_PERMISSIONS.contains(&p.as_str()))
    {
        return Ok(None);
    }
    let content = content.ok_or_else(|| ServiceError::new("personal_page_manifest_missing"))?;
    if content.len() > MAX_SOURCE_FILE_BYTES {
        return Err(invalid());
    }
    let manifest: PersonalPageAiManifest =
        serde_json::from_slice(content).map_err(|_| invalid())?;
    validate_manifest(&manifest, target, permissions)?;
    Ok(Some(manifest))
}

fn identifier(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 80
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

fn text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= limit
        && !value.chars().any(char::is_control)
}

fn validate_manifest(
    m: &PersonalPageAiManifest,
    target: &str,
    permissions: &[String],
) -> Result<(), ServiceError> {
    let grants = [
        !m.context_types.is_empty(),
        !m.ui_actions.is_empty(),
        m.model,
        m.conversation_handoff,
        !m.read_tools.is_empty(),
        !m.write_tools.is_empty(),
    ];
    if m.schema_version != 1
        || m.context_types.len() > 16
        || m.ui_actions.len() > 16
        || m.read_tools.len() + m.write_tools.len() > 32
        || !grants.contains(&true)
        || PAGE_PERMISSIONS
            .iter()
            .zip(grants)
            .any(|(permission, needed)| permissions.iter().any(|p| p == permission) != needed)
        || (target != "web_mcp_application"
            && (!m.read_tools.is_empty() || !m.write_tools.is_empty()))
    {
        return Err(ServiceError::new("personal_page_permission_mismatch"));
    }
    let mut names = HashSet::new();
    for context in &m.context_types {
        if !identifier(&context.context_type)
            || !identifier(&context.schema_version)
            || !names.insert(&context.context_type)
        {
            return Err(invalid());
        }
        schema::validate(&context.data_schema)?;
    }
    names.clear();
    for action in &m.ui_actions {
        if !identifier(&action.action_id)
            || !names.insert(&action.action_id)
            || !text(&action.title, 80)
            || !text(&action.description, 1_000)
            || [&action.preconditions, &action.expected_effects]
                .iter()
                .any(|notes| notes.len() > 16 || notes.iter().any(|note| !text(note, 500)))
        {
            return Err(invalid());
        }
        schema::validate_action(&action.input_schema)?;
    }
    names.clear();
    for tool in m.read_tools.iter().chain(&m.write_tools) {
        if !identifier(&tool.name)
            || !names.insert(&tool.name)
            || !text(&tool.title, 80)
            || !text(&tool.description, 1_000)
        {
            return Err(invalid());
        }
        schema::validate(&tool.input_schema)?;
    }
    Ok(())
}

struct PagePackage {
    root: PathBuf,
    application_id: String,
    version: String,
    declaration: Option<PersonalPageAiManifest>,
}

impl AppServerServices {
    pub(super) fn personal_page_manifest(
        &self,
        project: &CapabilityCreationProjectRecord,
    ) -> Result<Option<PersonalPageAiManifest>, ServiceError> {
        let permissions: Vec<String> =
            serde_json::from_str(&project.requested_permissions_json).map_err(|_| invalid())?;
        if !permissions
            .iter()
            .any(|p| PAGE_PERMISSIONS.contains(&p.as_str()))
        {
            return Ok(None);
        }
        let root = self.personal_source_root(project)?;
        let files = validate_installed_source(
            &root,
            &project.target_type,
            project.source_digest.as_deref().ok_or_else(invalid)?,
        )?;
        declaration(
            &project.target_type,
            &permissions,
            files.get(SOURCE_PATH).map(Vec::as_slice),
        )
    }

    fn verified_page_package(
        &self,
        project: &CapabilityCreationProjectRecord,
    ) -> Result<PagePackage, ServiceError> {
        let id = &project.creation_project_id;
        let digest = project.package_digest.as_deref().ok_or_else(invalid)?;
        let profile = self.data_dir.as_deref().ok_or_else(invalid)?;
        let build = self
            .persistence()?
            .capability_creation_build(id, digest)
            .map_err(persistence_error)?
            .ok_or_else(invalid)?;
        let archive_path = self
            .capability_projects_root()?
            .join(id)
            .join(&build.package_relative_path);
        imports::check_path(profile, &archive_path)?;
        let bytes = imports::read_archive(&archive_path)?;
        if format!("sha256:{:x}", Sha256::digest(&bytes)) != digest {
            return Err(ServiceError::new(
                "capability_creation_package_digest_mismatch",
            ));
        }
        let archive = imports::decode_archive(&bytes)?;
        let plugin: serde_json::Value = serde_json::from_slice(
            archive
                .get("generated/plugin-manifest.json")
                .ok_or_else(invalid)?,
        )
        .map_err(|_| invalid())?;
        let application: serde_json::Value = serde_json::from_slice(
            archive
                .get("generated/personal-application.json")
                .ok_or_else(invalid)?,
        )
        .map_err(|_| invalid())?;
        let version = plugin["version"].as_str().ok_or_else(invalid)?.to_owned();
        validate_personal_export(&bytes, project, &version)?;
        let root = self.personal_install_path(id, digest)?;
        imports::check_path(profile, &root)?;
        validate_existing_install(
            &root,
            id,
            digest,
            project.source_digest.as_deref().ok_or_else(invalid)?,
        )?;
        imports::verify_package_tree(&root, &bytes, false)?;
        let permissions: Vec<String> =
            serde_json::from_str(&project.requested_permissions_json).map_err(|_| invalid())?;
        let declaration = declaration(
            &project.target_type,
            &permissions,
            archive
                .get(&format!("source/{SOURCE_PATH}"))
                .map(Vec::as_slice),
        )?;
        Ok(PagePackage {
            root,
            application_id: application["appId"]
                .as_str()
                .ok_or_else(invalid)?
                .to_owned(),
            version,
            declaration,
        })
    }

    pub(crate) fn personal_page_binding(
        &mut self,
        p: &PersonalPageBindingParams,
        _: i64,
    ) -> Result<PersonalPageBindingSnapshot, ServiceError> {
        validate_project_id(&p.creation_project_id)?;
        validate_digest(&p.expected_package_digest, "personal_page_binding_invalid")?;
        self.ensure_personal_capability(&p.creation_project_id)?;
        let (profile, profile_id) = self.data_context()?;
        self.persistence_mut()?
            .check_personal_application_data_admission(&profile_id)
            .map_err(persistence_error)?;
        let project = self
            .persistence()?
            .capability_creation_project(&p.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(invalid)?;
        let installation = self
            .persistence()?
            .personal_capability_installation(&p.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("personal_capability_not_installed"))?;
        if project.status != "installed"
            || !installation.enabled
            || !matches!(
                project.target_type.as_str(),
                "web_application" | "web_mcp_application"
            )
        {
            return Err(ServiceError::new("personal_page_not_ready"));
        }
        if project.package_digest.as_deref() != Some(&p.expected_package_digest)
            || installation.package_digest != p.expected_package_digest
            || installation.installed_relative_path
                != format!(
                    "personal-capabilities/{}/{}",
                    p.creation_project_id,
                    p.expected_package_digest.trim_start_matches("sha256:")
                )
        {
            return Err(ServiceError::new("personal_page_binding_changed"));
        }
        let package = self.verified_page_package(&project)?;
        let page_ai = package
            .declaration
            .ok_or_else(|| ServiceError::new("personal_page_ai_not_authorized"))?;
        let permissions: Vec<String> =
            serde_json::from_str(&project.requested_permissions_json).map_err(|_| invalid())?;
        let slot = self
            .persistence()?
            .personal_application_data_slot(&profile_id, &p.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("personal_application_data_slot_missing"))?;
        if slot.package_digest != p.expected_package_digest {
            return Err(ServiceError::new("personal_page_binding_changed"));
        }
        let data_root = profile.join(&slot.relative_path);
        imports::check_path(&profile, &data_root)?;
        if !data_root.is_dir() {
            return Err(ServiceError::new(
                "personal_application_data_source_missing",
            ));
        }
        let mut normalized = permissions.clone();
        normalized.sort();
        let permission_digest = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&(&normalized, &page_ai)).map_err(|_| invalid())?)
        );
        let data_revision = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&slot).map_err(|_| invalid())?)
        );
        let runtime_binding = if project.target_type == "web_mcp_application" {
            Some(PersonalPageRuntimeBinding {
                package_root: path_string(&package.root)?,
                data_root: path_string(&data_root)?,
                harness_home: path_string(&profile.join("ai-runtime/cilai-application-harness"))?,
                package_digest: p.expected_package_digest.clone(),
                revision: installation.state_revision,
            })
        } else {
            None
        };
        Ok(PersonalPageBindingSnapshot {
            profile_id,
            creation_project_id: p.creation_project_id.clone(),
            application_id: package.application_id,
            name: project.title,
            version: package.version,
            package_digest: installation.package_digest,
            permission_digest,
            source_digest: project.source_digest.ok_or_else(invalid)?,
            target_type: project.target_type,
            permissions,
            enabled: installation.enabled,
            installation_revision: installation.state_revision,
            project_revision: project.state_revision,
            active_data_slot_id: slot.slot_id,
            data_revision,
            page_ai,
            runtime_binding,
        })
    }
}
