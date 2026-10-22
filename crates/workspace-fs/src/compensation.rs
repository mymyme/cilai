use crate::{
    WorkspaceCancellationToken, WorkspaceChangeBatch, WorkspaceRescanReason,
    WorkspaceRescanRequest, WorkspaceScanError, WorkspaceScanEvent, WorkspaceScanOutcome,
    scan_workspace,
};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceCompensationStatus {
    Idle,
    ScanPending,
    ScanRunning,
    PausedAfterInterruption,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCompensationScan {
    pub generation: u64,
    pub requests: Vec<WorkspaceRescanRequest>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceCompensationLifecycleErrorKind {
    NoActiveScan,
    GenerationMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceCompensationLifecycleError {
    kind: WorkspaceCompensationLifecycleErrorKind,
    expected_generation: Option<u64>,
    actual_generation: u64,
}

impl WorkspaceCompensationLifecycleError {
    #[must_use]
    pub const fn kind(&self) -> WorkspaceCompensationLifecycleErrorKind {
        self.kind
    }
}

impl std::fmt::Display for WorkspaceCompensationLifecycleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.kind, self.expected_generation) {
            (WorkspaceCompensationLifecycleErrorKind::NoActiveScan, _) => {
                write!(
                    formatter,
                    "no compensation scan is active for generation {}",
                    self.actual_generation
                )
            }
            (WorkspaceCompensationLifecycleErrorKind::GenerationMismatch, Some(expected)) => {
                write!(
                    formatter,
                    "compensation scan generation mismatch: expected {expected}, got {}",
                    self.actual_generation
                )
            }
            _ => formatter.write_str("compensation scan lifecycle error"),
        }
    }
}

impl std::error::Error for WorkspaceCompensationLifecycleError {}

#[derive(Debug)]
pub struct WorkspaceCompensationCoordinator {
    next_generation: u64,
    active: Option<WorkspaceCompensationScan>,
    pending: BTreeMap<WorkspaceRescanReason, Option<String>>,
    paused: bool,
}

impl Default for WorkspaceCompensationCoordinator {
    fn default() -> Self {
        Self {
            next_generation: 1,
            active: None,
            pending: BTreeMap::new(),
            paused: false,
        }
    }
}

impl WorkspaceCompensationCoordinator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn for_app_server_start() -> Self {
        let mut coordinator = Self::new();
        coordinator.app_server_started();
        coordinator
    }

    pub fn app_server_started(&mut self) {
        self.request(WorkspaceRescanRequest {
            reason: WorkspaceRescanReason::AppServerStarted,
            diagnostic: None,
        });
    }

    pub fn watcher_subscribed(&mut self, is_resubscription: bool) {
        self.request(WorkspaceRescanRequest {
            reason: if is_resubscription {
                WorkspaceRescanReason::WatcherResubscribed
            } else {
                WorkspaceRescanReason::WatcherSubscribed
            },
            diagnostic: None,
        });
    }

    pub fn ingest_change_batch(&mut self, batch: &WorkspaceChangeBatch) {
        for request in &batch.rescan_requests {
            self.request(request.clone());
        }
    }

    pub fn request(&mut self, request: WorkspaceRescanRequest) {
        self.pending
            .entry(request.reason)
            .and_modify(|diagnostic| {
                if diagnostic.is_none() {
                    diagnostic.clone_from(&request.diagnostic);
                }
            })
            .or_insert(request.diagnostic);
    }

    #[must_use]
    pub fn status(&self) -> WorkspaceCompensationStatus {
        if self.paused {
            WorkspaceCompensationStatus::PausedAfterInterruption
        } else if self.active.is_some() {
            WorkspaceCompensationStatus::ScanRunning
        } else if self.pending.is_empty() {
            WorkspaceCompensationStatus::Idle
        } else {
            WorkspaceCompensationStatus::ScanPending
        }
    }

    #[must_use]
    pub fn start_next(&mut self) -> Option<WorkspaceCompensationScan> {
        if self.paused || self.active.is_some() || self.pending.is_empty() {
            return None;
        }
        let requests = std::mem::take(&mut self.pending)
            .into_iter()
            .map(|(reason, diagnostic)| WorkspaceRescanRequest { reason, diagnostic })
            .collect();
        let scan = WorkspaceCompensationScan {
            generation: self.next_generation,
            requests,
        };
        self.next_generation = self.next_generation.saturating_add(1);
        self.active = Some(scan.clone());
        Some(scan)
    }

    /// 执行当前可启动的补偿扫描，并自动记录完成或中断状态。
    ///
    /// 扫描失败时保留原触发原因并暂停自动重试；调用方记录并处理错误后必须显式调用
    /// [`Self::resume_after_interruption`]，避免不可读目录形成无界重试循环。
    ///
    /// # Errors
    ///
    /// 工作区扫描失败时返回原始扫描错误。
    pub fn run_next(
        &mut self,
        root: &Path,
        cancellation: &WorkspaceCancellationToken,
        on_event: impl FnMut(WorkspaceScanEvent),
    ) -> Result<Option<WorkspaceScanOutcome>, WorkspaceScanError> {
        let Some(scan) = self.start_next() else {
            return Ok(None);
        };
        match scan_workspace(root, cancellation, on_event) {
            Ok(outcome @ WorkspaceScanOutcome::Completed(_)) => {
                self.active = None;
                Ok(Some(outcome))
            }
            Ok(outcome @ WorkspaceScanOutcome::Cancelled(_)) => {
                self.requeue_interrupted_scan(scan, Some("compensation scan cancelled".to_owned()));
                Ok(Some(outcome))
            }
            Err(error) => {
                self.requeue_interrupted_scan(scan, Some(error.to_string()));
                Err(error)
            }
        }
    }

    /// # Errors
    ///
    /// 没有活动扫描或 generation 与活动扫描不一致时返回错误。
    pub fn complete(&mut self, generation: u64) -> Result<(), WorkspaceCompensationLifecycleError> {
        self.take_active(generation).map(|_| ())
    }

    /// # Errors
    ///
    /// 没有活动扫描或 generation 与活动扫描不一致时返回错误。
    pub fn interrupt(
        &mut self,
        generation: u64,
        diagnostic: Option<String>,
    ) -> Result<(), WorkspaceCompensationLifecycleError> {
        let scan = self.take_active(generation)?;
        self.requeue_interrupted_scan(scan, diagnostic);
        Ok(())
    }

    fn requeue_interrupted_scan(
        &mut self,
        scan: WorkspaceCompensationScan,
        diagnostic: Option<String>,
    ) {
        self.active = None;
        for request in scan.requests {
            self.request(request);
        }
        self.request(WorkspaceRescanRequest {
            reason: WorkspaceRescanReason::CompensationScanInterrupted,
            diagnostic,
        });
        self.paused = true;
    }

    pub fn resume_after_interruption(&mut self) {
        self.paused = false;
    }

    fn take_active(
        &mut self,
        generation: u64,
    ) -> Result<WorkspaceCompensationScan, WorkspaceCompensationLifecycleError> {
        let Some(active) = self.active.as_ref() else {
            return Err(WorkspaceCompensationLifecycleError {
                kind: WorkspaceCompensationLifecycleErrorKind::NoActiveScan,
                expected_generation: None,
                actual_generation: generation,
            });
        };
        if active.generation != generation {
            return Err(WorkspaceCompensationLifecycleError {
                kind: WorkspaceCompensationLifecycleErrorKind::GenerationMismatch,
                expected_generation: Some(active.generation),
                actual_generation: generation,
            });
        }
        Ok(self.active.take().expect("active scan was checked above"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{WorkspaceScanEvent, WorkspaceScanSummary};
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn startup_and_initial_subscription_coalesce_into_one_full_scan() {
        let fixture = TestWorkspace::new("startup");
        fs::write(fixture.root.join("one.md"), "one").unwrap();
        let mut coordinator = WorkspaceCompensationCoordinator::for_app_server_start();
        coordinator.watcher_subscribed(false);
        let mut paths = Vec::new();
        let outcome = coordinator
            .run_next(&fixture.root, &WorkspaceCancellationToken::new(), |event| {
                if let WorkspaceScanEvent::Entry(entry) = event {
                    paths.push(entry.path.to_string());
                }
            })
            .unwrap()
            .unwrap();
        assert_eq!(paths, ["one.md"]);
        assert_eq!(
            outcome,
            WorkspaceScanOutcome::Completed(WorkspaceScanSummary {
                entries_examined: 1,
                entries_emitted: 1,
                files_emitted: 1,
                ..WorkspaceScanSummary::default()
            })
        );
        assert_eq!(coordinator.status(), WorkspaceCompensationStatus::Idle);
        assert!(
            coordinator
                .run_next(&fixture.root, &WorkspaceCancellationToken::new(), |_| {})
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn resubscription_always_schedules_a_new_generation() {
        let mut coordinator = WorkspaceCompensationCoordinator::new();
        coordinator.watcher_subscribed(false);
        let first = coordinator.start_next().unwrap();
        coordinator.complete(first.generation).unwrap();
        coordinator.watcher_subscribed(true);
        let second = coordinator.start_next().unwrap();
        assert!(second.generation > first.generation);
        assert_eq!(
            second.requests,
            [WorkspaceRescanRequest {
                reason: WorkspaceRescanReason::WatcherResubscribed,
                diagnostic: None,
            }]
        );
    }

    #[test]
    fn overflow_batch_schedules_full_scan_and_discards_path_only_work() {
        let mut coordinator = WorkspaceCompensationCoordinator::new();
        coordinator.ingest_change_batch(&WorkspaceChangeBatch {
            paths: Vec::new(),
            rescan_requests: vec![WorkspaceRescanRequest {
                reason: WorkspaceRescanReason::QueueCapacityExceeded,
                diagnostic: None,
            }],
        });
        let scan = coordinator.start_next().unwrap();
        assert_eq!(
            scan.requests[0].reason,
            WorkspaceRescanReason::QueueCapacityExceeded
        );
    }

    #[test]
    fn request_during_active_scan_runs_in_follow_up_generation() {
        let mut coordinator = WorkspaceCompensationCoordinator::new();
        coordinator.app_server_started();
        let first = coordinator.start_next().unwrap();
        coordinator.watcher_subscribed(true);
        assert!(coordinator.start_next().is_none());
        coordinator.complete(first.generation).unwrap();
        let follow_up = coordinator.start_next().unwrap();
        assert_eq!(
            follow_up.requests[0].reason,
            WorkspaceRescanReason::WatcherResubscribed
        );
    }

    #[test]
    fn cancelled_scan_pauses_until_explicit_resume_without_losing_reason() {
        let fixture = TestWorkspace::new("cancelled");
        let mut coordinator = WorkspaceCompensationCoordinator::new();
        coordinator.app_server_started();
        let cancellation = WorkspaceCancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            coordinator
                .run_next(&fixture.root, &cancellation, |_| {})
                .unwrap(),
            Some(WorkspaceScanOutcome::Cancelled(_))
        ));
        assert_eq!(
            coordinator.status(),
            WorkspaceCompensationStatus::PausedAfterInterruption
        );
        assert!(coordinator.start_next().is_none());
        coordinator.resume_after_interruption();
        let retry = coordinator.start_next().unwrap();
        assert!(
            retry
                .requests
                .iter()
                .any(|request| request.reason == WorkspaceRescanReason::AppServerStarted)
        );
        assert!(retry.requests.iter().any(|request| {
            request.reason == WorkspaceRescanReason::CompensationScanInterrupted
        }));
    }

    #[test]
    fn stale_generation_cannot_complete_current_scan() {
        let mut coordinator = WorkspaceCompensationCoordinator::new();
        coordinator.app_server_started();
        let scan = coordinator.start_next().unwrap();
        let error = coordinator.complete(scan.generation + 1).unwrap_err();
        assert_eq!(
            error.kind(),
            WorkspaceCompensationLifecycleErrorKind::GenerationMismatch
        );
        assert_eq!(
            coordinator.status(),
            WorkspaceCompensationStatus::ScanRunning
        );
    }

    #[test]
    fn unavailable_root_pauses_without_automatic_retry_and_keeps_diagnostic() {
        let fixture = TestWorkspace::new("unavailable-root");
        let mut coordinator = WorkspaceCompensationCoordinator::new();
        coordinator.app_server_started();
        assert_eq!(
            coordinator.status(),
            WorkspaceCompensationStatus::ScanPending
        );
        fs::remove_dir(&fixture.root).unwrap();
        let error = coordinator
            .run_next(&fixture.root, &WorkspaceCancellationToken::new(), |_| {})
            .unwrap_err();
        assert_eq!(error.kind(), crate::WorkspaceScanErrorKind::RootUnavailable);
        assert_eq!(
            coordinator.status(),
            WorkspaceCompensationStatus::PausedAfterInterruption
        );
        assert!(coordinator.start_next().is_none());
        coordinator.resume_after_interruption();
        let retry = coordinator.start_next().unwrap();
        let interrupted = retry
            .requests
            .iter()
            .find(|request| request.reason == WorkspaceRescanReason::CompensationScanInterrupted)
            .unwrap();
        assert!(
            interrupted
                .diagnostic
                .as_deref()
                .is_some_and(|diagnostic| diagnostic.contains("resolve scan root"))
        );
        fs::create_dir(&fixture.root).unwrap();
    }

    struct TestWorkspace {
        root: PathBuf,
    }

    impl TestWorkspace {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "contexthub-compensation-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            Self { root }
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}
