//! 工作区监听：具体变化只检查受影响路径，初始化、手动请求及失去路径范围的通知检查全树。

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use contexthub_workspace_fs::{WorkspaceChangeHintKind, WorkspaceWatchSignal, WorkspaceWatcher};
use tokio::sync::mpsc;

use crate::AppServerServices;
use crate::workspace::IncrementalScanProgress;
use crate::workspace_index::WorkspaceIndexBoard;

/// 变化停止多久之后才扫。文档要求"文件在稳定后读取"：编辑器保存往往是
/// 写临时文件再改名，中间会连着抛好几个事件，落在这个窗口里合成一次。
const QUIET_PERIOD: Duration = Duration::from_secs(2);

/// 同一个工作区两次索引更新之间的下限，合并持续写入产生的变化；
/// 没有这个下限，一次持续写入（比如 git checkout）会让扫描排成一列首尾相接地跑。
const MIN_RESCAN_INTERVAL: Duration = Duration::from_secs(15);

/// 只收到"可能有变化"这类推测信号时的补扫下限。
///
/// iCloud 目录几乎不间断地产生这类信号，按普通节奏跟着扫等于一直在扫。设成十分钟：
/// 真漏掉的事件最迟十分钟后被补上，而稳态下几乎不产生额外扫描。
const SPECULATIVE_RESCAN_INTERVAL: Duration = Duration::from_mins(10);

/// 多久对一次工作区名单。用户新连一个文件夹之后要能被盯上，
/// 而这件事没有事件通知，只能定期对。
const RECONCILE_INTERVAL: Duration = Duration::from_secs(20);

/// 两批提交之间让出服务主锁的时间，够排队的界面请求拿到锁。
const BATCH_YIELD: Duration = Duration::from_millis(15);

/// 一次唤醒里最多扫几个工作区，避免一次性把主锁占住太久。
const MAX_SCANS_PER_TICK: usize = 1;

struct Watched {
    _watcher: WorkspaceWatcher,
    local_root: String,
    dirty_since: Option<Instant>,
    /// 这次变脏是确实有文件动了，还是只是推测。决定用哪个间隔。
    dirty_is_concrete: bool,
    last_scan: Option<Instant>,
    /// 用户明确要求现在就建（刚添加文件夹、点了重新扫描）。跳过防抖与间隔下限。
    requested: bool,
    paths: HashSet<String>,
    full_scan: bool,
}

/// 启动监听主管。返回的任务句柄由调用方持有，进程退出时随之结束。
pub fn spawn_until_shutdown(
    services: Arc<Mutex<AppServerServices>>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let (signals_tx, mut signals_rx) = mpsc::unbounded_channel::<(String, WorkspaceWatchSignal)>();
    let (requests_tx, mut requests_rx) = mpsc::unbounded_channel::<String>();
    let board = services
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .index_board();
    board.attach_requests(requests_tx);
    tokio::spawn(async move {
        let mut watched: HashMap<String, Watched> = HashMap::new();
        let mut reconciled_at: Option<Instant> = None;
        loop {
            if *shutdown.borrow() {
                break;
            }
            // 明确的建索引请求多半来自刚添加的文件夹，它还不在监听名单里，
            // 要立刻对一次名单，而不是等下一个对名单周期。
            let mut requested = Vec::new();
            while let Ok(workspace_id) = requests_rx.try_recv() {
                requested.push(workspace_id);
            }
            if !requested.is_empty()
                || reconciled_at.is_none_or(|at| at.elapsed() >= RECONCILE_INTERVAL)
            {
                reconcile(&services, &board, &mut watched, &signals_tx);
                reconciled_at = Some(Instant::now());
            }
            for workspace_id in requested {
                match watched.get_mut(&workspace_id) {
                    Some(entry) => {
                        entry.requested = true;
                        entry.full_scan = true;
                    }
                    // 对过名单还不在：目录盯不上（已移走、没权限）或不是本地工作区。
                    None => board.fail(&workspace_id, "workspace_watch_unavailable"),
                }
            }

            // 把这一轮攒下的信号全部收走再决定扫谁：一次保存可能抛出十几个事件。
            while let Ok((workspace_id, signal)) = signals_rx.try_recv() {
                if let Some(entry) = watched.get_mut(&workspace_id) {
                    entry.dirty_since = Some(Instant::now());
                    // 一旦收到确凿的变化，这一轮就按确凿处理，不被后续的推测信号降级。
                    entry.dirty_is_concrete |= matches!(classify(&signal), SignalKind::Concrete);
                    match signal {
                        WorkspaceWatchSignal::Change(hint) => {
                            entry.paths.insert(hint.path.as_str().to_owned());
                        }
                        WorkspaceWatchSignal::RescanRequired(_) => entry.full_scan = true,
                    }
                }
            }

            rescan_settled(&services, &board, &mut watched);
            tokio::select! {
                changed=shutdown.changed()=> { if changed.is_err() || *shutdown.borrow() { break; } }
                ()=tokio::time::sleep(Duration::from_millis(500))=>{}
            }
        }
    })
}

/// 让监听名单与登记的工作区一致：新连的要盯上，已移除的要停掉。
fn reconcile(
    services: &Arc<Mutex<AppServerServices>>,
    board: &WorkspaceIndexBoard,
    watched: &mut HashMap<String, Watched>,
    signals_tx: &mpsc::UnboundedSender<(String, WorkspaceWatchSignal)>,
) {
    let Ok(mut guard) = services.lock() else {
        return;
    };
    let Ok(roots) = guard.watched_workspace_roots() else {
        return;
    };
    drop(guard);

    let known: Vec<String> = roots.iter().map(|(id, _)| id.clone()).collect();
    watched.retain(|id, _| known.contains(id));
    board.forget_except(&known);

    for (workspace_id, local_root) in roots {
        if watched
            .get(&workspace_id)
            .is_some_and(|entry| entry.local_root == local_root)
        {
            continue;
        }
        let sender = signals_tx.clone();
        let id_for_signal = workspace_id.clone();
        // 回调跑在监听后端线程上，只允许送个信号就走——扫描和摘要都不能在这里做。
        let started =
            WorkspaceWatcher::start(Path::new(&local_root), move |signal| {
                match classify(&signal) {
                    SignalKind::Ignorable => {}
                    SignalKind::Concrete | SignalKind::Speculative => {
                        let _ = sender.send((id_for_signal.clone(), signal));
                    }
                }
            });
        // 目录可能已经被移走或没有权限。盯不上就下一轮再试，不影响别的工作区。
        if let Ok(watcher) = started {
            watched.insert(
                workspace_id,
                Watched {
                    _watcher: watcher,
                    local_root,
                    // 刚订阅上就先当作脏的：没盯着的这段时间里发生了什么，谁也不知道。
                    // 不补这一次，上次退出后到这次启动前的改动会一直不进索引——
                    // 界面于是长期显示一份早已不成立的文件树。
                    dirty_since: Some(Instant::now()),
                    // 补这一次是确定要做的：没盯着的那段时间里发生了什么谁也不知道。
                    dirty_is_concrete: true,
                    last_scan: None,
                    requested: false,
                    paths: HashSet::new(),
                    full_scan: true,
                },
            );
        }
    }
}

/// 信号的确定程度。两者的防抖节奏差一个量级。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SignalKind {
    /// 确实有文件被建、被改、被删或被改名。
    Concrete,
    /// 只是"这里可能有变化"。不能当成事实，也不能完全不管。
    Speculative,
    /// 与内容无关，直接丢掉。
    Ignorable,
}

/// 判断一个监听信号有多确定。
///
/// 这个区分是必须的，因为工作区常常在 iCloud 里。实测这个目录在没有任何文件被修改的
/// 45 秒内收到：AmbiguousRename 24 次、Metadata 22 次——都是同步进程搅动目录的副产物。
/// 把它们当成"要重扫"，就会形成"扫描 → 目录被动过 → 再扫描"的闭环：每 22 秒写一份
/// 22,347 行的快照，库在一小时里从 166 MB 涨到 1.9 GB。
///
/// 但也不能一律忽略：AmbiguousRename 的本意是"我可能漏了事件"，真漏了就必须补。
/// 所以它归入推测档——按很稀疏的下限补扫，而不是每次都扫。
fn classify(signal: &WorkspaceWatchSignal) -> SignalKind {
    match signal {
        WorkspaceWatchSignal::RescanRequired(_) => SignalKind::Speculative,
        WorkspaceWatchSignal::Change(hint) => classify_hint(hint.kind),
    }
}

/// 单独一层，好在测试里直接给种类——相对路径类型是私有的，构造不出完整的 hint。
fn classify_hint(kind: WorkspaceChangeHintKind) -> SignalKind {
    match kind {
        WorkspaceChangeHintKind::Create
        | WorkspaceChangeHintKind::Modify
        | WorkspaceChangeHintKind::Remove
        | WorkspaceChangeHintKind::RenameFrom
        | WorkspaceChangeHintKind::RenameTo => SignalKind::Concrete,
        WorkspaceChangeHintKind::Metadata => SignalKind::Ignorable,
        WorkspaceChangeHintKind::Other => SignalKind::Speculative,
    }
}

/// 现在该不该扫这个工作区。
///
/// 两个条件缺一不可：变化已经停了一段时间（编辑器保存会连着抛好几个事件），
/// 且距上次索引更新够久，避免持续写入让任务首尾相接。
fn should_rescan(
    dirty_for: Option<Duration>,
    dirty_is_concrete: bool,
    since_last_scan: Option<Duration>,
) -> bool {
    let Some(dirty_for) = dirty_for else {
        return false;
    };
    if dirty_for < QUIET_PERIOD {
        return false;
    }
    let floor = if dirty_is_concrete {
        MIN_RESCAN_INTERVAL
    } else {
        SPECULATIVE_RESCAN_INTERVAL
    };
    since_last_scan.is_none_or(|elapsed| elapsed >= floor)
}

/// 扫描那些"已经安静下来"或被明确要求的工作区。
fn rescan_settled(
    services: &Arc<Mutex<AppServerServices>>,
    board: &WorkspaceIndexBoard,
    watched: &mut HashMap<String, Watched>,
) {
    let ready: Vec<String> = watched
        .iter()
        .filter(|(_, entry)| {
            entry.requested
                || should_rescan(
                    entry.dirty_since.map(|since| since.elapsed()),
                    entry.dirty_is_concrete,
                    entry.last_scan.map(|at| at.elapsed()),
                )
        })
        .map(|(id, _)| id.clone())
        .take(MAX_SCANS_PER_TICK)
        .collect();

    for workspace_id in ready {
        let Ok(now) = crate::unix_epoch_millis() else {
            continue;
        };
        board.begin_scan(&workspace_id);
        let entry = &watched[&workspace_id];
        let paths = (!entry.full_scan).then(|| entry.paths.iter().cloned().collect::<Vec<_>>());
        let result = rescan(services, board, &workspace_id, now, paths.as_deref());
        let completed = result.is_ok();
        match result {
            Ok(()) => board.finish(&workspace_id),
            Err(error) => board.fail(&workspace_id, error.reason),
        }

        if let Some(entry) = watched.get_mut(&workspace_id) {
            entry.last_scan = Some(Instant::now());
            entry.dirty_is_concrete = false;
            entry.requested = false;
            if completed {
                entry.paths.clear();
                entry.full_scan = false;
            }
            // 扫失败也把 dirty 清掉：它多半是目录暂时读不到，留着会让这一个工作区
            // 每隔 MIN_RESCAN_INTERVAL 重试一次直到进程结束。下一次真实变化会重新置脏。
            // 失败原因留在索引进度里，界面据此提示并提供重试。
            entry.dirty_since = None;
        }
    }
}

/// 三段式：读取要持锁，走一遍目录和算摘要不需要，提交再持一次。整个捕获里最慢的是
/// 目录遍历和摘要，占住服务主锁会让界面在这段时间里完全卡住。
fn rescan(
    services: &Arc<Mutex<AppServerServices>>,
    board: &WorkspaceIndexBoard,
    workspace_id: &str,
    now: i64,
    paths: Option<&[String]>,
) -> Result<(), crate::ServiceError> {
    let started = Instant::now();
    let basis = {
        let mut services = services
            .lock()
            .map_err(|_| crate::ServiceError::new("workspace_index_unavailable"))?;
        services.prepare_watched_rescan(workspace_id)
    }
    .inspect_err(|error| log_service_failure(workspace_id, paths, "prepare", error, started))?;
    // 扫描与提交共用同一份指令：观测里的 id 由 snapshot_id 拼出来。
    let command = AppServerServices::watched_rescan_command(workspace_id, now);
    let stage = std::cell::Cell::new("scan");
    crate::workspace::log_fs_diagnostic(
        workspace_id,
        "",
        "index_start",
        "started",
        None,
        started.elapsed(),
    );
    let on_progress = |progress| match progress {
        IncrementalScanProgress::Scanned(examined) => board.scanned(workspace_id, examined),
        IncrementalScanProgress::HashingStarted(total) => {
            stage.set("hash");
            board.begin_hashing(workspace_id, total);
        }
        IncrementalScanProgress::Hashed(done) => board.hashed(workspace_id, done),
    };
    let cancellation = contexthub_workspace_fs::WorkspaceCancellationToken::new();
    let findings = match paths {
        Some(paths) => crate::workspace::observe_changed_paths(
            &basis,
            &command,
            paths,
            &cancellation,
            on_progress,
        ),
        None => crate::workspace::observe_incremental_changes(
            &basis,
            &command,
            &cancellation,
            on_progress,
        ),
    }
    .map_err(|error| {
        crate::workspace::log_index_failure(
            workspace_id,
            paths,
            stage.get(),
            &error,
            started.elapsed(),
        );
        crate::ServiceError::new(crate::workspace::index_failure_reason(&error))
    })?;
    // 分批提交、批间放锁：一次写完上万条会把服务主锁占住好几秒，界面请求全部排队。
    let batches = findings.into_batches(crate::workspace::COMMIT_BATCH_SIZE);
    let total = batches.len();
    for (index, batch) in batches.into_iter().enumerate() {
        board.committing(workspace_id, index as u64, total as u64);
        let result = {
            let mut services = services
                .lock()
                .map_err(|_| crate::ServiceError::new("workspace_index_unavailable"))?;
            services.commit_watched_rescan(
                &command.for_batch(index, total),
                batch,
                index + 1 == total && paths.is_none(),
            )
        };
        result.inspect_err(|error| {
            log_service_failure(workspace_id, paths, "commit", error, started);
        })?;
        // 放锁之后要真的让出去：服务主锁不保证公平，这里紧接着再去拿，排队的界面请求
        // 就一直抢不到。实测不让的话，界面请求照样要等全部批次写完。
        if index + 1 < total {
            std::thread::sleep(BATCH_YIELD);
        }
    }
    Ok(())
}

fn log_service_failure(
    workspace_id: &str,
    paths: Option<&[String]>,
    stage: &'static str,
    error: &crate::ServiceError,
    started: Instant,
) {
    crate::workspace::log_fs_diagnostic(
        workspace_id,
        &paths.map_or_else(String::new, |paths| paths.join("\n")),
        stage,
        error.reason,
        None,
        started.elapsed(),
    );
}

#[cfg(test)]
mod tests {
    use super::{
        MIN_RESCAN_INTERVAL, QUIET_PERIOD, SPECULATIVE_RESCAN_INTERVAL, SignalKind, classify_hint,
        should_rescan,
    };
    use contexthub_workspace_fs::WorkspaceChangeHintKind;
    use std::time::Duration;

    #[test]
    fn 内容变化要重扫() {
        for kind in [
            WorkspaceChangeHintKind::Create,
            WorkspaceChangeHintKind::Modify,
            WorkspaceChangeHintKind::Remove,
            WorkspaceChangeHintKind::RenameFrom,
            WorkspaceChangeHintKind::RenameTo,
        ] {
            assert_eq!(classify_hint(kind), SignalKind::Concrete, "{kind:?}");
        }
    }

    #[test]
    fn 元数据变化不重扫否则扫描会喂给自己() {
        // 扫描要读遍所有文件，macOS 因此更新 lastuseddate 扩展属性，FSEvents 报成
        // 元数据变化。把它当成变化就会形成闭环，实测一小时把库从 166 MB 撑到 1.9 GB。
        assert_eq!(
            classify_hint(WorkspaceChangeHintKind::Metadata),
            SignalKind::Ignorable
        );
    }

    #[test]
    fn 刚订阅上就该补扫一次() {
        // 没盯着的那段时间里发生的变化，只能靠这一次补扫发现。
        assert!(should_rescan(Some(QUIET_PERIOD), true, None));
    }

    #[test]
    fn 没有变化时不扫() {
        assert!(!should_rescan(None, true, None));
    }

    #[test]
    fn 变化还没停下来时不扫() {
        // 编辑器保存常常是写临时文件再改名，中间连着抛好几个事件。
        assert!(!should_rescan(Some(QUIET_PERIOD / 2), true, None));
    }

    #[test]
    fn 安静下来之后才扫() {
        assert!(should_rescan(Some(QUIET_PERIOD), true, None));
    }

    #[test]
    fn 距上次扫描太近时先等着() {
        // 没有这个下限，一次持续写入会让全量扫描首尾相接地排队跑。
        assert!(!should_rescan(
            Some(QUIET_PERIOD),
            true,
            Some(MIN_RESCAN_INTERVAL / 2)
        ));
    }

    #[test]
    fn 间隔够了就继续扫() {
        assert!(should_rescan(
            Some(QUIET_PERIOD),
            true,
            Some(MIN_RESCAN_INTERVAL)
        ));
    }

    #[test]
    fn 长时间没动过的工作区不会因为间隔到了就凭空扫一次() {
        assert!(!should_rescan(None, true, Some(Duration::from_hours(1))));
    }

    #[test]
    fn 推测信号按很稀疏的下限补扫() {
        // iCloud 目录几乎不间断产生这类信号，按普通间隔跟着扫等于一直在扫。
        assert!(!should_rescan(
            Some(QUIET_PERIOD),
            false,
            Some(MIN_RESCAN_INTERVAL)
        ));
        assert!(should_rescan(
            Some(QUIET_PERIOD),
            false,
            Some(SPECULATIVE_RESCAN_INTERVAL)
        ));
    }
}
