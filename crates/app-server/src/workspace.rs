use contexthub_application::{
    CreateLocalWorkspaceCommand, FileSnapshotRepository, LocalFileObservation, LocalScanSnapshot,
    LocalSnapshotCommitResult, ProjectionExportOutcome, RecomputeFileStateCommand, SnapshotKind,
    UnitOfWork, WorkspaceFileRecord, WorkspaceRecord, WorkspaceRepository, create_local_workspace,
    export_workspace_projection, recompute_file_state,
};
use contexthub_domain::{
    BaselinePathRelation, EventId, FilePermission, FilePermissions, FileStateInput, LocalNodeKind,
    LocalScanState, MaterializationState, MetadataValidity, PlatformConnection,
    PlatformDocumentState, SubmissionState, TaskId, WorkspaceId, WorkspaceKind,
};
use contexthub_persistence_sqlite::{
    FileContentIndex, FileSearchScan, PersistenceError, SqlitePersistence,
};
use contexthub_workspace_fs::{
    ChecksumAlgorithm, ChecksumCacheDecision, ChecksumCacheFingerprint, ChecksumCacheRecord,
    ChecksumUseCase, ChecksumValue, LocalWorkspaceRootError, WorkspaceBrowseError,
    WorkspaceCancellationToken, WorkspaceChecksumCancellationToken, WorkspaceChecksumError,
    WorkspaceChecksumOutcome, WorkspaceEntryKind, WorkspaceProjectionWriter, WorkspaceScanEntry,
    WorkspaceScanError, WorkspaceScanEvent, WorkspaceScanOutcome, checksum_cache_decision,
    checksum_workspace_file, inspect_local_workspace_root, inspect_workspace_entry, scan_workspace,
};
use sha2::{Digest as _, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

mod search_text;

#[derive(Debug)]
pub enum CreateLocalWorkspaceError {
    Persistence(PersistenceError),
    Root(LocalWorkspaceRootError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionAttempt {
    Current(ProjectionExportOutcome),
    Failed { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationWithProjection<T> {
    pub business: T,
    pub projection: ProjectionAttempt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureLocalSnapshotCommand {
    pub workspace_id: WorkspaceId,
    pub snapshot_id: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub observed_at: i64,
    pub completed_at: i64,
}

#[derive(Debug)]
pub enum CaptureLocalSnapshotError {
    Persistence(PersistenceError),
    WorkspaceMissing,
    Scan(WorkspaceScanError),
    Cancelled,
    Incomplete { issue_count: u64 },
    ObservationOutOfRange,
    Checksum(WorkspaceChecksumError),
    EntryUnavailable(WorkspaceBrowseError),
    SourceChanged,
    TextIndex(contexthub_workspace_fs::TextResourceError),
}

impl std::fmt::Display for CaptureLocalSnapshotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Persistence(error) => write!(formatter, "persist local snapshot: {error}"),
            Self::WorkspaceMissing => {
                formatter.write_str("local snapshot workspace does not exist")
            }
            Self::Scan(error) => write!(formatter, "scan local snapshot: {error}"),
            Self::Cancelled => formatter.write_str("local snapshot scan was cancelled"),
            Self::Incomplete { issue_count } => write!(
                formatter,
                "local snapshot scan is incomplete because it reported {issue_count} issues"
            ),
            Self::ObservationOutOfRange => {
                formatter.write_str("local snapshot observation exceeds database range")
            }
            Self::Checksum(error) => write!(formatter, "checksum local snapshot file: {error}"),
            Self::EntryUnavailable(error) => {
                write!(formatter, "inspect local snapshot file: {error}")
            }
            Self::SourceChanged => {
                formatter.write_str("local snapshot file changed during indexing")
            }
            Self::TextIndex(error) => write!(formatter, "index local file text: {error}"),
        }
    }
}

impl std::error::Error for CaptureLocalSnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persistence(error) => Some(error),
            Self::Scan(error) => Some(error),
            Self::Checksum(error) => Some(error),
            Self::EntryUnavailable(error) => Some(error),
            Self::TextIndex(error) => Some(error),
            Self::WorkspaceMissing
            | Self::Cancelled
            | Self::Incomplete { .. }
            | Self::ObservationOutOfRange
            | Self::SourceChanged => None,
        }
    }
}

impl From<PersistenceError> for CaptureLocalSnapshotError {
    fn from(error: PersistenceError) -> Self {
        Self::Persistence(error)
    }
}

impl std::fmt::Display for CreateLocalWorkspaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Persistence(error) => write!(formatter, "persist local workspace: {error}"),
            Self::Root(error) => write!(formatter, "validate local workspace root: {error}"),
        }
    }
}

impl std::error::Error for CreateLocalWorkspaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persistence(error) => Some(error),
            Self::Root(error) => Some(error),
        }
    }
}

/// 预检目录并原子创建仅本地工作区及首次扫描任务。
///
/// `protected_roots` 至少应包含 App Server 数据目录，避免工作区扫描自身数据库、日志和运行文件。
///
/// # Errors
///
/// 所选目录不可用、目录受保护，或创建事务无法提交时返回错误。
pub fn create_preflighted_local_workspace(
    persistence: &mut SqlitePersistence,
    candidate: &Path,
    protected_roots: &[PathBuf],
    mut command: CreateLocalWorkspaceCommand,
) -> Result<OperationWithProjection<WorkspaceRecord>, CreateLocalWorkspaceError> {
    let inspection = inspect_local_workspace_root(candidate, protected_roots)
        .map_err(CreateLocalWorkspaceError::Root)?;
    let existing = persistence
        .transact(|repositories| repositories.workspaces())
        .map_err(CreateLocalWorkspaceError::Persistence)?
        .into_iter()
        .find(|workspace| {
            workspace.workspace_kind == WorkspaceKind::Local
                && (workspace.local_root == inspection.canonical_root
                    || (workspace.local_root_identity.is_some()
                        && workspace.local_root_identity == inspection.local_root_identity))
        });
    if let Some(mut workspace) = existing {
        if workspace.local_root != inspection.canonical_root {
            workspace = persistence
                .relocate_local_workspace_root(
                    &workspace,
                    &inspection.canonical_root,
                    command.occurred_at,
                )
                .map_err(CreateLocalWorkspaceError::Persistence)?;
        }
        let projection =
            attempt_projection(persistence, &workspace.workspace_id, command.occurred_at);
        return Ok(OperationWithProjection {
            business: workspace,
            projection,
        });
    }
    command.canonical_local_root = inspection.canonical_root;
    command.local_root_identity = inspection.local_root_identity;
    let workspace = persistence
        .transact(|repositories| create_local_workspace(repositories, &command))
        .map_err(CreateLocalWorkspaceError::Persistence)?;
    let projection = attempt_projection(persistence, &workspace.workspace_id, command.occurred_at);
    Ok(OperationWithProjection {
        business: workspace,
        projection,
    })
}

/// 扫描一个已登记工作区，并仅在扫描完整时原子提交完整文件快照与耐久事件。
///
/// 扫描取消或出现任一隔离问题时不会把未观察到的旧文件解释为删除，也不会写入部分快照。
///
/// # Errors
///
/// 工作区不存在、扫描失败或不完整、时间与大小越界，或快照事务无法提交时返回错误。
pub fn capture_full_local_snapshot(
    persistence: &mut SqlitePersistence,
    command: &CaptureLocalSnapshotCommand,
    cancellation: &WorkspaceCancellationToken,
) -> Result<OperationWithProjection<LocalSnapshotCommitResult>, CaptureLocalSnapshotError> {
    let workspace = persistence
        .transact(|repositories| repositories.workspace(&command.workspace_id))
        .map_err(CaptureLocalSnapshotError::Persistence)?;
    let workspace = workspace.ok_or(CaptureLocalSnapshotError::WorkspaceMissing)?;
    let generation = persistence
        .transact(|repositories| repositories.next_local_snapshot_generation(&command.workspace_id))
        .map_err(CaptureLocalSnapshotError::Persistence)?;
    let content_index = persistence.file_content_index()?;
    let search_scan = content_index.begin_search_scan()?;
    let mut entries = Vec::new();
    let outcome = scan_workspace(Path::new(&workspace.local_root), cancellation, |event| {
        if let WorkspaceScanEvent::Entry(entry) = event {
            entries.push(entry);
        }
    })
    .map_err(CaptureLocalSnapshotError::Scan)?;
    let summary = match outcome {
        WorkspaceScanOutcome::Completed(summary) => summary,
        WorkspaceScanOutcome::Cancelled(_) => return Err(CaptureLocalSnapshotError::Cancelled),
    };
    if summary.issues != 0 {
        return Err(CaptureLocalSnapshotError::Incomplete {
            issue_count: summary.issues,
        });
    }
    let observations = entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            local_observation(
                &workspace.local_root,
                command,
                index,
                entry,
                &content_index,
                &search_scan,
            )
        })
        .collect::<Result<Vec<_>, CaptureLocalSnapshotError>>()?;
    content_index.flush()?;
    let result = persistence
        .transact(|repositories| {
            repositories.commit_local_snapshot(&LocalScanSnapshot {
                snapshot_id: command.snapshot_id.clone(),
                workspace_id: command.workspace_id.clone(),
                generation,
                snapshot_kind: SnapshotKind::Full,
                observed_at: command.observed_at,
                completed_at: command.completed_at,
                issue_count: 0,
                event_id: command.event_id.clone(),
                correlation_id: command.correlation_id.clone(),
                observations,
                removed_normalized_paths: Vec::new(),
                covers_whole_tree: true,
            })
        })
        .map_err(CaptureLocalSnapshotError::Persistence)?;
    drop(search_scan);
    content_index.collect_unused_search_content()?;
    initialize_created_file_states(persistence, command, &result)?;
    let projection = attempt_projection(persistence, &command.workspace_id, command.completed_at);
    Ok(OperationWithProjection {
        business: result,
        projection,
    })
}

/// 只把变化的部分写成一次增量快照。
///
/// 全量捕获每次都把工作区里每一条记录重新写一遍观测。实测这个工作区一次要 8.7 秒，
/// 其中目录遍历只占 0.47 秒——余下 95% 花在把两万多条没变的记录重写一遍上，而那一次
/// 真正变化的只有 41 条。手动扫描年代这只是慢；接上文件监听之后它按变化频率反复发生。
///
/// 首次和手动检查遍历全树；具体文件监听事件只遍历受影响路径，删除也限制在同一范围。
///
/// # Errors
///
/// 工作区不存在、扫描不完整或持久化失败时返回错误。
/// 增量捕获的第一段：读出比对所需的事实。要持锁。
pub struct IncrementalScanBasis {
    workspace_root: String,
    known: HashMap<String, WorkspaceFileRecord>,
    content_index: Arc<FileContentIndex>,
}

/// 把捕获失败翻译成界面能分辨的原因代码。
#[must_use]
pub fn index_failure_reason(error: &CaptureLocalSnapshotError) -> &'static str {
    match error {
        CaptureLocalSnapshotError::Persistence(_) => "workspace_index_write_failed",
        CaptureLocalSnapshotError::WorkspaceMissing => "workspace_not_found",
        CaptureLocalSnapshotError::Scan(error) => match error.kind() {
            contexthub_workspace_fs::WorkspaceScanErrorKind::RootNotDirectory => {
                "workspace_root_not_directory"
            }
            _ => "workspace_root_unavailable",
        },
        CaptureLocalSnapshotError::EntryUnavailable(error) => match error.kind() {
            contexthub_workspace_fs::WorkspaceBrowseErrorKind::RootUnavailable => {
                "workspace_root_unavailable"
            }
            contexthub_workspace_fs::WorkspaceBrowseErrorKind::TargetUnavailable => {
                "workspace_target_unavailable"
            }
            contexthub_workspace_fs::WorkspaceBrowseErrorKind::OutsideWorkspace => {
                "workspace_path_outside"
            }
            contexthub_workspace_fs::WorkspaceBrowseErrorKind::NotDirectory => {
                "workspace_target_not_directory"
            }
            contexthub_workspace_fs::WorkspaceBrowseErrorKind::Ignored => {
                "workspace_target_ignored"
            }
            contexthub_workspace_fs::WorkspaceBrowseErrorKind::UnsupportedFileType => {
                "workspace_target_unsupported"
            }
        },
        CaptureLocalSnapshotError::Cancelled => "workspace_index_cancelled",
        CaptureLocalSnapshotError::Incomplete { .. } => "workspace_scan_incomplete",
        CaptureLocalSnapshotError::ObservationOutOfRange => "workspace_observation_out_of_range",
        CaptureLocalSnapshotError::Checksum(_) | CaptureLocalSnapshotError::SourceChanged => {
            "workspace_checksum_failed"
        }
        CaptureLocalSnapshotError::TextIndex(_) => "workspace_text_index_failed",
    }
}

fn fs_diagnostic(
    workspace_id: &str,
    path: &str,
    stage: &'static str,
    reason: &'static str,
    error: Option<&(dyn std::error::Error + 'static)>,
    elapsed: Duration,
) -> String {
    let error_kind = error.map_or_else(
        || "none".to_owned(),
        |error| {
            if let Some(error) = error.downcast_ref::<contexthub_workspace_fs::WorkspaceScanError>()
            {
                format!("Scan{:?}", error.kind())
            } else if let Some(error) = error.downcast_ref::<WorkspaceBrowseError>() {
                format!("Browse{:?}", error.kind())
            } else {
                "Other".to_owned()
            }
        },
    );
    let mut cause = error;
    let mut io = None;
    while let Some(error) = cause {
        if let Some(value) = error.downcast_ref::<std::io::Error>() {
            io = Some(value);
            break;
        }
        cause = error.source();
    }
    let kind = io.map_or_else(|| "none".to_owned(), |error| format!("{:?}", error.kind()));
    let errno = io
        .and_then(std::io::Error::raw_os_error)
        .map_or_else(|| "none".to_owned(), |value| value.to_string());
    format!(
        "workspace_fs_diagnostic workspace_hash={:x} path_hash={:x} stage={stage} reason={reason} error_kind={error_kind} io_kind={kind} errno={errno} elapsed_ms={}",
        Sha256::digest(workspace_id.as_bytes()),
        Sha256::digest(path.as_bytes()),
        elapsed.as_millis()
    )
}

pub(crate) fn log_fs_diagnostic(
    workspace_id: &str,
    path: &str,
    stage: &'static str,
    reason: &'static str,
    error: Option<&(dyn std::error::Error + 'static)>,
    elapsed: Duration,
) {
    eprintln!(
        "{}",
        fs_diagnostic(workspace_id, path, stage, reason, error, elapsed)
    );
}

pub(crate) fn log_index_failure(
    workspace_id: &str,
    paths: Option<&[String]>,
    stage: &'static str,
    error: &CaptureLocalSnapshotError,
    elapsed: Duration,
) {
    let cause: Option<&(dyn std::error::Error + 'static)> = match error {
        CaptureLocalSnapshotError::Scan(error) => Some(error),
        CaptureLocalSnapshotError::EntryUnavailable(error) => Some(error),
        CaptureLocalSnapshotError::Checksum(error) => Some(error),
        _ => None,
    };
    log_fs_diagnostic(
        workspace_id,
        &paths.map_or_else(String::new, |paths| paths.join("\n")),
        stage,
        index_failure_reason(error),
        cause,
        elapsed,
    );
}

/// 目录遍历每检查这么多条目报告一次进度。逐条报告会让进度登记的锁比扫描本身还忙。
const PROGRESS_REPORT_STRIDE: u64 = 256;

/// 增量捕获第二段的进度，供界面显示"索引进行到哪了"。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IncrementalScanProgress {
    /// 目录遍历已检查的条目数。
    Scanned(u64),
    /// 遍历结束，共有这么多条变化要计算摘要并写入。
    HashingStarted(u64),
    /// 已处理完的变化条数。
    Hashed(u64),
}

/// 增量捕获的第二段产物：只有变化的观测，以及不见了的路径。不需要持锁。
pub struct IncrementalScanFindings {
    search_scan: Option<Arc<FileSearchScan>>,
    observations: Vec<LocalFileObservation>,
    removed_normalized_paths: Vec<String>,
}

/// 一批最多提交多少条观测。
///
/// 提交要持服务主锁。新连一个两万文件的文件夹，一次写完实测要占锁近三秒，界面在这段
/// 时间里所有请求都得排队；分批写、批间放锁，界面请求就能插进来。
pub const COMMIT_BATCH_SIZE: usize = 1000;

impl IncrementalScanFindings {
    /// 拆成若干批。前面各批只新增或更新记录；删除只随最后一批提交——只有走完最后一批，
    /// 删除范围已由观测阶段限定，分批中途失败不提交删除。至少返回一批。
    #[must_use]
    pub fn into_batches(self, batch_size: usize) -> Vec<Self> {
        let batch_size = batch_size.max(1);
        let mut observations = self.observations.into_iter().peekable();
        let mut batches = Vec::new();
        while observations.peek().is_some() {
            batches.push(Self {
                search_scan: self.search_scan.clone(),
                observations: observations.by_ref().take(batch_size).collect(),
                removed_normalized_paths: Vec::new(),
            });
        }
        match batches.last_mut() {
            Some(last) => last.removed_normalized_paths = self.removed_normalized_paths,
            None => batches.push(Self {
                search_scan: self.search_scan,
                observations: Vec::new(),
                removed_normalized_paths: self.removed_normalized_paths,
            }),
        }
        batches
    }

    #[must_use]
    pub fn observation_count(&self) -> usize {
        self.observations.len()
    }
}

/// 读出增量比对所需的事实。
///
/// # Errors
///
/// 工作区不存在或持久化失败时返回错误。
pub fn prepare_incremental_scan(
    persistence: &mut SqlitePersistence,
    workspace_id: &WorkspaceId,
) -> Result<IncrementalScanBasis, CaptureLocalSnapshotError> {
    let workspace = persistence
        .transact(|repositories| repositories.workspace(workspace_id))
        .map_err(CaptureLocalSnapshotError::Persistence)?
        .ok_or(CaptureLocalSnapshotError::WorkspaceMissing)?;
    // 只取当前还在磁盘上的记录。比对和"谁不见了"都只关心它们，而历史上标为缺失的
    // 旧记录可能是好几倍——本机实测 49,081 条里有 26,732 条是缺失的，一起读出来
    // 只是白白拉长持锁时间。
    //
    // 这也顺带避开一个坑：同一条路径常常两种记录并存（实测 19,728 条），按路径建
    // 索引时若让缺失的那份覆盖了存在的那份，每一条都会被判成变化，增量退化成全量。
    let known = persistence
        .transact(|repositories| repositories.present_workspace_files(workspace_id))
        .map_err(CaptureLocalSnapshotError::Persistence)?
        .into_iter()
        .map(|file| (file.normalized_relative_path.clone(), file))
        .collect::<HashMap<_, _>>();
    Ok(IncrementalScanBasis {
        workspace_root: workspace.local_root,
        known,
        content_index: persistence.file_content_index()?,
    })
}

/// 走一遍目录，挑出与记录不一致的部分。
///
/// 这一段只使用共享内容索引连接，调用方应当在服务锁外执行——它是整个捕获里最慢的一步，
/// 占住服务主锁会让界面在每次文件变化时卡住。
///
/// # Errors
///
/// 扫描失败、被取消或扫描不完整时返回错误。
pub fn observe_incremental_changes(
    basis: &IncrementalScanBasis,
    command: &CaptureLocalSnapshotCommand,
    cancellation: &WorkspaceCancellationToken,
    mut on_progress: impl FnMut(IncrementalScanProgress),
) -> Result<IncrementalScanFindings, CaptureLocalSnapshotError> {
    let mut entries = Vec::new();
    let outcome = scan_workspace(
        Path::new(&basis.workspace_root),
        cancellation,
        |event| match event {
            WorkspaceScanEvent::Entry(entry) => entries.push(entry),
            WorkspaceScanEvent::Progress(progress)
                if progress.entries_examined % PROGRESS_REPORT_STRIDE == 0 =>
            {
                on_progress(IncrementalScanProgress::Scanned(progress.entries_examined));
            }
            WorkspaceScanEvent::Progress(_) | WorkspaceScanEvent::Issue(_) => {}
        },
    )
    .map_err(CaptureLocalSnapshotError::Scan)?;
    let summary = match outcome {
        WorkspaceScanOutcome::Completed(summary) => summary,
        WorkspaceScanOutcome::Cancelled(_) => return Err(CaptureLocalSnapshotError::Cancelled),
    };
    // 扫描不完整就不能推断"谁不见了"——那会把读不到的目录整片标成缺失。
    if summary.issues != 0 {
        return Err(CaptureLocalSnapshotError::Incomplete {
            issue_count: summary.issues,
        });
    }

    observations_for_entries(
        basis,
        command,
        entries,
        None,
        cancellation,
        summary.entries_examined,
        on_progress,
    )
}

/// 仅检查文件监听给出的路径；目录变化只遍历对应子树。
///
/// # Errors
/// 已存在路径不可读、子树扫描不完整、内容读取或缓存写入失败时返回错误。
pub(crate) fn observe_changed_paths(
    basis: &IncrementalScanBasis,
    command: &CaptureLocalSnapshotCommand,
    paths: &[String],
    cancellation: &WorkspaceCancellationToken,
    on_progress: impl FnMut(IncrementalScanProgress),
) -> Result<IncrementalScanFindings, CaptureLocalSnapshotError> {
    let mut roots = paths.to_vec();
    roots.sort();
    roots.dedup();
    let all: HashSet<String> = roots.iter().cloned().collect();
    roots.retain(|path| {
        !path
            .match_indices('/')
            .any(|(index, _)| all.contains(&path[..index]))
    });
    let mut entries = Vec::new();
    for path in &roots {
        let relative = contexthub_domain::WorkspaceRelativePath::parse(path)
            .map_err(|_| CaptureLocalSnapshotError::ObservationOutOfRange)?;
        match inspect_workspace_entry(Path::new(&basis.workspace_root), &relative) {
            Ok(entry) => {
                let directory = entry.kind == WorkspaceEntryKind::Directory;
                entries.push(entry.clone());
                if directory {
                    let mut children = Vec::new();
                    let outcome = scan_workspace(
                        &Path::new(&basis.workspace_root).join(path),
                        cancellation,
                        |event| {
                            if let WorkspaceScanEvent::Entry(entry) = event {
                                children.push(entry);
                            }
                        },
                    )
                    .map_err(CaptureLocalSnapshotError::Scan)?;
                    match outcome {
                        WorkspaceScanOutcome::Completed(summary) if summary.issues == 0 => {}
                        WorkspaceScanOutcome::Completed(summary) => {
                            return Err(CaptureLocalSnapshotError::Incomplete {
                                issue_count: summary.issues,
                            });
                        }
                        WorkspaceScanOutcome::Cancelled(_) => {
                            return Err(CaptureLocalSnapshotError::Cancelled);
                        }
                    }
                    verify_content_entry(&basis.workspace_root, &entry)?;
                    for mut child in children {
                        child.path = contexthub_domain::WorkspaceRelativePath::parse(&format!(
                            "{path}/{}",
                            child.path.as_str()
                        ))
                        .map_err(|_| CaptureLocalSnapshotError::ObservationOutOfRange)?;
                        entries.push(child);
                    }
                }
            }
            Err(error) if error.is_missing() => {}
            Err(error) => return Err(CaptureLocalSnapshotError::EntryUnavailable(error)),
        }
    }
    let examined = entries.len() as u64;
    observations_for_entries(
        basis,
        command,
        entries,
        Some(&roots),
        cancellation,
        examined,
        on_progress,
    )
}

fn observations_for_entries(
    basis: &IncrementalScanBasis,
    command: &CaptureLocalSnapshotCommand,
    entries: Vec<WorkspaceScanEntry>,
    affected: Option<&[String]>,
    cancellation: &WorkspaceCancellationToken,
    entries_examined: u64,
    mut on_progress: impl FnMut(IncrementalScanProgress),
) -> Result<IncrementalScanFindings, CaptureLocalSnapshotError> {
    let search_scan = basis.content_index.begin_search_scan()?;
    let mut seen = HashSet::new();
    let mut changed = Vec::new();
    for entry in entries {
        let path = entry.path.as_str().to_owned();
        let cached = basis.known.get(&path);
        seen.insert(path);
        if cached.is_some_and(|record| is_unchanged(&entry, record)) {
            if entry.kind != WorkspaceEntryKind::File {
                continue;
            }
            let (identity, fingerprint) = content_fingerprint(&entry)?;
            if let Some(checksum) = basis.content_index.lookup(identity, &fingerprint)?
                && cached.and_then(|record| record.checksum_value.as_ref()) == Some(&checksum)
                && (!search_text::visible_path(&entry)
                    || basis.content_index.has_search_content(&checksum)?)
            {
                continue;
            }
        }
        changed.push(entry);
    }
    on_progress(IncrementalScanProgress::Scanned(entries_examined));
    on_progress(IncrementalScanProgress::HashingStarted(changed.len() as u64));
    let observations = changed
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            if cancellation.is_cancelled() {
                return Err(CaptureLocalSnapshotError::Cancelled);
            }
            let observation = local_observation(
                &basis.workspace_root,
                command,
                index,
                entry,
                &basis.content_index,
                &search_scan,
            );
            on_progress(IncrementalScanProgress::Hashed(index as u64 + 1));
            observation
        })
        .collect::<Result<Vec<_>, CaptureLocalSnapshotError>>()?;
    basis.content_index.flush()?;
    let removed_normalized_paths = basis
        .known
        .iter()
        .filter(|(path, record)| {
            record.presence_state == "present"
                && !seen.contains(*path)
                && affected.is_none_or(|roots| {
                    roots
                        .iter()
                        .any(|root| path.as_str() == root || path.starts_with(&format!("{root}/")))
                })
        })
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    Ok(IncrementalScanFindings {
        search_scan: Some(search_scan),
        observations,
        removed_normalized_paths,
    })
}

/// 把变化提交成一次增量快照。要持锁。
///
/// `generation` 在这里读，而不是在第一段读：扫描期间可能有别的写入落库，
/// 拿旧的 `generation` 提交会被 `commit_local_snapshot` 判为冲突——那正是我们要的，
/// 这一轮作废，下一轮重来，而不是把一份基于过期事实的快照写进去。
///
/// # Errors
///
/// 持久化失败或与并发写入冲突时返回错误。
pub fn commit_incremental_snapshot(
    persistence: &mut SqlitePersistence,
    command: &CaptureLocalSnapshotCommand,
    findings: IncrementalScanFindings,
) -> Result<OperationWithProjection<LocalSnapshotCommitResult>, CaptureLocalSnapshotError> {
    commit_observations(persistence, command, findings, true)
}

/// 提交一批已确认的变化。只有全树扫描的最后一批将工作区标为就绪；局部扫描仅提交其范围内的变化。
///
/// # Errors
///
/// 持久化失败或与并发写入冲突时返回错误。
pub fn commit_incremental_batch(
    persistence: &mut SqlitePersistence,
    command: &CaptureLocalSnapshotCommand,
    findings: IncrementalScanFindings,
    is_last: bool,
) -> Result<OperationWithProjection<LocalSnapshotCommitResult>, CaptureLocalSnapshotError> {
    commit_observations(persistence, command, findings, is_last)
}

impl CaptureLocalSnapshotCommand {
    /// 分批提交时每一批用的指令。快照与事件编号各批不同，否则第二批会撞上第一批的主键；
    /// 观测里的文件编号在扫描时已经按整轮编好，拆批不影响它们的唯一性。
    #[must_use]
    pub fn for_batch(&self, index: usize, total: usize) -> Self {
        if total <= 1 {
            return self.clone();
        }
        let suffix = format!(":batch-{}", index + 1);
        Self {
            workspace_id: self.workspace_id.clone(),
            snapshot_id: format!("{}{suffix}", self.snapshot_id),
            event_id: EventId::new(format!("{}{suffix}", self.event_id.as_str())),
            correlation_id: self.correlation_id.clone(),
            observed_at: self.observed_at,
            completed_at: self.completed_at,
        }
    }
}

/// 单个条目的优先索引：在锁外取得它的观测（含摘要），供随后单独提交。
///
/// 用户在全量索引建完之前点开的文件，不应排在整棵树后面。
///
/// # Errors
///
/// 条目时间或大小越界、摘要计算失败时返回错误。
pub(crate) fn observe_single_entry(
    workspace_root: &str,
    command: &CaptureLocalSnapshotCommand,
    entry: WorkspaceScanEntry,
    content_index: &Arc<FileContentIndex>,
) -> Result<IncrementalScanFindings, CaptureLocalSnapshotError> {
    let search_scan = content_index.begin_search_scan()?;
    let observation = local_observation(
        workspace_root,
        command,
        0,
        entry,
        content_index,
        &search_scan,
    )?;
    content_index.flush()?;
    Ok(IncrementalScanFindings {
        search_scan: Some(search_scan),
        observations: vec![observation],
        removed_normalized_paths: Vec::new(),
    })
}

/// 把单个条目的观测提交成一次不完整的快照：只补录这一条，不判断删除，
/// 也不把工作区标成索引完成。要持锁。
///
/// # Errors
///
/// 持久化失败或与并发写入冲突时返回错误。
pub(crate) fn commit_single_entry(
    persistence: &mut SqlitePersistence,
    command: &CaptureLocalSnapshotCommand,
    findings: IncrementalScanFindings,
) -> Result<OperationWithProjection<LocalSnapshotCommitResult>, CaptureLocalSnapshotError> {
    commit_observations(persistence, command, findings, false)
}

fn commit_observations(
    persistence: &mut SqlitePersistence,
    command: &CaptureLocalSnapshotCommand,
    findings: IncrementalScanFindings,
    covers_whole_tree: bool,
) -> Result<OperationWithProjection<LocalSnapshotCommitResult>, CaptureLocalSnapshotError> {
    let generation = persistence
        .transact(|repositories| repositories.next_local_snapshot_generation(&command.workspace_id))
        .map_err(CaptureLocalSnapshotError::Persistence)?;
    let result = persistence
        .transact(|repositories| {
            repositories.commit_local_snapshot(&LocalScanSnapshot {
                snapshot_id: command.snapshot_id.clone(),
                workspace_id: command.workspace_id.clone(),
                generation,
                snapshot_kind: SnapshotKind::Incremental,
                observed_at: command.observed_at,
                completed_at: command.completed_at,
                issue_count: 0,
                event_id: command.event_id.clone(),
                correlation_id: command.correlation_id.clone(),
                observations: findings.observations,
                removed_normalized_paths: findings.removed_normalized_paths,
                covers_whole_tree,
            })
        })
        .map_err(CaptureLocalSnapshotError::Persistence)?;
    drop(findings.search_scan);
    persistence
        .file_content_index()?
        .collect_unused_search_content()?;
    initialize_created_file_states(persistence, command, &result)?;
    let projection = attempt_projection(persistence, &command.workspace_id, command.completed_at);
    Ok(OperationWithProjection {
        business: result,
        projection,
    })
}

/// 这条记录与磁盘上的现状完全一致吗。
///
/// 复用常规扫描的 checksum 缓存判据：它判定可以复用旧摘要，就说明身份、大小、修改时间
/// 三者都没动——那正是"这条不用重写"的定义。
fn is_unchanged(entry: &WorkspaceScanEntry, cached: &WorkspaceFileRecord) -> bool {
    if cached.presence_state != "present" {
        return false;
    }
    let expected_type = match entry.kind {
        WorkspaceEntryKind::File => "file",
        WorkspaceEntryKind::Directory => "directory",
        WorkspaceEntryKind::Symlink => "symlink",
    };
    if cached.file_type != expected_type {
        return false;
    }
    if entry.kind != WorkspaceEntryKind::File {
        // 目录与符号链接没有摘要可比，身份一致即视为未变。
        return cached.file_identity == entry.file_identity;
    }
    let size_bytes = entry.byte_len.and_then(|len| i64::try_from(len).ok());
    let modified_at_fs = entry.modified_at.and_then(|modified_at| {
        modified_at
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
    });
    cached_routine_checksum(entry, size_bytes, modified_at_fs, Some(cached)).is_some()
}

fn initialize_created_file_states(
    persistence: &mut SqlitePersistence,
    command: &CaptureLocalSnapshotCommand,
    result: &LocalSnapshotCommitResult,
) -> Result<(), CaptureLocalSnapshotError> {
    let workspace_kind = persistence
        .transact(|repositories| repositories.workspace(&command.workspace_id))
        .map_err(CaptureLocalSnapshotError::Persistence)?
        .ok_or(CaptureLocalSnapshotError::WorkspaceMissing)?
        .workspace_kind;
    persistence
        .transact(|repositories| {
            let platform_connection = if repositories
                .platform_binding(&command.workspace_id)?
                .is_some()
            {
                PlatformConnection::Bound
            } else {
                PlatformConnection::LocalOnly
            };
            let created_ids = result
                .created_local_file_ids
                .iter()
                .map(String::as_str)
                .collect::<std::collections::HashSet<_>>();
            // 只读这次快照新增的记录：新建的记录首次出现在本快照里。按工作区全量读取的话，
            // 分批写入首次索引时每一批都要把已写入的全部记录再读一遍，越写越慢。
            let files = repositories
                .workspace_files_first_seen_in(&command.workspace_id, &command.snapshot_id)?;
            for file in files.into_iter().filter(|file| {
                created_ids.contains(file.local_file_id.as_str()) && file.file_type == "file"
            }) {
                recompute_file_state(
                    repositories,
                    &RecomputeFileStateCommand {
                        workspace_id: command.workspace_id.clone(),
                        local_file_id: file.local_file_id.clone(),
                        expected_state_revision: None,
                        input: FileStateInput {
                            workspace_kind,
                            platform_connection,
                            platform_state: if platform_connection == PlatformConnection::LocalOnly
                            {
                                PlatformDocumentState::NotApplicable
                            } else {
                                PlatformDocumentState::Unknown
                            },
                            materialization_state: MaterializationState::Materialized,
                            scan_state: LocalScanState::Current,
                            node_kind: LocalNodeKind::File,
                            baseline_path_relation: BaselinePathRelation::Unchanged,
                            baseline_version_id: None,
                            baseline_checksum_algorithm: None,
                            baseline_checksum_value: None,
                            latest_published_version_id: None,
                            latest_published_checksum_algorithm: None,
                            latest_published_checksum_value: None,
                            local_checksum_algorithm: file.checksum_algorithm,
                            local_checksum_value: file.checksum_value,
                            active_confirmation: None,
                            submission_state: SubmissionState::None,
                            active_conflict: None,
                            permissions: FilePermissions::from_allowed([
                                FilePermission::Access,
                                FilePermission::Confirm,
                                FilePermission::Submit,
                                FilePermission::Review,
                            ]),
                            metadata_validity: MetadataValidity::Valid,
                        },
                        confirmation_invalidation_id: format!(
                            "{}:initial-state-invalidation:{}",
                            command.snapshot_id, file.local_file_id
                        ),
                        confirmation_invalidation_event_id: EventId::new(format!(
                            "{}:initial-state-invalidation-event:{}",
                            command.event_id.as_str(),
                            file.local_file_id
                        )),
                        derived_conflict_id: format!(
                            "{}:initial-state-conflict:{}",
                            command.snapshot_id, file.local_file_id
                        ),
                        event_id: EventId::new(format!(
                            "{}:initial-state:{}",
                            command.event_id.as_str(),
                            file.local_file_id
                        )),
                        correlation_id: command.correlation_id.clone(),
                        computed_at: command.completed_at,
                    },
                )?;
            }
            Ok(())
        })
        .map_err(CaptureLocalSnapshotError::Persistence)
}

fn local_observation(
    workspace_root: &str,
    command: &CaptureLocalSnapshotCommand,
    index: usize,
    entry: WorkspaceScanEntry,
    content_index: &FileContentIndex,
    search_scan: &FileSearchScan,
) -> Result<LocalFileObservation, CaptureLocalSnapshotError> {
    let modified_at_fs = entry
        .modified_at
        .map(|modified_at| {
            modified_at
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|duration| i64::try_from(duration.as_millis()).ok())
                .ok_or(CaptureLocalSnapshotError::ObservationOutOfRange)
        })
        .transpose()?;
    let size_bytes = entry
        .byte_len
        .map(i64::try_from)
        .transpose()
        .map_err(|_| CaptureLocalSnapshotError::ObservationOutOfRange)?;
    let checksum =
        if entry.kind == WorkspaceEntryKind::File {
            let (identity, fingerprint) = content_fingerprint(&entry)?;
            let value = content_index.get_or_compute(identity, &fingerprint, || {
                match checksum_workspace_file(
                    Path::new(workspace_root),
                    &entry.path,
                    &WorkspaceChecksumCancellationToken::new(),
                    |_| {},
                )
                .map_err(CaptureLocalSnapshotError::Checksum)?
                {
                    WorkspaceChecksumOutcome::Completed(result) => {
                        verify_content_entry(workspace_root, &entry)?;
                        if Some(result.observation.size_bytes) != entry.byte_len
                            || Some(result.observation.modified_at) != entry.modified_at
                        {
                            return Err(CaptureLocalSnapshotError::SourceChanged);
                        }
                        Ok(result.checksum.value)
                    }
                    WorkspaceChecksumOutcome::Cancelled(_) => {
                        Err(CaptureLocalSnapshotError::Cancelled)
                    }
                }
            })?;
            if search_text::visible_path(&entry) {
                search_scan
                    .index_content(&value, || search_text::read(workspace_root, &entry, &value))?;
            }
            verify_content_entry(workspace_root, &entry)?;
            Some(ChecksumValue {
                algorithm: ChecksumAlgorithm::Sha256,
                value,
            })
        } else {
            None
        };
    let path = entry.path.as_str().to_owned();
    Ok(LocalFileObservation {
        proposed_local_file_id: format!("{}:entry:{}", command.snapshot_id, index + 1),
        relative_path: path.clone(),
        normalized_relative_path: path,
        file_identity: entry.file_identity,
        file_type: match entry.kind {
            WorkspaceEntryKind::File => "file",
            WorkspaceEntryKind::Directory => "directory",
            WorkspaceEntryKind::Symlink => "symlink",
        }
        .to_owned(),
        size_bytes,
        modified_at_fs,
        checksum_algorithm: checksum
            .as_ref()
            .map(|value| value.algorithm.as_str().to_owned()),
        checksum_value: checksum.map(|value| value.value),
    })
}

fn content_fingerprint(
    entry: &WorkspaceScanEntry,
) -> Result<(&str, String), CaptureLocalSnapshotError> {
    let identity = entry
        .file_identity
        .as_deref()
        .ok_or(CaptureLocalSnapshotError::ObservationOutOfRange)?;
    let size = entry
        .byte_len
        .ok_or(CaptureLocalSnapshotError::ObservationOutOfRange)?;
    let modified = entry
        .modified_at
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .ok_or(CaptureLocalSnapshotError::ObservationOutOfRange)?;
    Ok((identity, format!("{size}:{}", modified.as_nanos())))
}

fn verify_content_entry(
    root: &str,
    expected: &WorkspaceScanEntry,
) -> Result<(), CaptureLocalSnapshotError> {
    let actual = inspect_workspace_entry(Path::new(root), &expected.path)
        .map_err(CaptureLocalSnapshotError::EntryUnavailable)?;
    if actual.file_identity != expected.file_identity
        || actual.kind != expected.kind
        || actual.byte_len != expected.byte_len
        || actual.modified_at != expected.modified_at
    {
        return Err(CaptureLocalSnapshotError::SourceChanged);
    }
    Ok(())
}

fn cached_routine_checksum(
    entry: &WorkspaceScanEntry,
    size_bytes: Option<i64>,
    modified_at_fs: Option<i64>,
    cached: Option<&WorkspaceFileRecord>,
) -> Option<ChecksumValue> {
    let cached = cached?;
    if cached.file_type != "file" || cached.presence_state != "present" {
        return None;
    }
    let current = ChecksumCacheFingerprint {
        size_bytes: u64::try_from(size_bytes?).ok()?,
        modified_at: UNIX_EPOCH + Duration::from_millis(u64::try_from(modified_at_fs?).ok()?),
        file_identity: entry.file_identity.clone(),
    };
    let cached_record = ChecksumCacheRecord {
        fingerprint: ChecksumCacheFingerprint {
            size_bytes: u64::try_from(cached.size_bytes?).ok()?,
            modified_at: UNIX_EPOCH
                + Duration::from_millis(u64::try_from(cached.modified_at_fs?).ok()?),
            file_identity: cached.file_identity.clone(),
        },
        checksum: ChecksumValue {
            algorithm: ChecksumAlgorithm::try_from(cached.checksum_algorithm.as_deref()?).ok()?,
            value: cached.checksum_value.clone()?,
        },
    };
    match checksum_cache_decision(ChecksumUseCase::RoutineScan, &current, Some(&cached_record)) {
        ChecksumCacheDecision::Reuse => Some(cached_record.checksum),
        ChecksumCacheDecision::Recompute(_) => None,
    }
}

pub(crate) fn attempt_projection(
    persistence: &mut SqlitePersistence,
    workspace_id: &WorkspaceId,
    generated_at: i64,
) -> ProjectionAttempt {
    let mut writer = WorkspaceProjectionWriter;
    match export_workspace_projection(persistence, &mut writer, workspace_id, generated_at) {
        Ok(outcome) => ProjectionAttempt::Current(outcome),
        Err(error) => ProjectionAttempt::Failed {
            reason: format!("{error:?}"),
        },
    }
}

#[must_use]
pub fn local_workspace_command(
    workspace_id: &str,
    display_name: &str,
    scan_task_id: &str,
    event_id: &str,
    correlation_id: &str,
    created_by_client: &str,
    occurred_at: i64,
) -> CreateLocalWorkspaceCommand {
    CreateLocalWorkspaceCommand {
        workspace_id: WorkspaceId::new(workspace_id),
        display_name: display_name.to_owned(),
        canonical_local_root: String::new(),
        local_root_identity: None,
        scan_task_id: TaskId::new(scan_task_id),
        event_id: EventId::new(event_id),
        correlation_id: correlation_id.to_owned(),
        created_by_client: created_by_client.to_owned(),
        occurred_at,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn index_failures_preserve_root_target_and_ignored_distinctions() {
        let root = std::env::temp_dir().join(format!("ch-index-errors-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("file.txt"), "data").unwrap();
        let relative = contexthub_domain::WorkspaceRelativePath::parse("missing").unwrap();
        let root_error =
            contexthub_workspace_fs::inspect_workspace_entry(&root.join("absent-root"), &relative)
                .unwrap_err();
        let target_error =
            contexthub_workspace_fs::inspect_workspace_entry(&root, &relative).unwrap_err();
        let ignored = contexthub_workspace_fs::inspect_workspace_entry(
            &root,
            &contexthub_domain::WorkspaceRelativePath::parse(".git/config").unwrap(),
        )
        .unwrap_err();
        let not_directory = contexthub_workspace_fs::list_workspace_directory(
            &root,
            Some(&contexthub_domain::WorkspaceRelativePath::parse("file.txt").unwrap()),
        )
        .unwrap_err();
        for (error, reason) in [
            (root_error, "workspace_root_unavailable"),
            (target_error, "workspace_target_unavailable"),
            (ignored, "workspace_target_ignored"),
            (not_directory, "workspace_target_not_directory"),
        ] {
            assert_eq!(
                super::index_failure_reason(&super::CaptureLocalSnapshotError::EntryUnavailable(
                    error
                )),
                reason
            );
        }
        let scan_error = contexthub_workspace_fs::scan_workspace(
            &root.join("file.txt"),
            &contexthub_workspace_fs::WorkspaceCancellationToken::new(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(
            super::index_failure_reason(&super::CaptureLocalSnapshotError::Scan(scan_error)),
            "workspace_root_not_directory"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn index_failures_keep_outside_and_unsupported_targets_separate() {
        let root = std::path::PathBuf::from("/tmp")
            .join(format!("ch-index-targets-{}", uuid::Uuid::new_v4()));
        let inside = root.join("inside");
        let outside = root.join("outside");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("file.txt"), "data").unwrap();
        std::os::unix::fs::symlink(&outside, inside.join("escape")).unwrap();
        let socket = std::os::unix::net::UnixListener::bind(inside.join("special.sock")).unwrap();
        for (path, reason) in [
            ("escape/file.txt", "workspace_path_outside"),
            ("special.sock", "workspace_target_unsupported"),
        ] {
            let error = contexthub_workspace_fs::inspect_workspace_entry(
                &inside,
                &contexthub_domain::WorkspaceRelativePath::parse(path).unwrap(),
            )
            .unwrap_err();
            assert_eq!(
                super::index_failure_reason(&super::CaptureLocalSnapshotError::EntryUnavailable(
                    error
                )),
                reason
            );
        }
        drop(socket);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn filesystem_diagnostics_retain_stage_and_errno_without_raw_inputs() {
        let root =
            std::env::temp_dir().join(format!("ch-index-diagnostic-{}", uuid::Uuid::new_v4()));
        let error = contexthub_workspace_fs::inspect_workspace_entry(
            &root,
            &contexthub_domain::WorkspaceRelativePath::parse("private-name.txt").unwrap(),
        )
        .unwrap_err();
        let diagnostic = super::fs_diagnostic(
            "private-workspace",
            "private-name.txt",
            "scan",
            "workspace_root_unavailable",
            Some(&error),
            std::time::Duration::from_millis(17),
        );
        assert!(diagnostic.contains("stage=scan"));
        assert!(diagnostic.contains("io_kind=NotFound"));
        assert!(diagnostic.contains("error_kind=BrowseRootUnavailable"));
        assert!(diagnostic.contains("elapsed_ms=17"));
        assert!(!diagnostic.contains("errno=none"));
        assert!(!diagnostic.contains("private-workspace"));
        assert!(!diagnostic.contains("private-name.txt"));
        assert!(!diagnostic.contains(root.to_str().unwrap()));
    }

    #[test]
    fn filesystem_diagnostics_follow_scan_and_resolver_io_sources() {
        let root = std::env::temp_dir().join(format!(
            "ch-index-nested-diagnostic-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("file.txt");
        std::fs::write(&file, "data").unwrap();
        let cases = [
            (
                contexthub_workspace_fs::list_workspace_directory(&file, None).unwrap_err(),
                std::fs::read_dir(&file).unwrap_err(),
            ),
            (
                contexthub_workspace_fs::inspect_workspace_entry(
                    &root,
                    &contexthub_domain::WorkspaceRelativePath::parse("missing-parent/child.txt")
                        .unwrap(),
                )
                .unwrap_err(),
                std::fs::canonicalize(root.join("missing-parent")).unwrap_err(),
            ),
        ];
        for (error, expected) in cases {
            let diagnostic = super::fs_diagnostic(
                "workspace",
                "path",
                "read_directory",
                "workspace_target_unavailable",
                Some(&error),
                std::time::Duration::ZERO,
            );
            assert!(diagnostic.contains(&format!("io_kind={:?}", expected.kind())));
            assert!(diagnostic.contains(&format!("errno={}", expected.raw_os_error().unwrap())));
            assert!(diagnostic.contains("error_kind=BrowseTargetUnavailable"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    use super::*;
    use contexthub_application::{TaskRepository, WorkspaceRepository};
    use contexthub_domain::{LocalWorkspaceStatus, TaskState, WorkspaceRelativePath};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn record(path: &str, presence: &str, identity: &str) -> WorkspaceFileRecord {
        WorkspaceFileRecord {
            local_file_id: format!("file-{path}-{presence}"),
            relative_path: path.to_owned(),
            normalized_relative_path: path.to_owned(),
            file_identity: Some(identity.to_owned()),
            file_type: "file".to_owned(),
            baseline_relation: None,
            size_bytes: Some(10),
            modified_at_fs: Some(5),
            presence_state: presence.to_owned(),
            display_state: None,
            local_change_state: None,
            confirmation_state: None,
            conflict_state: None,
            state_revision: Some(0),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some("cached-checksum".to_owned()),
            baseline_version_id: None,
        }
    }

    #[test]
    fn 未变化的文件不再写进增量观测() {
        let entry = WorkspaceScanEntry {
            path: WorkspaceRelativePath::parse("notes/a.md").unwrap(),
            kind: WorkspaceEntryKind::File,
            file_identity: Some("dev:1:ino:2".to_owned()),
            byte_len: Some(10),
            modified_at: Some(UNIX_EPOCH + Duration::from_millis(5)),
        };
        assert!(is_unchanged(
            &entry,
            &record("notes/a.md", "present", "dev:1:ino:2")
        ));

        let mut moved = entry;
        moved.file_identity = Some("dev:1:ino:9".to_owned());
        assert!(!is_unchanged(
            &moved,
            &record("notes/a.md", "present", "dev:1:ino:2")
        ));
    }

    #[test]
    fn 已缺失的旧记录不能当作未变化的依据() {
        // 同一条路径常常既有当前存在的记录，也有历史上标为缺失的旧记录——本机实测
        // 有 19,728 条路径两者并存。拿缺失的那一份去比，每一条都会被判成变化，
        // 增量就退化回全量：实测那一版只把写入从 22,388 行减到 19,797 行。
        let entry = WorkspaceScanEntry {
            path: WorkspaceRelativePath::parse("notes/a.md").unwrap(),
            kind: WorkspaceEntryKind::File,
            file_identity: Some("dev:1:ino:2".to_owned()),
            byte_len: Some(10),
            modified_at: Some(UNIX_EPOCH + Duration::from_millis(5)),
        };
        assert!(!is_unchanged(
            &entry,
            &record("notes/a.md", "missing", "dev:1:ino:2")
        ));
    }

    fn observation(path: &str) -> LocalFileObservation {
        LocalFileObservation {
            proposed_local_file_id: format!("s:{path}"),
            relative_path: path.to_owned(),
            normalized_relative_path: path.to_owned(),
            file_identity: None,
            file_type: "file".to_owned(),
            size_bytes: Some(1),
            modified_at_fs: Some(1),
            checksum_algorithm: None,
            checksum_value: None,
        }
    }

    #[test]
    fn 分批时删除只随最后一批提交() {
        let findings = IncrementalScanFindings {
            search_scan: None,
            observations: (0..5)
                .map(|index| observation(&format!("f{index}")))
                .collect(),
            removed_normalized_paths: vec!["gone".to_owned()],
        };
        let batches = findings.into_batches(2);
        assert_eq!(
            batches
                .iter()
                .map(IncrementalScanFindings::observation_count)
                .collect::<Vec<_>>(),
            vec![2, 2, 1]
        );
        assert!(
            batches[..2]
                .iter()
                .all(|batch| batch.removed_normalized_paths.is_empty())
        );
        assert_eq!(batches[2].removed_normalized_paths, vec!["gone"]);
    }

    #[test]
    fn 没有新增只有删除时仍产出一批() {
        let findings = IncrementalScanFindings {
            search_scan: None,
            observations: Vec::new(),
            removed_normalized_paths: vec!["gone".to_owned()],
        };
        let batches = findings.into_batches(COMMIT_BATCH_SIZE);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].removed_normalized_paths, vec!["gone"]);
    }

    #[test]
    fn 分批指令各批编号不同且单批时保持原样() {
        let command = CaptureLocalSnapshotCommand {
            workspace_id: WorkspaceId::new("w"),
            snapshot_id: "snap".to_owned(),
            event_id: EventId::new("event"),
            correlation_id: "corr".to_owned(),
            observed_at: 1,
            completed_at: 1,
        };
        assert_eq!(command.for_batch(0, 1), command);
        let first = command.for_batch(0, 3);
        let second = command.for_batch(1, 3);
        assert_ne!(first.snapshot_id, second.snapshot_id);
        assert_ne!(first.event_id, second.event_id);
        assert_eq!(first.correlation_id, "corr");
    }

    #[test]
    fn routine_scan_reuses_checksum_only_for_unchanged_stable_file() {
        let entry = WorkspaceScanEntry {
            path: WorkspaceRelativePath::parse("notes/a.md").unwrap(),
            kind: WorkspaceEntryKind::File,
            file_identity: Some("dev:1:ino:2".to_owned()),
            byte_len: Some(10),
            modified_at: Some(UNIX_EPOCH + Duration::from_millis(5)),
        };
        let cached = WorkspaceFileRecord {
            local_file_id: "file-1".to_owned(),
            relative_path: "notes/a.md".to_owned(),
            normalized_relative_path: "notes/a.md".to_owned(),
            file_identity: Some("dev:1:ino:2".to_owned()),
            file_type: "file".to_owned(),
            baseline_relation: None,
            size_bytes: Some(10),
            modified_at_fs: Some(5),
            presence_state: "present".to_owned(),
            display_state: None,
            local_change_state: None,
            confirmation_state: None,
            conflict_state: None,
            state_revision: Some(0),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some("cached-checksum".to_owned()),
            baseline_version_id: None,
        };

        assert_eq!(
            cached_routine_checksum(&entry, Some(10), Some(5), Some(&cached)),
            Some(ChecksumValue {
                algorithm: ChecksumAlgorithm::Sha256,
                value: "cached-checksum".to_owned(),
            })
        );
        let mut changed = entry;
        changed.file_identity = Some("dev:1:ino:3".to_owned());
        assert_eq!(
            cached_routine_checksum(&changed, Some(10), Some(5), Some(&cached)),
            None
        );
    }

    #[test]
    fn app_server_creates_local_workspace_and_queued_scan_after_preflight() {
        let fixture = TestFixture::new("create");
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        let workspace = create_preflighted_local_workspace(
            &mut persistence,
            &fixture.workspace,
            std::slice::from_ref(&fixture.data),
            local_workspace_command("w1", "Workspace", "scan1", "event1", "corr1", "desktop", 1),
        )
        .unwrap();
        assert_eq!(
            workspace.business.local_status,
            LocalWorkspaceStatus::Scanning
        );
        assert_eq!(
            workspace.business.local_root,
            fs::canonicalize(&fixture.workspace)
                .unwrap()
                .to_str()
                .unwrap()
        );
        assert!(matches!(
            workspace.projection,
            ProjectionAttempt::Current(_)
        ));
        assert!(fixture.workspace.join(".cilai/export-state.json").is_file());
        persistence
            .transact(|repositories| {
                let task = repositories.task(&TaskId::new("scan1"))?.unwrap();
                assert_eq!(task.state, TaskState::Queued);
                assert_eq!(task.workspace_id.as_ref().unwrap().as_str(), "w1");
                assert_eq!(repositories.workspaces()?.len(), 1);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn 已有云端工作区时仍能连接本地文件夹() {
        // 云端工作区的根记录的是云端标识而不是磁盘路径。重叠检查若把它当成本地目录去
        // 解析，就会报"已连接的文件夹读不到"，任何本地文件夹都连不上。
        let fixture = TestFixture::new("cloud-neighbour");
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        persistence
            .transact(|repositories| {
                repositories.insert_workspace(&contexthub_application::WorkspaceRecord {
                    workspace_id: WorkspaceId::new("cloud-1"),
                    workspace_kind: WorkspaceKind::CloudTeam,
                    display_name: "团队空间".to_owned(),
                    local_root: "cilai-cloud://team-space-1".to_owned(),
                    local_root_identity: None,
                    local_status: LocalWorkspaceStatus::Ready,
                    state_revision: 0,
                    last_local_scan_at: None,
                    created_at: 1,
                    updated_at: 1,
                })
            })
            .unwrap();
        let workspace = create_preflighted_local_workspace(
            &mut persistence,
            &fixture.workspace,
            std::slice::from_ref(&fixture.data),
            local_workspace_command("w1", "Workspace", "scan1", "event1", "corr1", "desktop", 1),
        )
        .unwrap();
        assert_eq!(workspace.business.workspace_id.as_str(), "w1");
    }

    #[test]
    fn app_server_rejects_duplicate_and_data_directory_overlap_before_writing() {
        let fixture = TestFixture::new("conflicts");
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        create_preflighted_local_workspace(
            &mut persistence,
            &fixture.workspace,
            std::slice::from_ref(&fixture.data),
            local_workspace_command("w1", "One", "scan1", "event1", "corr1", "desktop", 1),
        )
        .unwrap();
        let duplicate = create_preflighted_local_workspace(
            &mut persistence,
            &fixture.workspace,
            std::slice::from_ref(&fixture.data),
            local_workspace_command("w2", "Two", "scan2", "event2", "corr2", "desktop", 2),
        )
        .unwrap();
        assert_eq!(duplicate.business.workspace_id.as_str(), "w1");
        let environment = create_preflighted_local_workspace(
            &mut persistence,
            &fixture.data,
            std::slice::from_ref(&fixture.data),
            local_workspace_command("w3", "Three", "scan3", "event3", "corr3", "desktop", 3),
        )
        .unwrap_err();
        assert!(matches!(environment, CreateLocalWorkspaceError::Root(_)));
        persistence
            .transact(|repositories| {
                assert_eq!(repositories.workspaces()?.len(), 1);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn projection_conflict_is_reported_separately_after_workspace_business_commit() {
        let fixture = TestFixture::new("projection-conflict");
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        fs::write(fixture.workspace.join(".cilai"), "reserved conflict").unwrap();
        let result = create_preflighted_local_workspace(
            &mut persistence,
            &fixture.workspace,
            std::slice::from_ref(&fixture.data),
            local_workspace_command("w1", "Workspace", "scan1", "event1", "corr1", "desktop", 1),
        )
        .unwrap();
        assert_eq!(result.business.workspace_id.as_str(), "w1");
        assert!(matches!(
            result.projection,
            ProjectionAttempt::Failed { .. }
        ));
        persistence
            .transact(|repositories| {
                assert!(repositories.workspace(&WorkspaceId::new("w1"))?.is_some());
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn app_server_commits_complete_scans_and_retains_identity_after_move() {
        let fixture = TestFixture::new("snapshot");
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        create_preflighted_local_workspace(
            &mut persistence,
            &fixture.workspace,
            std::slice::from_ref(&fixture.data),
            local_workspace_command("w1", "Workspace", "scan1", "event1", "corr1", "desktop", 1),
        )
        .unwrap();
        fs::create_dir(fixture.workspace.join("notes")).unwrap();
        fs::write(fixture.workspace.join("notes/a.md"), "first").unwrap();
        let first = capture_full_local_snapshot(
            &mut persistence,
            &CaptureLocalSnapshotCommand {
                workspace_id: WorkspaceId::new("w1"),
                snapshot_id: "snapshot-1".to_owned(),
                event_id: EventId::new("snapshot-event-1"),
                correlation_id: "snapshot-correlation-1".to_owned(),
                observed_at: 10,
                completed_at: 11,
            },
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
        assert_eq!(first.business.created_local_file_ids.len(), 2);
        persistence
            .transact(|repositories| {
                let files = repositories.workspace_files(&WorkspaceId::new("w1"))?;
                let note = files
                    .iter()
                    .find(|file| file.relative_path == "notes/a.md")
                    .expect("scanned note");
                assert_eq!(note.local_change_state.as_deref(), Some("added"));
                assert_eq!(note.confirmation_state.as_deref(), Some("not_required"));
                assert_eq!(note.display_state.as_deref(), Some("local_ready"));
                assert_eq!(note.state_revision, Some(0));
                Ok(())
            })
            .unwrap();

        fs::rename(
            fixture.workspace.join("notes"),
            fixture.workspace.join("archive"),
        )
        .unwrap();
        let second = capture_full_local_snapshot(
            &mut persistence,
            &CaptureLocalSnapshotCommand {
                workspace_id: WorkspaceId::new("w1"),
                snapshot_id: "snapshot-2".to_owned(),
                event_id: EventId::new("snapshot-event-2"),
                correlation_id: "snapshot-correlation-2".to_owned(),
                observed_at: 20,
                completed_at: 21,
            },
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
        assert_eq!(second.business.retained_local_file_ids.len(), 2);
        assert!(second.business.created_local_file_ids.is_empty());
        assert!(second.business.missing_local_file_ids.is_empty());
    }

    #[test]
    fn nested_workspaces_share_content_and_only_changed_paths_are_updated() {
        let fixture = TestFixture::new("shared-content");
        let child = fixture.workspace.join("docs");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("a.txt"), "original").unwrap();
        fs::write(fixture.workspace.join("b.txt"), "untouched").unwrap();
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        for (id, root) in [("parent", &fixture.workspace), ("child", &child)] {
            connect_and_snapshot(&mut persistence, &fixture, id, root);
        }
        assert_eq!(shared_content_count(&fixture.database), 2);
        let parent = persistence
            .transact(|repo| repo.present_workspace_files(&WorkspaceId::new("parent")))
            .unwrap();
        let child_files = persistence
            .transact(|repo| repo.present_workspace_files(&WorkspaceId::new("child")))
            .unwrap();
        assert_eq!(
            parent
                .iter()
                .find(|file| file.relative_path == "docs/a.txt")
                .unwrap()
                .checksum_value,
            child_files
                .iter()
                .find(|file| file.relative_path == "a.txt")
                .unwrap()
                .checksum_value
        );

        fs::write(child.join("a.txt"), "changed content").unwrap();
        let basis =
            prepare_incremental_scan(&mut persistence, &WorkspaceId::new("parent")).unwrap();
        let findings = observe_changed_paths(
            &basis,
            &snapshot_command("parent", 2),
            &["docs/a.txt".to_owned()],
            &WorkspaceCancellationToken::new(),
            |_| {},
        )
        .unwrap();
        assert_eq!(findings.observations.len(), 1);
        assert_eq!(findings.observations[0].relative_path, "docs/a.txt");
        assert!(findings.removed_normalized_paths.is_empty());
        commit_incremental_batch(
            &mut persistence,
            &snapshot_command("parent", 2),
            findings,
            false,
        )
        .unwrap();

        fs::rename(&child, fixture.workspace.join("archive")).unwrap();
        let basis =
            prepare_incremental_scan(&mut persistence, &WorkspaceId::new("parent")).unwrap();
        let findings = observe_changed_paths(
            &basis,
            &snapshot_command("parent", 3),
            &["docs".to_owned(), "archive".to_owned()],
            &WorkspaceCancellationToken::new(),
            |_| {},
        )
        .unwrap();
        assert!(
            findings
                .observations
                .iter()
                .all(|entry| entry.relative_path.starts_with("archive"))
        );
        assert!(
            findings
                .removed_normalized_paths
                .contains(&"docs/a.txt".to_owned())
        );
        assert!(
            !findings
                .removed_normalized_paths
                .contains(&"b.txt".to_owned())
        );
        commit_incremental_batch(
            &mut persistence,
            &snapshot_command("parent", 3),
            findings,
            false,
        )
        .unwrap();
        let files = persistence
            .transact(|repo| repo.present_workspace_files(&WorkspaceId::new("parent")))
            .unwrap();
        assert!(files.iter().any(|file| file.relative_path == "b.txt"));
        assert!(
            files
                .iter()
                .any(|file| file.relative_path == "archive/a.txt")
        );
        assert!(
            !files
                .iter()
                .any(|file| file.relative_path.starts_with("docs/"))
        );
    }

    fn search_text_hits(
        persistence: &mut SqlitePersistence,
        query: &str,
    ) -> Vec<contexthub_protocol::GlobalSearchItem> {
        crate::global_search::query(
            persistence,
            &contexthub_protocol::GlobalSearchParams {
                query: query.to_owned(),
                limit: 100,
            },
        )
        .unwrap()
        .items
    }

    #[test]
    fn file_text_search_tracks_content_move_delete_and_literal_queries() {
        let fixture = TestFixture::new("full-text");
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        fs::write(
            fixture.workspace.join("a.md"),
            "第一段：独有正文关键词。成本100%_done，MixedCase。\n",
        )
        .unwrap();
        fs::write(fixture.workspace.join(".hidden.md"), "秘密搜索词").unwrap();
        fs::write(fixture.workspace.join("b.bin"), b"\x00binary-marker").unwrap();
        connect_and_snapshot(&mut persistence, &fixture, "w1", &fixture.workspace);
        for query in ["正文", "独有正文关键词", "100%_done", "mixedcase"] {
            let hits = search_text_hits(&mut persistence, query);
            assert_eq!(hits.len(), 1, "{query}");
            assert_eq!(hits[0].relative_path.as_deref(), Some("a.md"));
            assert!(hits[0].subtitle.contains("独有正文关键词"));
        }
        assert_eq!(search_text_hits(&mut persistence, "%_").len(), 1);
        assert!(search_text_hits(&mut persistence, "秘密搜索词").is_empty());
        assert!(search_text_hits(&mut persistence, "binary-marker").is_empty());
        let before = search_text_hits(&mut persistence, "独有正文关键词")[0]
            .object_id
            .clone();
        fs::rename(
            fixture.workspace.join("a.md"),
            fixture.workspace.join("renamed.md"),
        )
        .unwrap();
        capture_full_local_snapshot(
            &mut persistence,
            &snapshot_command("w1", 2),
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
        let moved = search_text_hits(&mut persistence, "独有正文关键词");
        assert_eq!(moved[0].object_id, before);
        assert_eq!(moved[0].relative_path.as_deref(), Some("renamed.md"));
        fs::write(fixture.workspace.join("renamed.md"), "全新正文内容").unwrap();
        let basis = prepare_incremental_scan(&mut persistence, &WorkspaceId::new("w1")).unwrap();
        let command = snapshot_command("w1", 3);
        let findings = observe_changed_paths(
            &basis,
            &command,
            &["renamed.md".to_owned()],
            &WorkspaceCancellationToken::new(),
            |_| {},
        )
        .unwrap();
        commit_incremental_snapshot(&mut persistence, &command, findings).unwrap();
        assert!(search_text_hits(&mut persistence, "独有正文关键词").is_empty());
        assert_eq!(search_text_hits(&mut persistence, "全新正文").len(), 1);
        let old_checksum = format!(
            "{:x}",
            Sha256::digest("第一段：独有正文关键词。成本100%_done，MixedCase。\n".as_bytes())
        );
        assert!(
            !persistence
                .file_content_index()
                .unwrap()
                .has_search_content(&old_checksum)
                .unwrap()
        );
        drop(basis);
        drop(persistence);
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        assert_eq!(search_text_hits(&mut persistence, "全新正文").len(), 1);
        fs::remove_file(fixture.workspace.join("renamed.md")).unwrap();
        assert!(search_text_hits(&mut persistence, "全新正文").is_empty());
        capture_full_local_snapshot(
            &mut persistence,
            &snapshot_command("w1", 4),
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
        assert!(search_text_hits(&mut persistence, "全新正文").is_empty());
        fs::write(
            fixture.workspace.join("restored.md"),
            "第一段：独有正文关键词。成本100%_done，MixedCase。\n",
        )
        .unwrap();
        capture_full_local_snapshot(
            &mut persistence,
            &snapshot_command("w1", 5),
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
        assert_eq!(
            search_text_hits(&mut persistence, "独有正文关键词").len(),
            1
        );
    }

    #[test]
    fn file_text_search_backfills_existing_metadata_without_rehashing_and_classifies_limits() {
        let fixture = TestFixture::new("full-text-backfill");
        fs::write(
            fixture.workspace.join("document.txt"),
            "\u{feff}旧版迁移正文",
        )
        .unwrap();
        fs::write(
            fixture.workspace.join("encoding.txt"),
            [0xff, 0xfe, 0x01, 0x02],
        )
        .unwrap();
        fs::write(
            fixture.workspace.join("large.txt"),
            vec![b'x'; usize::try_from(search_text::MAX_SEARCH_TEXT_BYTES).unwrap() + 1],
        )
        .unwrap();
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        connect_and_snapshot(&mut persistence, &fixture, "w1", &fixture.workspace);
        drop(persistence);
        let connection = rusqlite::Connection::open(&fixture.database).unwrap();
        let statuses: Vec<String> = connection
            .prepare("SELECT status FROM file_search_documents ORDER BY status")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(statuses, ["indexed", "too_large", "unsupported_encoding"]);
        connection
            .execute("DELETE FROM file_search_text", [])
            .unwrap();
        connection
            .execute("DELETE FROM file_search_documents", [])
            .unwrap();
        drop(connection);
        let mut persistence = SqlitePersistence::open(&fixture.database, "0.1.0").unwrap();
        assert!(search_text_hits(&mut persistence, "迁移正文").is_empty());
        let basis = prepare_incremental_scan(&mut persistence, &WorkspaceId::new("w1")).unwrap();
        let command = snapshot_command("w1", 2);
        let findings = observe_incremental_changes(
            &basis,
            &command,
            &WorkspaceCancellationToken::new(),
            |_| {},
        )
        .unwrap();
        commit_incremental_snapshot(&mut persistence, &command, findings).unwrap();
        assert_eq!(search_text_hits(&mut persistence, "迁移正文").len(), 1);
        let basis = prepare_incremental_scan(&mut persistence, &WorkspaceId::new("w1")).unwrap();
        let findings = observe_incremental_changes(
            &basis,
            &snapshot_command("w1", 3),
            &WorkspaceCancellationToken::new(),
            |_| {},
        )
        .unwrap();
        assert!(findings.observations.is_empty());
    }

    fn shared_content_count(database: &Path) -> i64 {
        rusqlite::Connection::open(database)
            .unwrap()
            .query_row(
                "SELECT count(*) FROM shared_file_content_index",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn connect_and_snapshot(
        persistence: &mut SqlitePersistence,
        fixture: &TestFixture,
        id: &str,
        root: &Path,
    ) {
        create_preflighted_local_workspace(
            persistence,
            root,
            std::slice::from_ref(&fixture.data),
            local_workspace_command(
                id,
                id,
                &format!("scan-{id}"),
                &format!("event-{id}"),
                id,
                "desktop",
                1,
            ),
        )
        .unwrap();
        capture_full_local_snapshot(
            persistence,
            &snapshot_command(id, 1),
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
    }

    fn snapshot_command(workspace_id: &str, number: i64) -> CaptureLocalSnapshotCommand {
        CaptureLocalSnapshotCommand {
            workspace_id: WorkspaceId::new(workspace_id),
            snapshot_id: format!("snapshot-{workspace_id}-{number}"),
            event_id: EventId::new(format!("snapshot-event-{workspace_id}-{number}")),
            correlation_id: format!("snapshot-correlation-{workspace_id}-{number}"),
            observed_at: number * 10,
            completed_at: number * 10 + 1,
        }
    }

    struct TestFixture {
        parent: PathBuf,
        data: PathBuf,
        database: PathBuf,
        workspace: PathBuf,
    }

    impl TestFixture {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let parent = std::env::temp_dir().join(format!(
                "ch-local-workspace-{label}-{}-{nanos}",
                std::process::id()
            ));
            let data = parent.join("data");
            let workspace = parent.join("workspace");
            fs::create_dir_all(&data).unwrap();
            fs::create_dir(&workspace).unwrap();
            let database = data.join("contexthub.db");
            Self {
                parent,
                data,
                database,
                workspace,
            }
        }
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.parent).unwrap();
        }
    }
}
