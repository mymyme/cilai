use crate::{WorkspaceChecksumCancellationToken, join_workspace_path};
use contexthub_domain::WorkspaceRelativePath;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::Path;
use std::time::SystemTime;

const CHECKSUM_READ_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksumAlgorithm {
    Sha256,
}

impl ChecksumAlgorithm {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnsupportedChecksumAlgorithm;

impl std::fmt::Display for UnsupportedChecksumAlgorithm {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("unsupported checksum algorithm")
    }
}

impl std::error::Error for UnsupportedChecksumAlgorithm {}

impl TryFrom<&str> for ChecksumAlgorithm {
    type Error = UnsupportedChecksumAlgorithm;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "sha256" => Ok(Self::Sha256),
            _ => Err(UnsupportedChecksumAlgorithm),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChecksumValue {
    pub algorithm: ChecksumAlgorithm,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ChecksumFileObservation {
    pub size_bytes: u64,
    pub modified_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceFileChecksum {
    pub checksum: ChecksumValue,
    pub observation: ChecksumFileObservation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceChecksumProgress {
    pub bytes_read: u64,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceChecksumOutcome {
    Completed(WorkspaceFileChecksum),
    Cancelled(WorkspaceChecksumProgress),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceChecksumErrorKind {
    RootUnavailable,
    TargetUnavailable,
    OutsideWorkspace,
    NotRegularFile,
    SourceNotMaterialized,
    OpenFailed,
    MetadataUnavailable,
    ModifiedTimeUnavailable,
    ReadFailed,
    ChangedDuringRead,
}

#[derive(Debug)]
pub struct WorkspaceChecksumError {
    kind: WorkspaceChecksumErrorKind,
    source: Option<std::io::Error>,
}

impl WorkspaceChecksumError {
    #[must_use]
    pub const fn kind(&self) -> WorkspaceChecksumErrorKind {
        self.kind
    }

    fn io(kind: WorkspaceChecksumErrorKind, source: std::io::Error) -> Self {
        Self {
            kind,
            source: Some(source),
        }
    }

    fn without_source(kind: WorkspaceChecksumErrorKind) -> Self {
        Self { kind, source: None }
    }
}

impl std::fmt::Display for WorkspaceChecksumError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.kind, &self.source) {
            (WorkspaceChecksumErrorKind::RootUnavailable, Some(source)) => {
                write!(formatter, "resolve checksum root: {source}")
            }
            (WorkspaceChecksumErrorKind::TargetUnavailable, Some(source)) => {
                write!(formatter, "resolve checksum target: {source}")
            }
            (WorkspaceChecksumErrorKind::OutsideWorkspace, _) => {
                formatter.write_str("checksum target is outside the workspace")
            }
            (WorkspaceChecksumErrorKind::NotRegularFile, _) => {
                formatter.write_str("checksum target is not a regular file")
            }
            (WorkspaceChecksumErrorKind::SourceNotMaterialized, _) => {
                formatter.write_str("checksum target is not materialized locally")
            }
            (WorkspaceChecksumErrorKind::OpenFailed, Some(source)) => {
                write!(formatter, "open checksum target: {source}")
            }
            (WorkspaceChecksumErrorKind::MetadataUnavailable, Some(source)) => {
                write!(formatter, "read checksum target metadata: {source}")
            }
            (WorkspaceChecksumErrorKind::ModifiedTimeUnavailable, Some(source)) => {
                write!(formatter, "read checksum target modified time: {source}")
            }
            (WorkspaceChecksumErrorKind::ReadFailed, Some(source)) => {
                write!(formatter, "read checksum target content: {source}")
            }
            (WorkspaceChecksumErrorKind::ChangedDuringRead, _) => {
                formatter.write_str("checksum target changed while it was being read")
            }
            _ => formatter.write_str("workspace checksum failed"),
        }
    }
}

impl std::error::Error for WorkspaceChecksumError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksumUseCase {
    RoutineScan,
    Confirmation,
    Submission,
    ConflictResolution,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChecksumCacheFingerprint {
    pub size_bytes: u64,
    pub modified_at: SystemTime,
    pub file_identity: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChecksumCacheRecord {
    pub fingerprint: ChecksumCacheFingerprint,
    pub checksum: ChecksumValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksumRecomputeReason {
    StrongVerificationRequired,
    CacheMiss,
    StableFileIdentityUnavailable,
    ObservationChanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChecksumCacheDecision {
    Reuse,
    Recompute(ChecksumRecomputeReason),
}

/// 判断普通扫描能否复用缓存。确认、提交和冲突解决始终要求重新读取内容。
#[must_use]
pub fn checksum_cache_decision(
    use_case: ChecksumUseCase,
    current: &ChecksumCacheFingerprint,
    cached: Option<&ChecksumCacheRecord>,
) -> ChecksumCacheDecision {
    if use_case != ChecksumUseCase::RoutineScan {
        return ChecksumCacheDecision::Recompute(
            ChecksumRecomputeReason::StrongVerificationRequired,
        );
    }
    let Some(cached) = cached else {
        return ChecksumCacheDecision::Recompute(ChecksumRecomputeReason::CacheMiss);
    };
    let (Some(current_identity), Some(cached_identity)) = (
        current.file_identity.as_deref(),
        cached.fingerprint.file_identity.as_deref(),
    ) else {
        return ChecksumCacheDecision::Recompute(
            ChecksumRecomputeReason::StableFileIdentityUnavailable,
        );
    };
    if current_identity != cached_identity
        || current.size_bytes != cached.fingerprint.size_bytes
        || current.modified_at != cached.fingerprint.modified_at
    {
        return ChecksumCacheDecision::Recompute(ChecksumRecomputeReason::ObservationChanged);
    }
    ChecksumCacheDecision::Reuse
}

/// 对工作区中的普通文件流式计算 SHA-256，并在计算期间报告字节进度。
///
/// 路径必须是已经验证的业务相对路径。符号链接和非普通文件不会被读取；解析后的
/// 目标必须仍位于工作区内。计算前后会比较文件描述符与当前路径元数据，发现大小或
/// 修改时间变化时拒绝返回摘要。
///
/// # Errors
///
/// 根或目标不可访问、目标越界或不是普通文件、读取失败、修改时间不可用，或文件在
/// 读取期间变化时返回错误。
pub fn checksum_workspace_file(
    root: &Path,
    path: &WorkspaceRelativePath,
    cancellation: &WorkspaceChecksumCancellationToken,
    mut on_progress: impl FnMut(WorkspaceChecksumProgress),
) -> Result<WorkspaceChecksumOutcome, WorkspaceChecksumError> {
    let canonical_root = std::fs::canonicalize(root).map_err(|error| {
        WorkspaceChecksumError::io(WorkspaceChecksumErrorKind::RootUnavailable, error)
    })?;
    let unresolved_target = join_workspace_path(&canonical_root, path);
    let unresolved_metadata = std::fs::symlink_metadata(&unresolved_target).map_err(|error| {
        WorkspaceChecksumError::io(WorkspaceChecksumErrorKind::TargetUnavailable, error)
    })?;
    if !unresolved_metadata.file_type().is_file() {
        return Err(WorkspaceChecksumError::without_source(
            WorkspaceChecksumErrorKind::NotRegularFile,
        ));
    }
    if contexthub_platform_adapter::filesystem::is_cloud_placeholder_file(&unresolved_metadata) {
        return Err(WorkspaceChecksumError::without_source(
            WorkspaceChecksumErrorKind::SourceNotMaterialized,
        ));
    }
    let canonical_target = std::fs::canonicalize(&unresolved_target).map_err(|error| {
        WorkspaceChecksumError::io(WorkspaceChecksumErrorKind::TargetUnavailable, error)
    })?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(WorkspaceChecksumError::without_source(
            WorkspaceChecksumErrorKind::OutsideWorkspace,
        ));
    }
    checksum_open_file(&canonical_target, cancellation, &mut on_progress)
}

fn checksum_open_file(
    canonical_target: &Path,
    cancellation: &WorkspaceChecksumCancellationToken,
    on_progress: &mut impl FnMut(WorkspaceChecksumProgress),
) -> Result<WorkspaceChecksumOutcome, WorkspaceChecksumError> {
    let mut file =
        contexthub_platform_adapter::filesystem::open_workspace_file_for_read(canonical_target)
            .map_err(|error| {
                WorkspaceChecksumError::io(WorkspaceChecksumErrorKind::OpenFailed, error)
            })?;
    let before =
        contexthub_platform_adapter::filesystem::opened_file_metadata_with_stable_identity(&file)
            .map_err(|error| {
            WorkspaceChecksumError::io(WorkspaceChecksumErrorKind::MetadataUnavailable, error)
        })?;
    if !before.metadata.is_file() {
        return Err(WorkspaceChecksumError::without_source(
            WorkspaceChecksumErrorKind::NotRegularFile,
        ));
    }
    let before_modified = modified_time(&before.metadata)?;
    let mut progress = WorkspaceChecksumProgress {
        bytes_read: 0,
        total_bytes: before.metadata.len(),
    };
    on_progress(progress);
    if cancellation.is_cancelled() {
        return Ok(WorkspaceChecksumOutcome::Cancelled(progress));
    }

    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; CHECKSUM_READ_BUFFER_SIZE].into_boxed_slice();
    loop {
        if cancellation.is_cancelled() {
            return Ok(WorkspaceChecksumOutcome::Cancelled(progress));
        }
        let read = file.read(&mut buffer).map_err(|error| {
            WorkspaceChecksumError::io(WorkspaceChecksumErrorKind::ReadFailed, error)
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        progress.bytes_read += read as u64;
        on_progress(progress);
    }

    verify_unchanged(&file, canonical_target, &before, before_modified, progress)?;
    let digest = hasher.finalize();
    let mut value = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(WorkspaceChecksumOutcome::Completed(WorkspaceFileChecksum {
        checksum: ChecksumValue {
            algorithm: ChecksumAlgorithm::Sha256,
            value,
        },
        observation: ChecksumFileObservation {
            size_bytes: before.metadata.len(),
            modified_at: before_modified,
        },
    }))
}

fn modified_time(metadata: &std::fs::Metadata) -> Result<SystemTime, WorkspaceChecksumError> {
    metadata.modified().map_err(|error| {
        WorkspaceChecksumError::io(WorkspaceChecksumErrorKind::ModifiedTimeUnavailable, error)
    })
}

fn verify_unchanged(
    file: &std::fs::File,
    canonical_target: &Path,
    before: &contexthub_platform_adapter::filesystem::FileMetadataSnapshot,
    before_modified: SystemTime,
    progress: WorkspaceChecksumProgress,
) -> Result<(), WorkspaceChecksumError> {
    let descriptor_after =
        contexthub_platform_adapter::filesystem::opened_file_metadata_with_stable_identity(file)
            .map_err(|error| {
                WorkspaceChecksumError::io(WorkspaceChecksumErrorKind::MetadataUnavailable, error)
            })?;
    let path_after =
        contexthub_platform_adapter::filesystem::metadata_with_stable_identity(canonical_target)
            .map_err(|error| {
                WorkspaceChecksumError::io(WorkspaceChecksumErrorKind::ChangedDuringRead, error)
            })?;
    let descriptor_modified = modified_time(&descriptor_after.metadata)?;
    let path_modified = modified_time(&path_after.metadata)?;
    if progress.bytes_read != before.metadata.len()
        || descriptor_after.metadata.len() != before.metadata.len()
        || path_after.metadata.len() != before.metadata.len()
        || descriptor_modified != before_modified
        || path_modified != before_modified
        || descriptor_after.stable_identity != before.stable_identity
        || path_after.stable_identity != before.stable_identity
    {
        return Err(WorkspaceChecksumError::without_source(
            WorkspaceChecksumErrorKind::ChangedDuringRead,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn sha256_uses_known_vector_and_reports_streamed_progress() {
        let fixture = TestWorkspace::new("known-vector");
        fs::write(fixture.root.join("abc.txt"), "abc").unwrap();
        let mut progress = Vec::new();
        let outcome = checksum_workspace_file(
            &fixture.root,
            &WorkspaceRelativePath::parse("abc.txt").unwrap(),
            &WorkspaceChecksumCancellationToken::new(),
            |value| progress.push(value),
        )
        .unwrap();
        let WorkspaceChecksumOutcome::Completed(result) = outcome else {
            panic!("checksum unexpectedly cancelled");
        };
        assert_eq!(result.checksum.algorithm.as_str(), "sha256");
        assert_eq!(
            result.checksum.value,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(result.observation.size_bytes, 3);
        assert_eq!(progress.first().unwrap().bytes_read, 0);
        assert_eq!(progress.last().unwrap().bytes_read, 3);
        assert!(progress.windows(2).all(|window| {
            window[0].bytes_read <= window[1].bytes_read
                && window[0].total_bytes == window[1].total_bytes
        }));
    }

    #[test]
    fn large_file_is_cancelled_at_a_chunk_boundary_without_partial_digest() {
        let fixture = TestWorkspace::new("cancel-large");
        fs::write(
            fixture.root.join("large.bin"),
            vec![7_u8; CHECKSUM_READ_BUFFER_SIZE * 3],
        )
        .unwrap();
        let cancellation = WorkspaceChecksumCancellationToken::new();
        let callback_token = cancellation.clone();
        let outcome = checksum_workspace_file(
            &fixture.root,
            &WorkspaceRelativePath::parse("large.bin").unwrap(),
            &cancellation,
            |progress| {
                if progress.bytes_read >= CHECKSUM_READ_BUFFER_SIZE as u64 {
                    callback_token.cancel();
                }
            },
        )
        .unwrap();
        assert_eq!(
            outcome,
            WorkspaceChecksumOutcome::Cancelled(WorkspaceChecksumProgress {
                bytes_read: CHECKSUM_READ_BUFFER_SIZE as u64,
                total_bytes: (CHECKSUM_READ_BUFFER_SIZE * 3) as u64,
            })
        );
    }

    #[test]
    fn content_change_during_read_is_rejected() {
        let fixture = TestWorkspace::new("changed-during-read");
        let file = fixture.root.join("changing.bin");
        fs::write(&file, vec![1_u8; CHECKSUM_READ_BUFFER_SIZE * 2]).unwrap();
        let mut changed = false;
        let error = checksum_workspace_file(
            &fixture.root,
            &WorkspaceRelativePath::parse("changing.bin").unwrap(),
            &WorkspaceChecksumCancellationToken::new(),
            |progress| {
                if progress.bytes_read > 0 && !changed {
                    let mut content = fs::read(&file).unwrap();
                    content.push(2);
                    fs::write(&file, content).unwrap();
                    changed = true;
                }
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), WorkspaceChecksumErrorKind::ChangedDuringRead);
    }

    #[test]
    fn cache_requires_identity_and_strong_operations_always_recompute() {
        let modified_at = UNIX_EPOCH + Duration::from_secs(10);
        let fingerprint = ChecksumCacheFingerprint {
            size_bytes: 3,
            modified_at,
            file_identity: Some("device:1:file:2".to_owned()),
        };
        let cached = ChecksumCacheRecord {
            fingerprint: fingerprint.clone(),
            checksum: ChecksumValue {
                algorithm: ChecksumAlgorithm::Sha256,
                value: "abc".to_owned(),
            },
        };
        assert_eq!(
            checksum_cache_decision(ChecksumUseCase::RoutineScan, &fingerprint, Some(&cached)),
            ChecksumCacheDecision::Reuse
        );
        for use_case in [
            ChecksumUseCase::Confirmation,
            ChecksumUseCase::Submission,
            ChecksumUseCase::ConflictResolution,
        ] {
            assert_eq!(
                checksum_cache_decision(use_case, &fingerprint, Some(&cached)),
                ChecksumCacheDecision::Recompute(
                    ChecksumRecomputeReason::StrongVerificationRequired
                )
            );
        }
        let without_identity = ChecksumCacheFingerprint {
            file_identity: None,
            ..fingerprint
        };
        assert_eq!(
            checksum_cache_decision(
                ChecksumUseCase::RoutineScan,
                &without_identity,
                Some(&cached)
            ),
            ChecksumCacheDecision::Recompute(
                ChecksumRecomputeReason::StableFileIdentityUnavailable
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_not_hashed_even_when_target_is_inside_workspace() {
        use std::os::unix::fs::symlink;

        let fixture = TestWorkspace::new("checksum-symlink");
        fs::write(fixture.root.join("target.txt"), "target").unwrap();
        symlink("target.txt", fixture.root.join("link.txt")).unwrap();
        let error = checksum_workspace_file(
            &fixture.root,
            &WorkspaceRelativePath::parse("link.txt").unwrap(),
            &WorkspaceChecksumCancellationToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.kind(), WorkspaceChecksumErrorKind::NotRegularFile);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_file_returns_explicit_open_failed_error() {
        use std::os::unix::fs::PermissionsExt as _;

        let fixture = TestWorkspace::new("checksum-unreadable");
        let file = fixture.root.join("unreadable.txt");
        fs::write(&file, "unreadable").unwrap();
        let original_mode = fs::metadata(&file).unwrap().permissions().mode();
        let _permission_guard = PermissionGuard::new(file.clone(), original_mode);
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();

        let error = checksum_workspace_file(
            &fixture.root,
            &WorkspaceRelativePath::parse("unreadable.txt").unwrap(),
            &WorkspaceChecksumCancellationToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.kind(), WorkspaceChecksumErrorKind::OpenFailed);
    }

    struct TestWorkspace {
        parent: PathBuf,
        root: PathBuf,
    }

    impl TestWorkspace {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let parent = PathBuf::from("/tmp").join(format!(
                "ch-workspace-checksum-{label}-{}-{nanos}",
                std::process::id()
            ));
            let root = parent.join("workspace");
            fs::create_dir_all(&root).unwrap();
            Self { parent, root }
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.parent).unwrap();
        }
    }

    #[cfg(unix)]
    struct PermissionGuard {
        path: PathBuf,
        mode: u32,
    }

    #[cfg(unix)]
    impl PermissionGuard {
        fn new(path: PathBuf, mode: u32) -> Self {
            Self { path, mode }
        }
    }

    #[cfg(unix)]
    impl Drop for PermissionGuard {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode)).unwrap();
        }
    }
}
