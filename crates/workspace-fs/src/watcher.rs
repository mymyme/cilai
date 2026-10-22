use crate::fixed_ignore_match;
use contexthub_domain::WorkspaceRelativePath;
use notify::{
    Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{ModifyKind, RenameMode},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkspaceChangeHintKind {
    Create,
    Modify,
    Remove,
    RenameFrom,
    RenameTo,
    Metadata,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceChangeHint {
    pub path: WorkspaceRelativePath,
    pub kind: WorkspaceChangeHintKind,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceRescanRequest {
    pub reason: WorkspaceRescanReason,
    pub diagnostic: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkspaceRescanReason {
    AppServerStarted,
    WatcherSubscribed,
    WatcherResubscribed,
    WatcherError,
    WorkspaceRootChanged,
    InvalidEventPath,
    QueueCapacityExceeded,
    AmbiguousRename,
    UnscopedEvent,
    CompensationScanInterrupted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceWatchSignal {
    Change(WorkspaceChangeHint),
    RescanRequired(WorkspaceRescanRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspacePathRecheck {
    pub path: WorkspaceRelativePath,
    pub hints: BTreeSet<WorkspaceChangeHintKind>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceChangeBatch {
    pub paths: Vec<WorkspacePathRecheck>,
    pub rescan_requests: Vec<WorkspaceRescanRequest>,
}

impl WorkspaceChangeBatch {
    #[must_use]
    pub fn requires_full_rescan(&self) -> bool {
        !self.rescan_requests.is_empty()
    }
}

#[derive(Debug)]
pub struct WorkspaceEventCoalescer {
    window: Duration,
    max_paths: NonZeroUsize,
    deadline: Option<Instant>,
    paths: BTreeMap<WorkspaceRelativePath, BTreeSet<WorkspaceChangeHintKind>>,
    rescan_requests: BTreeMap<WorkspaceRescanReason, Option<String>>,
}

impl WorkspaceEventCoalescer {
    #[must_use]
    pub fn new(window: Duration, max_paths: NonZeroUsize) -> Self {
        Self {
            window,
            max_paths,
            deadline: None,
            paths: BTreeMap::new(),
            rescan_requests: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, signal: WorkspaceWatchSignal, observed_at: Instant) {
        self.deadline.get_or_insert(observed_at + self.window);
        match signal {
            WorkspaceWatchSignal::Change(change) if self.rescan_requests.is_empty() => {
                if !self.paths.contains_key(&change.path)
                    && self.paths.len() == self.max_paths.get()
                {
                    self.paths.clear();
                    self.rescan_requests
                        .insert(WorkspaceRescanReason::QueueCapacityExceeded, None);
                    return;
                }
                self.paths
                    .entry(change.path)
                    .or_default()
                    .insert(change.kind);
            }
            WorkspaceWatchSignal::Change(_) => {}
            WorkspaceWatchSignal::RescanRequired(request) => {
                self.paths.clear();
                self.rescan_requests
                    .entry(request.reason)
                    .and_modify(|diagnostic| {
                        if diagnostic.is_none() {
                            diagnostic.clone_from(&request.diagnostic);
                        }
                    })
                    .or_insert(request.diagnostic);
            }
        }
    }

    #[must_use]
    pub fn drain_ready(&mut self, now: Instant) -> Option<WorkspaceChangeBatch> {
        self.deadline
            .is_some_and(|deadline| now >= deadline)
            .then(|| self.drain())
    }

    #[must_use]
    pub fn flush(&mut self) -> Option<WorkspaceChangeBatch> {
        self.deadline.map(|_| self.drain())
    }

    fn drain(&mut self) -> WorkspaceChangeBatch {
        self.deadline = None;
        let paths = std::mem::take(&mut self.paths)
            .into_iter()
            .map(|(path, hints)| WorkspacePathRecheck { path, hints })
            .collect();
        let rescan_requests = std::mem::take(&mut self.rescan_requests)
            .into_iter()
            .map(|(reason, diagnostic)| WorkspaceRescanRequest { reason, diagnostic })
            .collect();
        WorkspaceChangeBatch {
            paths,
            rescan_requests,
        }
    }
}

#[derive(Debug)]
pub struct WorkspaceWatcher {
    root: PathBuf,
    _watcher: RecommendedWatcher,
}

impl WorkspaceWatcher {
    /// 递归监听工作区，并将原生事件转换成平台无关的重新检查提示。
    ///
    /// 回调运行于监听后端线程，调用方必须快速接收信号，不能在回调中扫描或计算摘要。
    ///
    /// # Errors
    ///
    /// 工作区根无法解析、不是目录，或监听后端无法创建和订阅时返回错误。
    pub fn start(
        root: &Path,
        mut on_signal: impl FnMut(WorkspaceWatchSignal) + Send + 'static,
    ) -> Result<Self, WorkspaceWatchError> {
        let canonical_root = std::fs::canonicalize(root).map_err(|error| {
            WorkspaceWatchError::root(WorkspaceWatchErrorKind::RootUnavailable, error)
        })?;
        let metadata = std::fs::metadata(&canonical_root).map_err(|error| {
            WorkspaceWatchError::root(WorkspaceWatchErrorKind::RootUnavailable, error)
        })?;
        if !metadata.is_dir() {
            return Err(WorkspaceWatchError::without_source(
                WorkspaceWatchErrorKind::RootNotDirectory,
            ));
        }

        let callback_root = canonical_root.clone();
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| match result {
                Ok(event) => {
                    for signal in workspace_watch_signals(&callback_root, &event) {
                        on_signal(signal);
                    }
                }
                Err(error) => on_signal(rescan_with_diagnostic(
                    WorkspaceRescanReason::WatcherError,
                    error.to_string(),
                )),
            },
            Config::default(),
        )
        .map_err(WorkspaceWatchError::backend)?;
        watcher
            .watch(&canonical_root, RecursiveMode::Recursive)
            .map_err(WorkspaceWatchError::backend)?;
        Ok(Self {
            root: canonical_root,
            _watcher: watcher,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceWatchErrorKind {
    RootUnavailable,
    RootNotDirectory,
    BackendUnavailable,
}

#[derive(Debug)]
pub struct WorkspaceWatchError {
    kind: WorkspaceWatchErrorKind,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl WorkspaceWatchError {
    #[must_use]
    pub const fn kind(&self) -> WorkspaceWatchErrorKind {
        self.kind
    }

    fn root(kind: WorkspaceWatchErrorKind, source: std::io::Error) -> Self {
        Self {
            kind,
            source: Some(Box::new(source)),
        }
    }

    fn backend(source: notify::Error) -> Self {
        Self {
            kind: WorkspaceWatchErrorKind::BackendUnavailable,
            source: Some(Box::new(source)),
        }
    }

    fn without_source(kind: WorkspaceWatchErrorKind) -> Self {
        Self { kind, source: None }
    }
}

impl std::fmt::Display for WorkspaceWatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.kind, &self.source) {
            (WorkspaceWatchErrorKind::RootUnavailable, Some(source)) => {
                write!(formatter, "resolve workspace watch root: {source}")
            }
            (WorkspaceWatchErrorKind::RootNotDirectory, _) => {
                formatter.write_str("workspace watch root is not a directory")
            }
            (WorkspaceWatchErrorKind::BackendUnavailable, Some(source)) => {
                write!(formatter, "start workspace watcher: {source}")
            }
            _ => formatter.write_str("workspace watcher failed"),
        }
    }
}

impl std::error::Error for WorkspaceWatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

#[must_use]
fn workspace_watch_signals(root: &Path, event: &Event) -> Vec<WorkspaceWatchSignal> {
    let hints = event_path_hints(event);
    if hints.is_empty() {
        if matches!(event.kind, EventKind::Access(_)) {
            return Vec::new();
        }
        let reason = if matches!(event.kind, EventKind::Modify(ModifyKind::Name(_))) {
            WorkspaceRescanReason::AmbiguousRename
        } else {
            WorkspaceRescanReason::UnscopedEvent
        };
        return vec![rescan(reason)];
    }

    hints
        .into_iter()
        .filter_map(|(absolute_path, kind)| normalize_event_path(root, &absolute_path, kind))
        .collect()
}

fn event_path_hints(event: &Event) -> Vec<(PathBuf, WorkspaceChangeHintKind)> {
    match &event.kind {
        EventKind::Create(_) => hints_for_all_paths(event, WorkspaceChangeHintKind::Create),
        EventKind::Remove(_) => hints_for_all_paths(event, WorkspaceChangeHintKind::Remove),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() == 2 => vec![
            (event.paths[0].clone(), WorkspaceChangeHintKind::RenameFrom),
            (event.paths[1].clone(), WorkspaceChangeHintKind::RenameTo),
        ],
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            hints_for_all_paths(event, WorkspaceChangeHintKind::RenameFrom)
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            hints_for_all_paths(event, WorkspaceChangeHintKind::RenameTo)
        }
        EventKind::Modify(ModifyKind::Name(_)) | EventKind::Access(_) => Vec::new(),
        EventKind::Modify(ModifyKind::Metadata(_)) => {
            hints_for_all_paths(event, WorkspaceChangeHintKind::Metadata)
        }
        EventKind::Modify(_) => hints_for_all_paths(event, WorkspaceChangeHintKind::Modify),
        EventKind::Any | EventKind::Other => {
            hints_for_all_paths(event, WorkspaceChangeHintKind::Other)
        }
    }
}

fn hints_for_all_paths(
    event: &Event,
    kind: WorkspaceChangeHintKind,
) -> Vec<(PathBuf, WorkspaceChangeHintKind)> {
    event
        .paths
        .iter()
        .cloned()
        .map(|path| (path, kind))
        .collect()
}

fn normalize_event_path(
    root: &Path,
    absolute_path: &Path,
    kind: WorkspaceChangeHintKind,
) -> Option<WorkspaceWatchSignal> {
    let relative = match absolute_path.strip_prefix(root) {
        Ok(relative) if relative.as_os_str().is_empty() => {
            return Some(rescan(WorkspaceRescanReason::WorkspaceRootChanged));
        }
        Ok(relative) => relative,
        Err(_) => {
            return Some(rescan(WorkspaceRescanReason::InvalidEventPath));
        }
    };
    let Ok(path) = WorkspaceRelativePath::from_native(relative) else {
        return Some(rescan(WorkspaceRescanReason::InvalidEventPath));
    };
    if fixed_ignore_match(&path).is_some() {
        return None;
    }
    Some(WorkspaceWatchSignal::Change(WorkspaceChangeHint {
        path,
        kind,
    }))
}

fn rescan(reason: WorkspaceRescanReason) -> WorkspaceWatchSignal {
    WorkspaceWatchSignal::RescanRequired(WorkspaceRescanRequest {
        reason,
        diagnostic: None,
    })
}

fn rescan_with_diagnostic(
    reason: WorkspaceRescanReason,
    diagnostic: String,
) -> WorkspaceWatchSignal {
    WorkspaceWatchSignal::RescanRequired(WorkspaceRescanRequest {
        reason,
        diagnostic: Some(diagnostic),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, RemoveKind};
    #[cfg(target_os = "macos")]
    use std::sync::mpsc;
    use std::{
        fs,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn repeated_path_events_are_coalesced_until_window_expires() {
        let start = Instant::now();
        let mut coalescer = WorkspaceEventCoalescer::new(
            Duration::from_millis(200),
            NonZeroUsize::new(32).unwrap(),
        );
        for kind in [
            WorkspaceChangeHintKind::Create,
            WorkspaceChangeHintKind::Modify,
            WorkspaceChangeHintKind::Modify,
        ] {
            coalescer.push(change("docs/spec.md", kind), start);
        }
        assert!(
            coalescer
                .drain_ready(start + Duration::from_millis(199))
                .is_none()
        );
        let batch = coalescer
            .drain_ready(start + Duration::from_millis(200))
            .unwrap();
        assert_eq!(batch.paths.len(), 1);
        assert_eq!(
            batch.paths[0].hints,
            BTreeSet::from([
                WorkspaceChangeHintKind::Create,
                WorkspaceChangeHintKind::Modify,
            ])
        );
        assert!(!batch.requires_full_rescan());
    }

    #[test]
    fn rename_pair_rechecks_source_and_destination() {
        let root = Path::new("/workspace");
        let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
            .add_path(root.join("draft.md"))
            .add_path(root.join("final.md"));
        assert_eq!(
            workspace_watch_signals(root, &event),
            [
                change("draft.md", WorkspaceChangeHintKind::RenameFrom),
                change("final.md", WorkspaceChangeHintKind::RenameTo),
            ]
        );
    }

    #[test]
    fn atomic_replace_sequence_keeps_target_recheck() {
        let root = Path::new("/workspace");
        let events = [
            Event::new(EventKind::Create(CreateKind::File)).add_path(root.join(".document.md.tmp")),
            Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
                .add_path(root.join(".document.md.tmp"))
                .add_path(root.join("document.md")),
            Event::new(EventKind::Remove(RemoveKind::File)).add_path(root.join("document.md")),
        ];
        let start = Instant::now();
        let mut coalescer = WorkspaceEventCoalescer::new(
            Duration::from_millis(100),
            NonZeroUsize::new(16).unwrap(),
        );
        for event in events {
            for signal in workspace_watch_signals(root, &event) {
                coalescer.push(signal, start);
            }
        }
        let batch = coalescer.flush().unwrap();
        let target = batch
            .paths
            .iter()
            .find(|entry| entry.path.as_str() == "document.md")
            .unwrap();
        assert_eq!(
            target.hints,
            BTreeSet::from([
                WorkspaceChangeHintKind::Remove,
                WorkspaceChangeHintKind::RenameTo,
            ])
        );
    }

    #[test]
    fn ignored_and_access_events_do_not_enter_queue() {
        let root = Path::new("/workspace");
        for reserved in [".cilai", ".CILAI", ".contexthub"] {
            let ignored = Event::new(EventKind::Modify(ModifyKind::Any))
                .add_path(root.join(reserved).join("workspace.json"));
            assert!(workspace_watch_signals(root, &ignored).is_empty());
        }
        let access = Event::new(EventKind::Access(notify::event::AccessKind::Any))
            .add_path(root.join("docs/spec.md"));
        assert!(workspace_watch_signals(root, &access).is_empty());
    }

    #[test]
    fn event_storm_becomes_bounded_rescan_signal() {
        let start = Instant::now();
        let mut coalescer =
            WorkspaceEventCoalescer::new(Duration::from_millis(100), NonZeroUsize::new(2).unwrap());
        for path in ["a.md", "b.md", "c.md", "d.md"] {
            coalescer.push(change(path, WorkspaceChangeHintKind::Modify), start);
        }
        let batch = coalescer.flush().unwrap();
        assert!(batch.paths.is_empty());
        assert_eq!(
            batch.rescan_requests,
            [WorkspaceRescanRequest {
                reason: WorkspaceRescanReason::QueueCapacityExceeded,
                diagnostic: None,
            }]
        );
    }

    #[test]
    fn watcher_error_diagnostic_survives_bounded_coalescing() {
        let start = Instant::now();
        let mut coalescer =
            WorkspaceEventCoalescer::new(Duration::from_millis(100), NonZeroUsize::new(2).unwrap());
        coalescer.push(
            rescan_with_diagnostic(
                WorkspaceRescanReason::WatcherError,
                "backend stream failed".to_owned(),
            ),
            start,
        );
        coalescer.push(
            rescan_with_diagnostic(
                WorkspaceRescanReason::WatcherError,
                "later duplicate".to_owned(),
            ),
            start,
        );
        assert_eq!(
            coalescer.flush().unwrap().rescan_requests,
            [WorkspaceRescanRequest {
                reason: WorkspaceRescanReason::WatcherError,
                diagnostic: Some("backend stream failed".to_owned()),
            }]
        );
    }

    #[test]
    fn ambiguous_rename_and_root_change_require_rescan() {
        let root = Path::new("/workspace");
        let ambiguous = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Any)))
            .add_path(root.join("unknown.md"));
        assert_eq!(
            workspace_watch_signals(root, &ambiguous),
            [rescan(WorkspaceRescanReason::AmbiguousRename)]
        );
        let root_event = Event::new(EventKind::Modify(ModifyKind::Any)).add_path(root.to_owned());
        assert_eq!(
            workspace_watch_signals(root, &root_event),
            [rescan(WorkspaceRescanReason::WorkspaceRootChanged)]
        );
    }

    #[test]
    fn native_watcher_subscribes_to_workspace_root() {
        let fixture = TestWorkspace::new("native-watcher");
        let watcher = WorkspaceWatcher::start(&fixture.root, |_| {}).unwrap();
        assert_eq!(watcher.root(), fs::canonicalize(&fixture.root).unwrap());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires native FSEvents delivery; run explicitly for macOS release verification"]
    fn native_watcher_delivers_created_file_hint() {
        let fixture = TestWorkspace::new_native("native-delivery");
        let (sender, receiver) = mpsc::channel();
        let _watcher = WorkspaceWatcher::start(&fixture.root, move |signal| {
            let _ = sender.send(signal);
        })
        .unwrap();

        std::thread::sleep(Duration::from_millis(500));
        fs::write(fixture.root.join("observed.md"), "observed").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut observed_signals = Vec::new();
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match receiver.recv_timeout(remaining) {
                Ok(WorkspaceWatchSignal::Change(change))
                    if change.path.as_str() == "observed.md" =>
                {
                    return;
                }
                Ok(signal) => observed_signals.push(signal),
                Err(_) => break,
            }
        }
        panic!("native watcher did not report observed.md; observed {observed_signals:?}");
    }

    fn change(path: &str, kind: WorkspaceChangeHintKind) -> WorkspaceWatchSignal {
        WorkspaceWatchSignal::Change(WorkspaceChangeHint {
            path: WorkspaceRelativePath::parse(path).unwrap(),
            kind,
        })
    }

    struct TestWorkspace {
        root: PathBuf,
    }

    impl TestWorkspace {
        fn new(label: &str) -> Self {
            Self::under(&std::env::temp_dir(), label)
        }

        #[cfg(target_os = "macos")]
        fn new_native(label: &str) -> Self {
            Self::under(
                &std::env::current_dir()
                    .unwrap()
                    .join("target/native-watcher-tests"),
                label,
            )
        }

        fn under(parent: &Path, label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = parent.join(format!(
                "contexthub-watch-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}
