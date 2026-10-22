//! 逐层浏览与后台索引的请求入口。
//!
//! 这几条请求都只在读取工作区登记信息、提交结果时短暂持有服务主锁；读目录、算摘要
//! 这些慢步骤都在锁外。查索引进度完全不碰服务主锁。

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use contexthub_application::{UnitOfWork, WorkspaceRepository};
use contexthub_domain::{EventId, WorkspaceId, WorkspaceRelativePath};
use contexthub_protocol::{
    WorkspaceDirectoryEntry, WorkspaceIndexPathParams, WorkspaceIndexPathResult,
    WorkspaceIndexRequestParams, WorkspaceIndexRequestResult, WorkspaceIndexStatusParams,
    WorkspaceIndexStatusResult, WorkspaceListDirectoryParams, WorkspaceListDirectoryResult,
    business_error, success,
};
use contexthub_workspace_fs::{
    WorkspaceBrowseError, WorkspaceBrowseErrorKind, WorkspaceEntryKind, WorkspaceScanEntry,
    inspect_workspace_entry, list_workspace_directory,
};
use serde_json::{Value, json};

use crate::workspace_index::WorkspaceIndexBoard;
use crate::{
    AppServerServices, ConnectionRouter, ServiceError, invalid_params, parse_params,
    unix_epoch_millis,
};

pub(crate) const MAX_DIRECTORY_READS: usize = 4;

#[derive(Default)]
pub(crate) struct DirectoryReads {
    active: Mutex<std::collections::HashSet<(String, String)>>,
}

struct DirectoryReadLease {
    reads: Arc<DirectoryReads>,
    key: (String, String),
}

impl Drop for DirectoryReadLease {
    fn drop(&mut self) {
        self.reads
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.key);
    }
}

pub(crate) struct PreparedDirectoryRead {
    root: String,
    lease: DirectoryReadLease,
}

#[cfg(test)]
struct DirectoryReadGate {
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    released: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

#[cfg(test)]
type DirectoryReadGates =
    std::sync::Mutex<std::collections::HashMap<String, Arc<DirectoryReadGate>>>;

#[cfg(test)]
static DIRECTORY_READ_GATES: std::sync::LazyLock<DirectoryReadGates> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

#[cfg(test)]
pub(crate) struct DirectoryReadPause {
    root: String,
    release: std::sync::mpsc::Sender<()>,
}

#[cfg(test)]
impl Drop for DirectoryReadPause {
    fn drop(&mut self) {
        DIRECTORY_READ_GATES.lock().unwrap().remove(&self.root);
        let _ = self.release.send(());
    }
}

#[cfg(test)]
pub(crate) fn pause_directory_read(
    root: String,
) -> (DirectoryReadPause, tokio::sync::mpsc::UnboundedReceiver<()>) {
    let (entered, receiver) = tokio::sync::mpsc::unbounded_channel();
    let (release, released) = std::sync::mpsc::channel();
    DIRECTORY_READ_GATES.lock().unwrap().insert(
        root.clone(),
        Arc::new(DirectoryReadGate {
            entered,
            released: std::sync::Mutex::new(released),
        }),
    );
    (DirectoryReadPause { root, release }, receiver)
}

/// 本地工作区的根目录。云端工作区没有本地目录可浏览，明确拒绝。
fn local_workspace_root(
    router: &ConnectionRouter,
    workspace_id: &str,
) -> Result<String, ServiceError> {
    let services = router
        .services
        .as_ref()
        .ok_or_else(|| ServiceError::new("app_service_unavailable"))?;
    let mut services = services
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    services.local_workspace_root(workspace_id)
}

impl AppServerServices {
    pub(crate) fn prepare_workspace_list_directory(
        &mut self,
        params: &WorkspaceListDirectoryParams,
        _now: i64,
    ) -> Result<PreparedDirectoryRead, ServiceError> {
        let root = self.local_workspace_root(&params.workspace_id)?;
        let relative = params
            .relative_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .map(parse_relative)
            .transpose()?;
        let key = (
            params.workspace_id.clone(),
            relative
                .as_ref()
                .map_or_else(String::new, |path| path.as_str().to_owned()),
        );
        let mut active = self
            .directory_reads
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.contains(&key) {
            return Err(ServiceError::new("workspace_directory_busy"));
        }
        if active.len() >= MAX_DIRECTORY_READS {
            return Err(ServiceError::new("workspace_directory_capacity_reached"));
        }
        active.insert(key.clone());
        Ok(PreparedDirectoryRead {
            root,
            lease: DirectoryReadLease {
                reads: Arc::clone(&self.directory_reads),
                key,
            },
        })
    }

    fn local_workspace_root(&mut self, workspace_id: &str) -> Result<String, ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let workspace = persistence
            .transact(|repositories| repositories.workspace(&WorkspaceId::new(workspace_id)))
            .map_err(|_| ServiceError::new("workspace_state_failed"))?
            .ok_or_else(|| ServiceError::new("workspace_not_found"))?;
        if workspace.workspace_kind.as_str() != "local" {
            return Err(ServiceError::new("workspace_kind_unsupported"));
        }
        Ok(workspace.local_root)
    }
}

const fn browse_reason(error: &WorkspaceBrowseError) -> &'static str {
    match error.kind() {
        WorkspaceBrowseErrorKind::RootUnavailable => "workspace_root_unavailable",
        WorkspaceBrowseErrorKind::TargetUnavailable => "workspace_target_unavailable",
        WorkspaceBrowseErrorKind::OutsideWorkspace => "workspace_path_outside",
        WorkspaceBrowseErrorKind::NotDirectory => "workspace_target_not_directory",
        WorkspaceBrowseErrorKind::Ignored => "workspace_target_ignored",
        WorkspaceBrowseErrorKind::UnsupportedFileType => "workspace_target_unsupported",
    }
}

fn parse_relative(path: &str) -> Result<WorkspaceRelativePath, ServiceError> {
    WorkspaceRelativePath::parse(path).map_err(|_| ServiceError::new("workspace_path_invalid"))
}

const fn kind_label(kind: WorkspaceEntryKind) -> &'static str {
    match kind {
        WorkspaceEntryKind::File => "file",
        WorkspaceEntryKind::Directory => "directory",
        WorkspaceEntryKind::Symlink => "symlink",
    }
}

fn millis(entry: &WorkspaceScanEntry) -> Option<i64> {
    entry
        .modified_at
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn directory_entry(entry: &WorkspaceScanEntry) -> WorkspaceDirectoryEntry {
    let relative_path = entry.path.as_str().to_owned();
    WorkspaceDirectoryEntry {
        name: entry
            .path
            .segments()
            .last()
            .unwrap_or(relative_path.as_str())
            .to_owned(),
        relative_path,
        file_type: kind_label(entry.kind).to_owned(),
        size_bytes: entry.byte_len.and_then(|len| i64::try_from(len).ok()),
        modified_at: millis(entry),
    }
}

/// 读一层目录。目录排在前面，同类按名称排序，与文件树的展示顺序一致。
pub(crate) fn list_directory(
    workspace_root: &str,
    params: &WorkspaceListDirectoryParams,
) -> Result<WorkspaceListDirectoryResult, ServiceError> {
    #[cfg(test)]
    let read_gate = DIRECTORY_READ_GATES
        .lock()
        .unwrap()
        .get(workspace_root)
        .cloned();
    #[cfg(test)]
    if let Some(gate) = read_gate {
        let _ = gate.entered.send(());
        let _ = gate.released.lock().unwrap().recv();
    }
    let directory = params
        .relative_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(parse_relative)
        .transpose()?;
    let started = Instant::now();
    let listing = list_workspace_directory(Path::new(workspace_root), directory.as_ref()).map_err(
        |error| {
            crate::workspace::log_fs_diagnostic(
                &params.workspace_id,
                params.relative_path.as_deref().unwrap_or_default(),
                "read_directory",
                browse_reason(&error),
                Some(&error),
                started.elapsed(),
            );
            ServiceError::new(browse_reason(&error))
        },
    )?;
    let mut entries = listing
        .entries
        .iter()
        .map(directory_entry)
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        (left.file_type != "directory")
            .cmp(&(right.file_type != "directory"))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(WorkspaceListDirectoryResult {
        workspace_id: params.workspace_id.clone(),
        relative_path: directory.map(|path| path.as_str().to_owned()),
        entries,
        issue_count: i64::try_from(listing.issues.len()).unwrap_or(i64::MAX),
    })
}

pub(crate) fn execute_directory_read(
    prepared: PreparedDirectoryRead,
    params: &WorkspaceListDirectoryParams,
) -> Result<WorkspaceListDirectoryResult, ServiceError> {
    let started = Instant::now();
    crate::workspace::log_fs_diagnostic(
        &prepared.lease.key.0,
        &prepared.lease.key.1,
        "read_directory_start",
        "started",
        None,
        started.elapsed(),
    );
    let result = list_directory(&prepared.root, params);
    crate::workspace::log_fs_diagnostic(
        &prepared.lease.key.0,
        &prepared.lease.key.1,
        "read_directory_finished",
        result
            .as_ref()
            .map_or_else(|error| error.reason, |_| "completed"),
        None,
        started.elapsed(),
    );
    drop(prepared);
    result
}

fn respond<T: serde::Serialize>(
    id: contexthub_protocol::RequestId,
    message: &str,
    result: Result<T, ServiceError>,
) -> String {
    match result {
        Ok(value) => success(id, &value).expect("workspace browse result serializes"),
        Err(error) => business_error(id, error.reason, message, json!({})),
    }
}

impl ConnectionRouter {
    fn index_board(&mut self) -> Result<Arc<WorkspaceIndexBoard>, ServiceError> {
        if let Some(board) = &self.index_board {
            return Ok(Arc::clone(board));
        }
        let board = self
            .services
            .as_ref()
            .ok_or_else(|| ServiceError::new("app_service_unavailable"))?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .index_board();
        self.index_board = Some(Arc::clone(&board));
        Ok(board)
    }

    pub(crate) fn route_workspace_list_directory(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<WorkspaceListDirectoryParams>(params) else {
            return invalid_params(id, "invalid_method_params");
        };
        let result = self
            .services
            .as_ref()
            .ok_or_else(|| ServiceError::new("app_service_unavailable"))
            .and_then(|services| {
                services
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .prepare_workspace_list_directory(&params, 0)
            })
            .and_then(|prepared| execute_directory_read(prepared, &params));
        respond(id, "Workspace directory listing failed", result)
    }

    pub(crate) fn route_workspace_index_request(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<WorkspaceIndexRequestParams>(params) else {
            return invalid_params(id, "invalid_method_params");
        };
        let result = local_workspace_root(self, &params.workspace_id)
            .and_then(|_| self.index_board())
            .and_then(|board| board.request(&params.workspace_id))
            .map(|()| WorkspaceIndexRequestResult {
                workspace_id: params.workspace_id.clone(),
            });
        respond(id, "Workspace index request failed", result)
    }

    pub(crate) fn route_workspace_index_status(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<WorkspaceIndexStatusParams>(params) else {
            return invalid_params(id, "invalid_method_params");
        };
        let result = self.index_board().map(|board| {
            let progress = board.progress(&params.workspace_id);
            WorkspaceIndexStatusResult {
                workspace_id: params.workspace_id.clone(),
                phase: progress.phase.as_str().to_owned(),
                entries_examined: i64::try_from(progress.entries_examined).unwrap_or(i64::MAX),
                files_hashed: i64::try_from(progress.files_hashed).unwrap_or(i64::MAX),
                files_to_hash: i64::try_from(progress.files_to_hash).unwrap_or(i64::MAX),
                batches_committed: i64::try_from(progress.batches_committed).unwrap_or(i64::MAX),
                batches_total: i64::try_from(progress.batches_total).unwrap_or(i64::MAX),
                reason: progress.reason,
            }
        });
        respond(id, "Workspace index status failed", result)
    }

    /// 优先索引单个条目：锁内读根目录，锁外检查并算摘要，锁内只提交这一条。
    pub(crate) fn route_workspace_index_path(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<WorkspaceIndexPathParams>(params) else {
            return invalid_params(id, "invalid_method_params");
        };
        let result = self.index_path(&params);
        respond(id, "Workspace path indexing failed", result)
    }

    fn index_path(
        &self,
        params: &WorkspaceIndexPathParams,
    ) -> Result<WorkspaceIndexPathResult, ServiceError> {
        let now = unix_epoch_millis().map_err(ServiceError::new)?;
        let path = parse_relative(&params.relative_path)?;
        let root = local_workspace_root(self, &params.workspace_id)?;
        let entry = inspect_workspace_entry(Path::new(&root), &path)
            .map_err(|error| ServiceError::new(browse_reason(&error)))?;
        let unique = uuid::Uuid::new_v4();
        let command = crate::workspace::CaptureLocalSnapshotCommand {
            workspace_id: WorkspaceId::new(params.workspace_id.clone()),
            snapshot_id: format!("path-snapshot-{unique}"),
            event_id: EventId::new(format!("path-snapshot-event-{unique}")),
            correlation_id: format!("path-snapshot-correlation-{unique}"),
            observed_at: now,
            completed_at: now,
        };
        let content_index = self
            .services
            .as_ref()
            .ok_or_else(|| ServiceError::new("app_service_unavailable"))?
            .lock()
            .map_err(|_| ServiceError::new("workspace_index_unavailable"))?
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .file_content_index()
            .map_err(|_| ServiceError::new("workspace_index_write_failed"))?;
        let findings =
            crate::workspace::observe_single_entry(&root, &command, entry, &content_index)
                .map_err(|error| {
                    ServiceError::new(crate::workspace::index_failure_reason(&error))
                })?;
        let committed = {
            let services = self
                .services
                .as_ref()
                .ok_or_else(|| ServiceError::new("app_service_unavailable"))?;
            let mut services = services
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let persistence = services
                .persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
            crate::workspace::commit_single_entry(persistence, &command, findings).map_err(
                |error| ServiceError::new(crate::workspace::index_failure_reason(&error)),
            )?
        };
        let local_file_id = committed
            .business
            .created_local_file_ids
            .into_iter()
            .chain(committed.business.retained_local_file_ids)
            .next()
            .ok_or_else(|| ServiceError::new("workspace_index_write_failed"))?;
        Ok(WorkspaceIndexPathResult {
            workspace_id: params.workspace_id.clone(),
            local_file_id,
            relative_path: path.as_str().to_owned(),
        })
    }
}
