//! 个人资料备份与恢复。归档只读取当前资料，外部项目原文件单独列示。
use crate::{AppServerServices, ServiceError};
use contexthub_application::{UnitOfWork, WorkspaceRepository};
use contexthub_platform_adapter::{filesystem, security};
use contexthub_protocol::{
    PERSONAL_BACKUP_FORMAT, PERSONAL_DATA_VERSION, PersonalBackupArtifact,
    PersonalBackupCaptureParams, PersonalBackupFile, PersonalBackupManifest,
    PersonalBackupNativeHome, PersonalBackupPrepareParams, PersonalBackupWorkspace,
    PersonalDataCancelParams, PersonalDataCategory, PersonalDataIssue, PersonalDataListParams,
    PersonalDataOperation, PersonalDataOperationKind, PersonalDataOperationState,
    PersonalDataParams, PersonalDataQuiescence, PersonalDataResult, PersonalMaintenanceFence,
    PersonalProfileManifest, PersonalRestoreApplyPrepareParams, PersonalRestorePrepareParams,
    PersonalRestorePreview, PersonalRestoreWorkspacePlanParams,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};
const MAX_FILE: u64 = 128 * 1024 * 1024;
const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 50_000;
const MANIFEST: &str = "manifest.json";
mod workspaces;
fn error(reason: &'static str) -> ServiceError {
    ServiceError::new(reason)
}
fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
fn encoded(value: &impl serde::Serialize) -> Result<Vec<u8>, ServiceError> {
    serde_json::to_vec(value).map_err(|_| error("personal_data_record_invalid"))
}
fn fingerprint(value: &impl serde::Serialize) -> Result<String, ServiceError> {
    Ok(digest(&encoded(value)?))
}
fn issue(
    code: &str,
    category: Option<PersonalDataCategory>,
    path: Option<String>,
) -> PersonalDataIssue {
    PersonalDataIssue {
        code: code.into(),
        category,
        object_id: None,
        relative_path: path,
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0', ':'])
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
        && Path::new(path)
            .components()
            .all(|p| matches!(p, Component::Normal(_)))
}
fn inspect(path: &Path) -> Result<fs::Metadata, ServiceError> {
    let m = fs::symlink_metadata(path).map_err(|_| error("personal_data_read_failed"))?;
    if m.file_type().is_symlink() {
        return Err(error("personal_data_symlink_denied"));
    }
    Ok(m)
}
fn secure_dir(path: &Path) -> Result<(), ServiceError> {
    security::ensure_restricted_directory(path).map_err(|_| error("personal_data_write_failed"))
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ServiceError> {
    let parent = path
        .parent()
        .ok_or_else(|| error("personal_data_path_invalid"))?;
    secure_dir(parent)?;
    let mut file = security::create_new_restricted_file(path)
        .map_err(|_| error("personal_data_write_failed"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| error("personal_data_write_failed"))?;
    filesystem::sync_directory(parent).map_err(|_| error("personal_data_write_failed"))
}
fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>, ServiceError> {
    for ancestor in path.ancestors().skip(1) {
        if !inspect(ancestor)?.is_dir() {
            return Err(error("personal_data_path_invalid"));
        }
    }
    let before = filesystem::symlink_metadata_with_stable_identity(path)
        .map_err(|_| error("personal_data_read_failed"))?;
    if !before.metadata.is_file()
        || before.metadata.file_type().is_symlink()
        || before.metadata.len() > max
        || filesystem::is_cloud_placeholder_file(&before.metadata)
    {
        return Err(error("personal_data_file_invalid"));
    }
    let mut file = filesystem::open_regular_file_no_follow(path, false)
        .map_err(|_| error("personal_data_read_failed"))?;
    let opened = filesystem::opened_file_metadata_with_stable_identity(&file)
        .map_err(|_| error("personal_data_read_failed"))?;
    if opened.stable_identity != before.stable_identity {
        return Err(error("personal_data_file_changed"));
    }
    let mut bytes = vec![];
    Read::by_ref(&mut file)
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("personal_data_read_failed"))?;
    let after = filesystem::symlink_metadata_with_stable_identity(path)
        .map_err(|_| error("personal_data_file_changed"))?;
    if after.stable_identity != before.stable_identity
        || after.metadata.len() != before.metadata.len()
        || after.metadata.modified().ok() != before.metadata.modified().ok()
        || bytes.len() as u64 != before.metadata.len()
    {
        return Err(error("personal_data_file_changed"));
    }
    Ok(bytes)
}
fn operation_root(profile: &Path, id: &str) -> Result<PathBuf, ServiceError> {
    if !id.starts_with("data_") || !valid_id(&id[5..]) {
        return Err(error("personal_data_operation_invalid"));
    }
    let path = profile.join("personal-data-operations").join(id);
    secure_dir(&path)?;
    Ok(path)
}
fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ServiceError> {
    serde_json::from_value(value).map_err(|_| error("personal_data_request_invalid"))
}
fn db_error(e: &contexthub_persistence_sqlite::PersistenceError) -> ServiceError {
    error(e.reason().unwrap_or("personal_data_storage_failed"))
}
fn profile_manifest(profile: &Path) -> Result<PersonalProfileManifest, ServiceError> {
    let root = profile
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| error("personal_data_profile_not_ready"))?;
    let m: PersonalProfileManifest =
        serde_json::from_slice(&read_limited(&root.join("local-profile.json"), 4096)?)
            .map_err(|_| error("personal_data_profile_not_ready"))?;
    if m.schema_version != PERSONAL_DATA_VERSION
        || !valid_id(&m.profile_id)
        || profile.file_name().and_then(|v| v.to_str()) != Some(m.profile_id.as_str())
    {
        return Err(error("personal_data_profile_not_ready"));
    }
    Ok(m)
}

impl AppServerServices {
    pub(super) fn data_context(&self) -> Result<(PathBuf, String), ServiceError> {
        let profile = self
            .data_dir
            .clone()
            .ok_or_else(|| error("personal_data_profile_not_ready"))?;
        let id = self
            .personal_profile_id()
            .ok_or_else(|| error("personal_data_profile_not_ready"))?;
        for path in profile.ancestors() {
            let m = inspect(path)?;
            if !m.is_dir() {
                return Err(error("personal_data_path_invalid"));
            }
        }
        Ok((profile, id))
    }
    fn data_read(&self, profile: &str, id: &str) -> Result<PersonalDataOperation, ServiceError> {
        self.persistence
            .as_ref()
            .ok_or_else(|| error("persistence_unavailable"))?
            .personal_data_operation(profile, id)
            .map_err(|e| db_error(&e))?
            .ok_or_else(|| error("personal_data_operation_not_found"))
    }
    fn data_transition(
        &mut self,
        op: &PersonalDataOperation,
        expected: &PersonalDataOperationState,
    ) -> Result<PersonalDataOperation, ServiceError> {
        self.persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .transition_personal_data(op, expected)
            .map_err(|e| db_error(&e))
    }
    pub(crate) fn personal_data_rpc(
        &mut self,
        method: &str,
        params: Value,
        instance: &str,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let (profile, id) = self.data_context()?;
        let result = match method {
            "personalData/backupPrepare" => {
                self.data_backup_prepare(&profile, &id, parse(params)?, now)?
            }
            "personalData/backupCapture" => {
                self.data_backup_capture(&profile, &id, &parse(params)?, instance, now)?
            }
            "personalData/restorePrepare" => {
                self.data_restore_prepare(&profile, &id, &parse(params)?, now)?
            }
            "personalData/restoreWorkspacePlan" => {
                self.data_restore_workspace_plan(&profile, &id, &parse(params)?, now)?
            }
            "personalData/restoreApplyPrepare" => {
                self.data_restore_apply_prepare(&profile, &id, &parse(params)?, instance, now)?
            }
            "personalData/restoreAbortBeforeHelper" => self
                .persistence
                .as_mut()
                .ok_or_else(|| error("persistence_unavailable"))?
                .abort_personal_restore_before_helper(&id, instance, &parse(params)?, now)
                .map_err(|e| db_error(&e))?,
            "personalData/resolveInterrupted" => {
                let p: PersonalDataCancelParams = parse(params)?;
                self.data_resolve_interrupted(&profile, &id, &p, now)?
            }
            "personalData/get" | "personalData/reconcile" => {
                let p: PersonalDataParams = parse(params)?;
                self.data_read(&id, &p.operation_id)?
            }
            "personalData/list" => {
                let _: PersonalDataListParams = parse(params)?;
                return serde_json::to_value(
                    self.persistence
                        .as_ref()
                        .ok_or_else(|| error("persistence_unavailable"))?
                        .personal_data_operations(&id)
                        .map_err(|e| db_error(&e))?,
                )
                .map_err(|_| error("personal_data_record_invalid"));
            }
            "personalData/cancel" => {
                let p: PersonalDataCancelParams = parse(params)?;
                let mut op = self.data_read(&id, &p.operation_id)?;
                if op.preview_digest != p.preview_digest {
                    return Err(error("personal_data_preview_changed"));
                }
                if op.state == PersonalDataOperationState::Cancelled {
                    return serde_json::to_value(op)
                        .map_err(|_| error("personal_data_record_invalid"));
                }
                if !matches!(
                    op.state,
                    PersonalDataOperationState::Prepared
                        | PersonalDataOperationState::Incomplete
                        | PersonalDataOperationState::ReadyToApply
                ) {
                    return Err(error("personal_data_operation_unconfirmed"));
                }
                let old = op.state.clone();
                op.state = PersonalDataOperationState::Cancelled;
                op.updated_at = now;
                self.data_transition(&op, &old)?
            }
            "personalData/backupArtifact" => {
                let p: PersonalDataParams = parse(params)?;
                let op = self.data_read(&id, &p.operation_id)?;
                if op.kind != PersonalDataOperationKind::Backup
                    || op.state != PersonalDataOperationState::Completed
                {
                    return Err(error("personal_data_backup_not_complete"));
                }
                let path = operation_root(&profile, &op.operation_id)?.join("backup.zip");
                let bytes = read_limited(&path, MAX_TOTAL)?;
                if op.result.as_ref().and_then(|r| r.artifact_digest.as_ref())
                    != Some(&digest(&bytes))
                {
                    return Err(error("personal_data_artifact_changed"));
                }
                return serde_json::to_value(PersonalBackupArtifact {
                    operation_id: op.operation_id,
                    path: path.to_string_lossy().into_owned(),
                    digest: digest(&bytes),
                    bytes: bytes.len() as u64,
                })
                .map_err(|_| error("personal_data_record_invalid"));
            }
            _ => return Err(error("personal_data_method_unknown")),
        };
        serde_json::to_value(result).map_err(|_| error("personal_data_record_invalid"))
    }
    fn data_resolve_interrupted(
        &mut self,
        profile: &Path,
        id: &str,
        p: &PersonalDataCancelParams,
        now: i64,
    ) -> Result<PersonalDataOperation, ServiceError> {
        let mut op = self.data_read(id, &p.operation_id)?;
        if op.preview_digest != p.preview_digest {
            return Err(error("personal_data_preview_changed"));
        }
        if !matches!(
            op.state,
            PersonalDataOperationState::Capturing | PersonalDataOperationState::Applying
        ) {
            return Err(error("personal_data_operation_unconfirmed"));
        }
        self.persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .verify_interrupted_personal_data(id, &op.operation_id)
            .map_err(|e| db_error(&e))?;
        let old = op.state.clone();
        op.state = PersonalDataOperationState::Incomplete;
        op.updated_at = now;
        op.blockers = vec![issue(
            "personal_data_interrupted_result_unconfirmed",
            None,
            None,
        )];
        if old == PersonalDataOperationState::Capturing {
            let path = operation_root(profile, &op.operation_id)?.join("backup.zip");
            if path.exists() {
                let bytes = read_limited(&path, MAX_TOTAL)?;
                let (manifest, _) = validate_archive(&bytes)?;
                if manifest.profile.profile_id != id
                    || manifest.source_profile_path != profile.to_string_lossy()
                {
                    return Err(error("personal_data_artifact_changed"));
                }
                op.backup_manifest = Some(manifest);
                op.state = PersonalDataOperationState::Completed;
                op.blockers.clear();
                op.result = Some(PersonalDataResult {
                    artifact_relative_path: Some(format!(
                        "personal-data-operations/{}/backup.zip",
                        op.operation_id
                    )),
                    artifact_digest: Some(digest(&bytes)),
                    restored_profile_id: None,
                    retained_current_relative_path: None,
                    verified_categories: vec![
                        PersonalDataCategory::BusinessDatabase,
                        PersonalDataCategory::NativeHistory,
                        PersonalDataCategory::NativeMemory,
                        PersonalDataCategory::Skills,
                        PersonalDataCategory::Applications,
                        PersonalDataCategory::ApplicationData,
                        PersonalDataCategory::Attachments,
                        PersonalDataCategory::Configuration,
                    ],
                    pending_categories: vec![],
                    issues: vec![],
                    restored_workspaces: None,
                });
            }
        } else if profile_manifest(profile)?.profile_id != id {
            return Err(error("personal_data_profile_changed"));
        }
        self.data_transition(&op, &old)
    }
    #[expect(clippy::too_many_lines, reason = "同一准备步骤生成明确的范围和摘要")]
    fn data_backup_prepare(
        &mut self,
        profile: &Path,
        id: &str,
        p: PersonalBackupPrepareParams,
        now: i64,
    ) -> Result<PersonalDataOperation, ServiceError> {
        if p.client_operation_id.trim().is_empty() || p.client_operation_id.len() > 240 {
            return Err(error("personal_data_request_invalid"));
        }
        let hash = fingerprint(&p)?;
        let db = self
            .persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?;
        if let Some(old) = db
            .personal_data_by_client_operation(id, &p.client_operation_id, &hash)
            .map_err(|e| db_error(&e))?
        {
            return Ok(old);
        }
        let workspaces = db.transact(|r| r.workspaces()).map_err(|e| db_error(&e))?;
        let known: BTreeSet<_> = workspaces.iter().map(|w| w.workspace_id.as_str()).collect();
        if p.selection
            .include_workspace_files
            .iter()
            .any(|id| !known.contains(id.as_str()))
        {
            return Err(error("personal_data_workspace_unknown"));
        }
        let mut seen = BTreeSet::new();
        if p.selection
            .include_workspace_files
            .iter()
            .any(|id| !seen.insert(id))
        {
            return Err(error("personal_data_request_invalid"));
        }
        let snapshots = db
            .personal_data_workspace_snapshots(&p.selection.include_workspace_files)
            .map_err(|e| db_error(&e))?;
        let mut exclusions = vec![
            issue(
                "local_credentials_and_keys_excluded",
                Some(PersonalDataCategory::Configuration),
                None,
            ),
            issue(
                "browser_sessions_excluded",
                Some(PersonalDataCategory::Configuration),
                None,
            ),
            issue(
                "native_auth_and_environment_excluded",
                Some(PersonalDataCategory::Configuration),
                None,
            ),
        ];
        let workspaces = workspaces
            .into_iter()
            .map(|w| {
                let selected = p
                    .selection
                    .include_workspace_files
                    .iter()
                    .any(|id| id == w.workspace_id.as_str());
                if !selected {
                    exclusions.push(issue(
                        "workspace_originals_not_included",
                        Some(PersonalDataCategory::WorkspaceOriginals),
                        Some(w.local_root.clone()),
                    ));
                }
                PersonalBackupWorkspace {
                    workspace_id: w.workspace_id.as_str().into(),
                    display_name: w.display_name,
                    original_path: w.local_root,
                    original_files_included: false,
                    archive_prefix: selected.then(|| workspaces::prefix(w.workspace_id.as_str())),
                    content_snapshot: snapshots.get(w.workspace_id.as_str()).cloned(),
                }
            })
            .collect();
        let manifest = PersonalBackupManifest {
            format: PERSONAL_BACKUP_FORMAT.into(),
            format_version: 2,
            profile: profile_manifest(profile)?,
            source_profile_path: profile.to_string_lossy().into_owned(),
            schema_migration_version: 0,
            app_server_version: env!("CARGO_PKG_VERSION").into(),
            captured_at: now,
            complete: false,
            selection: p.selection.clone(),
            files: vec![],
            native_homes: vec![],
            workspaces,
            exclusions,
        };
        let preview_digest = fingerprint(&manifest)?;
        let op = PersonalDataOperation {
            operation_id: format!("data_{}", uuid::Uuid::new_v4().simple()),
            profile_id: id.into(),
            kind: PersonalDataOperationKind::Backup,
            state: PersonalDataOperationState::Prepared,
            preview_digest,
            created_at: now,
            updated_at: now,
            selection: Some(p.selection),
            backup_manifest: Some(manifest),
            restore_preview: None,
            blockers: vec![],
            result: None,
        };
        db.prepare_personal_data(&op, &p.client_operation_id, &hash)
            .map_err(|e| db_error(&e))
    }
    #[expect(
        clippy::too_many_lines,
        reason = "捕获的静止核验、快照发布及终态必须依次执行"
    )]
    fn data_backup_capture(
        &mut self,
        profile: &Path,
        id: &str,
        p: &PersonalBackupCaptureParams,
        instance: &str,
        now: i64,
    ) -> Result<PersonalDataOperation, ServiceError> {
        let mut op = self.data_read(id, &p.operation_id)?;
        if op.kind != PersonalDataOperationKind::Backup || op.preview_digest != p.preview_digest {
            return Err(error("personal_data_preview_changed"));
        }
        if op.state == PersonalDataOperationState::Completed {
            return Ok(op);
        }
        if !matches!(
            op.state,
            PersonalDataOperationState::Prepared | PersonalDataOperationState::Incomplete
        ) {
            return Err(error("personal_data_operation_unconfirmed"));
        }
        let old = op.state.clone();
        op.state = PersonalDataOperationState::Capturing;
        op.updated_at = now;
        self.data_transition(&op, &old)?;
        let capture = (|| {
            let proof = p
                .quiescence
                .as_ref()
                .ok_or_else(|| error("personal_data_quiescence_unconfirmed"))?;
            self.persistence
                .as_mut()
                .ok_or_else(|| error("persistence_unavailable"))?
                .verify_personal_data_quiescence(id, &op.operation_id, instance, proof)
                .map_err(|e| db_error(&e))?;
            let expected = discover_native_homes(profile)?;
            let mut supplied = proof
                .native_homes
                .iter()
                .map(|h| h.relative_path.clone())
                .collect::<Vec<_>>();
            supplied.sort();
            if supplied.windows(2).any(|v| v[0] == v[1]) || supplied != expected {
                return Err(error("personal_data_native_homes_unconfirmed"));
            }
            let root = operation_root(profile, &op.operation_id)?;
            let candidate = root.join(format!("capture-{}", uuid::Uuid::new_v4().simple()));
            secure_dir(&candidate)?;
            let mut manifest = op
                .backup_manifest
                .clone()
                .ok_or_else(|| error("personal_data_record_invalid"))?;
            manifest.captured_at = now;
            manifest.native_homes.clone_from(&proof.native_homes);
            manifest.files.clear();
            manifest.schema_migration_version = self
                .persistence
                .as_ref()
                .ok_or_else(|| error("persistence_unavailable"))?
                .snapshot_personal_data_database(&candidate.join("contexthub.db"))
                .map_err(|e| db_error(&e))?;
            let bases = workspaces::snapshots(&manifest)?;
            let database = self
                .persistence
                .as_ref()
                .ok_or_else(|| error("persistence_unavailable"))?;
            database
                .verify_personal_data_workspace_snapshot(&candidate.join("contexthub.db"), &bases)
                .map_err(|e| db_error(&e))?;
            if database
                .personal_data_workspace_snapshots(&bases.keys().cloned().collect::<Vec<_>>())
                .map_err(|e| db_error(&e))?
                != bases
            {
                return Err(error("personal_data_workspace_index_changed"));
            }
            let mut files = BTreeMap::new();
            workspaces::capture(&mut manifest, &mut files)?;
            files.insert(
                "profile/contexthub.db".into(),
                (
                    PersonalDataCategory::BusinessDatabase,
                    read_limited(&candidate.join("contexthub.db"), MAX_FILE)?,
                ),
            );
            for (name, category) in [
                ("skills", PersonalDataCategory::Skills),
                ("capability-projects", PersonalDataCategory::Applications),
                ("personal-capabilities", PersonalDataCategory::Applications),
                (
                    "personal-capability-data",
                    PersonalDataCategory::ApplicationData,
                ),
                (
                    "personal-capability-preview-data",
                    PersonalDataCategory::ApplicationData,
                ),
                (
                    "personal-capability-slots",
                    PersonalDataCategory::ApplicationData,
                ),
                ("attachments", PersonalDataCategory::Attachments),
                ("ai-runtime/attachments", PersonalDataCategory::Attachments),
                ("ai-runtime/runs", PersonalDataCategory::Attachments),
                (
                    "ai-runtime/artifact-versions",
                    PersonalDataCategory::Attachments,
                ),
                (
                    "ai-runtime/conversation-workspaces",
                    PersonalDataCategory::Attachments,
                ),
            ] {
                let path = profile.join(name);
                if path.exists() {
                    collect_tree(&path, &format!("profile/{name}"), category, &mut files)?;
                }
            }
            for home in &proof.native_homes {
                collect_native(
                    profile,
                    home,
                    &candidate,
                    &mut files,
                    &mut manifest.exclusions,
                )?;
            }
            for path in self
                .persistence
                .as_ref()
                .ok_or_else(|| error("persistence_unavailable"))?
                .personal_data_attachment_paths()
                .map_err(|e| db_error(&e))?
            {
                let relative = Path::new(&path)
                    .strip_prefix(profile)
                    .map_err(|_| error("personal_data_attachment_not_included"))?
                    .to_str()
                    .ok_or_else(|| error("personal_data_attachment_not_included"))?
                    .replace('\\', "/");
                if !files.contains_key(&format!("profile/{relative}")) {
                    return Err(error("personal_data_attachment_missing"));
                }
            }
            validate_profile_inventory(profile, &proof.native_homes)?;
            for name in [
                "ai-history-backup",
                "ai-runtime/history-migration-report.json",
                "schema-backups",
                "personal-data-operations",
                "personal-package-imports",
                "retired-material-cache",
                "run",
                "personal-storage-operations",
            ] {
                if profile.join(name).exists() {
                    manifest.exclusions.push(issue(
                        "local_recovery_or_transient_copy_excluded",
                        Some(PersonalDataCategory::Configuration),
                        Some(name.into()),
                    ));
                }
            }
            for name in [
                "ai-runtime/mcp-runtime",
                "ai-runtime/skill-snapshots",
                "ai-runtime/memory.lock",
                "model-credential-operation.lock",
            ] {
                if profile.join(name).exists() {
                    manifest.exclusions.push(issue(
                        "native_runtime_private_file_excluded",
                        Some(PersonalDataCategory::Configuration),
                        Some(name.into()),
                    ));
                }
            }
            let mut total = 0_u64;
            for (path, (category, bytes)) in &files {
                total = total
                    .checked_add(bytes.len() as u64)
                    .ok_or_else(|| error("personal_data_archive_too_large"))?;
                if total > MAX_TOTAL {
                    return Err(error("personal_data_archive_too_large"));
                }
                manifest.files.push(PersonalBackupFile {
                    archive_path: path.clone(),
                    category: category.clone(),
                    digest: digest(bytes),
                    bytes: bytes.len() as u64,
                });
            }
            if self
                .persistence
                .as_ref()
                .ok_or_else(|| error("persistence_unavailable"))?
                .personal_data_workspace_snapshots(&bases.keys().cloned().collect::<Vec<_>>())
                .map_err(|e| db_error(&e))?
                != bases
            {
                return Err(error("personal_data_workspace_index_changed"));
            }
            workspaces::verify_sources(&manifest)?;
            manifest.complete = true;
            let staged = candidate.join("backup.zip");
            write_archive(&staged, &manifest, &files)?;
            let bytes = read_limited(&staged, MAX_TOTAL)?;
            let (checked, _) = validate_archive(&bytes)?;
            if checked != manifest {
                return Err(error("personal_data_archive_invalid"));
            }
            let final_path = root.join("backup.zip");
            if final_path.exists() {
                return Err(error("personal_data_candidate_exists"));
            }
            fs::rename(staged, &final_path).map_err(|_| error("personal_data_write_failed"))?;
            filesystem::sync_directory(&root).map_err(|_| error("personal_data_write_failed"))?;
            Ok((manifest, digest(&bytes)))
        })();
        match capture {
            Ok((manifest, archive_digest)) => {
                op.state = PersonalDataOperationState::Completed;
                op.blockers.clear();
                op.backup_manifest = Some(manifest);
                op.result = Some(PersonalDataResult {
                    artifact_relative_path: Some(format!(
                        "personal-data-operations/{}/backup.zip",
                        op.operation_id
                    )),
                    artifact_digest: Some(archive_digest),
                    restored_profile_id: None,
                    retained_current_relative_path: None,
                    verified_categories: vec![
                        PersonalDataCategory::BusinessDatabase,
                        PersonalDataCategory::NativeHistory,
                        PersonalDataCategory::NativeMemory,
                        PersonalDataCategory::Skills,
                        PersonalDataCategory::Applications,
                        PersonalDataCategory::ApplicationData,
                        PersonalDataCategory::Attachments,
                        PersonalDataCategory::Configuration,
                    ],
                    pending_categories: vec![],
                    issues: vec![],
                    restored_workspaces: None,
                });
                if op
                    .selection
                    .as_ref()
                    .is_some_and(|selection| !selection.include_workspace_files.is_empty())
                {
                    op.result
                        .as_mut()
                        .ok_or_else(|| error("personal_data_record_invalid"))?
                        .verified_categories
                        .push(PersonalDataCategory::WorkspaceOriginals);
                }
            }
            Err(e) => {
                op.state = if operation_root(profile, &op.operation_id)?
                    .join("backup.zip")
                    .exists()
                {
                    PersonalDataOperationState::Capturing
                } else {
                    PersonalDataOperationState::Incomplete
                };
                op.blockers = vec![issue(e.reason, None, None)];
                op.result = Some(PersonalDataResult {
                    artifact_relative_path: None,
                    artifact_digest: None,
                    restored_profile_id: None,
                    retained_current_relative_path: None,
                    verified_categories: vec![],
                    pending_categories: vec![
                        PersonalDataCategory::BusinessDatabase,
                        PersonalDataCategory::NativeHistory,
                        PersonalDataCategory::NativeMemory,
                        PersonalDataCategory::Applications,
                        PersonalDataCategory::ApplicationData,
                        PersonalDataCategory::Skills,
                        PersonalDataCategory::Attachments,
                    ],
                    issues: op.blockers.clone(),
                    restored_workspaces: None,
                });
            }
        }
        op.updated_at = now;
        self.data_transition(&op, &PersonalDataOperationState::Capturing)
    }
    fn data_restore_prepare(
        &mut self,
        profile: &Path,
        id: &str,
        p: &PersonalRestorePrepareParams,
        now: i64,
    ) -> Result<PersonalDataOperation, ServiceError> {
        if p.client_operation_id.trim().is_empty()
            || p.client_operation_id.len() > 240
            || !Path::new(&p.archive_path).is_absolute()
        {
            return Err(error("personal_data_request_invalid"));
        }
        let hash = fingerprint(&p)?;
        if let Some(old) = self
            .persistence
            .as_ref()
            .ok_or_else(|| error("persistence_unavailable"))?
            .personal_data_by_client_operation(id, &p.client_operation_id, &hash)
            .map_err(|e| db_error(&e))?
        {
            return Ok(old);
        }
        let bytes = read_limited(Path::new(&p.archive_path), MAX_TOTAL)?;
        let (manifest, files) = validate_archive(&bytes)?;
        let op_id = format!("data_{}", uuid::Uuid::new_v4().simple());
        let root = operation_root(profile, &op_id)?;
        write_new(&root.join("restore-source.zip"), &bytes)?;
        let validation = root.join("validation.sqlite");
        write_new(
            &validation,
            &files
                .get("profile/contexthub.db")
                .ok_or_else(|| error("personal_data_database_missing"))?
                .1,
        )?;
        self.persistence
            .as_ref()
            .ok_or_else(|| error("persistence_unavailable"))?
            .verify_personal_data_snapshot_schema(&validation)
            .map_err(|e| db_error(&e))?;
        self.persistence
            .as_ref()
            .ok_or_else(|| error("persistence_unavailable"))?
            .verify_personal_data_workspace_snapshot(
                &validation,
                &workspaces::snapshots(&manifest)?,
            )
            .map_err(|e| db_error(&e))?;
        let preview = PersonalRestorePreview {
            source_profile: manifest.profile.clone(),
            current_profile_id: id.into(),
            target_profile_id: uuid::Uuid::new_v4().simple().to_string(),
            archive_digest: digest(&bytes),
            manifest_digest: fingerprint(&manifest)?,
            replaces_current_profile_directory: false,
            preserves_current_copy: true,
            file_count: u32::try_from(files.len())
                .map_err(|_| error("personal_data_archive_too_large"))?,
            total_bytes: files.values().map(|(_, b)| b.len() as u64).sum(),
            workspaces: manifest.workspaces.clone(),
            warnings: vec![
                issue(
                    "restored_connections_require_authorization",
                    Some(PersonalDataCategory::Configuration),
                    None,
                ),
                issue(
                    "restored_automations_paused",
                    Some(PersonalDataCategory::Configuration),
                    None,
                ),
                issue(
                    "restored_notifications_disabled",
                    Some(PersonalDataCategory::Configuration),
                    None,
                ),
                issue(
                    "restored_tools_require_confirmation",
                    Some(PersonalDataCategory::Applications),
                    None,
                ),
            ],
            workspace_destination: None,
        };
        let op = PersonalDataOperation {
            operation_id: op_id,
            profile_id: id.into(),
            kind: PersonalDataOperationKind::Restore,
            state: PersonalDataOperationState::Prepared,
            preview_digest: fingerprint(&preview)?,
            created_at: now,
            updated_at: now,
            selection: None,
            backup_manifest: Some(manifest),
            restore_preview: Some(preview),
            blockers: vec![],
            result: None,
        };
        self.persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .prepare_personal_data(&op, &p.client_operation_id, &hash)
            .map_err(|e| db_error(&e))
    }
    fn data_restore_workspace_plan(
        &mut self,
        profile: &Path,
        id: &str,
        params: &PersonalRestoreWorkspacePlanParams,
        now: i64,
    ) -> Result<PersonalDataOperation, ServiceError> {
        let mut op = self.data_read(id, &params.operation_id)?;
        if op.kind != PersonalDataOperationKind::Restore
            || op.state != PersonalDataOperationState::Prepared
            || params.client_operation_id.trim().is_empty()
            || params.client_operation_id.len() > 240
        {
            return Err(error("personal_data_operation_unconfirmed"));
        }
        let preview = op
            .restore_preview
            .as_mut()
            .ok_or_else(|| error("personal_data_record_invalid"))?;
        if !preview
            .workspaces
            .iter()
            .any(|workspace| workspace.original_files_included)
        {
            return Err(error("personal_data_workspace_destination_not_required"));
        }
        let destination = workspaces::destination(
            Path::new(&params.destination_parent),
            profile,
            &preview.target_profile_id,
        )?;
        if let Some(existing) = &preview.workspace_destination {
            if *existing == destination {
                return Ok(op);
            }
            return Err(error("personal_data_preview_changed"));
        }
        if op.preview_digest != params.preview_digest {
            return Err(error("personal_data_preview_changed"));
        }
        preview.workspace_destination = Some(destination);
        op.preview_digest = fingerprint(preview)?;
        op.updated_at = now;
        self.persistence
            .as_mut()
            .ok_or_else(|| error("persistence_unavailable"))?
            .plan_personal_workspace_restore(&op, &params.preview_digest)
            .map_err(|e| db_error(&e))
    }
    fn data_restore_apply_prepare(
        &mut self,
        profile: &Path,
        id: &str,
        p: &PersonalRestoreApplyPrepareParams,
        instance: &str,
        now: i64,
    ) -> Result<PersonalDataOperation, ServiceError> {
        let mut op = self.data_read(id, &p.operation_id)?;
        if op.kind != PersonalDataOperationKind::Restore
            || op.preview_digest != p.preview_digest
            || p.expected_current_profile_id != id
        {
            return Err(error("personal_data_preview_changed"));
        }
        if !matches!(
            op.state,
            PersonalDataOperationState::Prepared
                | PersonalDataOperationState::Incomplete
                | PersonalDataOperationState::ReadyToApply
        ) {
            return Err(error("personal_data_operation_unconfirmed"));
        }
        let prior = op.state.clone();
        let verified = match p.quiescence.as_ref() {
            Some(proof) => self
                .persistence
                .as_mut()
                .ok_or_else(|| error("persistence_unavailable"))?
                .verify_personal_data_quiescence(id, &op.operation_id, instance, proof)
                .map_err(|e| db_error(&e)),
            None => Err(error("personal_data_quiescence_unconfirmed")),
        };
        if let Err(reason) = verified {
            let old = op.state.clone();
            op.state = PersonalDataOperationState::Incomplete;
            op.updated_at = now;
            op.blockers = vec![issue(reason.reason, None, None)];
            return self.data_transition(&op, &old);
        }
        let proof = p
            .quiescence
            .as_ref()
            .ok_or_else(|| error("personal_data_quiescence_unconfirmed"))?;
        let root = operation_root(profile, &op.operation_id)?;
        let bytes = read_limited(&root.join("restore-source.zip"), MAX_TOTAL)?;
        let (m, _) = validate_archive(&bytes)?;
        let preview = op
            .restore_preview
            .as_ref()
            .ok_or_else(|| error("personal_data_record_invalid"))?;
        if digest(&bytes) != preview.archive_digest || fingerprint(&m)? != preview.manifest_digest {
            return Err(error("personal_data_artifact_changed"));
        }
        workspaces::verify_destination(preview, profile)?;
        let proof_path = root.join("restore-quiescence.json");
        let proof_bytes = encoded(proof)?;
        if proof_path.exists() {
            let _ = read_limited(&proof_path, 256 * 1024)?;
            let temp = root.join(format!(
                "restore-quiescence-{}.tmp",
                uuid::Uuid::new_v4().simple()
            ));
            write_new(&temp, &proof_bytes)?;
            fs::rename(temp, &proof_path).map_err(|_| error("personal_data_write_failed"))?;
            filesystem::sync_directory(&root).map_err(|_| error("personal_data_write_failed"))?;
        } else {
            write_new(&proof_path, &proof_bytes)?;
        }

        op.state = PersonalDataOperationState::ReadyToApply;
        op.blockers.clear();
        op.updated_at = now;
        self.data_transition(&op, &prior)
    }
}

type ArchiveFiles = BTreeMap<String, (PersonalDataCategory, Vec<u8>)>;
fn collect_tree(
    source: &Path,
    prefix: &str,
    category: PersonalDataCategory,
    files: &mut ArchiveFiles,
) -> Result<(), ServiceError> {
    let meta = inspect(source)?;
    if meta.is_file() {
        if files.len() >= MAX_FILES {
            return Err(error("personal_data_archive_too_large"));
        }
        if !safe_relative(prefix) {
            return Err(error("personal_data_path_invalid"));
        }
        files.insert(prefix.into(), (category, read_limited(source, MAX_FILE)?));
        return Ok(());
    }
    if !meta.is_dir() {
        return Err(error("personal_data_file_invalid"));
    }
    for entry in fs::read_dir(source).map_err(|_| error("personal_data_read_failed"))? {
        let entry = entry.map_err(|_| error("personal_data_read_failed"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| error("personal_data_path_invalid"))?;
        collect_tree(
            &entry.path(),
            &format!("{prefix}/{name}"),
            category.clone(),
            files,
        )?;
    }
    Ok(())
}
fn native_path(path: &str) -> bool {
    safe_relative(path)
        && path.starts_with("ai-runtime/")
        && (path.starts_with("ai-runtime/model-connections/")
            || matches!(
                path,
                "ai-runtime/cilai-harness/official-runtime"
                    | "ai-runtime/cilai-application-harness/official-runtime"
            ))
}
pub(super) fn discover_native_homes(profile: &Path) -> Result<Vec<String>, ServiceError> {
    let mut homes = vec![];
    for name in ["cilai-harness", "cilai-application-harness"] {
        let rel = format!("ai-runtime/{name}/official-runtime");
        if profile.join(&rel).exists() {
            if !inspect(&profile.join(&rel))?.is_dir() {
                return Err(error("personal_data_native_homes_unconfirmed"));
            }
            homes.push(rel);
        }
    }
    let scopes = profile.join("ai-runtime/model-connections");
    if scopes.exists() {
        for entry in fs::read_dir(&scopes).map_err(|_| error("personal_data_read_failed"))? {
            let entry = entry.map_err(|_| error("personal_data_read_failed"))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| error("personal_data_path_invalid"))?;
            if name.len() != 64
                || !name.bytes().all(|b| b.is_ascii_hexdigit())
                || !inspect(&entry.path())?.is_dir()
            {
                return Err(error("personal_data_native_homes_unconfirmed"));
            }
            for kind in ["workbench", "application"] {
                let home = entry.path().join(kind).join("official-runtime");
                if home.exists() {
                    if !inspect(&home)?.is_dir() {
                        return Err(error("personal_data_native_homes_unconfirmed"));
                    }
                    homes.push(format!(
                        "ai-runtime/model-connections/{name}/{kind}/official-runtime"
                    ));
                }
            }
        }
    }
    homes.sort();
    Ok(homes)
}
fn collect_native(
    profile: &Path,
    home: &PersonalBackupNativeHome,
    staging: &Path,
    files: &mut ArchiveFiles,
    exclusions: &mut Vec<PersonalDataIssue>,
) -> Result<(), ServiceError> {
    if !native_path(&home.relative_path) {
        return Err(error("personal_data_native_homes_unconfirmed"));
    }
    let root = profile.join(&home.relative_path);
    inspect(&root)?;
    for entry in fs::read_dir(&root).map_err(|_| error("personal_data_read_failed"))? {
        let entry = entry.map_err(|_| error("personal_data_read_failed"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| error("personal_data_path_invalid"))?;
        let category = match name.as_str() {
            "sessions"
            | "archived_sessions"
            | "history.jsonl"
            | "session_index.jsonl"
            | "state_5.sqlite"
            | "goals_1.sqlite"
            | "queue_1.sqlite" => Some(PersonalDataCategory::NativeHistory),
            "memories" | "memories_v2" | "memories_extensions" | "memory" | "memories_1.sqlite" => {
                Some(PersonalDataCategory::NativeMemory)
            }
            _ => None,
        };
        if let Some(category) = category {
            if name.ends_with(".sqlite") {
                let snapshot_root = staging
                    .join("native-snapshots")
                    .join(digest(home.relative_path.as_bytes()).trim_start_matches("sha256:"))
                    .join(&name);
                secure_dir(&snapshot_root)?;
                let copy = snapshot_root.join("source.sqlite");
                write_new(&copy, &read_limited(&entry.path(), MAX_FILE)?)?;
                for suffix in ["-wal", "-shm"] {
                    let source = root.join(format!("{name}{suffix}"));
                    if source.exists() {
                        write_new(
                            &snapshot_root.join(format!("source.sqlite{suffix}")),
                            &read_limited(&source, MAX_FILE)?,
                        )?;
                    }
                }
                let snapshot = snapshot_root.join("snapshot.sqlite");
                contexthub_persistence_sqlite::materialize_native_database(&copy, &snapshot)
                    .map_err(|e| db_error(&e))?;
                files.insert(
                    format!("profile/{}/{}", home.relative_path, name.clone()),
                    (category, read_limited(&snapshot, MAX_FILE)?),
                );
            } else {
                collect_tree(
                    &entry.path(),
                    &format!("profile/{}/{}", home.relative_path, name),
                    category,
                    files,
                )?;
            }
        } else {
            exclusions.push(issue(
                "native_runtime_private_file_excluded",
                Some(PersonalDataCategory::Configuration),
                Some(format!("{}/{name}", home.relative_path)),
            ));
        }
    }
    Ok(())
}
fn validate_profile_inventory(
    profile: &Path,
    _homes: &[PersonalBackupNativeHome],
) -> Result<(), ServiceError> {
    for entry in fs::read_dir(profile).map_err(|_| error("personal_data_read_failed"))? {
        let entry = entry.map_err(|_| error("personal_data_read_failed"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| error("personal_data_path_invalid"))?;
        inspect(&entry.path())?;
        if !matches!(
            name.as_str(),
            "contexthub.db"
                | "contexthub.db-wal"
                | "contexthub.db-shm"
                | "run"
                | "ai-runtime"
                | "skills"
                | "capability-projects"
                | "personal-capabilities"
                | "personal-capability-data"
                | "personal-capability-preview-data"
                | "personal-capability-slots"
                | "attachments"
                | "personal-data-operations"
                | "personal-package-imports"
                | "schema-backups"
                | "ai-history-backup"
                | "personal-storage-operations"
                | "retired-material-cache"
                | "model-credential-operation.lock"
                | "credentials"
        ) {
            return Err(error("personal_data_profile_files_unclassified"));
        }
    }
    let runtime = profile.join("ai-runtime");
    if runtime.exists() {
        for entry in fs::read_dir(runtime).map_err(|_| error("personal_data_read_failed"))? {
            let entry = entry.map_err(|_| error("personal_data_read_failed"))?;
            inspect(&entry.path())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| error("personal_data_path_invalid"))?;
            if !matches!(
                name.as_str(),
                "attachments"
                    | "runs"
                    | "artifact-versions"
                    | "conversation-workspaces"
                    | "cilai-harness"
                    | "cilai-application-harness"
                    | "model-connections"
                    | "skill-snapshots"
                    | "mcp-runtime"
                    | "memory.lock"
                    | "history-migration-report.json"
            ) {
                return Err(error("personal_data_profile_files_unclassified"));
            }
        }
    }
    Ok(())
}
fn write_archive(
    path: &Path,
    manifest: &PersonalBackupManifest,
    files: &ArchiveFiles,
) -> Result<(), ServiceError> {
    let file = security::create_new_restricted_file(path)
        .map_err(|_| error("personal_data_write_failed"))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    zip.start_file(MANIFEST, options)
        .map_err(|_| error("personal_data_archive_failed"))?;
    zip.write_all(&encoded(manifest)?)
        .map_err(|_| error("personal_data_write_failed"))?;
    for (name, (_, bytes)) in files {
        zip.start_file(name, options)
            .map_err(|_| error("personal_data_archive_failed"))?;
        zip.write_all(bytes)
            .map_err(|_| error("personal_data_write_failed"))?;
    }
    let file = zip
        .finish()
        .map_err(|_| error("personal_data_archive_failed"))?;
    file.sync_all()
        .map_err(|_| error("personal_data_write_failed"))
}
fn validate_archive(bytes: &[u8]) -> Result<(PersonalBackupManifest, ArchiveFiles), ServiceError> {
    let mut zip = ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| error("personal_data_archive_invalid"))?;
    if zip.len() > MAX_FILES + 1 {
        return Err(error("personal_data_archive_too_large"));
    }
    let mut raw = BTreeMap::new();
    let mut total = 0_u64;
    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|_| error("personal_data_archive_invalid"))?;
        let name = file
            .name()
            .map_err(|_| error("personal_data_archive_path_invalid"))?
            .into_owned();
        let mode = file.unix_mode().unwrap_or(0o100_600) & 0o170_000;
        if !safe_relative(&name)
            || file.is_dir()
            || !matches!(mode, 0 | 0o100_000)
            || file.size() > MAX_FILE
            || raw.contains_key(&name)
        {
            return Err(error("personal_data_archive_path_invalid"));
        }
        total = total
            .checked_add(file.size())
            .ok_or_else(|| error("personal_data_archive_too_large"))?;
        if total > MAX_TOTAL {
            return Err(error("personal_data_archive_too_large"));
        }
        let mut content = vec![];
        Read::by_ref(&mut file)
            .take(MAX_FILE + 1)
            .read_to_end(&mut content)
            .map_err(|_| error("personal_data_archive_invalid"))?;
        if content.len() as u64 != file.size() {
            return Err(error("personal_data_archive_invalid"));
        }
        raw.insert(name, content);
    }
    let manifest: PersonalBackupManifest = serde_json::from_slice(
        &raw.remove(MANIFEST)
            .ok_or_else(|| error("personal_data_manifest_missing"))?,
    )
    .map_err(|_| error("personal_data_manifest_invalid"))?;
    if manifest.schema_migration_version
        != contexthub_persistence_sqlite::personal_data_schema_version()
    {
        return Err(error("personal_data_backup_schema_unsupported"));
    }
    if manifest.format != PERSONAL_BACKUP_FORMAT
        || !matches!(manifest.format_version, 1 | 2)
        || !manifest.complete
        || manifest.profile.schema_version != PERSONAL_DATA_VERSION
        || !valid_id(&manifest.profile.profile_id)
        || manifest.files.len() != raw.len()
    {
        return Err(error("personal_data_manifest_invalid"));
    }
    let mut files = BTreeMap::new();
    for f in &manifest.files {
        if !safe_relative(&f.archive_path)
            || !(f.archive_path.starts_with("profile/")
                || f.archive_path.starts_with("workspace-originals/"))
            || files.contains_key(&f.archive_path)
        {
            return Err(error("personal_data_archive_path_invalid"));
        }
        let bytes = raw
            .remove(&f.archive_path)
            .ok_or_else(|| error("personal_data_archive_file_missing"))?;
        if bytes.len() as u64 != f.bytes || digest(&bytes) != f.digest {
            return Err(error("personal_data_archive_digest_mismatch"));
        }
        if f.category != PersonalDataCategory::WorkspaceOriginals
            && !allowed_archive_path(&f.archive_path, &f.category, &manifest.native_homes)
        {
            return Err(error("personal_data_archive_path_invalid"));
        }
        files.insert(f.archive_path.clone(), (f.category.clone(), bytes));
    }
    if !files.contains_key("profile/contexthub.db") {
        return Err(error("personal_data_database_missing"));
    }
    workspaces::validate_files(&manifest, &files)?;
    Ok((manifest, files))
}
fn allowed_archive_path(
    path: &str,
    category: &PersonalDataCategory,
    homes: &[PersonalBackupNativeHome],
) -> bool {
    match category {
        PersonalDataCategory::BusinessDatabase => path == "profile/contexthub.db",
        PersonalDataCategory::Skills => path.starts_with("profile/skills/"),
        PersonalDataCategory::Applications => {
            path.starts_with("profile/capability-projects/")
                || path.starts_with("profile/personal-capabilities/")
        }
        PersonalDataCategory::ApplicationData => {
            path.starts_with("profile/personal-capability-data/")
                || path.starts_with("profile/personal-capability-preview-data/")
                || path.starts_with("profile/personal-capability-slots/")
        }
        PersonalDataCategory::Attachments => {
            path.starts_with("profile/attachments/")
                || path.starts_with("profile/ai-runtime/attachments/")
                || path.starts_with("profile/ai-runtime/runs/")
                || path.starts_with("profile/ai-runtime/artifact-versions/")
                || path.starts_with("profile/ai-runtime/conversation-workspaces/")
        }
        PersonalDataCategory::NativeHistory | PersonalDataCategory::NativeMemory => {
            homes.iter().any(|h| {
                if !native_path(&h.relative_path) {
                    return false;
                }
                let prefix = format!("profile/{}/", h.relative_path);
                let Some(relative) = path.strip_prefix(&prefix) else {
                    return false;
                };
                let first = relative.split('/').next().unwrap_or("");
                match category {
                    PersonalDataCategory::NativeHistory => matches!(
                        first,
                        "sessions"
                            | "archived_sessions"
                            | "history.jsonl"
                            | "session_index.jsonl"
                            | "state_5.sqlite"
                            | "goals_1.sqlite"
                            | "queue_1.sqlite"
                    ),
                    PersonalDataCategory::NativeMemory => {
                        matches!(
                            first,
                            "memories"
                                | "memories_v2"
                                | "memories_extensions"
                                | "memory"
                                | "memories_1.sqlite"
                        )
                    }
                    _ => false,
                }
            })
        }
        PersonalDataCategory::Configuration | PersonalDataCategory::WorkspaceOriginals => false,
    }
}

/// 仅供 Host 关闭其拥有的本地服务后调用。原资料目录原位保留，最后切换资料清单。
/// # Errors
/// 任一锁、候选、静止、摘要或读回校验失败时保留当前清单和原资料。
pub fn apply_restore_offline(
    business: &Path,
    operation_id: &str,
    preview_digest: &str,
) -> Result<Value, String> {
    offline_restore(business, operation_id, preview_digest)
        .map_err(|e| e.reason.into())
        .and_then(|op| serde_json::to_value(op).map_err(|_| "personal_data_record_invalid".into()))
}
#[expect(
    clippy::too_many_lines,
    reason = "独占锁、候选核验、资料切换和持久回执按顺序执行"
)]
fn offline_restore(
    business: &Path,
    operation_id: &str,
    preview_digest: &str,
) -> Result<PersonalDataOperation, ServiceError> {
    if !business.is_absolute()
        || business.file_name().and_then(|n| n.to_str()) != Some("business")
        || business
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            != Some("cn.cilai.personal.workbench")
    {
        return Err(error("personal_data_path_invalid"));
    }
    for ancestor in business.ancestors() {
        if !inspect(ancestor)?.is_dir() {
            return Err(error("personal_data_path_invalid"));
        }
    }
    let _business_lock =
        security::try_lock_restricted_file(&business.join("personal-data-restore.lock"))
            .map_err(|_| error("personal_data_restore_lock_failed"))?
            .ok_or_else(|| error("personal_data_restore_busy"))?;
    let manifest_path = business.join("local-profile.json");
    let current_bytes = read_limited(&manifest_path, 4096)?;
    let current: PersonalProfileManifest = serde_json::from_slice(&current_bytes)
        .map_err(|_| error("personal_data_profile_not_ready"))?;
    if !valid_id(&current.profile_id) || current.schema_version != PERSONAL_DATA_VERSION {
        return Err(error("personal_data_profile_not_ready"));
    }
    let profile = business.join("profiles").join(&current.profile_id);
    if !inspect(&profile)?.is_dir() {
        return Err(error("personal_data_profile_not_ready"));
    }
    let _instance_lock = security::try_lock_restricted_file(&profile.join("run/instance.lock"))
        .map_err(|_| error("personal_data_restore_lock_failed"))?
        .ok_or_else(|| error("personal_data_restore_requires_offline"))?;
    contexthub_persistence_sqlite::validate_no_active_writer(&profile.join("contexthub.db"))
        .map_err(|_| error("personal_data_restore_requires_offline"))?;
    let mut db = contexthub_persistence_sqlite::SqlitePersistence::open(
        &profile.join("contexthub.db"),
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| db_error(&e))?;
    let mut op = db
        .personal_data_operation(&current.profile_id, operation_id)
        .map_err(|e| db_error(&e))?
        .ok_or_else(|| error("personal_data_operation_not_found"))?;
    if op.preview_digest == preview_digest
        && op.kind == PersonalDataOperationKind::Restore
        && op.state == PersonalDataOperationState::Completed
        && op
            .result
            .as_ref()
            .and_then(|r| r.restored_profile_id.as_deref())
            == Some(current.profile_id.as_str())
    {
        return Ok(op);
    }
    if op.preview_digest != preview_digest
        || op.kind != PersonalDataOperationKind::Restore
        || op.state != PersonalDataOperationState::ReadyToApply
    {
        return Err(error("personal_data_operation_unconfirmed"));
    }
    let root = operation_root(&profile, operation_id)?;
    let proof: PersonalDataQuiescence = serde_json::from_slice(&read_limited(
        &root.join("restore-quiescence.json"),
        256 * 1024,
    )?)
    .map_err(|_| error("personal_data_quiescence_unconfirmed"))?;
    let fence = read_maintenance_fence(&mut db, &current.profile_id, &proof)?;
    db.verify_personal_data_quiescence(
        &current.profile_id,
        operation_id,
        &fence.instance_fence,
        &proof,
    )
    .map_err(|e| db_error(&e))?;
    let source = read_limited(&root.join("restore-source.zip"), MAX_TOTAL)?;
    let (manifest, files) = validate_archive(&source)?;
    let preview = op
        .restore_preview
        .clone()
        .ok_or_else(|| error("personal_data_record_invalid"))?;
    if preview.current_profile_id != current.profile_id
        || preview.archive_digest != digest(&source)
        || preview.manifest_digest != fingerprint(&manifest)?
        || !valid_id(&preview.target_profile_id)
        || preview.target_profile_id == current.profile_id
    {
        return Err(error("personal_data_preview_changed"));
    }
    let target = business.join("profiles").join(&preview.target_profile_id);
    if target.exists() {
        return Err(error("personal_data_restore_target_exists"));
    }
    let candidate = business.join("profiles").join(format!(
        "restore-candidate-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let workspace_target = workspaces::verify_destination(&preview, &profile)?;
    write_new(
        &root.join("restore-materialization.json"),
        &encoded(&json!({
            "operationId":op.operation_id,"previewDigest":op.preview_digest,
            "retainedProfilePath":profile,"candidateProfilePath":candidate,
            "targetProfilePath":target,"workspaceTargetPath":workspace_target,
        }))?,
    )?;
    let mut applying = op.clone();
    applying.state = PersonalDataOperationState::Applying;
    db.transition_personal_data(&applying, &PersonalDataOperationState::ReadyToApply)
        .map_err(|e| db_error(&e))?;
    secure_dir(&candidate)?;
    for (path, (_, bytes)) in &files {
        if let Some(relative) = path.strip_prefix("profile/") {
            write_new(&candidate.join(relative), bytes)?;
        }
    }
    let restored_originals = workspaces::restore_originals(&manifest, &files, &preview, &profile)?;
    let restored_workspaces = &restored_originals.workspaces;
    let mut candidate_db = contexthub_persistence_sqlite::SqlitePersistence::open(
        &candidate.join("contexthub.db"),
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| db_error(&e))?;
    candidate_db
        .prepare_restored_personal_profile(&manifest.profile.profile_id, &preview.target_profile_id)
        .map_err(|e| db_error(&e))?;
    workspaces::rebind_restored(
        &mut candidate_db,
        &manifest,
        restored_workspaces,
        op.updated_at,
    )?;
    let restored_bases = candidate_db
        .personal_data_workspace_snapshots(
            &restored_workspaces
                .iter()
                .map(|workspace| workspace.workspace_id.clone())
                .collect::<Vec<_>>(),
        )
        .map_err(|e| db_error(&e))?;
    let pending = if manifest.native_homes.is_empty() {
        vec![]
    } else {
        vec![
            PersonalDataCategory::NativeHistory,
            PersonalDataCategory::NativeMemory,
        ]
    };
    let mut issues = preview.warnings.clone();
    if !pending.is_empty() {
        issues.push(issue(
            "native_history_paths_require_relocation",
            Some(PersonalDataCategory::NativeHistory),
            None,
        ));
    }
    op.state = PersonalDataOperationState::Completed;
    op.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| error("personal_data_clock_invalid"))?
        .as_millis()
        .try_into()
        .map_err(|_| error("personal_data_clock_invalid"))?;
    op.result = Some(PersonalDataResult {
        artifact_relative_path: None,
        artifact_digest: Some(preview.archive_digest.clone()),
        restored_profile_id: Some(preview.target_profile_id.clone()),
        retained_current_relative_path: Some(format!("profiles/{}", current.profile_id)),
        verified_categories: vec![
            PersonalDataCategory::BusinessDatabase,
            PersonalDataCategory::Skills,
            PersonalDataCategory::Applications,
            PersonalDataCategory::ApplicationData,
            PersonalDataCategory::Attachments,
            PersonalDataCategory::Configuration,
        ],
        pending_categories: pending,
        issues,
        restored_workspaces: (!restored_workspaces.is_empty()).then(|| restored_workspaces.clone()),
    });
    if op
        .result
        .as_ref()
        .is_some_and(|result| result.restored_workspaces.is_some())
    {
        op.result
            .as_mut()
            .ok_or_else(|| error("personal_data_record_invalid"))?
            .verified_categories
            .push(PersonalDataCategory::WorkspaceOriginals);
    }
    let mut target_receipt = op.clone();
    target_receipt
        .profile_id
        .clone_from(&preview.target_profile_id);
    candidate_db
        .record_personal_data_restore_receipt(&target_receipt)
        .map_err(|e| db_error(&e))?;
    drop(candidate_db);
    verify_materialized_files(&candidate, &files)?;
    workspaces::verify_restored(&preview, &restored_originals, &restored_bases)?;
    write_new(&root.join("retained-current-manifest.json"), &current_bytes)?;
    fs::rename(&candidate, &target).map_err(|_| error("personal_data_write_failed"))?;
    filesystem::sync_directory(
        target
            .parent()
            .ok_or_else(|| error("personal_data_path_invalid"))?,
    )
    .map_err(|_| error("personal_data_write_failed"))?;
    let new_manifest = PersonalProfileManifest {
        schema_version: PERSONAL_DATA_VERSION,
        profile_id: preview.target_profile_id.clone(),
        display_name: manifest.profile.display_name.clone(),
    };
    let new_bytes = encoded(&new_manifest)?;
    let temp = business.join(format!(
        "local-profile.json.restore-{}",
        uuid::Uuid::new_v4().simple()
    ));
    write_new(&temp, &new_bytes)?;
    if read_limited(&manifest_path, 4096)? != current_bytes {
        return Err(error("personal_data_profile_changed"));
    }
    // 新目标及预切换回执均已落盘后才替换唯一活动资料清单。
    fs::rename(temp, &manifest_path).map_err(|_| error("personal_data_manifest_switch_failed"))?;
    filesystem::sync_directory(business)
        .map_err(|_| error("personal_data_manifest_switch_unconfirmed"))?;
    if read_limited(&manifest_path, 4096)? != new_bytes {
        return Err(error("personal_data_manifest_switch_unconfirmed"));
    }
    db.transition_personal_data(&op, &PersonalDataOperationState::Applying)
        .map_err(|e| db_error(&e))?;
    db.personal_writer_rpc(
        "personalMaintenance/release",
        json!({"fenceId":proof.token,"owner":proof.owner}),
        &current.profile_id,
        &fence.instance_fence,
        op.updated_at,
    )
    .map_err(|e| db_error(&e))?;
    Ok(op)
}
fn read_maintenance_fence(
    db: &mut contexthub_persistence_sqlite::SqlitePersistence,
    profile: &str,
    proof: &PersonalDataQuiescence,
) -> Result<PersonalMaintenanceFence, ServiceError> {
    db.personal_data_maintenance_fence(profile, &proof.token)
        .map_err(|e| db_error(&e))
}
fn verify_materialized_files(candidate: &Path, files: &ArchiveFiles) -> Result<(), ServiceError> {
    for (path, (_, expected)) in files {
        let Some(rel) = path.strip_prefix("profile/") else {
            continue;
        };
        if rel == "contexthub.db" {
            continue;
        }
        if read_limited(&candidate.join(rel), MAX_FILE)? != *expected {
            return Err(error("personal_data_restore_readback_failed"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_protocol::{ModelExecutionOwner, PersonalBackupSelection};
    include!("personal_data_workspace_tests.rs");
    struct Fixture {
        root: PathBuf,
        profile: PathBuf,
        id: String,
        services: AppServerServices,
        owner: ModelExecutionOwner,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .canonicalize()
                .unwrap()
                .join(format!(
                    "cilai-personal-data-{}",
                    uuid::Uuid::new_v4().simple()
                ))
                .join("cn.cilai.personal.workbench/business");
            let id = uuid::Uuid::new_v4().simple().to_string();
            let profile = root.join("profiles").join(&id);
            secure_dir(&profile).unwrap();
            secure_dir(&profile.join("run")).unwrap();
            write_new(
                &root.join("local-profile.json"),
                &encoded(&PersonalProfileManifest {
                    schema_version: 2,
                    profile_id: id.clone(),
                    display_name: "测试资料".into(),
                })
                .unwrap(),
            )
            .unwrap();
            let mut db = contexthub_persistence_sqlite::SqlitePersistence::open(
                &profile.join("contexthub.db"),
                "test",
            )
            .unwrap();
            db.begin_personal_profile_upgrade(&id, "test-instance")
                .unwrap();
            db.finish_personal_profile_upgrade(&id, "test-instance", "test-digest")
                .unwrap();
            let owner = ModelExecutionOwner {
                instance_id: "test-host".into(),
                pid: std::process::id(),
                start_fingerprint:
                    contexthub_platform_adapter::instance::process_start_fingerprint(
                        std::process::id(),
                    )
                    .unwrap()
                    .expect("current test process fingerprint"),
            };
            Self {
                root,
                profile: profile.clone(),
                id,
                services: AppServerServices::with_data_dir(Some(db), Some(profile)),
                owner,
            }
        }
        fn prepare(&mut self) -> PersonalDataOperation {
            self.services
                .data_backup_prepare(
                    &self.profile,
                    &self.id,
                    PersonalBackupPrepareParams {
                        selection: PersonalBackupSelection::default(),
                        client_operation_id: uuid::Uuid::new_v4().to_string(),
                    },
                    1,
                )
                .unwrap()
        }
        fn proof(&mut self, op: &PersonalDataOperation) -> PersonalDataQuiescence {
            let fence=self.services.persistence.as_mut().unwrap().personal_writer_rpc("personalMaintenance/acquire",json!({"operationId":op.operation_id,"previewDigest":op.preview_digest,"owner":self.owner}),&self.id,"test-instance",2).unwrap();
            PersonalDataQuiescence {
                protocol_version: 1,
                profile_id: self.id.clone(),
                owner: self.owner.clone(),
                token: fence["fenceId"].as_str().unwrap().into(),
                writer_registry_digest: digest(b"[]"),
                covered_writers: [
                    "foreground",
                    "automation",
                    "review",
                    "memory",
                    "page_ai",
                    "authentication",
                    "personal_package_mcp",
                    "user_mcp",
                    "attachments",
                    "local_operations",
                    "ledger",
                    "native_history",
                    "runtime_initialization",
                ]
                .map(String::from)
                .to_vec(),
                native_homes: vec![],
            }
        }
        fn capture(
            &mut self,
            op: &PersonalDataOperation,
            proof: Option<PersonalDataQuiescence>,
        ) -> PersonalDataOperation {
            self.services
                .data_backup_capture(
                    &self.profile,
                    &self.id,
                    &PersonalBackupCaptureParams {
                        operation_id: op.operation_id.clone(),
                        preview_digest: op.preview_digest.clone(),
                        client_operation_id: "capture".into(),
                        quiescence: proof,
                    },
                    "test-instance",
                    3,
                )
                .unwrap()
        }
        fn release(&mut self, proof: &PersonalDataQuiescence) {
            self.services
                .persistence
                .as_mut()
                .unwrap()
                .personal_writer_rpc(
                    "personalMaintenance/release",
                    json!({"fenceId":proof.token,"owner":proof.owner}),
                    &self.id,
                    "test-instance",
                    4,
                )
                .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.services.persistence.take();
            fs::remove_dir_all(self.root.parent().unwrap().parent().unwrap()).unwrap();
        }
    }
    #[test]
    fn backup_without_guard_is_incomplete_and_never_returns_an_artifact() {
        let mut f = Fixture::new();
        let op = f.prepare();
        let failed = f.capture(&op, None);
        assert_eq!(failed.state, PersonalDataOperationState::Incomplete);
        assert_eq!(
            failed.blockers[0].code,
            "personal_data_quiescence_unconfirmed"
        );
        assert!(failed.result.unwrap().artifact_digest.is_none());
        assert!(
            !operation_root(&f.profile, &op.operation_id)
                .unwrap()
                .join("backup.zip")
                .exists()
        );
    }
    #[test]
    fn empty_registry_does_not_replace_complete_writer_coverage() {
        let mut f = Fixture::new();
        let op = f.prepare();
        let mut proof = f.proof(&op);
        proof.covered_writers.clear();
        let failed = f.capture(&op, Some(proof));
        assert_eq!(failed.state, PersonalDataOperationState::Incomplete);
        assert_eq!(
            failed.blockers[0].code,
            "personal_data_writer_coverage_unconfirmed"
        );
    }
    #[test]
    fn active_local_transport_blocks_backup_even_when_model_leases_are_empty() {
        let mut f = Fixture::new();
        f.services.persistence.as_mut().unwrap().personal_writer_rpc("profileWriter/prepare",json!({"clientOperationId":"active-local","kind":"artifact","resourceId":"artifact","resourceRevision":"1","owner":f.owner,"launchProtocol":1,"processMode":"remote_request"}),&f.id,"test-instance",1).unwrap();
        let op = f.prepare();
        let proof = f.proof(&op);
        let failed = f.capture(&op, Some(proof));
        assert_eq!(failed.state, PersonalDataOperationState::Incomplete);
        assert_eq!(failed.blockers[0].code, "personal_data_writer_active");
    }
    #[test]
    fn backup_contains_committed_wal_and_excludes_credentials_browser_and_native_environment() {
        let mut f = Fixture::new();
        let db = f
            .services
            .persistence
            .as_ref()
            .unwrap()
            .database_path()
            .to_owned();
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute("INSERT INTO app_metadata(key,value,updated_at) VALUES('backup-marker','wal-visible',1)",[]).unwrap();
        conn.execute("INSERT INTO employee_service_credentials(tenant_id,membership_id,capability_type,api_key,key_prefix,gateway_url,issued_at,updated_at) VALUES('fake','fake','fake','synthetic-secret-never-export','synthetic','https://example.invalid',1,1)",[]).unwrap();
        secure_dir(&f.profile.join("skills/user-packages/test/v1")).unwrap();
        write_new(
            &f.profile.join("skills/user-packages/test/v1/SKILL.md"),
            b"personal skill",
        )
        .unwrap();
        secure_dir(&f.profile.join("credentials")).unwrap();
        write_new(&f.profile.join("credentials/master.key"), b"synthetic-vault-key-never-export").unwrap();
        write_new(&f.profile.join("credentials/entry.vault"), b"synthetic-vault-ciphertext-never-export").unwrap();
        let op = f.prepare();
        let proof = f.proof(&op);
        let done = f.capture(&op, Some(proof));
        assert_eq!(
            done.state,
            PersonalDataOperationState::Completed,
            "{:?}",
            done.blockers
        );
        let bytes = read_limited(
            &operation_root(&f.profile, &op.operation_id)
                .unwrap()
                .join("backup.zip"),
            MAX_TOTAL,
        )
        .unwrap();
        let (m, files) = validate_archive(&bytes).unwrap();
        assert!(m.complete);
        assert!(files.contains_key("profile/skills/user-packages/test/v1/SKILL.md"));
        assert!(!files.keys().any(|path| path.starts_with("profile/credentials/")));
        assert!(!bytes.windows(b"synthetic-vault-key-never-export".len()).any(|part| part == b"synthetic-vault-key-never-export"));
        let dbfile = f.root.join("verify.sqlite");
        write_new(&dbfile, &files["profile/contexthub.db"].1).unwrap();
        let check = rusqlite::Connection::open(dbfile).unwrap();
        assert_eq!(
            check
                .query_row(
                    "SELECT value FROM app_metadata WHERE key='backup-marker'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "wal-visible"
        );
        assert_eq!(
            check
                .query_row(
                    "SELECT COUNT(*) FROM employee_service_credentials",
                    [],
                    |r| r.get::<_, u32>(0)
                )
                .unwrap(),
            0
        );
        assert!(
            m.exclusions
                .iter()
                .any(|i| i.code == "local_credentials_and_keys_excluded")
        );
    }
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "单次恢复覆盖准备、维护重确认、离线切换与原资料保留的完整链路"
    )]
    fn restore_uses_frozen_validated_candidate_and_keeps_current_directory_until_manifest_switch() {
        let mut f = Fixture::new();
        secure_dir(&f.profile.join("personal-capability-data/app")).unwrap();
        write_new(
            &f.profile.join("personal-capability-data/app/marker.txt"),
            b"original",
        )
        .unwrap();
        write_new(
            &f.profile
                .join("personal-capability-preview-data/app/preview.txt"),
            b"preview-user-input",
        )
        .unwrap();
        let backup = f.prepare();
        let backup_proof = f.proof(&backup);
        let done = f.capture(&backup, Some(backup_proof.clone()));
        assert_eq!(
            done.state,
            PersonalDataOperationState::Completed,
            "{:?}",
            done.blockers
        );
        f.release(&backup_proof);
        let archive = operation_root(&f.profile, &backup.operation_id)
            .unwrap()
            .join("backup.zip");
        let restore = f
            .services
            .data_restore_prepare(
                &f.profile,
                &f.id,
                &PersonalRestorePrepareParams {
                    archive_path: archive.to_string_lossy().into_owned(),
                    client_operation_id: "restore".into(),
                },
                5,
            )
            .unwrap();
        let target = restore
            .restore_preview
            .as_ref()
            .unwrap()
            .target_profile_id
            .clone();
        assert_ne!(target, f.id);
        fs::write(archive, b"changed original zip").unwrap();
        let proof = f.proof(&restore);
        let ready = f
            .services
            .data_restore_apply_prepare(
                &f.profile,
                &f.id,
                &PersonalRestoreApplyPrepareParams {
                    operation_id: restore.operation_id.clone(),
                    preview_digest: restore.preview_digest.clone(),
                    expected_current_profile_id: f.id.clone(),
                    client_operation_id: "apply".into(),
                    quiescence: Some(proof),
                },
                "test-instance",
                6,
            )
            .unwrap();
        assert_eq!(ready.state, PersonalDataOperationState::ReadyToApply);
        let saved_proof: PersonalDataQuiescence = serde_json::from_slice(
            &read_limited(
                &operation_root(&f.profile, &ready.operation_id)
                    .unwrap()
                    .join("restore-quiescence.json"),
                256 * 1024,
            )
            .unwrap(),
        )
        .unwrap();
        f.release(&saved_proof);
        let fresh_proof = f.proof(&ready);
        assert_ne!(saved_proof.token, fresh_proof.token);
        let ready = f
            .services
            .data_restore_apply_prepare(
                &f.profile,
                &f.id,
                &PersonalRestoreApplyPrepareParams {
                    operation_id: ready.operation_id.clone(),
                    preview_digest: ready.preview_digest.clone(),
                    expected_current_profile_id: f.id.clone(),
                    client_operation_id: "explicit-reconfirm-after-maintenance-ended".into(),
                    quiescence: Some(fresh_proof),
                },
                "test-instance",
                7,
            )
            .unwrap();
        f.services.persistence.take();
        let result = offline_restore(&f.root, &ready.operation_id, &ready.preview_digest).unwrap();
        assert_eq!(result.state, PersonalDataOperationState::Completed);
        assert_eq!(
            fs::read(f.profile.join("personal-capability-data/app/marker.txt")).unwrap(),
            b"original"
        );
        assert_eq!(
            fs::read(
                f.root
                    .join("profiles")
                    .join(&target)
                    .join("personal-capability-data/app/marker.txt")
            )
            .unwrap(),
            b"original"
        );
        assert_eq!(
            fs::read(
                f.profile
                    .join("personal-capability-preview-data/app/preview.txt")
            )
            .unwrap(),
            b"preview-user-input"
        );
        assert_eq!(
            fs::read(
                f.root
                    .join("profiles")
                    .join(&target)
                    .join("personal-capability-preview-data/app/preview.txt")
            )
            .unwrap(),
            b"preview-user-input"
        );
        let manifest: PersonalProfileManifest =
            serde_json::from_slice(&fs::read(f.root.join("local-profile.json")).unwrap()).unwrap();
        assert_eq!(manifest.profile_id, target);
    }
    #[test]
    fn corrupt_archive_is_rejected_without_preparing_or_changing_current_profile() {
        let mut f = Fixture::new();
        let original = fs::read(f.root.join("local-profile.json")).unwrap();
        let path = f.root.join("corrupt.zip");
        write_new(&path, b"invalid archive").unwrap();
        let result = f.services.data_restore_prepare(
            &f.profile,
            &f.id,
            &PersonalRestorePrepareParams {
                archive_path: path.to_string_lossy().into_owned(),
                client_operation_id: "bad".into(),
            },
            1,
        );
        assert_eq!(result.unwrap_err().reason, "personal_data_archive_invalid");
        assert_eq!(
            fs::read(f.root.join("local-profile.json")).unwrap(),
            original
        );
        assert!(
            f.services
                .persistence
                .as_ref()
                .unwrap()
                .personal_data_operations(&f.id)
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn unclassified_profile_file_does_not_become_a_successful_partial_backup() {
        let mut f = Fixture::new();
        write_new(&f.profile.join("unknown-user-material.bin"), b"retain").unwrap();
        let op = f.prepare();
        let proof = f.proof(&op);
        let failed = f.capture(&op, Some(proof));
        assert_eq!(failed.state, PersonalDataOperationState::Incomplete);
        assert_eq!(
            failed.blockers[0].code,
            "personal_data_profile_files_unclassified"
        );
        assert_eq!(
            fs::read(f.profile.join("unknown-user-material.bin")).unwrap(),
            b"retain"
        );
    }
    #[test]
    fn native_auth_and_config_are_never_valid_backup_entries() {
        let home = PersonalBackupNativeHome {
            relative_path: "ai-runtime/cilai-harness/official-runtime".into(),
            runtime_version: None,
        };
        assert!(!allowed_archive_path(
            "profile/ai-runtime/cilai-harness/official-runtime/auth.json",
            &PersonalDataCategory::NativeHistory,
            &[home]
        ));
        for category in [PersonalDataCategory::Configuration, PersonalDataCategory::NativeHistory, PersonalDataCategory::Attachments] {
            assert!(!allowed_archive_path("profile/credentials/master.key", &category, &[]));
            assert!(!allowed_archive_path("profile/credentials/entry.vault", &category, &[]));
        }
        assert!(!safe_relative("profile/../auth.json"));
        assert!(!safe_relative("C:/auth.json"));
    }
    #[test]
    fn missing_referenced_attachment_makes_the_backup_incomplete() {
        let mut f = Fixture::new();
        let db = f
            .services
            .persistence
            .as_ref()
            .unwrap()
            .database_path()
            .to_owned();
        let connection = rusqlite::Connection::open(db).unwrap();
        let value = json!({"messageId":"missing-attachment-message","conversationId":"fixture-conversation","attachments":[{"attachmentId":"missing","localPath":f.profile.join("ai-runtime/attachments/missing.txt").to_string_lossy()}]});
        connection.execute("INSERT INTO ai_messages(message_id,position,message_json) VALUES('missing-attachment-message',0,?1)",[value.to_string()]).unwrap();
        let op = f.prepare();
        let proof = f.proof(&op);
        let failed = f.capture(&op, Some(proof));
        assert_eq!(failed.state, PersonalDataOperationState::Incomplete);
        assert_eq!(failed.blockers[0].code, "personal_data_attachment_missing");
    }
    #[test]
    fn native_committed_wal_is_snapshotted_without_auth_or_tool_environment() {
        let mut f = Fixture::new();
        let home = f.profile.join("ai-runtime/cilai-harness/official-runtime");
        secure_dir(&home).unwrap();
        let native = rusqlite::Connection::open(home.join("state_5.sqlite")).unwrap();
        native.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE test_history(value TEXT); INSERT INTO test_history VALUES('committed native history');").unwrap();
        write_new(&home.join("auth.json"), b"synthetic-native-secret").unwrap();
        write_new(&home.join("config.toml"), b"synthetic-runtime-environment").unwrap();
        let op = f.prepare();
        let mut proof = f.proof(&op);
        proof.native_homes = vec![PersonalBackupNativeHome {
            relative_path: "ai-runtime/cilai-harness/official-runtime".into(),
            runtime_version: Some("test-fixture".into()),
        }];
        let done = f.capture(&op, Some(proof));
        assert_eq!(
            done.state,
            PersonalDataOperationState::Completed,
            "{:?}",
            done.blockers
        );
        let bytes = read_limited(
            &operation_root(&f.profile, &op.operation_id)
                .unwrap()
                .join("backup.zip"),
            MAX_TOTAL,
        )
        .unwrap();
        let (_, files) = validate_archive(&bytes).unwrap();
        assert!(!files.keys().any(|path| path.ends_with("auth.json")
            || path.ends_with("config.toml")
            || path.ends_with("-wal")
            || path.ends_with("-shm")));
        let restored = f.root.join("native-snapshot.sqlite");
        write_new(
            &restored,
            &files["profile/ai-runtime/cilai-harness/official-runtime/state_5.sqlite"].1,
        )
        .unwrap();
        let check = rusqlite::Connection::open(restored).unwrap();
        assert_eq!(
            check
                .query_row("SELECT value FROM test_history", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "committed native history"
        );
    }
    #[test]
    #[ignore = "由父测试启动并核对真实退出"]
    fn maintenance_owner_process_probe() {
        println!("maintenance-owner-ready");
        std::io::stdout().flush().unwrap();
        let mut byte = [0_u8; 1];
        let _ = std::io::stdin().read(&mut byte);
    }
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "完整覆盖维护拥有者真实子进程退出和显式结果确认及栅栏恢复"
    )]
    fn interrupted_capture_requires_checked_owner_exit_and_does_not_repeat_file_actions() {
        use std::io::BufRead;
        let mut f = Fixture::new();
        let parent = f.owner.clone();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "personal_data::tests::maintenance_owner_process_probe",
                "--ignored",
                "--nocapture",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            assert_ne!(output.read_line(&mut line).unwrap(), 0);
            if line.contains("maintenance-owner-ready") {
                break;
            }
        }
        f.owner = ModelExecutionOwner {
            instance_id: "child-maintenance-owner".into(),
            pid: child.id(),
            start_fingerprint: contexthub_platform_adapter::instance::process_start_fingerprint(
                child.id(),
            )
            .unwrap()
            .unwrap(),
        };
        let mut op = f.prepare();
        let proof = f.proof(&op);
        op.state = PersonalDataOperationState::Capturing;
        f.services
            .data_transition(&op, &PersonalDataOperationState::Prepared)
            .unwrap();
        let params = PersonalDataCancelParams {
            operation_id: op.operation_id.clone(),
            preview_digest: op.preview_digest.clone(),
        };
        let denied_release = f
            .services
            .persistence
            .as_mut()
            .unwrap()
            .personal_writer_rpc(
                "personalMaintenance/release",
                json!({"fenceId":proof.token,"owner":f.owner}),
                &f.id,
                "test-instance",
                4,
            )
            .unwrap_err();
        assert_eq!(
            denied_release.reason(),
            Some("personal_data_operation_unconfirmed")
        );
        assert_eq!(
            f.services
                .data_resolve_interrupted(&f.profile, &f.id, &params, 4)
                .unwrap_err()
                .reason,
            "personal_maintenance_owner_alive"
        );
        drop(child.stdin.take());
        assert!(child.wait().unwrap().success());
        let incomplete = f
            .services
            .data_resolve_interrupted(&f.profile, &f.id, &params, 5)
            .unwrap();
        assert_eq!(incomplete.state, PersonalDataOperationState::Incomplete);
        assert!(
            !operation_root(&f.profile, &op.operation_id)
                .unwrap()
                .join("backup.zip")
                .exists()
        );
        let fences = f
            .services
            .persistence
            .as_mut()
            .unwrap()
            .personal_writer_rpc(
                "personalMaintenance/list",
                json!({}),
                &f.id,
                "test-instance",
                6,
            )
            .unwrap();
        assert_eq!(fences[0]["state"], "draining");
        let released = f
            .services
            .persistence
            .as_mut()
            .unwrap()
            .personal_writer_rpc(
                "personalMaintenance/recover",
                json!({"fenceId":proof.token,"owner":parent}),
                &f.id,
                "test-instance",
                7,
            )
            .unwrap();
        assert_eq!(released["state"], "released");
    }
    #[test]
    fn restore_without_checked_guard_is_persistently_incomplete_and_retains_candidate() {
        let mut f = Fixture::new();
        let backup = f.prepare();
        let proof = f.proof(&backup);
        assert_eq!(
            f.capture(&backup, Some(proof.clone())).state,
            PersonalDataOperationState::Completed
        );
        f.release(&proof);
        let archive = operation_root(&f.profile, &backup.operation_id)
            .unwrap()
            .join("backup.zip");
        let restore = f
            .services
            .data_restore_prepare(
                &f.profile,
                &f.id,
                &PersonalRestorePrepareParams {
                    archive_path: archive.to_string_lossy().into_owned(),
                    client_operation_id: "restore-without-maintenance".into(),
                },
                5,
            )
            .unwrap();
        let result = f
            .services
            .data_restore_apply_prepare(
                &f.profile,
                &f.id,
                &PersonalRestoreApplyPrepareParams {
                    operation_id: restore.operation_id.clone(),
                    preview_digest: restore.preview_digest.clone(),
                    expected_current_profile_id: f.id.clone(),
                    client_operation_id: "apply-no-proof".into(),
                    quiescence: None,
                },
                "test-instance",
                6,
            )
            .unwrap();
        assert_eq!(result.state, PersonalDataOperationState::Incomplete);
        assert_eq!(
            result.blockers[0].code,
            "personal_data_quiescence_unconfirmed"
        );
        assert!(
            operation_root(&f.profile, &restore.operation_id)
                .unwrap()
                .join("restore-source.zip")
                .is_file()
        );
        assert_eq!(profile_manifest(&f.profile).unwrap().profile_id, f.id);
    }
    #[test]
    fn backup_includes_conversation_files_and_excludes_exact_private_execution_homes_and_locks() {
        let mut f = Fixture::new();
        for (name, content) in [
            (
                "ai-runtime/artifact-versions/artifact_one/v1.txt",
                b"version".as_slice(),
            ),
            (
                "ai-runtime/conversation-workspaces/conversation_one/workspace/result.txt",
                b"workspace".as_slice(),
            ),
            (
                "ai-runtime/mcp-runtime/connection_one/lease_one/private.txt",
                b"secret-canary".as_slice(),
            ),
            (
                "ai-runtime/skill-snapshots/stale/SKILL.md",
                b"cache".as_slice(),
            ),
            ("ai-runtime/memory.lock", b"".as_slice()),
            ("model-credential-operation.lock", b"".as_slice()),
            ("ai-runtime/history-migration-report.json",b"{\"schemaVersion\":1,\"databaseBackup\":\"retained-old.sqlite\",\"nativeBackup\":\"retained-old.native\",\"conversations\":[]}".as_slice()),
        ] {
            write_new(&f.profile.join(name), content).unwrap();
        }
        let op = f.prepare();
        let proof = f.proof(&op);
        let done = f.capture(&op, Some(proof));
        assert_eq!(
            done.state,
            PersonalDataOperationState::Completed,
            "{:?}",
            done.blockers
        );
        let archive = read_limited(
            &operation_root(&f.profile, &op.operation_id)
                .unwrap()
                .join("backup.zip"),
            MAX_TOTAL,
        )
        .unwrap();
        let (manifest, files) = validate_archive(&archive).unwrap();
        assert_eq!(
            manifest.schema_migration_version,
            contexthub_persistence_sqlite::personal_data_schema_version()
        );
        assert!(files.contains_key("profile/ai-runtime/artifact-versions/artifact_one/v1.txt"));
        assert!(files.contains_key(
            "profile/ai-runtime/conversation-workspaces/conversation_one/workspace/result.txt"
        ));
        assert!(files.keys().all(|name| !name.contains("mcp-runtime")
            && !name.ends_with("memory.lock")
            && !name.ends_with("model-credential-operation.lock")));
        for name in [
            "ai-runtime/mcp-runtime",
            "ai-runtime/skill-snapshots",
            "ai-runtime/history-migration-report.json",
            "ai-runtime/memory.lock",
            "model-credential-operation.lock",
        ] {
            assert!(
                manifest
                    .exclusions
                    .iter()
                    .any(|issue| issue.relative_path.as_deref() == Some(name))
            );
        }
        assert_eq!(
            fs::read(
                f.profile
                    .join("ai-runtime/mcp-runtime/connection_one/lease_one/private.txt")
            )
            .unwrap(),
            b"secret-canary"
        );
        let mut older = manifest;
        older.schema_migration_version = 72;
        let old_path = f.root.join("schema72.zip");
        write_archive(&old_path, &older, &files).unwrap();
        assert_eq!(
            validate_archive(&read_limited(&old_path, MAX_TOTAL).unwrap())
                .unwrap_err()
                .reason,
            "personal_data_backup_schema_unsupported"
        );
    }
    #[test]
    fn unknown_runtime_root_is_not_silently_omitted_from_complete_backup() {
        let mut f = Fixture::new();
        write_new(
            &f.profile.join("ai-runtime/unknown-business-state/data"),
            b"keep",
        )
        .unwrap();
        let op = f.prepare();
        let proof = f.proof(&op);
        let done = f.capture(&op, Some(proof));
        assert_eq!(done.state, PersonalDataOperationState::Incomplete);
        assert_eq!(
            done.blockers[0].code,
            "personal_data_profile_files_unclassified"
        );
        assert!(
            f.profile
                .join("ai-runtime/unknown-business-state/data")
                .exists()
        );
    }
}
