use super::{
    ArchiveFiles, MAX_FILE, MAX_FILES, MAX_TOTAL, ServiceError, db_error, digest, error,
    fingerprint, inspect, issue, read_limited, safe_relative, secure_dir, write_new,
};
use contexthub_domain::WorkspaceRelativePath;
use contexthub_platform_adapter::filesystem;
use contexthub_protocol::{
    PersonalBackupManifest, PersonalBackupWorkspace, PersonalDataCategory, PersonalDataIssue,
    PersonalRestorePreview, PersonalRestoreWorkspaceDestination, PersonalRestoredWorkspace,
    PersonalWorkspaceContentSnapshot,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub(super) fn prefix(id: &str) -> String {
    format!(
        "workspace-originals/{:x}/",
        sha2::Sha256::digest(id.as_bytes())
    )
}
use sha2::Digest;

pub(super) fn snapshots(
    manifest: &PersonalBackupManifest,
) -> Result<BTreeMap<String, PersonalWorkspaceContentSnapshot>, ServiceError> {
    let selected: BTreeSet<_> = manifest.selection.include_workspace_files.iter().collect();
    if selected.len() != manifest.selection.include_workspace_files.len() {
        return Err(error("personal_data_manifest_invalid"));
    }
    let mut result = BTreeMap::new();
    let mut workspace_ids = BTreeSet::new();
    for workspace in &manifest.workspaces {
        if workspace.workspace_id.is_empty() || !workspace_ids.insert(&workspace.workspace_id) {
            return Err(error("personal_data_manifest_invalid"));
        }
        if selected.contains(&workspace.workspace_id) {
            let snapshot = workspace
                .content_snapshot
                .as_ref()
                .ok_or_else(|| error("personal_data_workspace_index_unready"))?;
            validate_basis(workspace, snapshot)?;
            if result
                .insert(workspace.workspace_id.clone(), snapshot.clone())
                .is_some()
            {
                return Err(error("personal_data_manifest_invalid"));
            }
        } else if workspace.original_files_included
            || workspace.content_snapshot.is_some()
            || workspace.archive_prefix.is_some()
        {
            return Err(error("personal_data_manifest_invalid"));
        }
    }
    if selected.len() != result.len() {
        return Err(error("personal_data_workspace_unknown"));
    }
    Ok(result)
}

fn validate_basis(
    workspace: &PersonalBackupWorkspace,
    snapshot: &PersonalWorkspaceContentSnapshot,
) -> Result<(), ServiceError> {
    let mut content = snapshot.clone();
    content.digest.clear();
    if snapshot.protocol_version != 1
        || snapshot.workspace_id != workspace.workspace_id
        || snapshot.root_identity.is_empty()
        || snapshot.root_path != workspace.original_path
        || snapshot.workspace_revision < 0
        || snapshot.full_generation <= 0
        || snapshot.latest_generation < snapshot.full_generation
        || snapshot.full_snapshot_id.is_empty()
        || fingerprint(&content)? != snapshot.digest
        || workspace.archive_prefix.as_deref() != Some(prefix(&workspace.workspace_id).as_str())
        || snapshot.entries.len() > MAX_FILES
    {
        return Err(error("personal_data_workspace_snapshot_invalid"));
    }
    let mut paths = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for entry in &snapshot.entries {
        if !safe_relative(&entry.relative_path)
            || entry.relative_path.split('/').count() > 128
            || entry.file_identity.is_empty()
            || entry.local_file_id.is_empty()
            || entry.state_revision < 0
            || !ids.insert(&entry.local_file_id)
            || !matches!(entry.file_type.as_str(), "file" | "directory")
            || paths
                .insert(entry.relative_path.clone(), entry.file_type.clone())
                .is_some()
            || ignored(&entry.relative_path)?
        {
            return Err(error("personal_data_workspace_snapshot_invalid"));
        }
        if entry.file_type == "file"
            && (entry.bytes.is_none_or(|size| size > MAX_FILE)
                || !entry.sha256.as_ref().is_some_and(|hash| {
                    hash.len() == 64
                        && hash
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                }))
        {
            return Err(error("personal_data_workspace_snapshot_invalid"));
        }
    }
    for path in paths.keys() {
        let mut parent = Path::new(path).parent();
        while let Some(value) = parent.filter(|p| !p.as_os_str().is_empty()) {
            if value
                .to_str()
                .and_then(|p| paths.get(p))
                .map(String::as_str)
                != Some("directory")
            {
                return Err(error("personal_data_workspace_snapshot_invalid"));
            }
            parent = value.parent();
        }
    }
    Ok(())
}
fn ignored(relative: &str) -> Result<bool, ServiceError> {
    let path = WorkspaceRelativePath::parse(relative)
        .map_err(|_| error("personal_data_workspace_path_invalid"))?;
    Ok(contexthub_workspace_fs::fixed_ignore_match(&path).is_some())
}
fn root_identity(root: &Path) -> Result<String, ServiceError> {
    for ancestor in root.ancestors() {
        if !inspect(ancestor)?.is_dir() {
            return Err(error("personal_data_workspace_path_invalid"));
        }
    }
    let meta = filesystem::symlink_metadata_with_stable_identity(root)
        .map_err(|_| error("personal_data_workspace_unavailable"))?;
    if filesystem::is_cloud_placeholder_file(&meta.metadata) {
        return Err(error("personal_data_workspace_placeholder"));
    }
    Ok(meta.stable_identity)
}
#[derive(Eq, PartialEq)]
struct Member {
    kind: String,
    identity: String,
    bytes: Option<u64>,
    modified: Option<i64>,
}
fn member(path: &Path) -> Result<Member, ServiceError> {
    let meta = filesystem::symlink_metadata_with_stable_identity(path)
        .map_err(|_| error("personal_data_workspace_unavailable"))?;
    if meta.metadata.file_type().is_symlink()
        || filesystem::is_cloud_placeholder_file(&meta.metadata)
        || !(meta.metadata.is_file() || meta.metadata.is_dir())
    {
        return Err(error("personal_data_workspace_file_invalid"));
    }
    Ok(Member {
        kind: if meta.metadata.is_dir() {
            "directory"
        } else {
            "file"
        }
        .into(),
        identity: meta.stable_identity,
        bytes: meta.metadata.is_file().then_some(meta.metadata.len()),
        modified: meta
            .metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .and_then(|time| i64::try_from(time.as_millis()).ok()),
    })
}
fn inventory(
    root: &Path,
    snapshot: &PersonalWorkspaceContentSnapshot,
    exclusions: &mut Vec<PersonalDataIssue>,
) -> Result<BTreeMap<String, Member>, ServiceError> {
    if root_identity(root)? != snapshot.root_identity {
        return Err(error("personal_data_workspace_root_changed"));
    }
    let mut members = BTreeMap::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        for entry in fs::read_dir(root.join(&relative))
            .map_err(|_| error("personal_data_workspace_unavailable"))?
        {
            let entry = entry.map_err(|_| error("personal_data_workspace_unavailable"))?;
            let path = relative.join(entry.file_name());
            let portable = path
                .to_str()
                .ok_or_else(|| error("personal_data_workspace_path_invalid"))?
                .replace('\\', "/");
            if !safe_relative(&portable) || portable.split('/').count() > 128 {
                return Err(error("personal_data_workspace_path_invalid"));
            }
            if ignored(&portable)? {
                exclusions.push(issue(
                    "workspace_index_policy_excluded",
                    Some(PersonalDataCategory::WorkspaceOriginals),
                    Some(format!("{}/{}", snapshot.workspace_id, portable)),
                ));
                continue;
            }
            let found = member(&entry.path())?;
            if found.kind == "directory" {
                pending.push(path);
            }
            if members.insert(portable, found).is_some() || members.len() > MAX_FILES {
                return Err(error("personal_data_workspace_snapshot_invalid"));
            }
        }
    }
    if members.len() != snapshot.entries.len() {
        return Err(error("personal_data_workspace_members_changed"));
    }
    for expected in &snapshot.entries {
        let actual = members
            .get(&expected.relative_path)
            .ok_or_else(|| error("personal_data_workspace_members_changed"))?;
        if actual.kind != expected.file_type
            || actual.identity != expected.file_identity
            || (actual.kind == "file"
                && (actual.bytes != expected.bytes || actual.modified != expected.modified_at))
        {
            return Err(error("personal_data_workspace_content_changed"));
        }
    }
    Ok(members)
}

pub(super) fn capture(
    manifest: &mut PersonalBackupManifest,
    files: &mut ArchiveFiles,
) -> Result<(), ServiceError> {
    let _ = snapshots(manifest)?;
    let mut total = files
        .values()
        .map(|(_, bytes)| bytes.len() as u64)
        .sum::<u64>();
    for workspace in &mut manifest.workspaces {
        let Some(snapshot) = workspace.content_snapshot.as_ref() else {
            continue;
        };
        let root = Path::new(&workspace.original_path);
        let before = inventory(root, snapshot, &mut manifest.exclusions)?;
        for entry in snapshot
            .entries
            .iter()
            .filter(|entry| entry.file_type == "file")
        {
            let bytes = read_limited(&root.join(&entry.relative_path), MAX_FILE)?;
            if entry.bytes != Some(bytes.len() as u64)
                || entry.sha256.as_ref().map(|hash| format!("sha256:{hash}"))
                    != Some(digest(&bytes))
            {
                return Err(error("personal_data_workspace_content_changed"));
            }
            total = total
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| error("personal_data_archive_too_large"))?;
            files.insert(
                format!("{}{}", prefix(&workspace.workspace_id), entry.relative_path),
                (PersonalDataCategory::WorkspaceOriginals, bytes),
            );
            if files.len() > MAX_FILES || total > MAX_TOTAL {
                return Err(error("personal_data_archive_too_large"));
            }
        }
        if inventory(root, snapshot, &mut Vec::new())? != before
            || root_identity(root)? != snapshot.root_identity
        {
            return Err(error("personal_data_workspace_content_changed"));
        }
        workspace.original_files_included = true;
    }
    Ok(())
}

fn verify_source(
    root: &Path,
    snapshot: &PersonalWorkspaceContentSnapshot,
) -> Result<(), ServiceError> {
    let before = inventory(root, snapshot, &mut Vec::new())?;
    for entry in snapshot
        .entries
        .iter()
        .filter(|entry| entry.file_type == "file")
    {
        let bytes = read_limited(&root.join(&entry.relative_path), MAX_FILE)?;
        if entry.sha256.as_ref().map(|hash| format!("sha256:{hash}")) != Some(digest(&bytes)) {
            return Err(error("personal_data_workspace_content_changed"));
        }
    }
    if inventory(root, snapshot, &mut Vec::new())? != before {
        return Err(error("personal_data_workspace_content_changed"));
    }
    Ok(())
}

pub(super) fn verify_sources(manifest: &PersonalBackupManifest) -> Result<(), ServiceError> {
    for workspace in &manifest.workspaces {
        if let Some(snapshot) = workspace.content_snapshot.as_ref() {
            verify_source(Path::new(&workspace.original_path), snapshot)?;
        }
    }
    Ok(())
}

pub(super) fn validate_files(
    manifest: &PersonalBackupManifest,
    files: &ArchiveFiles,
) -> Result<(), ServiceError> {
    let snapshots = snapshots(manifest)?;
    if manifest.format_version == 1 && !snapshots.is_empty() {
        return Err(error("personal_data_manifest_invalid"));
    }
    let mut expected = BTreeSet::new();
    for workspace in &manifest.workspaces {
        let Some(snapshot) = snapshots.get(&workspace.workspace_id) else {
            continue;
        };
        if !workspace.original_files_included {
            return Err(error("personal_data_manifest_invalid"));
        }
        for entry in snapshot
            .entries
            .iter()
            .filter(|entry| entry.file_type == "file")
        {
            let path = format!("{}{}", prefix(&workspace.workspace_id), entry.relative_path);
            let (category, bytes) = files
                .get(&path)
                .ok_or_else(|| error("personal_data_archive_file_missing"))?;
            if *category != PersonalDataCategory::WorkspaceOriginals
                || entry.bytes != Some(bytes.len() as u64)
                || entry.sha256.as_ref().map(|hash| format!("sha256:{hash}")) != Some(digest(bytes))
            {
                return Err(error("personal_data_archive_digest_mismatch"));
            }
            expected.insert(path);
        }
    }
    if files.iter().any(|(path, (category, _))| {
        *category == PersonalDataCategory::WorkspaceOriginals && !expected.contains(path)
    }) {
        return Err(error("personal_data_archive_path_invalid"));
    }
    Ok(())
}

pub(super) fn destination(
    parent: &Path,
    profile: &Path,
    target_profile: &str,
) -> Result<PersonalRestoreWorkspaceDestination, ServiceError> {
    if !parent.is_absolute() {
        return Err(error("personal_data_workspace_destination_invalid"));
    }
    let identity = root_identity(parent)?;
    let canonical = fs::canonicalize(parent)
        .map_err(|_| error("personal_data_workspace_destination_invalid"))?;
    let namespace = profile
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| error("personal_data_path_invalid"))?;
    let target = canonical.join(format!("Cilai-Restore-{target_profile}"));
    if target.starts_with(namespace) || namespace.starts_with(&target) {
        return Err(error("personal_data_workspace_destination_protected"));
    }
    if fs::symlink_metadata(&target).is_ok() {
        return Err(error("personal_data_restore_target_exists"));
    }
    Ok(PersonalRestoreWorkspaceDestination {
        parent_path: canonical
            .to_str()
            .ok_or_else(|| error("personal_data_workspace_path_invalid"))?
            .into(),
        parent_identity: identity,
        target_path: target
            .to_str()
            .ok_or_else(|| error("personal_data_workspace_path_invalid"))?
            .into(),
    })
}
pub(super) fn verify_destination(
    preview: &PersonalRestorePreview,
    profile: &Path,
) -> Result<Option<PathBuf>, ServiceError> {
    if !preview
        .workspaces
        .iter()
        .any(|workspace| workspace.original_files_included)
    {
        return Ok(None);
    }
    let plan = preview
        .workspace_destination
        .as_ref()
        .ok_or_else(|| error("personal_data_workspace_destination_required"))?;
    if destination(
        Path::new(&plan.parent_path),
        profile,
        &preview.target_profile_id,
    )? != *plan
    {
        return Err(error("personal_data_workspace_destination_changed"));
    }
    Ok(Some(PathBuf::from(&plan.target_path)))
}

pub(super) struct RestoredOriginals {
    pub workspaces: Vec<PersonalRestoredWorkspace>,
    target_identity: Option<String>,
}

pub(super) fn restore_originals(
    manifest: &PersonalBackupManifest,
    files: &ArchiveFiles,
    preview: &PersonalRestorePreview,
    profile: &Path,
) -> Result<RestoredOriginals, ServiceError> {
    let Some(target) = verify_destination(preview, profile)? else {
        return Ok(RestoredOriginals {
            workspaces: Vec::new(),
            target_identity: None,
        });
    };
    fs::create_dir(&target).map_err(|_| error("personal_data_restore_target_exists"))?;
    secure_dir(&target)?;
    let target_identity = root_identity(&target)?;
    let mut restored = Vec::new();
    for workspace in &manifest.workspaces {
        let Some(snapshot) = workspace.content_snapshot.as_ref() else {
            continue;
        };
        let root = target.join(format!(
            "{:x}",
            sha2::Sha256::digest(workspace.workspace_id.as_bytes())
        ));
        fs::create_dir(&root).map_err(|_| error("personal_data_restore_target_exists"))?;
        secure_dir(&root)?;
        let mut ordered = snapshot.entries.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|entry| {
            (
                entry.relative_path.split('/').count(),
                entry.relative_path.as_str(),
            )
        });
        for entry in ordered {
            let path = root.join(&entry.relative_path);
            if entry.file_type == "directory" {
                secure_dir(&path)?;
            } else {
                let (_, bytes) = files
                    .get(&format!(
                        "{}{}",
                        prefix(&workspace.workspace_id),
                        entry.relative_path
                    ))
                    .ok_or_else(|| error("personal_data_archive_file_missing"))?;
                write_new(&path, bytes)?;
                if read_limited(&path, MAX_FILE)? != *bytes {
                    return Err(error("personal_data_restore_readback_failed"));
                }
            }
        }
        filesystem::sync_directory(&root).map_err(|_| error("personal_data_write_failed"))?;
        let mut actual = snapshot.clone();
        actual.root_identity = root_identity(&root)?;
        for entry in &mut actual.entries {
            let found = member(&root.join(&entry.relative_path))?;
            entry.file_identity = found.identity;
            entry.modified_at = found.modified;
        }
        inventory(&root, &actual, &mut Vec::new())?;
        for entry in actual
            .entries
            .iter()
            .filter(|entry| entry.file_type == "file")
        {
            if entry.sha256.as_ref().map(|hash| format!("sha256:{hash}"))
                != Some(digest(&read_limited(
                    &root.join(&entry.relative_path),
                    MAX_FILE,
                )?))
            {
                return Err(error("personal_data_restore_readback_failed"));
            }
        }
        inventory(&root, &actual, &mut Vec::new())?;
        restored.push(PersonalRestoredWorkspace {
            workspace_id: workspace.workspace_id.clone(),
            source_path: workspace.original_path.clone(),
            restored_path: root
                .to_str()
                .ok_or_else(|| error("personal_data_workspace_path_invalid"))?
                .into(),
            content_snapshot_digest: snapshot.digest.clone(),
        });
    }
    filesystem::sync_directory(&target)
        .and_then(|()| {
            filesystem::sync_directory(
                target
                    .parent()
                    .ok_or_else(|| std::io::Error::other("parent missing"))?,
            )
        })
        .map_err(|_| error("personal_data_write_failed"))?;
    Ok(RestoredOriginals {
        workspaces: restored,
        target_identity: Some(target_identity),
    })
}

pub(super) fn verify_restored(
    preview: &PersonalRestorePreview,
    restored: &RestoredOriginals,
    bases: &BTreeMap<String, PersonalWorkspaceContentSnapshot>,
) -> Result<(), ServiceError> {
    let Some(plan) = preview.workspace_destination.as_ref() else {
        return if restored.workspaces.is_empty() && bases.is_empty() {
            Ok(())
        } else {
            Err(error("personal_data_workspace_destination_required"))
        };
    };
    if root_identity(Path::new(&plan.parent_path))? != plan.parent_identity
        || restored.target_identity.as_deref()
            != Some(root_identity(Path::new(&plan.target_path))?.as_str())
    {
        return Err(error("personal_data_workspace_destination_changed"));
    }
    let expected = restored
        .workspaces
        .iter()
        .map(|workspace| {
            format!(
                "{:x}",
                sha2::Sha256::digest(workspace.workspace_id.as_bytes())
            )
        })
        .collect::<BTreeSet<_>>();
    let actual = fs::read_dir(&plan.target_path)
        .map_err(|_| error("personal_data_restore_readback_failed"))?
        .map(|entry| {
            entry
                .map_err(|_| error("personal_data_restore_readback_failed"))?
                .file_name()
                .into_string()
                .map_err(|_| error("personal_data_workspace_path_invalid"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if expected != actual || bases.len() != restored.workspaces.len() {
        return Err(error("personal_data_workspace_members_changed"));
    }
    for basis in bases.values() {
        verify_source(Path::new(&basis.root_path), basis)?;
    }
    Ok(())
}

pub(super) fn rebind_restored(
    db: &mut contexthub_persistence_sqlite::SqlitePersistence,
    manifest: &PersonalBackupManifest,
    restored: &[PersonalRestoredWorkspace],
    now: i64,
) -> Result<(), ServiceError> {
    let mut locations = BTreeMap::new();
    for workspace in restored {
        let snapshot = manifest
            .workspaces
            .iter()
            .find(|w| w.workspace_id == workspace.workspace_id)
            .and_then(|w| w.content_snapshot.as_ref())
            .ok_or_else(|| error("personal_data_record_invalid"))?;
        let root = Path::new(&workspace.restored_path);
        let mut entries = BTreeMap::new();
        for expected in &snapshot.entries {
            let found = member(&root.join(&expected.relative_path))?;
            entries.insert(
                expected.relative_path.clone(),
                (found.identity, found.modified),
            );
        }
        locations.insert(
            workspace.workspace_id.clone(),
            (
                workspace.restored_path.clone(),
                root_identity(root)?,
                entries,
            ),
        );
    }
    db.restore_personal_workspace_locations(&snapshots(manifest)?, &locations, now)
        .map_err(|e| db_error(&e))
}
