//! 工作区后台索引的进度与调度入口。
//!
//! 索引由文件监听主管在服务主锁之外建立（见 `workspace_watch`）。这里记下每个工作区
//! 走到了哪一步，并把"现在就建一次"的请求递给监听主管。它有自己的小锁，读进度不碰
//! 服务主锁——主锁在提交索引时会被占住一段时间，进度查询若也排在后面，界面就看不到
//! 任何进展。

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::mpsc::UnboundedSender;

use crate::ServiceError;

/// 一个工作区的索引所处阶段。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexPhase {
    /// 没有正在进行的索引。
    Idle,
    /// 已请求，等监听主管排到它。
    Queued,
    /// 正在遍历目录。
    Scanning,
    /// 目录已走完，正在为有变化的文件计算摘要。
    Hashing,
    /// 正在把结果写进索引。
    Committing,
    /// 上一次索引失败，原因见 `reason`。
    Failed,
}

impl IndexPhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Queued => "queued",
            Self::Scanning => "scanning",
            Self::Hashing => "hashing",
            Self::Committing => "committing",
            Self::Failed => "failed",
        }
    }
}

/// 某个工作区此刻的索引进度。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexProgress {
    pub phase: IndexPhase,
    /// 目录遍历已经检查过的条目数。
    pub entries_examined: u64,
    /// 摘要阶段已完成的文件数。
    pub files_hashed: u64,
    /// 摘要阶段需要处理的文件总数。
    pub files_to_hash: u64,
    /// 写入阶段已提交的批数。
    pub batches_committed: u64,
    /// 写入阶段总批数。
    pub batches_total: u64,
    /// 失败原因代码，仅 `Failed` 时有值。
    pub reason: Option<String>,
}

impl IndexProgress {
    const fn idle() -> Self {
        Self {
            phase: IndexPhase::Idle,
            entries_examined: 0,
            files_hashed: 0,
            files_to_hash: 0,
            batches_committed: 0,
            batches_total: 0,
            reason: None,
        }
    }

    const fn with_phase(phase: IndexPhase) -> Self {
        Self {
            phase,
            entries_examined: 0,
            files_hashed: 0,
            files_to_hash: 0,
            batches_committed: 0,
            batches_total: 0,
            reason: None,
        }
    }
}

#[derive(Default)]
struct BoardState {
    progress: HashMap<String, IndexProgress>,
    requests: Option<UnboundedSender<String>>,
}

/// 各工作区索引进度的登记处。
#[derive(Default)]
pub struct WorkspaceIndexBoard {
    state: Mutex<BoardState>,
}

impl WorkspaceIndexBoard {
    fn with_state<T>(&self, operation: impl FnOnce(&mut BoardState) -> T) -> T {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        operation(&mut state)
    }

    /// 监听主管启动时登记自己的请求入口。没有登记时，建索引请求会明确失败。
    pub fn attach_requests(&self, sender: UnboundedSender<String>) {
        self.with_state(|state| state.requests = Some(sender));
    }

    /// 请求为工作区立即建一次索引。
    ///
    /// # Errors
    ///
    /// 本进程没有运行监听主管（例如瞬时实例），或监听主管已退出时返回错误。
    pub(crate) fn request(&self, workspace_id: &str) -> Result<(), ServiceError> {
        self.with_state(|state| {
            let sender = state
                .requests
                .as_ref()
                .ok_or_else(|| ServiceError::new("workspace_index_unavailable"))?;
            sender
                .send(workspace_id.to_owned())
                .map_err(|_| ServiceError::new("workspace_index_unavailable"))?;
            let current = state
                .progress
                .get(workspace_id)
                .map(|progress| progress.phase);
            // 已经在跑的不退回排队：请求只是让下一轮尽快到来，不打断当前这轮。
            if !matches!(
                current,
                Some(IndexPhase::Scanning | IndexPhase::Hashing | IndexPhase::Committing)
            ) {
                state.progress.insert(
                    workspace_id.to_owned(),
                    IndexProgress::with_phase(IndexPhase::Queued),
                );
            }
            Ok(())
        })
    }

    #[must_use]
    pub fn progress(&self, workspace_id: &str) -> IndexProgress {
        self.with_state(|state| {
            state
                .progress
                .get(workspace_id)
                .cloned()
                .unwrap_or_else(IndexProgress::idle)
        })
    }

    pub(crate) fn begin_scan(&self, workspace_id: &str) {
        self.with_state(|state| {
            state.progress.insert(
                workspace_id.to_owned(),
                IndexProgress::with_phase(IndexPhase::Scanning),
            );
        });
    }

    pub(crate) fn scanned(&self, workspace_id: &str, entries_examined: u64) {
        self.update(workspace_id, |progress| {
            progress.entries_examined = entries_examined;
        });
    }

    pub(crate) fn begin_hashing(&self, workspace_id: &str, files_to_hash: u64) {
        self.update(workspace_id, |progress| {
            progress.phase = IndexPhase::Hashing;
            progress.files_to_hash = files_to_hash;
            progress.files_hashed = 0;
        });
    }

    pub(crate) fn hashed(&self, workspace_id: &str, files_hashed: u64) {
        self.update(workspace_id, |progress| {
            progress.files_hashed = files_hashed;
        });
    }

    pub(crate) fn committing(&self, workspace_id: &str, committed: u64, total: u64) {
        self.update(workspace_id, |progress| {
            progress.phase = IndexPhase::Committing;
            progress.batches_committed = committed;
            progress.batches_total = total;
        });
    }

    pub(crate) fn finish(&self, workspace_id: &str) {
        self.with_state(|state| {
            state.progress.remove(workspace_id);
        });
    }

    pub(crate) fn fail(&self, workspace_id: &str, reason: &str) {
        self.with_state(|state| {
            let mut failed = IndexProgress::with_phase(IndexPhase::Failed);
            failed.reason = Some(reason.to_owned());
            state.progress.insert(workspace_id.to_owned(), failed);
        });
    }

    /// 工作区已从登记中移除：它的进度不再有意义。
    pub(crate) fn forget_except(&self, known: &[String]) {
        self.with_state(|state| state.progress.retain(|id, _| known.contains(id)));
    }

    fn update(&self, workspace_id: &str, change: impl FnOnce(&mut IndexProgress)) {
        self.with_state(|state| {
            if let Some(progress) = state.progress.get_mut(workspace_id) {
                change(progress);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{IndexPhase, WorkspaceIndexBoard};

    #[test]
    fn 没有监听主管时请求建索引会明确失败() {
        let board = WorkspaceIndexBoard::default();
        let error = board.request("w1").unwrap_err();
        assert_eq!(error.reason, "workspace_index_unavailable");
        assert_eq!(board.progress("w1").phase, IndexPhase::Idle);
    }

    #[test]
    fn 请求后进入排队并把工作区递给监听主管() {
        let board = WorkspaceIndexBoard::default();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        board.attach_requests(sender);
        board.request("w1").unwrap();
        assert_eq!(receiver.try_recv().unwrap(), "w1");
        assert_eq!(board.progress("w1").phase, IndexPhase::Queued);
    }

    #[test]
    fn 正在进行的索引不会被新请求打回排队() {
        let board = WorkspaceIndexBoard::default();
        let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
        board.attach_requests(sender);
        board.begin_scan("w1");
        board.scanned("w1", 42);
        board.request("w1").unwrap();
        let progress = board.progress("w1");
        assert_eq!(progress.phase, IndexPhase::Scanning);
        assert_eq!(progress.entries_examined, 42);
    }

    #[test]
    fn 阶段依次推进且完成后回到空闲() {
        let board = WorkspaceIndexBoard::default();
        board.begin_scan("w1");
        board.scanned("w1", 10);
        board.begin_hashing("w1", 4);
        board.hashed("w1", 3);
        let hashing = board.progress("w1");
        assert_eq!(hashing.phase, IndexPhase::Hashing);
        assert_eq!((hashing.files_hashed, hashing.files_to_hash), (3, 4));
        board.committing("w1", 1, 3);
        let committing = board.progress("w1");
        assert_eq!(committing.phase, IndexPhase::Committing);
        assert_eq!(
            (committing.batches_committed, committing.batches_total),
            (1, 3)
        );
        board.finish("w1");
        assert_eq!(board.progress("w1").phase, IndexPhase::Idle);
    }

    #[test]
    fn 失败会保留原因直到下一轮开始() {
        let board = WorkspaceIndexBoard::default();
        board.begin_scan("w1");
        board.fail("w1", "workspace_scan_incomplete");
        let failed = board.progress("w1");
        assert_eq!(failed.phase, IndexPhase::Failed);
        assert_eq!(failed.reason.as_deref(), Some("workspace_scan_incomplete"));
        board.begin_scan("w1");
        assert_eq!(board.progress("w1").reason, None);
    }
}
