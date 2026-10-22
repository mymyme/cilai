//! 逐层浏览与单条目检查。
//!
//! 新连的大文件夹要等全量索引建完才能看，添加动作就会卡很久。逐层浏览只读当前展开的
//! 那一层目录、不读文件内容，让用户在索引建立期间就能进目录看；单条目检查让用户点开的
//! 那个文件不必排在整个索引后面。两者的判定规则都与全量扫描一致，避免浏览看到的和
//! 索引收进来的不是同一批东西。

use crate::{
    WorkspaceEntryKind, WorkspaceScanEntry, WorkspaceScanEvent, WorkspaceScanIssue,
    WorkspaceScanOutcome, entry_kind, fixed_ignore_match, join_workspace_path,
    resolve_existing_workspace_path, scan_directory_level,
};
use contexthub_domain::WorkspaceRelativePath;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceBrowseErrorKind {
    /// 工作区根目录不存在或无法解析。
    RootUnavailable,
    /// 目标不存在或无法读取元数据。
    TargetUnavailable,
    /// 目标解析后落在工作区之外。
    OutsideWorkspace,
    /// 要展开的目标不是目录（含指向目录的符号链接，浏览不跟随符号链接）。
    NotDirectory,
    /// 目标位于固定忽略目录内，不属于工作区内容。
    Ignored,
    /// 目标类型不受支持（设备文件、套接字等）。
    UnsupportedFileType,
}

#[derive(Debug)]
pub struct WorkspaceBrowseError {
    kind: WorkspaceBrowseErrorKind,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl WorkspaceBrowseError {
    #[must_use]
    pub fn is_missing(&self) -> bool {
        self.source
            .as_ref()
            .and_then(|error| error.downcast_ref::<std::io::Error>())
            .is_some_and(|error| {
                matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                )
            })
    }

    #[must_use]
    pub const fn kind(&self) -> WorkspaceBrowseErrorKind {
        self.kind
    }

    const fn new(kind: WorkspaceBrowseErrorKind) -> Self {
        Self { kind, source: None }
    }

    fn io(kind: WorkspaceBrowseErrorKind, source: std::io::Error) -> Self {
        Self::caused_by(kind, source)
    }

    fn caused_by(
        kind: WorkspaceBrowseErrorKind,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            source: Some(Box::new(source)),
        }
    }
}

impl std::fmt::Display for WorkspaceBrowseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self.kind {
            WorkspaceBrowseErrorKind::RootUnavailable => "workspace root unavailable",
            WorkspaceBrowseErrorKind::TargetUnavailable => "workspace target unavailable",
            WorkspaceBrowseErrorKind::OutsideWorkspace => "target is outside the workspace",
            WorkspaceBrowseErrorKind::NotDirectory => "target is not a directory",
            WorkspaceBrowseErrorKind::Ignored => "target is inside an ignored directory",
            WorkspaceBrowseErrorKind::UnsupportedFileType => "target file type is unsupported",
        };
        match &self.source {
            Some(source) => write!(formatter, "{label}: {source}"),
            None => formatter.write_str(label),
        }
    }
}

impl std::error::Error for WorkspaceBrowseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn std::error::Error + 'static))
    }
}

/// 一层目录的读取结果。条目按名称排序；读不到的单个条目进 `issues`，不影响其他条目。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceDirectoryListing {
    pub entries: Vec<WorkspaceScanEntry>,
    pub issues: Vec<WorkspaceScanIssue>,
}

/// 读出工作区内一个目录的直接子项。`directory` 为 `None` 表示工作区根目录。
///
/// # Errors
///
/// 根目录或目标不可用、目标越出工作区、不是目录或位于固定忽略目录内时返回错误。
pub fn list_workspace_directory(
    root: &Path,
    directory: Option<&WorkspaceRelativePath>,
) -> Result<WorkspaceDirectoryListing, WorkspaceBrowseError> {
    let canonical_root = std::fs::canonicalize(root).map_err(|error| {
        WorkspaceBrowseError::io(WorkspaceBrowseErrorKind::RootUnavailable, error)
    })?;
    let (absolute_directory, relative_directory) = match directory {
        None => (canonical_root, PathBuf::new()),
        Some(directory) => {
            reject_ignored(directory)?;
            // 浏览与扫描一样不跟随符号链接：指向目录的链接在树里是一个不可展开的条目。
            let metadata =
                std::fs::symlink_metadata(join_workspace_path(&canonical_root, directory))
                    .map_err(|error| {
                        WorkspaceBrowseError::io(WorkspaceBrowseErrorKind::TargetUnavailable, error)
                    })?;
            if !metadata.is_dir() {
                return Err(WorkspaceBrowseError::new(
                    WorkspaceBrowseErrorKind::NotDirectory,
                ));
            }
            let absolute = resolve_existing_workspace_path(&canonical_root, directory)
                .map_err(map_resolve_error)?;
            let relative = directory.segments().collect::<PathBuf>();
            (absolute, relative)
        }
    };
    let mut listing = WorkspaceDirectoryListing::default();
    let outcome = scan_directory_level(
        absolute_directory,
        relative_directory,
        |event| match event {
            WorkspaceScanEvent::Entry(entry) => listing.entries.push(entry),
            WorkspaceScanEvent::Issue(issue) => listing.issues.push(issue),
            WorkspaceScanEvent::Progress(_) => {}
        },
    )
    .map_err(|error| {
        WorkspaceBrowseError::caused_by(WorkspaceBrowseErrorKind::TargetUnavailable, error)
    })?;
    match outcome {
        WorkspaceScanOutcome::Completed(_) => Ok(listing),
        // 这里用的取消令牌从不触发；真出现就说明扫描器行为变了，不能当作读完了。
        WorkspaceScanOutcome::Cancelled(_) => Err(WorkspaceBrowseError::io(
            WorkspaceBrowseErrorKind::TargetUnavailable,
            std::io::Error::other("directory listing was cancelled"),
        )),
    }
}

/// 检查工作区内单个条目，产出与全量扫描同口径的扫描条目。不跟随末端符号链接。
///
/// # Errors
///
/// 根目录或目标不可用、目标越出工作区、位于固定忽略目录内或类型不受支持时返回错误。
pub fn inspect_workspace_entry(
    root: &Path,
    path: &WorkspaceRelativePath,
) -> Result<WorkspaceScanEntry, WorkspaceBrowseError> {
    reject_ignored(path)?;
    let canonical_root = std::fs::canonicalize(root).map_err(|error| {
        WorkspaceBrowseError::io(WorkspaceBrowseErrorKind::RootUnavailable, error)
    })?;
    let segments = path.segments().collect::<Vec<_>>();
    let (name, parents) = segments
        .split_last()
        .ok_or_else(|| WorkspaceBrowseError::new(WorkspaceBrowseErrorKind::TargetUnavailable))?;
    // 父目录要解析到真实位置并确认仍在工作区内；末端本身不跟随，符号链接按链接记录。
    let parent = if parents.is_empty() {
        canonical_root
    } else {
        let parent_path = WorkspaceRelativePath::parse(&parents.join("/"))
            .map_err(|_| WorkspaceBrowseError::new(WorkspaceBrowseErrorKind::TargetUnavailable))?;
        resolve_existing_workspace_path(&canonical_root, &parent_path).map_err(map_resolve_error)?
    };
    let absolute = parent.join(name);
    let snapshot =
        contexthub_platform_adapter::filesystem::symlink_metadata_with_stable_identity(&absolute)
            .map_err(|error| {
            WorkspaceBrowseError::io(WorkspaceBrowseErrorKind::TargetUnavailable, error)
        })?;
    let metadata = &snapshot.metadata;
    let kind = entry_kind(metadata.file_type())
        .ok_or_else(|| WorkspaceBrowseError::new(WorkspaceBrowseErrorKind::UnsupportedFileType))?;
    let modified_at = metadata.modified().map_err(|error| {
        WorkspaceBrowseError::io(WorkspaceBrowseErrorKind::TargetUnavailable, error)
    })?;
    Ok(WorkspaceScanEntry {
        path: path.clone(),
        kind,
        file_identity: Some(snapshot.stable_identity),
        byte_len: (kind == WorkspaceEntryKind::File).then_some(metadata.len()),
        modified_at: Some(modified_at),
    })
}

fn reject_ignored(path: &WorkspaceRelativePath) -> Result<(), WorkspaceBrowseError> {
    if fixed_ignore_match(path).is_some() {
        return Err(WorkspaceBrowseError::new(WorkspaceBrowseErrorKind::Ignored));
    }
    Ok(())
}

fn map_resolve_error(error: crate::WorkspaceResolveError) -> WorkspaceBrowseError {
    let kind = match error.kind() {
        crate::WorkspaceResolveErrorKind::RootUnavailable => {
            WorkspaceBrowseErrorKind::RootUnavailable
        }
        crate::WorkspaceResolveErrorKind::TargetUnavailable => {
            WorkspaceBrowseErrorKind::TargetUnavailable
        }
        crate::WorkspaceResolveErrorKind::OutsideWorkspace => {
            WorkspaceBrowseErrorKind::OutsideWorkspace
        }
    };
    WorkspaceBrowseError::caused_by(kind, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WorkspaceCancellationToken, scan_workspace};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let sequence = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
            let process = std::process::id();
            let path = std::env::temp_dir()
                .join(format!("cilai-browse-{label}-{process}-{nanos}-{sequence}"));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn path(value: &str) -> WorkspaceRelativePath {
        WorkspaceRelativePath::parse(value).unwrap()
    }

    fn names(listing: &WorkspaceDirectoryListing) -> Vec<&str> {
        listing
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect()
    }

    fn fixture() -> TempRoot {
        let root = TempRoot::new("tree");
        fs::create_dir_all(root.0.join("docs/deep/deeper")).unwrap();
        fs::create_dir_all(root.0.join(".git/objects")).unwrap();
        fs::create_dir_all(root.0.join(".cilai/cloud-pull")).unwrap();
        fs::create_dir_all(root.0.join(".contexthub")).unwrap();
        fs::create_dir_all(root.0.join("node_modules/pkg")).unwrap();
        fs::write(root.0.join("readme.md"), b"hello").unwrap();
        fs::write(root.0.join("docs/a.txt"), b"a").unwrap();
        fs::write(root.0.join("docs/deep/b.txt"), b"bb").unwrap();
        fs::write(root.0.join("docs/deep/deeper/c.txt"), b"ccc").unwrap();
        root
    }

    #[test]
    fn 根目录只读一层且跳过固定忽略目录() {
        let root = fixture();
        let listing = list_workspace_directory(&root.0, None).unwrap();
        assert_eq!(names(&listing), vec!["docs", "readme.md"]);
        assert!(listing.issues.is_empty());
    }

    #[test]
    fn 子目录的条目带完整相对路径且不进入更深一层() {
        let root = fixture();
        let listing = list_workspace_directory(&root.0, Some(&path("docs/deep"))).unwrap();
        assert_eq!(names(&listing), vec!["docs/deep/b.txt", "docs/deep/deeper"]);
        let file = &listing.entries[0];
        assert_eq!(file.kind, WorkspaceEntryKind::File);
        assert_eq!(file.byte_len, Some(2));
    }

    #[test]
    fn 逐层读到的条目与全量扫描口径一致() {
        let root = fixture();
        let mut full = Vec::new();
        scan_workspace(&root.0, &WorkspaceCancellationToken::new(), |event| {
            if let WorkspaceScanEvent::Entry(entry) = event {
                full.push(entry);
            }
        })
        .unwrap();
        let mut layered = Vec::new();
        let mut pending = vec![None::<WorkspaceRelativePath>];
        while let Some(directory) = pending.pop() {
            let listing = list_workspace_directory(&root.0, directory.as_ref()).unwrap();
            for entry in listing.entries {
                if entry.kind == WorkspaceEntryKind::Directory {
                    pending.push(Some(entry.path.clone()));
                }
                layered.push(entry);
            }
        }
        full.sort_by(|left, right| left.path.cmp(&right.path));
        layered.sort_by(|left, right| left.path.cmp(&right.path));
        assert_eq!(full, layered);
    }

    #[test]
    fn 不能展开文件和忽略目录() {
        let root = fixture();
        let not_directory =
            list_workspace_directory(&root.0, Some(&path("readme.md"))).unwrap_err();
        assert_eq!(not_directory.kind(), WorkspaceBrowseErrorKind::NotDirectory);
        let ignored = list_workspace_directory(&root.0, Some(&path(".git"))).unwrap_err();
        assert_eq!(ignored.kind(), WorkspaceBrowseErrorKind::Ignored);
        let missing = list_workspace_directory(&root.0, Some(&path("nope"))).unwrap_err();
        assert_eq!(missing.kind(), WorkspaceBrowseErrorKind::TargetUnavailable);
    }

    #[test]
    fn directory_read_errors_keep_the_scan_and_os_sources() {
        let root = fixture();
        let file = root.0.join("readme.md");
        let expected = fs::read_dir(&file).unwrap_err().raw_os_error();
        let error = list_workspace_directory(&file, None).unwrap_err();
        assert_eq!(error.kind(), WorkspaceBrowseErrorKind::TargetUnavailable);
        let scan = std::error::Error::source(&error)
            .unwrap()
            .downcast_ref::<crate::WorkspaceScanError>()
            .unwrap();
        assert_eq!(scan.kind(), crate::WorkspaceScanErrorKind::RootUnreadable);
        let io = std::error::Error::source(scan)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert!(expected.is_some());
        assert_eq!(io.raw_os_error(), expected);
    }

    #[test]
    fn entry_resolution_errors_keep_the_resolver_and_os_sources() {
        let root = fixture();
        let expected = fs::canonicalize(root.0.join("missing-parent"))
            .unwrap_err()
            .raw_os_error();
        let error =
            inspect_workspace_entry(&root.0, &path("missing-parent/child.txt")).unwrap_err();
        assert_eq!(error.kind(), WorkspaceBrowseErrorKind::TargetUnavailable);
        assert!(!error.is_missing());
        let resolver = std::error::Error::source(&error)
            .unwrap()
            .downcast_ref::<crate::WorkspaceResolveError>()
            .unwrap();
        assert_eq!(
            resolver.kind(),
            crate::WorkspaceResolveErrorKind::TargetUnavailable
        );
        let io = std::error::Error::source(resolver)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert!(expected.is_some());
        assert_eq!(io.raw_os_error(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn 指向外部的符号链接目录不能展开() {
        let root = fixture();
        let outside = TempRoot::new("outside");
        std::os::unix::fs::symlink(&outside.0, root.0.join("escape")).unwrap();
        let error = list_workspace_directory(&root.0, Some(&path("escape"))).unwrap_err();
        assert_eq!(error.kind(), WorkspaceBrowseErrorKind::NotDirectory);
    }

    #[test]
    fn 单条目检查与全量扫描产出同一条记录() {
        let root = fixture();
        let mut full = Vec::new();
        scan_workspace(&root.0, &WorkspaceCancellationToken::new(), |event| {
            if let WorkspaceScanEvent::Entry(entry) = event {
                full.push(entry);
            }
        })
        .unwrap();
        let expected = full
            .into_iter()
            .find(|entry| entry.path.as_str() == "docs/deep/b.txt")
            .unwrap();
        assert_eq!(
            inspect_workspace_entry(&root.0, &path("docs/deep/b.txt")).unwrap(),
            expected
        );
    }

    #[test]
    fn 单条目检查拒绝忽略目录和不存在的目标() {
        let root = fixture();
        let ignored = inspect_workspace_entry(&root.0, &path("node_modules/pkg")).unwrap_err();
        assert_eq!(ignored.kind(), WorkspaceBrowseErrorKind::Ignored);
        let missing = inspect_workspace_entry(&root.0, &path("docs/none.txt")).unwrap_err();
        assert_eq!(missing.kind(), WorkspaceBrowseErrorKind::TargetUnavailable);
    }

    #[cfg(unix)]
    #[test]
    fn 单条目检查不经由越界的父目录() {
        let root = fixture();
        let outside = TempRoot::new("outside-parent");
        fs::write(outside.0.join("secret.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside.0, root.0.join("link")).unwrap();
        let error = inspect_workspace_entry(&root.0, &path("link/secret.txt")).unwrap_err();
        assert_eq!(error.kind(), WorkspaceBrowseErrorKind::OutsideWorkspace);
    }
}
