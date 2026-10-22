use super::{AppServerServices, ServiceError, UserSkillSaveCommand};
use contexthub_persistence_sqlite::UserSkillRecord;
use contexthub_protocol::{
    SkillConversationCatalogItem, SkillConversationCatalogParams, SkillConversationCatalogResult,
    SkillDeleteParams, SkillDeleteResult, SkillDetail, SkillGetParams, SkillGetResult,
    SkillImportDirectoryParams, SkillImportParams, SkillImportResult, SkillListParams,
    SkillListResult, SkillPackageFile, SkillPackageFileInput, SkillRuntimeCatalogItem,
    SkillRuntimeCatalogParams, SkillRuntimeCatalogResult, SkillSaveParams, SkillSaveResult,
    SkillSetStateParams, SkillSetStateResult, SkillSummary,
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::ZipArchive;

const SKILL_FILE_NAME: &str = "SKILL.md";
const SYSTEM_CREATOR_ID: &str = "system.skill-creator";
const SYSTEM_CREATOR_NAME: &str = "技能创建器";
const SYSTEM_CREATOR_DESCRIPTION: &str = "创建工作台技能：从零创建、参考已有技能制作新技能，或把工作流程整理成可复用技能。用户要求实际创建技能时使用；仅使用现有技能完成任务、讨论技能设计或修改普通文件时不使用。";
const SYSTEM_CREATOR_REVISION: i64 = 2;
const SYSTEM_CAPABILITY_CREATOR_ID: &str = "system.capability-creator";
const SYSTEM_CAPABILITY_CREATOR_NAME: &str = "能力创建器";
const SYSTEM_CAPABILITY_CREATOR_DESCRIPTION: &str = "把用户要解决的业务问题转成受管能力创建项目，并判断应使用用户 Skill、本地 MCP、Web 应用或 Web+MCP。用户明确要求创建、开发、制作或改造个人工具、插件、MCP、应用时使用；仅讨论产品架构、使用已有能力或修改普通项目代码时不使用。";
const SYSTEM_CAPABILITY_CREATOR_REVISION: i64 = 5;
const MAX_PACKAGE_FILE_BYTES: usize = 256 * 1024;
const MAX_PACKAGE_BYTES: usize = 1024 * 1024;
const MAX_PACKAGE_FILES: usize = 64;
const MAX_ARCHIVE_ENTRIES: usize = 128;
const MAX_SKILL_FILE_BYTES: usize = 256 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024;
const SYSTEM_CREATOR_CONTENT: &str = include_str!("../skills/skill-creator/SKILL.md");
const SYSTEM_CAPABILITY_CREATOR_CONTENT: &str =
    include_str!("../skills/capability-creator/SKILL.md");

#[cfg(test)]
#[path = "skills_storage_tests.rs"]
mod storage_tests;

impl AppServerServices {
    pub(super) fn skill_list(
        &mut self,
        _: &SkillListParams,
    ) -> Result<SkillListResult, ServiceError> {
        let persistence = self.persistence()?;
        let mut skills: Vec<_> = persistence
            .list_user_skills()
            .map_err(skill_persistence_error)?
            .into_iter()
            .map(user_skill_summary)
            .collect();
        skills.push(system_creator_summary());
        skills.push(system_capability_creator_summary());
        Ok(SkillListResult { skills })
    }

    pub(super) fn skill_get(
        &mut self,
        params: &SkillGetParams,
    ) -> Result<SkillGetResult, ServiceError> {
        validate_skill_id(&params.skill_id)?;
        let record = self
            .persistence()?
            .user_skill(&params.skill_id)
            .map_err(skill_persistence_error)?
            .ok_or_else(|| ServiceError::new("skill_not_found"))?;
        let (instructions, files) = read_user_skill_package(&self.skills_root()?, &record)?;
        Ok(SkillGetResult {
            skill: user_skill_detail(record, instructions, files),
        })
    }

    pub(super) fn skill_save(
        &mut self,
        params: &SkillSaveParams,
        now: i64,
    ) -> Result<SkillSaveResult, ServiceError> {
        validate_save(params)?;
        if let Some(operation) = self
            .persistence()?
            .user_skill_operation(&params.client_operation_id)
            .map_err(skill_persistence_error)?
        {
            if operation.request_fingerprint != params.request_fingerprint
                || operation.operation_kind != "save"
                || params
                    .skill_id
                    .as_deref()
                    .is_some_and(|skill_id| skill_id != operation.skill_id)
            {
                return Err(ServiceError::new("operation_id_reused"));
            }
            let record = self
                .persistence()?
                .user_skill(&operation.skill_id)
                .map_err(skill_persistence_error)?
                .ok_or_else(|| ServiceError::new("skill_not_found"))?;
            let (instructions, files) = read_user_skill_package(&self.skills_root()?, &record)?;
            return Ok(SkillSaveResult {
                skill: user_skill_detail(record, instructions, files),
            });
        }

        let skill_id = match (&params.skill_id, params.expected_revision) {
            (None, None) => format!("skill_{}", Uuid::new_v4().simple()),
            (Some(skill_id), Some(_)) => {
                validate_skill_id(skill_id)?;
                skill_id.clone()
            }
            _ => return Err(ServiceError::new("skill_revision_required")),
        };
        let package_files = match (&params.skill_id, &params.files) {
            (None, files) => files.clone().unwrap_or_default(),
            (Some(_), Some(_)) => return Err(ServiceError::new("skill_files_read_only")),
            (Some(skill_id), None) => {
                let existing = self
                    .persistence()?
                    .user_skill(skill_id)
                    .map_err(skill_persistence_error)?
                    .ok_or_else(|| ServiceError::new("skill_not_found"))?;
                let (_, files) = read_user_skill_package(&self.skills_root()?, &existing)?;
                files
                    .into_iter()
                    .map(|file| SkillPackageFileInput {
                        path: file.path,
                        content: file.content,
                    })
                    .collect()
            }
        };
        let normalized_name = normalize_name(&params.name);
        validate_user_skill_name(&normalized_name)?;
        if let Some(existing) = self
            .persistence()?
            .user_skill_by_normalized_name(&normalized_name)
            .map_err(skill_persistence_error)?
            && existing.skill_id != skill_id
        {
            return Err(ServiceError::new("skill_name_conflict"));
        }
        let content = render_skill_file(&params.name, &params.description, &params.instructions)?;
        let skills_root = self.skills_root()?;
        let (relative_path, digest) = create_skill_package_candidate(
            &skills_root,
            &skill_id,
            content.as_bytes(),
            &package_files,
        )?;

        let command = UserSkillSaveCommand {
            client_operation_id: params.client_operation_id.clone(),
            request_fingerprint: params.request_fingerprint.clone(),
            skill_id: skill_id.clone(),
            name: params.name.trim().to_owned(),
            normalized_name,
            description: params.description.trim().to_owned(),
            managed_relative_path: relative_path,
            enabled: params.enabled,
            automatic: params.automatic,
            expected_revision: params.expected_revision,
            content_digest: digest,
            occurred_at: now,
        };
        let saved = self
            .persistence_mut()?
            .save_user_skill(&command)
            .map_err(skill_persistence_error)?;
        let (instructions, files) = read_user_skill_package(&skills_root, &saved)?;
        Ok(SkillSaveResult {
            skill: user_skill_detail(saved, instructions, files),
        })
    }

    pub(super) fn skill_import(
        &mut self,
        params: &SkillImportParams,
        now: i64,
    ) -> Result<SkillImportResult, ServiceError> {
        self.skill_import_package(
            &params.client_operation_id,
            &params.request_fingerprint,
            now,
            || read_skill_archive(Path::new(&params.archive_path)),
        )
    }

    pub(super) fn skill_import_directory(
        &mut self,
        params: &SkillImportDirectoryParams,
        now: i64,
    ) -> Result<SkillImportResult, ServiceError> {
        self.skill_import_package(
            &params.client_operation_id,
            &params.request_fingerprint,
            now,
            || {
                read_skill_directory(
                    Path::new(&params.skill_file_path),
                    Path::new(&params.workspace_root),
                )
            },
        )
    }

    fn skill_import_package(
        &mut self,
        client_operation_id: &str,
        request_fingerprint: &str,
        now: i64,
        read_package: impl FnOnce() -> Result<ImportedSkillPackage, ServiceError>,
    ) -> Result<SkillImportResult, ServiceError> {
        validate_operation(client_operation_id, request_fingerprint)?;
        if let Some(operation) = self
            .persistence()?
            .user_skill_operation(client_operation_id)
            .map_err(skill_persistence_error)?
        {
            if operation.request_fingerprint != request_fingerprint
                || operation.operation_kind != "save"
            {
                return Err(ServiceError::new("operation_id_reused"));
            }
            let record = self
                .persistence()?
                .user_skill(&operation.skill_id)
                .map_err(skill_persistence_error)?
                .ok_or_else(|| ServiceError::new("skill_not_found"))?;
            let (instructions, files) = read_user_skill_package(&self.skills_root()?, &record)?;
            return Ok(SkillImportResult {
                skill: user_skill_detail(record, instructions, files),
            });
        }

        let package = read_package()?;
        validate_save(&SkillSaveParams {
            skill_id: None,
            name: package.name.clone(),
            description: package.description.clone(),
            instructions: package.instructions.clone(),
            files: Some(package.files.clone()),
            enabled: true,
            automatic: false,
            expected_revision: None,
            client_operation_id: client_operation_id.to_owned(),
            request_fingerprint: request_fingerprint.to_owned(),
        })?;
        let normalized_name = normalize_name(&package.name);
        validate_user_skill_name(&normalized_name)?;
        if self
            .persistence()?
            .user_skill_by_normalized_name(&normalized_name)
            .map_err(skill_persistence_error)?
            .is_some()
        {
            return Err(ServiceError::new("skill_name_conflict"));
        }

        let skill_id = format!("skill_{}", Uuid::new_v4().simple());
        let skills_root = self.skills_root()?;
        let (relative_path, digest) = create_skill_package_candidate(
            &skills_root,
            &skill_id,
            package.skill_file.as_bytes(),
            &package.files,
        )?;
        let command = UserSkillSaveCommand {
            client_operation_id: client_operation_id.to_owned(),
            request_fingerprint: request_fingerprint.to_owned(),
            skill_id,
            name: package.name,
            normalized_name,
            description: package.description,
            managed_relative_path: relative_path,
            enabled: true,
            automatic: false,
            expected_revision: None,
            content_digest: digest,
            occurred_at: now,
        };
        let saved = self
            .persistence_mut()?
            .save_user_skill(&command)
            .map_err(skill_persistence_error)?;
        let (instructions, files) = read_user_skill_package(&skills_root, &saved)?;
        Ok(SkillImportResult {
            skill: user_skill_detail(saved, instructions, files),
        })
    }

    pub(super) fn skill_set_state(
        &mut self,
        params: &SkillSetStateParams,
        now: i64,
    ) -> Result<SkillSetStateResult, ServiceError> {
        validate_skill_id(&params.skill_id)?;
        validate_operation(&params.client_operation_id, &params.request_fingerprint)?;
        let skill = self
            .persistence_mut()?
            .set_user_skill_state(
                &params.skill_id,
                params.enabled,
                params.automatic,
                params.expected_revision,
                &params.client_operation_id,
                &params.request_fingerprint,
                now,
            )
            .map_err(skill_persistence_error)?;
        Ok(SkillSetStateResult {
            skill: user_skill_summary(skill),
        })
    }

    pub(super) fn skill_delete(
        &mut self,
        params: &SkillDeleteParams,
        now: i64,
    ) -> Result<SkillDeleteResult, ServiceError> {
        validate_skill_id(&params.skill_id)?;
        validate_operation(&params.client_operation_id, &params.request_fingerprint)?;
        if let Some(operation) = self
            .persistence()?
            .user_skill_operation(&params.client_operation_id)
            .map_err(skill_persistence_error)?
        {
            if operation.request_fingerprint != params.request_fingerprint
                || operation.operation_kind != "delete"
                || operation.skill_id != params.skill_id
            {
                return Err(ServiceError::new("operation_id_reused"));
            }
            return Ok(SkillDeleteResult {
                skill_id: params.skill_id.clone(),
                deleted: true,
            });
        }
        self.persistence()?
            .user_skill(&params.skill_id)
            .map_err(skill_persistence_error)?
            .ok_or_else(|| ServiceError::new("skill_not_found"))?;
        let deleted = self
            .persistence_mut()?
            .delete_user_skill(
                &params.skill_id,
                params.expected_revision,
                &params.client_operation_id,
                &params.request_fingerprint,
                now,
            )
            .map_err(skill_persistence_error)?;
        Ok(SkillDeleteResult {
            skill_id: params.skill_id.clone(),
            deleted,
        })
    }

    pub(super) fn skill_conversation_catalog(
        &mut self,
        _: &SkillConversationCatalogParams,
    ) -> Result<SkillConversationCatalogResult, ServiceError> {
        self.ensure_system_skill_creator()?;
        self.ensure_system_capability_creator()?;
        let mut skills = vec![
            SkillConversationCatalogItem {
                skill_id: SYSTEM_CREATOR_ID.to_owned(),
                name: SYSTEM_CREATOR_NAME.to_owned(),
                description: SYSTEM_CREATOR_DESCRIPTION.to_owned(),
                source: "system".to_owned(),
                automatic: true,
                state_revision: SYSTEM_CREATOR_REVISION,
            },
            SkillConversationCatalogItem {
                skill_id: SYSTEM_CAPABILITY_CREATOR_ID.to_owned(),
                name: SYSTEM_CAPABILITY_CREATOR_NAME.to_owned(),
                description: SYSTEM_CAPABILITY_CREATOR_DESCRIPTION.to_owned(),
                source: "system".to_owned(),
                automatic: true,
                state_revision: SYSTEM_CAPABILITY_CREATOR_REVISION,
            },
        ];
        skills.extend(
            self.persistence()?
                .list_user_skills()
                .map_err(skill_persistence_error)?
                .into_iter()
                .filter(|skill| skill.enabled)
                .map(|skill| SkillConversationCatalogItem {
                    skill_id: skill.skill_id,
                    name: skill.name,
                    description: skill.description,
                    source: "user".to_owned(),
                    automatic: skill.automatic,
                    state_revision: skill.state_revision,
                }),
        );
        Ok(SkillConversationCatalogResult { skills })
    }

    pub(super) fn skill_runtime_catalog(
        &mut self,
        _: &SkillRuntimeCatalogParams,
    ) -> Result<SkillRuntimeCatalogResult, ServiceError> {
        let creator_path = self.ensure_system_skill_creator()?;
        let capability_creator_path = self.ensure_system_capability_creator()?;
        let mut skills = vec![
            SkillRuntimeCatalogItem {
                skill_id: SYSTEM_CREATOR_ID.to_owned(),
                name: SYSTEM_CREATOR_NAME.to_owned(),
                description: SYSTEM_CREATOR_DESCRIPTION.to_owned(),
                source: "system".to_owned(),
                managed_path: path_string(&creator_path)?,
                automatic: true,
                state_revision: SYSTEM_CREATOR_REVISION,
                content_digest: sha256_hex(SYSTEM_CREATOR_CONTENT.as_bytes()),
            },
            SkillRuntimeCatalogItem {
                skill_id: SYSTEM_CAPABILITY_CREATOR_ID.to_owned(),
                name: SYSTEM_CAPABILITY_CREATOR_NAME.to_owned(),
                description: SYSTEM_CAPABILITY_CREATOR_DESCRIPTION.to_owned(),
                source: "system".to_owned(),
                managed_path: path_string(&capability_creator_path)?,
                automatic: true,
                state_revision: SYSTEM_CAPABILITY_CREATOR_REVISION,
                content_digest: sha256_hex(SYSTEM_CAPABILITY_CREATOR_CONTENT.as_bytes()),
            },
        ];
        let root = self.skills_root()?;
        for skill in self
            .persistence()?
            .list_user_skills()
            .map_err(skill_persistence_error)?
            .into_iter()
            .filter(|skill| skill.enabled)
        {
            let path = managed_skill_path(&root, &skill.managed_relative_path)?;
            verify_package_digest(&path, &skill.content_digest)?;
            skills.push(SkillRuntimeCatalogItem {
                skill_id: skill.skill_id,
                name: skill.name,
                description: skill.description,
                source: "user".to_owned(),
                managed_path: path_string(&path)?,
                automatic: skill.automatic,
                state_revision: skill.state_revision,
                content_digest: skill.content_digest,
            });
        }
        Ok(SkillRuntimeCatalogResult { skills })
    }

    pub(super) fn persistence(
        &self,
    ) -> Result<&contexthub_persistence_sqlite::SqlitePersistence, ServiceError> {
        self.persistence
            .as_ref()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))
    }

    pub(super) fn persistence_mut(
        &mut self,
    ) -> Result<&mut contexthub_persistence_sqlite::SqlitePersistence, ServiceError> {
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))
    }

    fn skills_root(&self) -> Result<PathBuf, ServiceError> {
        let root = self
            .data_dir
            .as_ref()
            .map(|path| path.join("skills"))
            .ok_or_else(|| ServiceError::new("data_dir_unavailable"))?;
        if fs::symlink_metadata(&root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(ServiceError::new("skill_path_invalid"));
        }
        Ok(root)
    }

    fn ensure_system_skill_creator(&self) -> Result<PathBuf, ServiceError> {
        let root = self.skills_root()?;
        let path = managed_skill_path(&root, "system/skill-creator/SKILL.md")?;
        if path.exists() {
            let current =
                fs::read(&path).map_err(|_| ServiceError::new("skill_file_read_failed"))?;
            if current != SYSTEM_CREATOR_CONTENT.as_bytes() {
                atomic_write(&path, SYSTEM_CREATOR_CONTENT.as_bytes())?;
            }
            return Ok(path);
        }
        atomic_write(&path, SYSTEM_CREATOR_CONTENT.as_bytes())?;
        Ok(path)
    }

    fn ensure_system_capability_creator(&self) -> Result<PathBuf, ServiceError> {
        let root = self.skills_root()?;
        let path = managed_skill_path(&root, "system/capability-creator/SKILL.md")?;
        if path.exists() {
            let current =
                fs::read(&path).map_err(|_| ServiceError::new("skill_file_read_failed"))?;
            if current != SYSTEM_CAPABILITY_CREATOR_CONTENT.as_bytes() {
                atomic_write(&path, SYSTEM_CAPABILITY_CREATOR_CONTENT.as_bytes())?;
            }
            return Ok(path);
        }
        atomic_write(&path, SYSTEM_CAPABILITY_CREATOR_CONTENT.as_bytes())?;
        Ok(path)
    }
}

pub(super) fn validate_save(params: &SkillSaveParams) -> Result<(), ServiceError> {
    let name = params.name.trim();
    let description = params.description.trim();
    let instructions = params.instructions.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err(ServiceError::new("skill_name_invalid"));
    }
    if description.is_empty() || description.chars().count() > 240 {
        return Err(ServiceError::new("skill_description_invalid"));
    }
    if instructions.is_empty() || instructions.chars().count() > 50_000 {
        return Err(ServiceError::new("skill_instructions_invalid"));
    }
    if name.contains(['\0', '\n', '\r'])
        || description.contains('\0')
        || instructions.contains('\0')
    {
        return Err(ServiceError::new("skill_content_invalid"));
    }
    if params.skill_id.is_some() && params.files.is_some() {
        return Err(ServiceError::new("skill_files_read_only"));
    }
    if let Some(files) = &params.files {
        validate_package_files(files)?;
    }
    validate_operation(&params.client_operation_id, &params.request_fingerprint)
}

fn validate_package_files(files: &[SkillPackageFileInput]) -> Result<(), ServiceError> {
    if files.len() > MAX_PACKAGE_FILES {
        return Err(ServiceError::new("skill_files_invalid"));
    }
    let mut total_bytes = 0usize;
    let mut paths = HashSet::new();
    for file in files {
        validate_package_relative_path(&file.path)?;
        let bytes = file.content.len();
        if bytes > MAX_PACKAGE_FILE_BYTES || file.content.contains('\0') {
            return Err(ServiceError::new("skill_file_content_invalid"));
        }
        total_bytes = total_bytes.saturating_add(bytes);
        if total_bytes > MAX_PACKAGE_BYTES || !paths.insert(file.path.to_lowercase()) {
            return Err(ServiceError::new("skill_files_invalid"));
        }
    }
    Ok(())
}

fn validate_package_relative_path(path: &str) -> Result<(), ServiceError> {
    if path.is_empty()
        || path.len() > 240
        || path.contains(['\0', '\\', ':'])
        || path.starts_with('/')
        || !matches!(
            path.split('/').next(),
            Some("scripts" | "references" | "assets")
        )
    {
        return Err(ServiceError::new("skill_file_path_invalid"));
    }
    let mut depth = 0usize;
    for component in path.split('/') {
        depth += 1;
        if component.is_empty()
            || matches!(component, "." | "..")
            || component.chars().any(char::is_control)
        {
            return Err(ServiceError::new("skill_file_path_invalid"));
        }
    }
    if !(2..=8).contains(&depth) {
        return Err(ServiceError::new("skill_file_path_invalid"));
    }
    Ok(())
}

fn validate_operation(operation_id: &str, fingerprint: &str) -> Result<(), ServiceError> {
    if operation_id.trim().is_empty() || fingerprint.len() != 64 {
        return Err(ServiceError::new("skill_operation_invalid"));
    }
    if !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServiceError::new("skill_operation_invalid"));
    }
    Ok(())
}

fn validate_skill_id(skill_id: &str) -> Result<(), ServiceError> {
    if skill_id.starts_with("skill_")
        && skill_id.len() == 38
        && skill_id
            .bytes()
            .skip(6)
            .all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(ServiceError::new("skill_id_invalid"))
    }
}

fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn validate_user_skill_name(normalized_name: &str) -> Result<(), ServiceError> {
    if [SYSTEM_CREATOR_NAME, SYSTEM_CAPABILITY_CREATOR_NAME]
        .iter()
        .map(|name| normalize_name(name))
        .any(|reserved| reserved == normalized_name)
    {
        return Err(ServiceError::new("skill_name_conflict"));
    }
    Ok(())
}

fn render_skill_file(
    name: &str,
    description: &str,
    instructions: &str,
) -> Result<String, ServiceError> {
    let name = serde_json::to_string(name.trim())
        .map_err(|_| ServiceError::new("skill_content_invalid"))?;
    let description = serde_json::to_string(description.trim())
        .map_err(|_| ServiceError::new("skill_content_invalid"))?;
    Ok(format!(
        "---\nname: {name}\ndescription: {description}\n---\n\n{}\n",
        instructions.trim()
    ))
}

#[derive(Debug)]
struct ImportedSkillPackage {
    name: String,
    description: String,
    instructions: String,
    skill_file: String,
    files: Vec<SkillPackageFileInput>,
}

fn read_skill_directory(
    skill_file_path: &Path,
    workspace_root: &Path,
) -> Result<ImportedSkillPackage, ServiceError> {
    if skill_file_path.file_name().and_then(|value| value.to_str()) != Some(SKILL_FILE_NAME) {
        return Err(ServiceError::new("skill_file_invalid"));
    }
    let metadata = fs::symlink_metadata(skill_file_path)
        .map_err(|_| ServiceError::new("skill_file_read_failed"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_SKILL_FILE_BYTES as u64
    {
        return Err(ServiceError::new("skill_file_invalid"));
    }
    let workspace_root =
        fs::canonicalize(workspace_root).map_err(|_| ServiceError::new("skill_path_invalid"))?;
    let skill_file_path = fs::canonicalize(skill_file_path)
        .map_err(|_| ServiceError::new("skill_file_read_failed"))?;
    let directory = skill_file_path
        .parent()
        .ok_or_else(|| ServiceError::new("skill_path_invalid"))?;
    if !directory.starts_with(&workspace_root) || directory == workspace_root {
        return Err(ServiceError::new("skill_path_invalid"));
    }
    for entry in fs::read_dir(directory).map_err(|_| ServiceError::new("skill_file_read_failed"))? {
        let entry = entry.map_err(|_| ServiceError::new("skill_file_read_failed"))?;
        let name = entry.file_name();
        if name != SKILL_FILE_NAME
            && name != ".DS_Store"
            && !["scripts", "references", "assets"]
                .iter()
                .any(|allowed| name == *allowed)
        {
            return Err(ServiceError::new("skill_directory_structure_invalid"));
        }
    }
    let skill_file = String::from_utf8(
        fs::read(&skill_file_path).map_err(|_| ServiceError::new("skill_file_read_failed"))?,
    )
    .map_err(|_| ServiceError::new("skill_file_invalid"))?;
    let (name, description, instructions) = parse_skill_file(&skill_file)?;
    let mut files = Vec::new();
    for category in ["scripts", "references", "assets"] {
        let category_path = directory.join(category);
        if category_path.exists() {
            collect_package_files(directory, &category_path, category, &mut files, true)?;
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let files = files
        .into_iter()
        .map(|file| SkillPackageFileInput {
            path: file.path,
            content: file.content,
        })
        .collect::<Vec<_>>();
    validate_package_files(&files)?;
    Ok(ImportedSkillPackage {
        name,
        description,
        instructions,
        skill_file,
        files,
    })
}

fn read_skill_archive(path: &Path) -> Result<ImportedSkillPackage, ServiceError> {
    let mut archive = open_skill_archive(path)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(ServiceError::new("skill_files_invalid"));
    }
    let mut skill_file = None;
    let mut files = Vec::new();
    let mut paths = HashSet::new();
    let mut total_bytes = 0usize;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| ServiceError::new("skill_archive_invalid"))?;
        if entry.encrypted()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
            || entry
                .name()
                .map_err(|_| ServiceError::new("skill_archive_invalid"))?
                .contains('\u{5c}')
        {
            return Err(ServiceError::new("skill_archive_invalid"));
        }
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| ServiceError::new("skill_archive_structure_invalid"))?;
        let relative = enclosed
            .to_str()
            .ok_or_else(|| ServiceError::new("skill_path_non_unicode"))?
            .replace(std::path::MAIN_SEPARATOR, "/");
        if relative.is_empty() || !paths.insert(relative.to_lowercase()) {
            return Err(ServiceError::new("skill_archive_structure_invalid"));
        }
        if entry.is_dir() {
            validate_archive_directory(&relative)?;
            continue;
        }
        if relative == SKILL_FILE_NAME {
            if entry.size() > MAX_SKILL_FILE_BYTES as u64 || skill_file.is_some() {
                return Err(ServiceError::new("skill_archive_structure_invalid"));
            }
            let mut bytes = Vec::new();
            (&mut entry)
                .take(MAX_SKILL_FILE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ServiceError::new("skill_archive_invalid"))?;
            if bytes.len() > MAX_SKILL_FILE_BYTES {
                return Err(ServiceError::new("skill_archive_structure_invalid"));
            }
            skill_file = Some(
                String::from_utf8(bytes)
                    .map_err(|_| ServiceError::new("skill_archive_content_invalid"))?,
            );
            continue;
        }
        validate_package_relative_path(&relative)?;
        if entry.size() > MAX_PACKAGE_FILE_BYTES as u64 {
            return Err(ServiceError::new("skill_file_content_invalid"));
        }
        let size = usize::try_from(entry.size())
            .map_err(|_| ServiceError::new("skill_file_content_invalid"))?;
        if files.len() >= MAX_PACKAGE_FILES {
            return Err(ServiceError::new("skill_files_invalid"));
        }
        let mut bytes = Vec::with_capacity(size);
        (&mut entry)
            .take(MAX_PACKAGE_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ServiceError::new("skill_archive_invalid"))?;
        if bytes.len() > MAX_PACKAGE_FILE_BYTES {
            return Err(ServiceError::new("skill_file_content_invalid"));
        }
        total_bytes = total_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| ServiceError::new("skill_files_invalid"))?;
        if total_bytes > MAX_PACKAGE_BYTES {
            return Err(ServiceError::new("skill_files_invalid"));
        }
        let content = String::from_utf8(bytes)
            .map_err(|_| ServiceError::new("skill_archive_content_invalid"))?;
        files.push(SkillPackageFileInput {
            path: relative,
            content,
        });
    }

    let skill_file =
        skill_file.ok_or_else(|| ServiceError::new("skill_archive_skill_file_missing"))?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let (name, description, instructions) = parse_skill_file(&skill_file)?;
    Ok(ImportedSkillPackage {
        name,
        description,
        instructions,
        skill_file,
        files,
    })
}

fn open_skill_archive(path: &Path) -> Result<ZipArchive<fs::File>, ServiceError> {
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("zip"))
    {
        return Err(ServiceError::new("skill_archive_type_invalid"));
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| ServiceError::new("skill_archive_read_failed"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_ARCHIVE_BYTES
    {
        return Err(ServiceError::new("skill_archive_invalid"));
    }
    let source =
        fs::File::open(path).map_err(|_| ServiceError::new("skill_archive_read_failed"))?;
    let archive =
        ZipArchive::new(source).map_err(|_| ServiceError::new("skill_archive_invalid"))?;
    Ok(archive)
}

fn validate_archive_directory(path: &str) -> Result<(), ServiceError> {
    let placeholder = format!("{}/.directory", path.trim_end_matches('/'));
    validate_package_relative_path(&placeholder)
        .map_err(|_| ServiceError::new("skill_archive_structure_invalid"))
}

fn parse_skill_file(content: &str) -> Result<(String, String, String), ServiceError> {
    let normalized = content.replace("\r\n", "\n");
    let rest = normalized
        .strip_prefix("---\n")
        .ok_or_else(|| ServiceError::new("skill_file_invalid"))?;
    let end = rest
        .find("\n---\n")
        .ok_or_else(|| ServiceError::new("skill_file_invalid"))?;
    let frontmatter = &rest[..end];
    let instructions = rest[end + 5..].trim().to_owned();
    let name = frontmatter_scalar(frontmatter, "name")?;
    let description = frontmatter_scalar(frontmatter, "description")?;
    Ok((name, description, instructions))
}

fn frontmatter_scalar(frontmatter: &str, key: &str) -> Result<String, ServiceError> {
    let lines = frontmatter.lines().collect::<Vec<_>>();
    let prefix = format!("{key}:");
    let mut value = None;
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        if let Some(raw) = line.strip_prefix(&prefix) {
            if value.is_some() {
                return Err(ServiceError::new("skill_file_invalid"));
            }
            let raw = raw.trim();
            if matches!(raw, ">" | "|" | ">-" | "|-" | ">+" | "|+") {
                let folded = raw.starts_with('>');
                let mut parts = Vec::new();
                index += 1;
                while index < lines.len()
                    && (lines[index].is_empty()
                        || lines[index].starts_with(' ')
                        || lines[index].starts_with('\t'))
                {
                    parts.push(lines[index].trim());
                    index += 1;
                }
                value = Some(if folded {
                    parts.join(" ")
                } else {
                    parts.join("\n")
                });
                continue;
            }
            value = Some(parse_frontmatter_inline(raw)?);
        }
        index += 1;
    }
    value
        .filter(|item| !item.trim().is_empty())
        .ok_or_else(|| ServiceError::new("skill_file_invalid"))
}

fn parse_frontmatter_inline(raw: &str) -> Result<String, ServiceError> {
    if raw.starts_with('"') {
        return serde_json::from_str(raw).map_err(|_| ServiceError::new("skill_file_invalid"));
    }
    if raw.starts_with('\'') {
        return raw
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .map(|value| value.replace("''", "'"))
            .ok_or_else(|| ServiceError::new("skill_file_invalid"));
    }
    Ok(raw.trim().to_owned())
}

fn read_user_skill_package(
    skills_root: &Path,
    record: &UserSkillRecord,
) -> Result<(String, Vec<SkillPackageFile>), ServiceError> {
    let path = managed_skill_path(skills_root, &record.managed_relative_path)?;
    verify_package_digest(&path, &record.content_digest)?;
    let content =
        fs::read_to_string(&path).map_err(|_| ServiceError::new("skill_file_read_failed"))?;
    let (_, _, instructions) = parse_skill_file(&content)?;
    let directory = path
        .parent()
        .ok_or_else(|| ServiceError::new("skill_path_invalid"))?;
    Ok((instructions, read_package_files(directory)?))
}

fn managed_skill_path(root: &Path, relative: &str) -> Result<PathBuf, ServiceError> {
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(ServiceError::new("skill_path_invalid"));
    }
    let path = root.join(relative_path);
    let mut current = root.to_path_buf();
    for component in relative_path.components() {
        current.push(component.as_os_str());
        if let Ok(metadata) = fs::symlink_metadata(&current)
            && metadata.file_type().is_symlink()
        {
            return Err(ServiceError::new("skill_path_invalid"));
        }
    }
    Ok(path)
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), ServiceError> {
    let parent = path
        .parent()
        .ok_or_else(|| ServiceError::new("skill_path_invalid"))?;
    fs::create_dir_all(parent).map_err(|_| ServiceError::new("skill_file_write_failed"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ServiceError::new("skill_path_non_unicode"))?;
    let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4().simple()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| ServiceError::new("skill_file_write_failed"))?;
    if file
        .write_all(content)
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = fs::remove_file(&temporary);
        return Err(ServiceError::new("skill_file_write_failed"));
    }
    if fs::rename(&temporary, path).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(ServiceError::new("skill_file_write_failed"));
    }
    Ok(())
}

fn create_skill_package_candidate(
    skills_root: &Path,
    skill_id: &str,
    skill_content: &[u8],
    files: &[SkillPackageFileInput],
) -> Result<(String, String), ServiceError> {
    validate_skill_id(skill_id)?;
    validate_package_files(files)?;
    let relative_path = format!(
        "user-packages/{skill_id}/{}/{SKILL_FILE_NAME}",
        Uuid::new_v4().simple()
    );
    let skill_file_path = managed_skill_path(skills_root, &relative_path)?;
    let directory = skill_file_path
        .parent()
        .ok_or_else(|| ServiceError::new("skill_path_invalid"))?;
    let parent = directory
        .parent()
        .ok_or_else(|| ServiceError::new("skill_path_invalid"))?;
    fs::create_dir_all(parent).map_err(|_| ServiceError::new("skill_file_write_failed"))?;
    managed_skill_path(skills_root, &relative_path)?;
    fs::create_dir(directory).map_err(|_| ServiceError::new("skill_file_write_failed"))?;
    atomic_write(&skill_file_path, skill_content)?;
    for file in files {
        let target = managed_skill_path(directory, &file.path)?;
        atomic_write(&target, file.content.as_bytes())?;
    }
    let digest = package_digest(directory)?;
    sync_skill_candidate(directory)?;
    for ancestor in directory.ancestors().skip(1) {
        contexthub_platform_adapter::filesystem::sync_directory(ancestor)
            .map_err(|_| ServiceError::new("skill_package_sync_failed"))?;
        if ancestor == skills_root {
            break;
        }
    }
    contexthub_platform_adapter::filesystem::sync_directory(
        skills_root
            .parent()
            .ok_or_else(|| ServiceError::new("skill_path_invalid"))?,
    )
    .map_err(|_| ServiceError::new("skill_package_sync_failed"))?;
    Ok((relative_path, digest))
}

fn sync_skill_candidate(directory: &Path) -> Result<(), ServiceError> {
    for entry in fs::read_dir(directory).map_err(|_| ServiceError::new("skill_file_read_failed"))? {
        let entry = entry.map_err(|_| ServiceError::new("skill_file_read_failed"))?;
        let metadata = entry
            .file_type()
            .map_err(|_| ServiceError::new("skill_file_read_failed"))?;
        if metadata.is_symlink() {
            return Err(ServiceError::new("skill_path_invalid"));
        }
        if metadata.is_dir() {
            sync_skill_candidate(&entry.path())?;
        }
    }
    contexthub_platform_adapter::filesystem::sync_directory(directory)
        .map_err(|_| ServiceError::new("skill_package_sync_failed"))
}

fn read_package_files(directory: &Path) -> Result<Vec<SkillPackageFile>, ServiceError> {
    let mut files = Vec::new();
    for category in ["scripts", "references", "assets"] {
        let category_root = directory.join(category);
        if category_root.exists() {
            collect_package_files(directory, &category_root, category, &mut files, false)?;
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let total = files.iter().try_fold(0usize, |total, file| {
        usize::try_from(file.size_bytes)
            .ok()
            .and_then(|size| total.checked_add(size))
    });
    if files.len() > MAX_PACKAGE_FILES || total.is_none_or(|total| total > MAX_PACKAGE_BYTES) {
        return Err(ServiceError::new("skill_files_invalid"));
    }
    Ok(files)
}

fn collect_package_files(
    package_root: &Path,
    directory: &Path,
    category: &str,
    files: &mut Vec<SkillPackageFile>,
    skip_generated_cache: bool,
) -> Result<(), ServiceError> {
    if fs::symlink_metadata(directory).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(ServiceError::new("skill_path_invalid"));
    }
    let mut entries = fs::read_dir(directory)
        .map_err(|_| ServiceError::new("skill_file_read_failed"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ServiceError::new("skill_file_read_failed"))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        if skip_generated_cache
            && (entry.file_name() == "__pycache__"
                || entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "pyc"))
        {
            continue;
        }
        let metadata = entry
            .file_type()
            .map_err(|_| ServiceError::new("skill_file_read_failed"))?;
        if metadata.is_symlink() {
            return Err(ServiceError::new("skill_path_invalid"));
        }
        let path = entry.path();
        if metadata.is_dir() {
            collect_package_files(package_root, &path, category, files, skip_generated_cache)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(package_root)
                .map_err(|_| ServiceError::new("skill_path_invalid"))?
                .to_str()
                .ok_or_else(|| ServiceError::new("skill_path_non_unicode"))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            validate_package_relative_path(&relative)?;
            if entry
                .metadata()
                .map_err(|_| ServiceError::new("skill_file_read_failed"))?
                .len()
                > MAX_PACKAGE_FILE_BYTES as u64
            {
                return Err(ServiceError::new("skill_file_content_invalid"));
            }
            let content =
                fs::read(&path).map_err(|_| ServiceError::new("skill_file_read_failed"))?;
            if content.len() > MAX_PACKAGE_FILE_BYTES {
                return Err(ServiceError::new("skill_file_content_invalid"));
            }
            let content = String::from_utf8(content)
                .map_err(|_| ServiceError::new("skill_file_content_invalid"))?;
            files.push(SkillPackageFile {
                path: relative,
                category: category.to_owned(),
                size_bytes: i64::try_from(content.len())
                    .map_err(|_| ServiceError::new("skill_file_content_invalid"))?,
                content,
            });
        }
    }
    Ok(())
}

fn package_digest(directory: &Path) -> Result<String, ServiceError> {
    let skill_content = fs::read(directory.join(SKILL_FILE_NAME))
        .map_err(|_| ServiceError::new("skill_file_read_failed"))?;
    let files = read_package_files(directory)?;
    if files.is_empty() {
        return Ok(sha256_hex(&skill_content));
    }
    let mut hasher = Sha256::new();
    update_package_digest(&mut hasher, SKILL_FILE_NAME, &skill_content);
    for file in files {
        update_package_digest(&mut hasher, &file.path, file.content.as_bytes());
    }
    let digest = hasher.finalize();
    Ok(format!("{digest:x}"))
}

fn update_package_digest(hasher: &mut Sha256, path: &str, content: &[u8]) {
    hasher.update((path.len() as u64).to_be_bytes());
    hasher.update(path.as_bytes());
    hasher.update((content.len() as u64).to_be_bytes());
    hasher.update(content);
}

fn verify_package_digest(path: &Path, expected: &str) -> Result<(), ServiceError> {
    let directory = path
        .parent()
        .ok_or_else(|| ServiceError::new("skill_path_invalid"))?;
    if package_digest(directory)? != expected {
        return Err(ServiceError::new("skill_file_digest_mismatch"));
    }
    Ok(())
}

fn sha256_hex(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    format!("{digest:x}")
}

fn path_string(path: &Path) -> Result<String, ServiceError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| ServiceError::new("skill_path_non_unicode"))
}

fn system_creator_summary() -> SkillSummary {
    SkillSummary {
        skill_id: SYSTEM_CREATOR_ID.to_owned(),
        name: SYSTEM_CREATOR_NAME.to_owned(),
        description: SYSTEM_CREATOR_DESCRIPTION.to_owned(),
        source: "system".to_owned(),
        enabled: true,
        automatic: true,
        state_revision: SYSTEM_CREATOR_REVISION,
        updated_at: 0,
    }
}

fn system_capability_creator_summary() -> SkillSummary {
    SkillSummary {
        skill_id: SYSTEM_CAPABILITY_CREATOR_ID.to_owned(),
        name: SYSTEM_CAPABILITY_CREATOR_NAME.to_owned(),
        description: SYSTEM_CAPABILITY_CREATOR_DESCRIPTION.to_owned(),
        source: "system".to_owned(),
        enabled: true,
        automatic: true,
        state_revision: SYSTEM_CAPABILITY_CREATOR_REVISION,
        updated_at: 0,
    }
}

fn user_skill_summary(skill: UserSkillRecord) -> SkillSummary {
    SkillSummary {
        skill_id: skill.skill_id,
        name: skill.name,
        description: skill.description,
        source: "user".to_owned(),
        enabled: skill.enabled,
        automatic: skill.automatic,
        state_revision: skill.state_revision,
        updated_at: skill.updated_at,
    }
}

fn user_skill_detail(
    skill: UserSkillRecord,
    instructions: String,
    files: Vec<SkillPackageFile>,
) -> SkillDetail {
    SkillDetail {
        skill_id: skill.skill_id,
        name: skill.name,
        description: skill.description,
        source: "user".to_owned(),
        instructions,
        files,
        enabled: skill.enabled,
        automatic: skill.automatic,
        state_revision: skill.state_revision,
        created_at: skill.created_at,
        updated_at: skill.updated_at,
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "错误转换作为 map_err 的函数指针接收所有权"
)]
pub(super) fn skill_persistence_error(
    error: contexthub_persistence_sqlite::PersistenceError,
) -> ServiceError {
    ServiceError::new(error.reason().unwrap_or("skill_persistence_failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    pub(super) fn write_skill_zip(path: &Path, entries: &[(&str, &str)]) {
        let file = fs::File::create(path).unwrap();
        let mut archive = ZipWriter::new(file);
        for (name, content) in entries {
            archive
                .start_file(*name, SimpleFileOptions::default())
                .unwrap();
            archive.write_all(content.as_bytes()).unwrap();
        }
        archive.finish().unwrap();
    }

    #[test]
    fn system_creator_is_discoverable_and_refreshes_only_managed_content() {
        let root = std::env::temp_dir().join(format!("cilai-creator-upgrade-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let persistence = contexthub_persistence_sqlite::SqlitePersistence::open(
            &root.join("contexthub.db"),
            "test",
        )
        .unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let source = root.join("skills/system/skill-creator/SKILL.md");
        atomic_write(&source, b"previous creator").unwrap();
        let bundled = root.join("ai-runtime/cilai-harness/skills/.system/skill-creator/SKILL.md");
        atomic_write(&bundled, b"upstream creator").unwrap();
        let catalog = services
            .skill_runtime_catalog(&SkillRuntimeCatalogParams {})
            .unwrap();
        assert_eq!(catalog.skills.len(), 2);
        let creator = &catalog.skills[0];
        assert_eq!(creator.skill_id, SYSTEM_CREATOR_ID);
        assert!(creator.automatic);
        assert_eq!(creator.state_revision, SYSTEM_CREATOR_REVISION);
        assert_eq!(
            creator.content_digest,
            package_digest(source.parent().unwrap()).unwrap()
        );
        let (name, description, _) =
            parse_skill_file(&fs::read_to_string(&source).unwrap()).unwrap();
        assert_eq!(name, creator.name);
        assert_eq!(description, creator.description);
        assert_eq!(fs::read_to_string(&bundled).unwrap(), "upstream creator");
        let listed = services.skill_list(&SkillListParams {}).unwrap().skills;
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].skill_id, SYSTEM_CREATOR_ID);
        assert_eq!(listed[0].source, "system");
        assert_eq!(listed[1].skill_id, SYSTEM_CAPABILITY_CREATOR_ID);
        assert_eq!(listed[1].source, "system");
        assert!(listed[1].description.starts_with("把用户要解决的业务问题"));
        assert!(!root.join("skills/system/image-generation").exists());
        let capability_creator_path = root.join("skills/system/capability-creator/SKILL.md");
        assert_eq!(
            catalog.skills[1].state_revision,
            SYSTEM_CAPABILITY_CREATOR_REVISION
        );
        assert_eq!(
            catalog.skills[1].content_digest,
            sha256_hex(SYSTEM_CAPABILITY_CREATOR_CONTENT.as_bytes())
        );
        assert_eq!(
            fs::read_to_string(capability_creator_path).unwrap(),
            SYSTEM_CAPABILITY_CREATOR_CONTENT
        );
        let conversation = services
            .skill_conversation_catalog(&SkillConversationCatalogParams {})
            .unwrap();
        assert!(conversation.skills[0].automatic);
        assert_eq!(
            conversation.skills[0].state_revision,
            creator.state_revision
        );
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn system_creator_is_listed_but_rejects_detail_and_management() {
        let root = std::env::temp_dir().join(format!("cilai-system-detail-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let persistence = contexthub_persistence_sqlite::SqlitePersistence::open(
            &root.join("contexthub.db"),
            "test",
        )
        .unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let listed = services.skill_list(&SkillListParams {}).unwrap().skills;
        let creator = &listed[0];
        let catalog = services
            .skill_conversation_catalog(&SkillConversationCatalogParams {})
            .unwrap();
        assert_eq!(creator.skill_id, catalog.skills[0].skill_id);
        assert_eq!(creator.name, catalog.skills[0].name);
        assert_eq!(creator.description, catalog.skills[0].description);
        assert_eq!(creator.state_revision, catalog.skills[0].state_revision);
        assert_eq!(creator.source, "system");
        assert!(
            services
                .persistence()
                .unwrap()
                .list_user_skills()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            services
                .skill_get(&SkillGetParams {
                    skill_id: SYSTEM_CREATOR_ID.to_owned(),
                })
                .unwrap_err()
                .reason,
            "skill_id_invalid"
        );

        let save = SkillSaveParams {
            skill_id: Some(SYSTEM_CREATOR_ID.to_owned()),
            name: creator.name.clone(),
            description: creator.description.clone(),
            instructions: "替换系统技能".to_owned(),
            files: None,
            enabled: true,
            automatic: false,
            expected_revision: Some(creator.state_revision),
            client_operation_id: "edit-system-skill".to_owned(),
            request_fingerprint: "a".repeat(64),
        };
        assert_eq!(
            services.skill_save(&save, 1).unwrap_err().reason,
            "skill_id_invalid"
        );
        assert_eq!(
            services
                .skill_set_state(
                    &SkillSetStateParams {
                        skill_id: SYSTEM_CREATOR_ID.to_owned(),
                        enabled: false,
                        automatic: false,
                        expected_revision: creator.state_revision,
                        client_operation_id: "disable-system-skill".to_owned(),
                        request_fingerprint: "b".repeat(64),
                    },
                    2
                )
                .unwrap_err()
                .reason,
            "skill_id_invalid"
        );
        assert_eq!(
            services
                .skill_delete(
                    &SkillDeleteParams {
                        skill_id: SYSTEM_CREATOR_ID.to_owned(),
                        expected_revision: creator.state_revision,
                        client_operation_id: "delete-system-skill".to_owned(),
                        request_fingerprint: "c".repeat(64),
                    },
                    3
                )
                .unwrap_err()
                .reason,
            "skill_id_invalid"
        );
        assert_eq!(
            services
                .skill_get(&SkillGetParams {
                    skill_id: "system.unknown".to_owned(),
                })
                .unwrap_err()
                .reason,
            "skill_id_invalid"
        );
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_skill_path_rejects_parent_traversal() {
        assert_eq!(
            managed_skill_path(Path::new("/tmp/skills"), "../outside/SKILL.md")
                .unwrap_err()
                .reason,
            "skill_path_invalid"
        );
    }

    #[cfg(unix)]
    #[test]
    fn skills_root_rejects_a_symbolic_link() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("contexthub-skill-link-{}", Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("contexthub-skill-outside-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("skills")).unwrap();
        let services = AppServerServices::with_data_dir(None, Some(root.clone()));

        assert_eq!(
            services.skills_root().unwrap_err().reason,
            "skill_path_invalid"
        );

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn render_and_read_frontmatter_boundary() {
        let rendered =
            render_skill_file("日报", "生成日报", "先汇总，再输出。").expect("render skill");
        assert!(rendered.starts_with("---\nname: \"日报\""));
        assert!(rendered.ends_with("先汇总，再输出。\n"));
    }

    #[test]
    fn user_skills_cannot_reuse_system_skill_names() {
        for name in [SYSTEM_CREATOR_NAME, SYSTEM_CAPABILITY_CREATOR_NAME] {
            assert_eq!(
                validate_user_skill_name(&normalize_name(name))
                    .unwrap_err()
                    .reason,
                "skill_name_conflict"
            );
        }
        validate_user_skill_name(&normalize_name("项目周报")).unwrap();
        validate_user_skill_name(&normalize_name("图片生成")).unwrap();
    }

    #[test]
    fn package_paths_reject_traversal_and_duplicate_case() {
        assert_eq!(
            validate_package_relative_path("scripts/../escape.sh")
                .unwrap_err()
                .reason,
            "skill_file_path_invalid"
        );
        assert_eq!(
            validate_package_files(&[
                SkillPackageFileInput {
                    path: "references/Guide.md".to_owned(),
                    content: "A".to_owned(),
                },
                SkillPackageFileInput {
                    path: "references/guide.md".to_owned(),
                    content: "B".to_owned(),
                },
            ])
            .unwrap_err()
            .reason,
            "skill_files_invalid"
        );
    }

    #[test]
    fn zip_import_installs_package_and_later_edits_preserve_resources() {
        let root = std::env::temp_dir().join(format!("contexthub-skill-import-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("weekly.zip");
        write_skill_zip(
            &archive_path,
            &[
                (
                    SKILL_FILE_NAME,
                    "---\nname: weekly-review\ndescription: >\n  Review the week and\n  produce a concise report.\n---\n\nRead references/format.md, then run scripts/check.sh.\n",
                ),
                (
                    "references/format.md",
                    "# Format\n\nProgress, risks, next steps.\n",
                ),
                ("scripts/check.sh", "#!/bin/sh\nprintf 'ok\\n'\n"),
            ],
        );
        let persistence = contexthub_persistence_sqlite::SqlitePersistence::open(
            &root.join("contexthub.db"),
            "test",
        )
        .unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let imported = services
            .skill_import(
                &SkillImportParams {
                    archive_path: archive_path.to_string_lossy().into_owned(),
                    client_operation_id: "import-user-skill".to_owned(),
                    request_fingerprint: "d".repeat(64),
                },
                20,
            )
            .unwrap()
            .skill;
        assert_eq!(imported.name, "weekly-review");
        assert_eq!(
            imported.description,
            "Review the week and produce a concise report."
        );
        assert_eq!(imported.files.len(), 2);

        let resource_edit = services
            .skill_save(
                &SkillSaveParams {
                    skill_id: Some(imported.skill_id.clone()),
                    name: imported.name.clone(),
                    description: imported.description.clone(),
                    instructions: imported.instructions.clone(),
                    files: Some(Vec::new()),
                    enabled: true,
                    automatic: false,
                    expected_revision: Some(imported.state_revision),
                    client_operation_id: "edit-imported-resource".to_owned(),
                    request_fingerprint: "f".repeat(64),
                },
                21,
            )
            .unwrap_err();
        assert_eq!(resource_edit.reason, "skill_files_read_only");

        let edited = services
            .skill_save(
                &SkillSaveParams {
                    skill_id: Some(imported.skill_id),
                    name: "weekly-review".to_owned(),
                    description: "Updated description".to_owned(),
                    instructions: imported.instructions,
                    files: None,
                    enabled: true,
                    automatic: false,
                    expected_revision: Some(imported.state_revision),
                    client_operation_id: "edit-imported-skill".to_owned(),
                    request_fingerprint: "e".repeat(64),
                },
                22,
            )
            .unwrap()
            .skill;
        assert_eq!(edited.files, imported.files);

        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_import_registers_generated_skill_once_and_rejects_outside_workspace() {
        let root =
            std::env::temp_dir().join(format!("contexthub-skill-directory-{}", Uuid::new_v4()));
        let workspace = root.join("workspace");
        let skill_directory = workspace.join("skills").join("weekly");
        fs::create_dir_all(skill_directory.join("scripts").join("__pycache__")).unwrap();
        fs::create_dir_all(skill_directory.join("references")).unwrap();
        fs::write(
            skill_directory.join(SKILL_FILE_NAME),
            "---\nname: weekly\ndescription: Weekly review\n---\n\nRead references/format.md.\n",
        )
        .unwrap();
        fs::write(skill_directory.join("references/format.md"), "# Format\n").unwrap();
        fs::write(
            skill_directory.join("scripts/__pycache__/generated.pyc"),
            [0, 1, 2],
        )
        .unwrap();
        let persistence = contexthub_persistence_sqlite::SqlitePersistence::open(
            &root.join("contexthub.db"),
            "test",
        )
        .unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let params = SkillImportDirectoryParams {
            skill_file_path: skill_directory
                .join(SKILL_FILE_NAME)
                .to_string_lossy()
                .into_owned(),
            workspace_root: workspace.to_string_lossy().into_owned(),
            client_operation_id: "generated-weekly".to_owned(),
            request_fingerprint: "a".repeat(64),
        };
        let first = services.skill_import_directory(&params, 20).unwrap().skill;
        assert_eq!(first.name, "weekly");
        assert_eq!(first.files.len(), 1);
        assert_eq!(
            services
                .skill_list(&SkillListParams {})
                .unwrap()
                .skills
                .iter()
                .filter(|skill| skill.source == "user")
                .count(),
            1
        );
        fs::remove_dir_all(&skill_directory).unwrap();
        let repeated = services.skill_import_directory(&params, 21).unwrap().skill;
        assert_eq!(repeated.skill_id, first.skill_id);
        let invalid_directory = workspace.join("skills").join("invalid");
        fs::create_dir_all(&invalid_directory).unwrap();
        fs::write(
            invalid_directory.join(SKILL_FILE_NAME),
            "---\nname: invalid\ndescription: Invalid package\n---\n\nContent.\n",
        )
        .unwrap();
        fs::write(invalid_directory.join("extra.txt"), "not included").unwrap();
        let invalid = SkillImportDirectoryParams {
            skill_file_path: invalid_directory
                .join(SKILL_FILE_NAME)
                .to_string_lossy()
                .into_owned(),
            client_operation_id: "invalid-directory".to_owned(),
            ..params.clone()
        };
        assert_eq!(
            services
                .skill_import_directory(&invalid, 22)
                .unwrap_err()
                .reason,
            "skill_directory_structure_invalid"
        );
        let outside = SkillImportDirectoryParams {
            skill_file_path: root
                .join("skills")
                .join(
                    services
                        .persistence()
                        .unwrap()
                        .user_skill(&first.skill_id)
                        .unwrap()
                        .unwrap()
                        .managed_relative_path,
                )
                .to_string_lossy()
                .into_owned(),
            client_operation_id: "outside-workspace".to_owned(),
            ..params
        };
        assert_eq!(
            services
                .skill_import_directory(&outside, 22)
                .unwrap_err()
                .reason,
            "skill_path_invalid"
        );
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_import_requires_skill_file_at_archive_root() {
        let root =
            std::env::temp_dir().join(format!("contexthub-skill-zip-root-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("nested.zip");
        write_skill_zip(
            &archive_path,
            &[(
                "nested/SKILL.md",
                "---\nname: nested\ndescription: invalid wrapper\n---\n\nDo the task.\n",
            )],
        );
        assert_eq!(
            read_skill_archive(&archive_path).unwrap_err().reason,
            "skill_file_path_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zip_import_rejects_path_traversal() {
        let root =
            std::env::temp_dir().join(format!("contexthub-skill-zip-path-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let archive_path = root.join("traversal.zip");
        write_skill_zip(
            &archive_path,
            &[
                (
                    SKILL_FILE_NAME,
                    "---\nname: traversal\ndescription: invalid path\n---\n\nDo the task.\n",
                ),
                ("../outside.txt", "must not escape"),
            ],
        );
        assert_eq!(
            read_skill_archive(&archive_path).unwrap_err().reason,
            "skill_archive_structure_invalid"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "单个用例连续验证技能创建、停用和删除后的目录状态"
    )]
    fn user_skill_crud_separates_management_and_conversation_catalogs() {
        let root = std::env::temp_dir().join(format!("contexthub-skills-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let persistence = contexthub_persistence_sqlite::SqlitePersistence::open(
            &root.join("contexthub.db"),
            "test",
        )
        .unwrap();
        let mut services = AppServerServices::with_data_dir(Some(persistence), Some(root.clone()));
        let saved = services
            .skill_save(
                &SkillSaveParams {
                    skill_id: None,
                    name: "周报整理".to_owned(),
                    description: "整理项目周报".to_owned(),
                    instructions: "先阅读 references/周报结构.md，再运行 scripts/check.py。"
                        .to_owned(),
                    files: Some(vec![
                        SkillPackageFileInput {
                            path: "references/周报结构.md".to_owned(),
                            content: "# 输出结构\n\n完成事项、风险、下周计划。".to_owned(),
                        },
                        SkillPackageFileInput {
                            path: "scripts/check.py".to_owned(),
                            content: "print('check')\n".to_owned(),
                        },
                    ]),
                    enabled: true,
                    automatic: false,
                    expected_revision: None,
                    client_operation_id: "save-user-skill".to_owned(),
                    request_fingerprint: "a".repeat(64),
                },
                10,
            )
            .unwrap()
            .skill;
        assert_eq!(
            saved.instructions,
            "先阅读 references/周报结构.md，再运行 scripts/check.py。"
        );
        assert_eq!(saved.files.len(), 2);
        assert_eq!(saved.files[0].path, "references/周报结构.md");
        assert_eq!(saved.files[1].path, "scripts/check.py");

        let managed = services.skill_list(&SkillListParams {}).unwrap();
        assert_eq!(managed.skills.len(), 3);
        assert_eq!(managed.skills[0].source, "user");

        let conversation = services
            .skill_conversation_catalog(&SkillConversationCatalogParams {})
            .unwrap();
        assert_eq!(conversation.skills.len(), 3);
        assert_eq!(conversation.skills[0].skill_id, SYSTEM_CREATOR_ID);
        assert_eq!(conversation.skills[0].source, "system");
        assert!(
            conversation
                .skills
                .iter()
                .any(|skill| skill.skill_id == saved.skill_id && skill.source == "user")
        );

        let detail = services
            .skill_get(&SkillGetParams {
                skill_id: saved.skill_id.clone(),
            })
            .unwrap()
            .skill;
        assert_eq!(detail.instructions, saved.instructions);
        assert_eq!(detail.files, saved.files);
        let runtime = services
            .skill_runtime_catalog(&SkillRuntimeCatalogParams {})
            .unwrap();
        assert_eq!(runtime.skills.len(), 3);
        assert!(runtime.skills.iter().all(|skill| {
            Path::new(&skill.managed_path).ends_with(SKILL_FILE_NAME)
                && !skill.content_digest.is_empty()
        }));
        let user_runtime = runtime
            .skills
            .iter()
            .find(|skill| skill.skill_id == saved.skill_id)
            .unwrap();
        let user_directory = Path::new(&user_runtime.managed_path).parent().unwrap();
        assert!(user_directory.join("references/周报结构.md").is_file());
        assert!(user_directory.join("scripts/check.py").is_file());

        let disabled = services
            .skill_set_state(
                &SkillSetStateParams {
                    skill_id: saved.skill_id.clone(),
                    enabled: false,
                    automatic: false,
                    expected_revision: saved.state_revision,
                    client_operation_id: "disable-user-skill".to_owned(),
                    request_fingerprint: "b".repeat(64),
                },
                11,
            )
            .unwrap()
            .skill;
        assert!(!disabled.enabled);
        assert_eq!(
            services
                .skill_conversation_catalog(&SkillConversationCatalogParams {})
                .unwrap()
                .skills
                .len(),
            2
        );
        assert_eq!(
            services
                .skill_runtime_catalog(&SkillRuntimeCatalogParams {})
                .unwrap()
                .skills
                .len(),
            2
        );

        assert!(
            services
                .skill_delete(
                    &SkillDeleteParams {
                        skill_id: saved.skill_id,
                        expected_revision: disabled.state_revision,
                        client_operation_id: "delete-user-skill".to_owned(),
                        request_fingerprint: "c".repeat(64),
                    },
                    12,
                )
                .unwrap()
                .deleted
        );
        let listed = services.skill_list(&SkillListParams {}).unwrap().skills;
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].skill_id, SYSTEM_CREATOR_ID);
        assert_eq!(listed[1].skill_id, SYSTEM_CAPABILITY_CREATOR_ID);
        assert!(listed.iter().all(|skill| skill.source == "system"));

        drop(services);
        fs::remove_dir_all(root).unwrap();
    }
}
