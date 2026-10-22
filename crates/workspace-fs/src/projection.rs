use crate::{LEGACY_WORKSPACE_METADATA_DIRECTORY, WORKSPACE_METADATA_DIRECTORY};
use contexthub_application::{
    ProjectionExportOutcome, ProjectionWriteExpectation, ProjectionWriter,
};
use contexthub_manifest::{EncodedManifestSet, ExportState, checksum_group};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const WORKSPACE_FILE: &str = "workspace.json";
const BASELINE_FILE: &str = "baseline-manifest.json";
const CONFIRMATION_FILE: &str = "confirmation-manifest.json";
const EXPORT_STATE_FILE: &str = "export-state.json";
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub struct WorkspaceProjectionWriter;

impl ProjectionWriter for WorkspaceProjectionWriter {
    type Error = ProjectionWriteError;

    fn write_projection(
        &mut self,
        workspace_root: &str,
        manifests: &EncodedManifestSet,
        expectation: &ProjectionWriteExpectation,
    ) -> Result<ProjectionExportOutcome, Self::Error> {
        write_projection(
            Path::new(workspace_root),
            manifests,
            &ExpectedProjectionState {
                exported_revision: expectation.exported_revision,
                manifest_checksum: expectation.manifest_checksum.clone(),
                allow_incomplete_recovery: expectation.allow_incomplete_recovery,
            },
        )
        .map(|outcome| match outcome {
            ProjectionWriteOutcome::Exported => ProjectionExportOutcome::Exported,
            ProjectionWriteOutcome::AlreadyCurrent => ProjectionExportOutcome::AlreadyCurrent,
        })
    }

    fn is_conflict(error: &Self::Error) -> bool {
        matches!(
            error.kind(),
            ProjectionWriteErrorKind::ReservedPathConflict
                | ProjectionWriteErrorKind::ExistingProjectionConflict
                | ProjectionWriteErrorKind::ExistingProjectionCorrupt
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedProjectionState {
    pub exported_revision: i64,
    pub manifest_checksum: Option<String>,
    pub allow_incomplete_recovery: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionWriteOutcome {
    Exported,
    AlreadyCurrent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionWriteErrorKind {
    RootUnavailable,
    ReservedPathConflict,
    ExistingProjectionConflict,
    ExistingProjectionCorrupt,
    WriteFailed,
    AtomicReplaceUnsupported,
}

#[derive(Debug)]
pub struct ProjectionWriteError {
    kind: ProjectionWriteErrorKind,
    path: PathBuf,
    source: Option<io::Error>,
}

impl ProjectionWriteError {
    #[must_use]
    pub const fn kind(&self) -> ProjectionWriteErrorKind {
        self.kind
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn plain(kind: ProjectionWriteErrorKind, path: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            path: path.into(),
            source: None,
        }
    }

    fn io(kind: ProjectionWriteErrorKind, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self {
            kind,
            path: path.into(),
            source: Some(source),
        }
    }
}

impl std::fmt::Display for ProjectionWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "projection {:?} at {}",
            self.kind,
            self.path.display()
        )?;
        if let Some(source) = &self.source {
            write!(formatter, ": {source}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ProjectionWriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

/// 将四类清单写入工作区保留目录；export-state 始终最后原子替换。
///
/// # Errors
///
/// 工作区或保留目录不可用、现有投影与数据库期望不一致、清单损坏，或任一文件不能刷新并原子替换时返回错误。
pub fn write_projection(
    workspace_root: &Path,
    manifests: &EncodedManifestSet,
    expected: &ExpectedProjectionState,
) -> Result<ProjectionWriteOutcome, ProjectionWriteError> {
    let canonical_root = workspace_root.canonicalize().map_err(|source| {
        ProjectionWriteError::io(
            ProjectionWriteErrorKind::RootUnavailable,
            workspace_root,
            source,
        )
    })?;
    if !canonical_root.is_dir() {
        return Err(ProjectionWriteError::plain(
            ProjectionWriteErrorKind::RootUnavailable,
            canonical_root,
        ));
    }
    let projection_directory = canonical_root.join(WORKSPACE_METADATA_DIRECTORY);
    migrate_legacy_projection(&canonical_root, &projection_directory, manifests, expected)?;
    prepare_projection_directory(&projection_directory)?;
    if inspect_existing(&projection_directory, manifests, expected)? {
        return Ok(ProjectionWriteOutcome::AlreadyCurrent);
    }
    atomic_write(
        &projection_directory,
        WORKSPACE_FILE,
        &manifests.workspace_json,
    )?;
    atomic_write(
        &projection_directory,
        BASELINE_FILE,
        &manifests.baseline_json,
    )?;
    atomic_write(
        &projection_directory,
        CONFIRMATION_FILE,
        &manifests.confirmation_json,
    )?;
    atomic_write(
        &projection_directory,
        EXPORT_STATE_FILE,
        &manifests.export_state_json,
    )?;
    sync_directory(&projection_directory)?;
    Ok(ProjectionWriteOutcome::Exported)
}

// 只迁移已经由当前工作区数据库授权的清单；不能接管原应用的其他工作区。
fn migrate_legacy_projection(
    root: &Path,
    target: &Path,
    incoming: &EncodedManifestSet,
    expected: &ExpectedProjectionState,
) -> Result<(), ProjectionWriteError> {
    let legacy = root.join(LEGACY_WORKSPACE_METADATA_DIRECTORY);
    match fs::symlink_metadata(&legacy) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(ProjectionWriteError::plain(
                ProjectionWriteErrorKind::ReservedPathConflict,
                legacy,
            ));
        }
        Err(source) => {
            return Err(ProjectionWriteError::io(
                ProjectionWriteErrorKind::RootUnavailable,
                legacy,
                source,
            ));
        }
    }
    match fs::symlink_metadata(target) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(ProjectionWriteError::plain(
                ProjectionWriteErrorKind::ExistingProjectionConflict,
                target,
            ));
        }
        Err(source) => {
            return Err(ProjectionWriteError::io(
                ProjectionWriteErrorKind::RootUnavailable,
                target,
                source,
            ));
        }
    }
    if legacy.canonicalize().map_err(|source| {
        ProjectionWriteError::io(ProjectionWriteErrorKind::RootUnavailable, &legacy, source)
    })? != legacy
    {
        return Err(ProjectionWriteError::plain(
            ProjectionWriteErrorKind::ReservedPathConflict,
            legacy,
        ));
    }
    let workspace_path = legacy.join(WORKSPACE_FILE);
    reject_symlink(&workspace_path)?;
    let bytes = fs::read(&workspace_path).map_err(|source| {
        ProjectionWriteError::io(
            ProjectionWriteErrorKind::ExistingProjectionCorrupt,
            &workspace_path,
            source,
        )
    })?;
    let parse = |bytes: &[u8]| {
        serde_json::from_slice::<contexthub_manifest::WorkspaceManifest>(bytes).map_err(|_| {
            ProjectionWriteError::plain(
                ProjectionWriteErrorKind::ExistingProjectionCorrupt,
                &workspace_path,
            )
        })
    };
    let previous_workspace = parse(&bytes)?;
    let current_workspace = parse(&incoming.workspace_json)?;
    if previous_workspace.workspace_id != current_workspace.workspace_id {
        return Err(ProjectionWriteError::plain(
            ProjectionWriteErrorKind::ExistingProjectionConflict,
            workspace_path,
        ));
    }
    inspect_existing(&legacy, incoming, expected)?;
    // 旧版本使用私有目录；先核验，避免移动后才收紧外部目录 ACL、破坏子项的继承权限。
    contexthub_platform_adapter::security::verify_restricted_path(&legacy, true).map_err(
        |source| ProjectionWriteError::io(ProjectionWriteErrorKind::WriteFailed, &legacy, source),
    )?;
    fs::rename(&legacy, target).map_err(|source| {
        ProjectionWriteError::io(ProjectionWriteErrorKind::WriteFailed, &legacy, source)
    })?;
    sync_directory(root)
}

fn prepare_projection_directory(path: &Path) -> Result<(), ProjectionWriteError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(ProjectionWriteError::plain(
                ProjectionWriteErrorKind::ReservedPathConflict,
                path,
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|source| {
                ProjectionWriteError::io(ProjectionWriteErrorKind::WriteFailed, path, source)
            })?;
        }
        Err(source) => {
            return Err(ProjectionWriteError::io(
                ProjectionWriteErrorKind::RootUnavailable,
                path,
                source,
            ));
        }
    }
    set_private_directory_permissions(path)?;
    Ok(())
}

fn inspect_existing(
    directory: &Path,
    incoming: &EncodedManifestSet,
    expected: &ExpectedProjectionState,
) -> Result<bool, ProjectionWriteError> {
    let state_path = directory.join(EXPORT_STATE_FILE);
    let state_bytes = match fs::read(&state_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if expected.manifest_checksum.is_some() {
                return Err(ProjectionWriteError::plain(
                    ProjectionWriteErrorKind::ExistingProjectionConflict,
                    state_path,
                ));
            }
            if !expected.allow_incomplete_recovery && managed_projection_file_exists(directory) {
                return Err(ProjectionWriteError::plain(
                    ProjectionWriteErrorKind::ExistingProjectionConflict,
                    directory,
                ));
            }
            return Ok(false);
        }
        Err(source) => {
            return Err(ProjectionWriteError::io(
                ProjectionWriteErrorKind::ExistingProjectionCorrupt,
                state_path,
                source,
            ));
        }
    };
    reject_symlink(&state_path)?;
    let state: ExportState = serde_json::from_slice(&state_bytes).map_err(|_| {
        ProjectionWriteError::plain(
            ProjectionWriteErrorKind::ExistingProjectionCorrupt,
            &state_path,
        )
    })?;
    let actual_checksum = existing_group_checksum(directory)?;
    if actual_checksum != state.manifest_checksum {
        return Err(ProjectionWriteError::plain(
            ProjectionWriteErrorKind::ExistingProjectionConflict,
            directory,
        ));
    }
    if state.database_export_revision == incoming.database_export_revision
        && state.manifest_checksum == incoming.manifest_checksum
    {
        return Ok(true);
    }
    if state.database_export_revision != expected.exported_revision
        || expected.manifest_checksum.as_deref() != Some(state.manifest_checksum.as_str())
    {
        return Err(ProjectionWriteError::plain(
            ProjectionWriteErrorKind::ExistingProjectionConflict,
            state_path,
        ));
    }
    Ok(false)
}

fn managed_projection_file_exists(directory: &Path) -> bool {
    [WORKSPACE_FILE, BASELINE_FILE, CONFIRMATION_FILE]
        .iter()
        .any(|name| directory.join(name).exists())
}

fn existing_group_checksum(directory: &Path) -> Result<String, ProjectionWriteError> {
    let paths = [
        directory.join(WORKSPACE_FILE),
        directory.join(BASELINE_FILE),
        directory.join(CONFIRMATION_FILE),
    ];
    let mut documents = Vec::with_capacity(paths.len());
    for path in &paths {
        reject_symlink(path)?;
        documents.push(fs::read(path).map_err(|source| {
            ProjectionWriteError::io(
                ProjectionWriteErrorKind::ExistingProjectionCorrupt,
                path,
                source,
            )
        })?);
    }
    Ok(checksum_group([
        documents[0].as_slice(),
        documents[1].as_slice(),
        documents[2].as_slice(),
    ]))
}

fn reject_symlink(path: &Path) -> Result<(), ProjectionWriteError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        ProjectionWriteError::io(
            ProjectionWriteErrorKind::ExistingProjectionCorrupt,
            path,
            source,
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ProjectionWriteError::plain(
            ProjectionWriteErrorKind::ExistingProjectionConflict,
            path,
        ));
    }
    Ok(())
}

fn atomic_write(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), ProjectionWriteError> {
    let target = directory.join(name);
    if let Ok(metadata) = fs::symlink_metadata(&target)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(ProjectionWriteError::plain(
            ProjectionWriteErrorKind::ReservedPathConflict,
            target,
        ));
    }
    let temporary = directory.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = write_and_replace(&temporary, &target, bytes);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_and_replace(
    temporary: &Path,
    target: &Path,
    bytes: &[u8],
) -> Result<(), ProjectionWriteError> {
    let mut file = contexthub_platform_adapter::security::create_new_restricted_file(temporary)
        .map_err(|source| {
            ProjectionWriteError::io(ProjectionWriteErrorKind::WriteFailed, temporary, source)
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| {
            ProjectionWriteError::io(ProjectionWriteErrorKind::WriteFailed, temporary, source)
        })?;
    atomic_replace(temporary, target)
}

// `std::fs::rename` 在两端都以覆盖既有目标的方式原子替换：Unix 端为 POSIX `rename(2)`
// 语义；Windows 端 Rust 标准库使用 `MoveFileExW` 并携带 `MOVEFILE_REPLACE_EXISTING`，
// 无需平台分支。`ProjectionWriteErrorKind::AtomicReplaceUnsupported` 保留为错误类型的
// 历史成员以避免破坏既有匹配，但不再被本函数触发。
fn atomic_replace(temporary: &Path, target: &Path) -> Result<(), ProjectionWriteError> {
    fs::rename(temporary, target).map_err(|source| {
        ProjectionWriteError::io(ProjectionWriteErrorKind::WriteFailed, target, source)
    })
}

fn sync_directory(path: &Path) -> Result<(), ProjectionWriteError> {
    contexthub_platform_adapter::filesystem::sync_directory(path).map_err(|source| {
        ProjectionWriteError::io(ProjectionWriteErrorKind::WriteFailed, path, source)
    })
}

fn set_private_directory_permissions(path: &Path) -> Result<(), ProjectionWriteError> {
    contexthub_platform_adapter::security::restrict_existing_path(path).map_err(|source| {
        ProjectionWriteError::io(ProjectionWriteErrorKind::WriteFailed, path, source)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use contexthub_manifest::{
        BaselineManifest, ConfirmationManifest, MANIFEST_SCHEMA_VERSION, ManifestSet,
        WorkspaceManifest,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn writes_export_state_last_and_repeats_identical_export_idempotently() {
        let root = test_root("idempotent");
        let encoded = encoded(1, 10);
        let expected = ExpectedProjectionState {
            exported_revision: 0,
            manifest_checksum: None,
            allow_incomplete_recovery: false,
        };
        assert_eq!(
            write_projection(&root, &encoded, &expected).unwrap(),
            ProjectionWriteOutcome::Exported
        );
        assert_eq!(
            write_projection(&root, &encoded, &expected).unwrap(),
            ProjectionWriteOutcome::AlreadyCurrent
        );
        let state: ExportState =
            serde_json::from_slice(&fs::read(root.join(".cilai/export-state.json")).unwrap())
                .unwrap();
        assert_eq!(state.manifest_checksum, encoded.manifest_checksum);
        assert!(!root.join(LEGACY_WORKSPACE_METADATA_DIRECTORY).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_change_and_reserved_symlink_are_explicit_conflicts() {
        let root = test_root("conflict");
        let first = encoded(1, 10);
        write_projection(
            &root,
            &first,
            &ExpectedProjectionState {
                exported_revision: 0,
                manifest_checksum: None,
                allow_incomplete_recovery: false,
            },
        )
        .unwrap();
        fs::write(root.join(".cilai/workspace.json"), b"{}\n").unwrap();
        let error = write_projection(
            &root,
            &encoded(2, 11),
            &ExpectedProjectionState {
                exported_revision: 1,
                manifest_checksum: Some(first.manifest_checksum),
                allow_incomplete_recovery: false,
            },
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            ProjectionWriteErrorKind::ExistingProjectionConflict
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_managed_files_require_database_authorized_failed_export_recovery() {
        let root = test_root("incomplete");
        fs::create_dir(root.join(".cilai")).unwrap();
        fs::write(root.join(".cilai/workspace.json"), b"external\n").unwrap();
        let manifests = encoded(1, 10);
        let error = write_projection(
            &root,
            &manifests,
            &ExpectedProjectionState {
                exported_revision: 0,
                manifest_checksum: None,
                allow_incomplete_recovery: false,
            },
        )
        .unwrap_err();
        assert_eq!(
            error.kind(),
            ProjectionWriteErrorKind::ExistingProjectionConflict
        );
        assert_eq!(
            write_projection(
                &root,
                &manifests,
                &ExpectedProjectionState {
                    exported_revision: 0,
                    manifest_checksum: None,
                    allow_incomplete_recovery: true,
                },
            )
            .unwrap(),
            ProjectionWriteOutcome::Exported
        );
        fs::remove_dir_all(root).unwrap();
    }

    pub(super) fn encoded(revision: i64, generated_at: i64) -> EncodedManifestSet {
        ManifestSet {
            workspace: WorkspaceManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                database_export_revision: revision,
                workspace_id: "w".to_owned(),
                project_id: None,
                platform_root: None,
                local_binding_id: "w".to_owned(),
                environment: "local".to_owned(),
                created_at: 1,
            },
            baseline: BaselineManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                database_export_revision: revision,
                files: Vec::new(),
            },
            confirmation: ConfirmationManifest {
                schema_version: MANIFEST_SCHEMA_VERSION,
                database_export_revision: revision,
                confirmations: Vec::new(),
            },
        }
        .encode(generated_at)
        .unwrap()
    }

    pub(super) fn test_root(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "contexthub-projection-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }
}

#[cfg(test)]
#[path = "projection_migration_tests.rs"]
mod migration_tests;
