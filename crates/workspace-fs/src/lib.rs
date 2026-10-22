#![doc = "工作区扫描、监听、摘要和便携投影适配器。"]

mod browse;
mod checksum;
mod compensation;
mod projection;
mod text_resource;
mod watcher;

use contexthub_domain::WorkspaceRelativePath;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::SystemTime;

pub use browse::*;
pub use checksum::*;
pub use compensation::*;
pub use projection::*;
pub use text_resource::*;
pub use watcher::*;

pub const WORKSPACE_METADATA_DIRECTORY: &str = ".cilai";
pub const LEGACY_WORKSPACE_METADATA_DIRECTORY: &str = ".contexthub";

/// 新旧工作区元数据目录都属于保留路径，旧名称仅用于迁移和访问隔离。
#[must_use]
pub fn is_workspace_metadata_directory(component: &str) -> bool {
    component.eq_ignore_ascii_case(WORKSPACE_METADATA_DIRECTORY)
        || component.eq_ignore_ascii_case(LEGACY_WORKSPACE_METADATA_DIRECTORY)
}

const REBUILDABLE_CACHE_DIRECTORIES: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    ".turbo",
    ".cache",
    "__pycache__",
    ".pytest_cache",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalWorkspaceRootErrorKind {
    RootUnavailable,
    RootNotDirectory,
    RootNonUnicode,
    ProtectedRootUnavailable,
    EnvironmentConflict,
}

#[derive(Debug)]
pub struct LocalWorkspaceRootError {
    kind: LocalWorkspaceRootErrorKind,
    conflicting_path: Option<PathBuf>,
    source: Option<std::io::Error>,
}

impl LocalWorkspaceRootError {
    #[must_use]
    pub const fn kind(&self) -> LocalWorkspaceRootErrorKind {
        self.kind
    }

    #[must_use]
    pub fn conflicting_path(&self) -> Option<&Path> {
        self.conflicting_path.as_deref()
    }

    fn io(kind: LocalWorkspaceRootErrorKind, path: &Path, source: std::io::Error) -> Self {
        Self {
            kind,
            conflicting_path: Some(path.to_owned()),
            source: Some(source),
        }
    }

    fn conflict(kind: LocalWorkspaceRootErrorKind, path: &Path) -> Self {
        Self {
            kind,
            conflicting_path: Some(path.to_owned()),
            source: None,
        }
    }
}

impl std::fmt::Display for LocalWorkspaceRootError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.kind, &self.conflicting_path, &self.source) {
            (LocalWorkspaceRootErrorKind::RootUnavailable, Some(path), Some(source)) => {
                write!(
                    formatter,
                    "resolve local workspace root {}: {source}",
                    path.display()
                )
            }
            (LocalWorkspaceRootErrorKind::RootNotDirectory, Some(path), _) => {
                write!(
                    formatter,
                    "local workspace root is not a directory: {}",
                    path.display()
                )
            }
            (LocalWorkspaceRootErrorKind::RootNonUnicode, Some(path), _) => {
                write!(
                    formatter,
                    "local workspace root is not valid Unicode: {}",
                    path.display()
                )
            }
            (LocalWorkspaceRootErrorKind::ProtectedRootUnavailable, Some(path), Some(source)) => {
                write!(
                    formatter,
                    "resolve protected application root {}: {source}",
                    path.display()
                )
            }
            (kind, Some(path), _) => write!(
                formatter,
                "local workspace root conflict {kind:?}: {}",
                path.display()
            ),
            _ => formatter.write_str("local workspace root validation failed"),
        }
    }
}

impl std::error::Error for LocalWorkspaceRootError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalWorkspaceRootInspection {
    pub canonical_root: String,
    pub local_root_identity: Option<String>,
}

/// 只读取本次所选本地目录及应用受保护边界，不访问其他工作区。
///
/// # Errors
///
/// 所选目录不可用、不能持久化为 Unicode，或涉及应用数据目录时返回错误。
pub fn inspect_local_workspace_root(
    candidate: &Path,
    protected_roots: &[PathBuf],
) -> Result<LocalWorkspaceRootInspection, LocalWorkspaceRootError> {
    let canonical_candidate = std::fs::canonicalize(candidate).map_err(|error| {
        LocalWorkspaceRootError::io(
            LocalWorkspaceRootErrorKind::RootUnavailable,
            candidate,
            error,
        )
    })?;
    let root_snapshot = contexthub_platform_adapter::filesystem::metadata_with_stable_identity(
        &canonical_candidate,
    )
    .map_err(|error| {
        LocalWorkspaceRootError::io(
            LocalWorkspaceRootErrorKind::RootUnavailable,
            candidate,
            error,
        )
    })?;
    let metadata = &root_snapshot.metadata;
    if !metadata.is_dir() {
        return Err(LocalWorkspaceRootError::conflict(
            LocalWorkspaceRootErrorKind::RootNotDirectory,
            &canonical_candidate,
        ));
    }
    let canonical_root = canonical_candidate.to_str().ok_or_else(|| {
        LocalWorkspaceRootError::conflict(
            LocalWorkspaceRootErrorKind::RootNonUnicode,
            &canonical_candidate,
        )
    })?;

    for protected in protected_roots {
        let canonical_protected = std::fs::canonicalize(protected).map_err(|error| {
            LocalWorkspaceRootError::io(
                LocalWorkspaceRootErrorKind::ProtectedRootUnavailable,
                protected,
                error,
            )
        })?;
        if canonical_candidate == canonical_protected
            || canonical_candidate.starts_with(&canonical_protected)
            || canonical_protected.starts_with(&canonical_candidate)
        {
            return Err(LocalWorkspaceRootError::conflict(
                LocalWorkspaceRootErrorKind::EnvironmentConflict,
                protected,
            ));
        }
    }

    let local_root_identity = Some(root_snapshot.stable_identity);

    Ok(LocalWorkspaceRootInspection {
        canonical_root: canonical_root.to_owned(),
        local_root_identity,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedIgnoreKind {
    ContextHubMetadata,
    SourceControlMetadata,
    RebuildableCache,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixedIgnoreMatch {
    pub kind: FixedIgnoreKind,
    pub component: String,
    pub component_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceResolveErrorKind {
    RootUnavailable,
    TargetUnavailable,
    OutsideWorkspace,
}

#[derive(Debug)]
pub struct WorkspaceResolveError {
    kind: WorkspaceResolveErrorKind,
    source: Option<std::io::Error>,
}

impl WorkspaceResolveError {
    #[must_use]
    pub const fn kind(&self) -> WorkspaceResolveErrorKind {
        self.kind
    }

    fn io(kind: WorkspaceResolveErrorKind, source: std::io::Error) -> Self {
        Self {
            kind,
            source: Some(source),
        }
    }

    fn outside_workspace() -> Self {
        Self {
            kind: WorkspaceResolveErrorKind::OutsideWorkspace,
            source: None,
        }
    }
}

impl std::fmt::Display for WorkspaceResolveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.kind, &self.source) {
            (WorkspaceResolveErrorKind::RootUnavailable, Some(source)) => {
                write!(formatter, "resolve workspace root: {source}")
            }
            (WorkspaceResolveErrorKind::TargetUnavailable, Some(source)) => {
                write!(formatter, "resolve workspace target: {source}")
            }
            (WorkspaceResolveErrorKind::OutsideWorkspace, _) => {
                formatter.write_str("resolved target is outside the workspace")
            }
            _ => formatter.write_str("workspace path resolution failed"),
        }
    }
}

impl std::error::Error for WorkspaceResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

/// 返回路径中命中的首个固定忽略目录。比较使用 ASCII 大小写不敏感语义，
/// 保证大小写不敏感文件系统上的保留目录不会因输入大小写不同而漏过。
#[must_use]
pub fn fixed_ignore_match(path: &WorkspaceRelativePath) -> Option<FixedIgnoreMatch> {
    path.segments()
        .enumerate()
        .find_map(|(component_index, component)| {
            fixed_ignore_kind(component).map(|kind| FixedIgnoreMatch {
                kind,
                component: component.to_owned(),
                component_index,
            })
        })
}

fn fixed_ignore_kind(component: &str) -> Option<FixedIgnoreKind> {
    if is_workspace_metadata_directory(component) {
        Some(FixedIgnoreKind::ContextHubMetadata)
    } else if component.eq_ignore_ascii_case(".git") {
        Some(FixedIgnoreKind::SourceControlMetadata)
    } else if REBUILDABLE_CACHE_DIRECTORIES
        .iter()
        .any(|candidate| component.eq_ignore_ascii_case(candidate))
    {
        Some(FixedIgnoreKind::RebuildableCache)
    } else {
        None
    }
}

/// 将已验证的业务相对路径机械拼接到工作区根目录；不访问文件系统。
#[must_use]
pub fn join_workspace_path(root: &Path, path: &WorkspaceRelativePath) -> PathBuf {
    path.segments()
        .fold(root.to_owned(), |joined, segment| joined.join(segment))
}

/// 解析一个已经存在的目标并确认符号链接解析后的真实位置仍位于工作区内。
///
/// # Errors
///
/// 工作区或目标无法 canonicalize，或目标解析后越出工作区时返回错误。
pub fn resolve_existing_workspace_path(
    root: &Path,
    path: &WorkspaceRelativePath,
) -> Result<PathBuf, WorkspaceResolveError> {
    let canonical_root = std::fs::canonicalize(root).map_err(|error| {
        WorkspaceResolveError::io(WorkspaceResolveErrorKind::RootUnavailable, error)
    })?;
    let target = join_workspace_path(&canonical_root, path);
    let canonical_target = std::fs::canonicalize(target).map_err(|error| {
        WorkspaceResolveError::io(WorkspaceResolveErrorKind::TargetUnavailable, error)
    })?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(WorkspaceResolveError::outside_workspace());
    }
    Ok(canonical_target)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceEntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceScanEntry {
    pub path: WorkspaceRelativePath,
    pub kind: WorkspaceEntryKind,
    pub file_identity: Option<String>,
    pub byte_len: Option<u64>,
    pub modified_at: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceScanIssueKind {
    ReadDirectory,
    ReadEntry,
    InvalidRelativePath,
    FileTypeUnavailable,
    MetadataUnavailable,
    ModifiedTimeUnavailable,
    UnsupportedFileType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceScanIssue {
    pub kind: WorkspaceScanIssueKind,
    pub native_relative_path: PathBuf,
    pub io_error_kind: Option<std::io::ErrorKind>,
    pub path_error: Option<contexthub_domain::WorkspacePathError>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceScanSummary {
    pub entries_examined: u64,
    pub entries_emitted: u64,
    pub files_emitted: u64,
    pub directories_emitted: u64,
    pub symlinks_emitted: u64,
    pub ignored_entries: u64,
    pub issues: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceScanProgress {
    pub entries_examined: u64,
    pub entries_emitted: u64,
    pub ignored_entries: u64,
    pub issues: u64,
    pub directories_pending: u64,
}

impl WorkspaceScanProgress {
    fn from_summary(summary: WorkspaceScanSummary, directories_pending: u64) -> Self {
        Self {
            entries_examined: summary.entries_examined,
            entries_emitted: summary.entries_emitted,
            ignored_entries: summary.ignored_entries,
            issues: summary.issues,
            directories_pending,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceScanEvent {
    Entry(WorkspaceScanEntry),
    Issue(WorkspaceScanIssue),
    Progress(WorkspaceScanProgress),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceScanOutcome {
    Completed(WorkspaceScanSummary),
    Cancelled(WorkspaceScanSummary),
}

impl WorkspaceScanOutcome {
    #[must_use]
    pub const fn summary(self) -> WorkspaceScanSummary {
        match self {
            Self::Completed(summary) | Self::Cancelled(summary) => summary,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WorkspaceCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl WorkspaceCancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

pub type WorkspaceScanCancellationToken = WorkspaceCancellationToken;
pub type WorkspaceChecksumCancellationToken = WorkspaceCancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceScanErrorKind {
    RootUnavailable,
    RootNotDirectory,
    RootUnreadable,
}

#[derive(Debug)]
pub struct WorkspaceScanError {
    kind: WorkspaceScanErrorKind,
    source: Option<std::io::Error>,
}

impl WorkspaceScanError {
    #[must_use]
    pub const fn kind(&self) -> WorkspaceScanErrorKind {
        self.kind
    }

    fn io(kind: WorkspaceScanErrorKind, source: std::io::Error) -> Self {
        Self {
            kind,
            source: Some(source),
        }
    }

    fn root_not_directory() -> Self {
        Self {
            kind: WorkspaceScanErrorKind::RootNotDirectory,
            source: None,
        }
    }
}

impl std::fmt::Display for WorkspaceScanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.kind, &self.source) {
            (WorkspaceScanErrorKind::RootUnavailable, Some(source)) => {
                write!(formatter, "resolve scan root: {source}")
            }
            (WorkspaceScanErrorKind::RootNotDirectory, _) => {
                formatter.write_str("scan root is not a directory")
            }
            (WorkspaceScanErrorKind::RootUnreadable, Some(source)) => {
                write!(formatter, "read scan root: {source}")
            }
            _ => formatter.write_str("workspace scan failed"),
        }
    }
}

impl std::error::Error for WorkspaceScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

enum PendingScanItem {
    Directory {
        absolute_path: PathBuf,
        relative_path: PathBuf,
        is_root: bool,
    },
    Entry {
        entry: Box<std::fs::DirEntry>,
        parent_relative_path: PathBuf,
    },
}

struct WorkspaceScanner<'a, F> {
    cancellation: &'a WorkspaceScanCancellationToken,
    on_event: &'a mut F,
    summary: WorkspaceScanSummary,
    pending: Vec<PendingScanItem>,
    directories_pending: u64,
    /// 是否进入子目录。逐层浏览只读一层，子目录照常作为条目输出但不展开。
    descend: bool,
}

/// 以稳定的前序顺序扫描工作区，并通过回调逐项输出条目、问题和进度。
///
/// 扫描只暂存单个目录的直接子项用于排序，不跟随符号链接，也不读取文件内容。
/// 单个条目异常会产生 `Issue` 并继续；取消返回 `Cancelled`，不作为错误。
///
/// # Errors
///
/// 根目录无法解析、不是目录或根目录无法读取时返回错误。
pub fn scan_workspace(
    root: &Path,
    cancellation: &WorkspaceScanCancellationToken,
    mut on_event: impl FnMut(WorkspaceScanEvent),
) -> Result<WorkspaceScanOutcome, WorkspaceScanError> {
    let mut scanner = WorkspaceScanner {
        cancellation,
        on_event: &mut on_event,
        summary: WorkspaceScanSummary::default(),
        pending: Vec::new(),
        directories_pending: 0,
        descend: true,
    };
    scanner.progress();
    if cancellation.is_cancelled() {
        return Ok(WorkspaceScanOutcome::Cancelled(scanner.summary));
    }

    let canonical_root = std::fs::canonicalize(root)
        .map_err(|error| WorkspaceScanError::io(WorkspaceScanErrorKind::RootUnavailable, error))?;
    let root_metadata = std::fs::metadata(&canonical_root)
        .map_err(|error| WorkspaceScanError::io(WorkspaceScanErrorKind::RootUnavailable, error))?;
    if !root_metadata.is_dir() {
        return Err(WorkspaceScanError::root_not_directory());
    }
    scanner.run(canonical_root, PathBuf::new())
}

/// 只读一个目录的直接子项，判定规则与全量扫描完全一致（固定忽略、不跟随符号链接、
/// 条目级问题照常上报），只是不进入子目录。
///
/// 调用方负责确认 `absolute_directory` 位于工作区内且确实是目录；`relative_directory`
/// 是它相对工作区根的路径，根目录传空路径。
pub(crate) fn scan_directory_level(
    absolute_directory: PathBuf,
    relative_directory: PathBuf,
    mut on_event: impl FnMut(WorkspaceScanEvent),
) -> Result<WorkspaceScanOutcome, WorkspaceScanError> {
    let cancellation = WorkspaceCancellationToken::new();
    let mut scanner = WorkspaceScanner {
        cancellation: &cancellation,
        on_event: &mut on_event,
        summary: WorkspaceScanSummary::default(),
        pending: Vec::new(),
        directories_pending: 0,
        descend: false,
    };
    scanner.run(absolute_directory, relative_directory)
}

impl<F> WorkspaceScanner<'_, F>
where
    F: FnMut(WorkspaceScanEvent),
{
    fn run(
        &mut self,
        canonical_root: PathBuf,
        root_relative_path: PathBuf,
    ) -> Result<WorkspaceScanOutcome, WorkspaceScanError> {
        self.pending.push(PendingScanItem::Directory {
            absolute_path: canonical_root,
            relative_path: root_relative_path,
            is_root: true,
        });
        self.directories_pending = 1;
        self.progress();

        while let Some(item) = self.pending.pop() {
            if self.cancellation.is_cancelled() {
                return Ok(WorkspaceScanOutcome::Cancelled(self.summary));
            }
            match item {
                PendingScanItem::Directory {
                    absolute_path,
                    relative_path,
                    is_root,
                } => self.read_directory(absolute_path, relative_path, is_root)?,
                PendingScanItem::Entry {
                    entry,
                    parent_relative_path,
                } => self.inspect_entry(&entry, &parent_relative_path),
            }
            self.progress();
        }
        Ok(WorkspaceScanOutcome::Completed(self.summary))
    }

    fn read_directory(
        &mut self,
        absolute_path: PathBuf,
        relative_path: PathBuf,
        is_root: bool,
    ) -> Result<(), WorkspaceScanError> {
        self.directories_pending -= 1;
        let read_directory = match std::fs::read_dir(absolute_path) {
            Ok(entries) => entries,
            Err(error) if is_root => {
                return Err(WorkspaceScanError::io(
                    WorkspaceScanErrorKind::RootUnreadable,
                    error,
                ));
            }
            Err(error) => {
                record_io_issue(
                    self.on_event,
                    &mut self.summary,
                    WorkspaceScanIssueKind::ReadDirectory,
                    relative_path,
                    &error,
                );
                return Ok(());
            }
        };
        let mut entries = Vec::new();
        for entry in read_directory {
            match entry {
                Ok(entry) => entries.push(entry),
                Err(error) => {
                    self.summary.entries_examined += 1;
                    record_io_issue(
                        self.on_event,
                        &mut self.summary,
                        WorkspaceScanIssueKind::ReadEntry,
                        relative_path.clone(),
                        &error,
                    );
                    self.progress();
                }
            }
        }
        entries.sort_by_key(std::fs::DirEntry::file_name);
        self.pending.extend(
            entries
                .into_iter()
                .rev()
                .map(|entry| PendingScanItem::Entry {
                    entry: Box::new(entry),
                    parent_relative_path: relative_path.clone(),
                }),
        );
        Ok(())
    }

    fn inspect_entry(&mut self, entry: &std::fs::DirEntry, parent_relative_path: &Path) {
        self.summary.entries_examined += 1;
        let native_relative_path = parent_relative_path.join(entry.file_name());
        let portable_path = match WorkspaceRelativePath::from_native(&native_relative_path) {
            Ok(path) => path,
            Err(error) => {
                record_path_issue(
                    self.on_event,
                    &mut self.summary,
                    native_relative_path,
                    error,
                );
                return;
            }
        };
        if fixed_ignore_match(&portable_path).is_some() {
            self.summary.ignored_entries += 1;
            return;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                record_io_issue(
                    self.on_event,
                    &mut self.summary,
                    WorkspaceScanIssueKind::FileTypeUnavailable,
                    native_relative_path,
                    &error,
                );
                return;
            }
        };
        let Some(kind) = entry_kind(file_type) else {
            record_issue(
                self.on_event,
                &mut self.summary,
                WorkspaceScanIssue {
                    kind: WorkspaceScanIssueKind::UnsupportedFileType,
                    native_relative_path,
                    io_error_kind: None,
                    path_error: None,
                },
            );
            return;
        };
        let absolute_path = entry.path();
        let entry_snapshot =
            match contexthub_platform_adapter::filesystem::symlink_metadata_with_stable_identity(
                &absolute_path,
            ) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    record_io_issue(
                        self.on_event,
                        &mut self.summary,
                        WorkspaceScanIssueKind::MetadataUnavailable,
                        native_relative_path,
                        &error,
                    );
                    return;
                }
            };
        let metadata = &entry_snapshot.metadata;
        let modified_at = metadata.modified().map_or_else(
            |error| {
                record_io_issue(
                    self.on_event,
                    &mut self.summary,
                    WorkspaceScanIssueKind::ModifiedTimeUnavailable,
                    native_relative_path.clone(),
                    &error,
                );
                None
            },
            Some,
        );
        let byte_len = (kind == WorkspaceEntryKind::File).then_some(metadata.len());
        record_entry(
            self.on_event,
            &mut self.summary,
            WorkspaceScanEntry {
                path: portable_path,
                kind,
                file_identity: Some(entry_snapshot.stable_identity),
                byte_len,
                modified_at,
            },
        );
        if kind == WorkspaceEntryKind::Directory && self.descend {
            self.pending.push(PendingScanItem::Directory {
                absolute_path,
                relative_path: native_relative_path,
                is_root: false,
            });
            self.directories_pending += 1;
        }
    }

    fn progress(&mut self) {
        emit_progress(self.on_event, self.summary, self.directories_pending);
    }
}

pub(crate) fn entry_kind(file_type: std::fs::FileType) -> Option<WorkspaceEntryKind> {
    if file_type.is_file() {
        Some(WorkspaceEntryKind::File)
    } else if file_type.is_dir() {
        Some(WorkspaceEntryKind::Directory)
    } else if file_type.is_symlink() {
        Some(WorkspaceEntryKind::Symlink)
    } else {
        None
    }
}

fn emit_progress(
    on_event: &mut impl FnMut(WorkspaceScanEvent),
    summary: WorkspaceScanSummary,
    directories_pending: u64,
) {
    on_event(WorkspaceScanEvent::Progress(
        WorkspaceScanProgress::from_summary(summary, directories_pending),
    ));
}

fn record_entry(
    on_event: &mut impl FnMut(WorkspaceScanEvent),
    summary: &mut WorkspaceScanSummary,
    entry: WorkspaceScanEntry,
) {
    summary.entries_emitted += 1;
    match entry.kind {
        WorkspaceEntryKind::File => summary.files_emitted += 1,
        WorkspaceEntryKind::Directory => summary.directories_emitted += 1,
        WorkspaceEntryKind::Symlink => summary.symlinks_emitted += 1,
    }
    on_event(WorkspaceScanEvent::Entry(entry));
}

fn record_issue(
    on_event: &mut impl FnMut(WorkspaceScanEvent),
    summary: &mut WorkspaceScanSummary,
    issue: WorkspaceScanIssue,
) {
    summary.issues += 1;
    on_event(WorkspaceScanEvent::Issue(issue));
}

fn record_io_issue(
    on_event: &mut impl FnMut(WorkspaceScanEvent),
    summary: &mut WorkspaceScanSummary,
    kind: WorkspaceScanIssueKind,
    native_relative_path: PathBuf,
    error: &std::io::Error,
) {
    record_issue(
        on_event,
        summary,
        WorkspaceScanIssue {
            kind,
            native_relative_path,
            io_error_kind: Some(error.kind()),
            path_error: None,
        },
    );
}

fn record_path_issue(
    on_event: &mut impl FnMut(WorkspaceScanEvent),
    summary: &mut WorkspaceScanSummary,
    native_relative_path: PathBuf,
    error: contexthub_domain::WorkspacePathError,
) {
    record_issue(
        on_event,
        summary,
        WorkspaceScanIssue {
            kind: WorkspaceScanIssueKind::InvalidRelativePath,
            native_relative_path,
            io_error_kind: None,
            path_error: Some(error),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn local_workspace_root_preflight_resolves_selected_directory_and_protects_app_data() {
        use std::os::unix::fs::symlink;

        let fixture = TestWorkspace::new("root-preflight");
        let existing = fixture.root.join("existing");
        let nested = existing.join("nested");
        let separate = fixture.root.join("separate");
        let protected = fixture.parent.join("app-data");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(&separate).unwrap();
        fs::create_dir(&protected).unwrap();
        let alias = fixture.parent.join("existing-alias");
        symlink(&existing, &alias).unwrap();

        let inspection = inspect_local_workspace_root(&alias, &[]).unwrap();
        assert_eq!(
            inspection.canonical_root,
            fs::canonicalize(&existing).unwrap().to_str().unwrap()
        );
        assert!(inspect_local_workspace_root(&nested, &[]).is_ok());
        assert!(inspect_local_workspace_root(&fixture.root, &[]).is_ok());
        let environment = inspect_local_workspace_root(&fixture.parent, &[protected]).unwrap_err();
        assert_eq!(
            environment.kind(),
            LocalWorkspaceRootErrorKind::EnvironmentConflict
        );

        let inspection = inspect_local_workspace_root(&separate, &[]).unwrap();
        assert_eq!(
            inspection.canonical_root,
            fs::canonicalize(&separate).unwrap().to_str().unwrap()
        );
        assert!(inspection.local_root_identity.is_some());
    }

    #[test]
    fn local_workspace_root_preflight_rejects_missing_and_file_roots() {
        let fixture = TestWorkspace::new("root-preflight-errors");
        let file = fixture.root.join("file.md");
        fs::write(&file, "file").unwrap();

        let missing = inspect_local_workspace_root(&fixture.root.join("missing"), &[]).unwrap_err();
        assert_eq!(missing.kind(), LocalWorkspaceRootErrorKind::RootUnavailable);
        let not_directory = inspect_local_workspace_root(&file, &[]).unwrap_err();
        assert_eq!(
            not_directory.kind(),
            LocalWorkspaceRootErrorKind::RootNotDirectory
        );
    }

    #[test]
    fn local_workspace_root_reports_repeatable_stable_identity() {
        let fixture = TestWorkspace::new("root-stable-identity");
        let first = inspect_local_workspace_root(&fixture.root, &[]).unwrap();
        let second = inspect_local_workspace_root(&fixture.root, &[]).unwrap();
        assert!(first.local_root_identity.is_some());
        assert_eq!(first.local_root_identity, second.local_root_identity);
    }

    #[test]
    fn fixed_ignore_rules_cover_reserved_and_rebuildable_directories() {
        for (input, expected) in [
            (".cilai/workspace.json", FixedIgnoreKind::ContextHubMetadata),
            ("nested/.CILAI/cache", FixedIgnoreKind::ContextHubMetadata),
            (
                ".contexthub/workspace.json",
                FixedIgnoreKind::ContextHubMetadata,
            ),
            ("repo/.GIT/config", FixedIgnoreKind::SourceControlMetadata),
            (
                "web/node_modules/pkg/index.js",
                FixedIgnoreKind::RebuildableCache,
            ),
            ("rust/TARGET/debug/app", FixedIgnoreKind::RebuildableCache),
            (
                "python/__pycache__/module.pyc",
                FixedIgnoreKind::RebuildableCache,
            ),
        ] {
            let matched =
                fixed_ignore_match(&WorkspaceRelativePath::parse(input).unwrap()).unwrap();
            assert_eq!(matched.kind, expected);
        }
        assert!(
            fixed_ignore_match(&WorkspaceRelativePath::parse("docs/target-notes.md").unwrap())
                .is_none()
        );
    }

    #[test]
    fn existing_target_resolves_inside_workspace() {
        let fixture = TestWorkspace::new("inside");
        let document = fixture.root.join("docs/design.md");
        fs::create_dir(fixture.root.join("docs")).unwrap();
        fs::write(&document, "design").unwrap();
        let resolved = resolve_existing_workspace_path(
            &fixture.root,
            &WorkspaceRelativePath::parse("docs/design.md").unwrap(),
        )
        .unwrap();
        assert_eq!(resolved, fs::canonicalize(document).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_that_resolves_outside_workspace_is_rejected() {
        use std::os::unix::fs::symlink;

        let fixture = TestWorkspace::new("symlink");
        let outside = fixture.parent.join("outside.txt");
        fs::write(&outside, "outside").unwrap();
        symlink(&outside, fixture.root.join("escape.txt")).unwrap();
        let error = resolve_existing_workspace_path(
            &fixture.root,
            &WorkspaceRelativePath::parse("escape.txt").unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), WorkspaceResolveErrorKind::OutsideWorkspace);
    }

    #[test]
    fn scan_is_stable_streaming_and_applies_fixed_ignores() {
        let fixture = TestWorkspace::new("stable-scan");
        fs::create_dir_all(fixture.root.join("docs")).unwrap();
        fs::create_dir_all(fixture.root.join("src")).unwrap();
        fs::create_dir_all(fixture.root.join("target/cache")).unwrap();
        fs::write(fixture.root.join("docs/b.md"), "b").unwrap();
        fs::write(fixture.root.join("docs/a.md"), "a").unwrap();
        fs::write(fixture.root.join("src/main.rs"), "main").unwrap();
        fs::write(fixture.root.join("target/cache/output"), "ignored").unwrap();
        fs::write(fixture.root.join(".git"), "gitdir: elsewhere").unwrap();

        let first = scan_trace(&fixture.root);
        let second = scan_trace(&fixture.root);
        assert_eq!(first.paths, second.paths);
        assert_eq!(first.file_identities, second.file_identities);
        assert!(first.file_identities.iter().all(Option::is_some));
        assert_eq!(first.issue_kinds, second.issue_kinds);
        assert_eq!(
            first.paths,
            ["docs", "docs/a.md", "docs/b.md", "src", "src/main.rs"]
        );
        assert!(first.issue_kinds.is_empty());
        assert_eq!(first.outcome, second.outcome);
        assert_eq!(
            first.outcome,
            WorkspaceScanOutcome::Completed(WorkspaceScanSummary {
                entries_examined: 7,
                entries_emitted: 5,
                files_emitted: 3,
                directories_emitted: 2,
                symlinks_emitted: 0,
                ignored_entries: 2,
                issues: 0,
            })
        );
        assert_eq!(first.progress.last().unwrap().directories_pending, 0);
    }

    #[test]
    fn cancellation_returns_partial_monotonic_progress() {
        let fixture = TestWorkspace::new("cancel-scan");
        for name in ["a.md", "b.md", "c.md"] {
            fs::write(fixture.root.join(name), name).unwrap();
        }
        let cancellation = WorkspaceScanCancellationToken::new();
        let callback_token = cancellation.clone();
        let mut progress = Vec::new();
        let outcome = scan_workspace(&fixture.root, &cancellation, |event| match event {
            WorkspaceScanEvent::Entry(_) => callback_token.cancel(),
            WorkspaceScanEvent::Progress(value) => progress.push(value),
            WorkspaceScanEvent::Issue(_) => {}
        })
        .unwrap();

        assert_eq!(
            outcome,
            WorkspaceScanOutcome::Cancelled(WorkspaceScanSummary {
                entries_examined: 1,
                entries_emitted: 1,
                files_emitted: 1,
                directories_emitted: 0,
                symlinks_emitted: 0,
                ignored_entries: 0,
                issues: 0,
            })
        );
        assert!(progress.windows(2).all(|window| {
            window[0].entries_examined <= window[1].entries_examined
                && window[0].entries_emitted <= window[1].entries_emitted
        }));
    }

    #[test]
    fn scan_reads_later_directories_only_when_reached() {
        let fixture = TestWorkspace::new("streaming-scan");
        fs::create_dir(fixture.root.join("a")).unwrap();
        fs::create_dir(fixture.root.join("z")).unwrap();
        let late_file = fixture.root.join("z/late.md");
        let mut paths = Vec::new();
        let outcome = scan_workspace(
            &fixture.root,
            &WorkspaceScanCancellationToken::new(),
            |event| {
                if let WorkspaceScanEvent::Entry(entry) = event {
                    if entry.path.as_str() == "a" {
                        fs::write(&late_file, "late").unwrap();
                    }
                    paths.push(entry.path.as_str().to_owned());
                }
            },
        )
        .unwrap();

        assert!(matches!(outcome, WorkspaceScanOutcome::Completed(_)));
        assert_eq!(paths, ["a", "z", "z/late.md"]);
    }

    #[test]
    fn disappearing_subdirectory_is_reported_without_blocking_siblings() {
        let fixture = TestWorkspace::new("disappearing-directory");
        let disappearing = fixture.root.join("a-vanish");
        fs::create_dir(&disappearing).unwrap();
        fs::write(disappearing.join("lost.md"), "lost").unwrap();
        fs::create_dir(fixture.root.join("z-keep")).unwrap();
        fs::write(fixture.root.join("z-keep/kept.md"), "kept").unwrap();
        let mut events = Vec::new();
        let outcome = scan_workspace(
            &fixture.root,
            &WorkspaceScanCancellationToken::new(),
            |event| {
                if matches!(
                    &event,
                    WorkspaceScanEvent::Entry(entry) if entry.path.as_str() == "a-vanish"
                ) {
                    fs::remove_dir_all(&disappearing).unwrap();
                }
                events.push(event);
            },
        )
        .unwrap();

        assert!(matches!(outcome, WorkspaceScanOutcome::Completed(_)));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkspaceScanEvent::Issue(issue)
                if issue.kind == WorkspaceScanIssueKind::ReadDirectory
                    && issue.native_relative_path == Path::new("a-vanish")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkspaceScanEvent::Entry(entry) if entry.path.as_str() == "z-keep/kept.md"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn moved_subdirectory_is_reported_without_following_its_new_location() {
        let fixture = TestWorkspace::new("moved-directory");
        let moving = fixture.root.join("a-move");
        let moved_outside_root = fixture.parent.join("moved-outside-root");
        fs::create_dir(&moving).unwrap();
        fs::write(moving.join("moved.md"), "moved").unwrap();
        fs::create_dir(fixture.root.join("z-keep")).unwrap();
        fs::write(fixture.root.join("z-keep/kept.md"), "kept").unwrap();
        let mut events = Vec::new();
        let outcome = scan_workspace(
            &fixture.root,
            &WorkspaceScanCancellationToken::new(),
            |event| {
                if matches!(
                    &event,
                    WorkspaceScanEvent::Entry(entry) if entry.path.as_str() == "a-move"
                ) {
                    fs::rename(&moving, &moved_outside_root).unwrap();
                }
                events.push(event);
            },
        )
        .unwrap();

        assert!(matches!(outcome, WorkspaceScanOutcome::Completed(_)));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkspaceScanEvent::Issue(issue)
                if issue.kind == WorkspaceScanIssueKind::ReadDirectory
                    && issue.native_relative_path == Path::new("a-move")
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            WorkspaceScanEvent::Entry(entry) if entry.path.as_str().ends_with("moved.md")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkspaceScanEvent::Entry(entry) if entry.path.as_str() == "z-keep/kept.md"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_subdirectory_is_reported_without_blocking_siblings() {
        use std::os::unix::fs::PermissionsExt as _;

        let fixture = TestWorkspace::new("unreadable-directory");
        let unreadable = fixture.root.join("a-unreadable");
        fs::create_dir(&unreadable).unwrap();
        fs::write(unreadable.join("hidden.md"), "hidden").unwrap();
        fs::write(fixture.root.join("z-visible.md"), "visible").unwrap();
        let original_mode = fs::metadata(&unreadable).unwrap().permissions().mode();
        let _permission_guard = PermissionGuard::new(unreadable.clone(), original_mode);
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

        let mut events = Vec::new();
        let outcome = scan_workspace(
            &fixture.root,
            &WorkspaceScanCancellationToken::new(),
            |event| events.push(event),
        )
        .unwrap();

        assert!(matches!(outcome, WorkspaceScanOutcome::Completed(_)));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkspaceScanEvent::Issue(issue)
                if issue.kind == WorkspaceScanIssueKind::ReadDirectory
                    && issue.native_relative_path == Path::new("a-unreadable")
                    && issue.io_error_kind == Some(std::io::ErrorKind::PermissionDenied)
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            WorkspaceScanEvent::Entry(entry) if entry.path.as_str() == "z-visible.md"
        )));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_root_returns_explicit_root_error() {
        use std::os::unix::fs::PermissionsExt as _;

        let fixture = TestWorkspace::new("unreadable-root");
        let original_mode = fs::metadata(&fixture.root).unwrap().permissions().mode();
        let _permission_guard = PermissionGuard::new(fixture.root.clone(), original_mode);
        fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o000)).unwrap();

        let error = scan_workspace(
            &fixture.root,
            &WorkspaceScanCancellationToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error.kind(), WorkspaceScanErrorKind::RootUnreadable);
    }

    #[cfg(unix)]
    #[test]
    fn invalid_native_name_is_reported_and_symlink_is_not_followed() {
        use std::os::unix::fs::symlink;

        let fixture = TestWorkspace::new("special-entries");
        fs::write(fixture.root.join("bad\\name.md"), "bad").unwrap();
        fs::write(fixture.root.join("good.md"), "good").unwrap();
        let outside_directory = fixture.parent.join("outside");
        fs::create_dir(&outside_directory).unwrap();
        fs::write(outside_directory.join("secret.md"), "secret").unwrap();
        symlink(&outside_directory, fixture.root.join("outside-link")).unwrap();

        let trace = scan_trace(&fixture.root);
        assert!(
            trace
                .issue_kinds
                .contains(&WorkspaceScanIssueKind::InvalidRelativePath)
        );
        assert!(trace.paths.contains(&"good.md".to_owned()));
        assert!(trace.paths.contains(&"outside-link".to_owned()));
        assert!(!trace.paths.iter().any(|path| path.contains("secret.md")));
        assert_eq!(trace.outcome.summary().symlinks_emitted, 1);
    }

    #[cfg(unix)]
    #[test]
    fn broken_and_looping_symlinks_are_emitted_without_recursion() {
        use std::os::unix::fs::symlink;

        let fixture = TestWorkspace::new("symlink-edge-cases");
        symlink("missing-target", fixture.root.join("broken-link")).unwrap();
        symlink(".", fixture.root.join("loop-link")).unwrap();

        let trace = scan_trace(&fixture.root);
        assert_eq!(trace.paths, ["broken-link", "loop-link"]);
        assert!(trace.issue_kinds.is_empty());
        assert_eq!(trace.outcome.summary().symlinks_emitted, 2);
        assert_eq!(trace.outcome.summary().entries_examined, 2);
    }

    #[test]
    fn overlong_native_target_returns_stable_target_unavailable_error() {
        let fixture = TestWorkspace::new("overlong-target");
        let path = WorkspaceRelativePath::parse(&"a".repeat(300)).unwrap();

        let error = resolve_existing_workspace_path(&fixture.root, &path).unwrap_err();
        assert_eq!(error.kind(), WorkspaceResolveErrorKind::TargetUnavailable);
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn invalid_scan_roots_return_explicit_errors() {
        let fixture = TestWorkspace::new("invalid-roots");
        let file = fixture.root.join("file.md");
        fs::write(&file, "file").unwrap();
        let cancellation = WorkspaceScanCancellationToken::new();
        let file_error = scan_workspace(&file, &cancellation, |_| {}).unwrap_err();
        assert_eq!(file_error.kind(), WorkspaceScanErrorKind::RootNotDirectory);
        let missing_error =
            scan_workspace(&fixture.root.join("missing"), &cancellation, |_| {}).unwrap_err();
        assert_eq!(
            missing_error.kind(),
            WorkspaceScanErrorKind::RootUnavailable
        );
    }

    struct ScanTrace {
        paths: Vec<String>,
        file_identities: Vec<Option<String>>,
        issue_kinds: Vec<WorkspaceScanIssueKind>,
        progress: Vec<WorkspaceScanProgress>,
        outcome: WorkspaceScanOutcome,
    }

    fn scan_trace(root: &Path) -> ScanTrace {
        let mut paths = Vec::new();
        let mut file_identities = Vec::new();
        let mut issue_kinds = Vec::new();
        let mut progress = Vec::new();
        let outcome =
            scan_workspace(
                root,
                &WorkspaceScanCancellationToken::new(),
                |event| match event {
                    WorkspaceScanEvent::Entry(entry) => {
                        paths.push(entry.path.as_str().to_owned());
                        file_identities.push(entry.file_identity);
                    }
                    WorkspaceScanEvent::Issue(issue) => issue_kinds.push(issue.kind),
                    WorkspaceScanEvent::Progress(value) => progress.push(value),
                },
            )
            .unwrap();
        ScanTrace {
            paths,
            file_identities,
            issue_kinds,
            progress,
            outcome,
        }
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
                "ch-workspace-path-{label}-{}-{nanos}",
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
