#![doc = "`ContextHub` Local App Server 的连接状态机与协议路由。"]

mod ai_business_ledger;
mod capability_creation;
pub mod capability_generation;
pub mod confirmation;
mod detached;
pub mod download_cleanup;
pub mod fault_injection;
mod file_hydration;
mod lifecycle;
pub use lifecycle::{AppServerLifecycle, LifecycleCaller, LifecycleClient, LifecycleRequest};
pub mod global_search;
mod mcp_connections;
mod model_connections;
mod model_usage;
mod personal_automation;
pub mod my_work;
pub mod personal_data;
mod personal_memory;
mod personal_storage;
mod personal_namespace_storage;
pub mod preview;
mod preview_binary;
mod profile_writers;
pub mod project_outcomes;
pub mod project_portfolio;
pub mod project_progress;
mod retired_material_cache;
pub mod skill_generation;
mod skills;
pub mod text_diff;
pub mod text_resource;
mod unified_search;
pub mod work_attention;
pub mod workspace;
mod workspace_browse;
pub mod workspace_index;
mod workspace_scan;
mod workspace_setup;
pub mod workspace_watch;

use contexthub_application::{
    DurableEventReader, PreviewRepository, UnitOfWork, WorkspaceRepository,
};
use contexthub_domain::{EventId, WorkspaceId};
use contexthub_persistence_sqlite::{
    AiStateChanges, AiStateEntityChanges, AiTurnClassificationRecord, AiTurnFinalizeCommand,
    SqlitePersistence, UserSkillSaveCommand,
};
use contexthub_protocol::{
    AgentIncidentReportParams, AgentIncidentReportResult, AiApplicationParams,
    AiConversationSetArchivedParams, AiConversationSetArchivedResult, AiStateApplyParams,
    AiStateApplyResult, AiStateDelta, AiStateEntityDelta, AiStateGetParams, AiStateGetResult,
    AiStateReplaceParams, AiStateReplaceResult, AiThreadAcknowledgeParams, AiThreadCleanupParams,
    AiThreadGetParams, AiThreadPrepareParams, AiThreadReleaseParams, AiTurnFinalizeParams,
    AiTurnFinalizeResult, AiWorkTaskCandidate, AiWorkTaskCandidatesParams,
    AiWorkTaskCandidatesResult, AttentionCaseListParams, AttentionCaseListResult,
    AttentionCaseTransitionParams, AttentionCaseTransitionResult, BUSINESS_ERROR,
    BusinessTaskCreateParams, BusinessTaskCreateResult, BusinessTaskGetParams,
    BusinessTaskGetResult, BusinessTaskMoveParams, BusinessTaskMoveResult,
    BusinessTaskTransitionParams, BusinessTaskTransitionResult, CapabilityCreationCreateParams,
    CapabilityCreationGetParams, CapabilityCreationListParams, CapabilityCreationPreviewParams,
    ClientMessage, ComputeTextDiffParams, ComputeTextDiffResult, ConfirmFileParams,
    ConfirmFileResult, ConfirmationHistoryParams, ConfirmationHistoryResult,
    DecisionRequestCreateParams, DecisionRequestCreateResult, DeliverableCreateParams,
    DeliverableCreateResult, DeliverableListParams, DeliverableListResult,
    DeliverableVersionCreateParams, DeliverableVersionCreateResult, DeliverableVersionReviewParams,
    DeliverableVersionReviewResult, DurableEventItem, EventReplayParams, EventReplayResult, Frame,
    GlobalSearchParams, GlobalSearchResult, INVALID_PARAMS, InitializeParams, InitializeResult,
    MAX_MESSAGE_BYTES, MAX_PAGE_SIZE, METHOD_NOT_FOUND, MyWorkGetParams, MyWorkGetResult,
    Notification, OpenPreviewParams, OpenPreviewResult, PROTOCOL_MAX_VERSION, PROTOCOL_MIN_VERSION,
    PersonalApplicationOpenParams, PhasePlanActivateParams, PhasePlanActivateResult,
    PhasePlanCreateParams, PhasePlanCreateResult, ProjectAttachWorkspaceParams,
    ProjectAttachWorkspaceResult, ProjectBlockerCreateParams, ProjectBlockerCreateResult,
    ProjectCreateParams, ProjectCreateResult, ProjectDeleteParams, ProjectDeleteResult,
    ProjectDetachWorkspaceParams, ProjectDetachWorkspaceResult, ProjectGetParams, ProjectGetResult,
    ProjectListParams, ProjectListResult, ProjectPhaseTransitionParams,
    ProjectPhaseTransitionResult, ProjectPortfolioListParams, ProjectPortfolioListResult,
    ProjectProgressGetParams, ProjectProgressGetResult, ProjectUpdateParams, ProjectUpdateResult,
    ProtocolLimits, ReadTextDiffParams, ReadTextDiffResult, ReadTextResourceParams,
    ReadTextResourceResult, Request, ResumeResult, ServerCapabilities, ServerInfo, SessionInfo,
    SkillConversationCatalogParams, SkillDeleteParams, SkillGetParams, SkillImportDirectoryParams,
    SkillImportParams, SkillListParams, SkillRunListParams, SkillRunRetryParams,
    SkillRuntimeCatalogParams, SkillSaveParams, SkillSetStateParams, UnifiedSearchParams,
    WorkspaceCreateParams, WorkspaceCreateResult, WorkspaceDeleteParams, WorkspaceDeleteResult,
    WorkspaceDirectoryCreateParams, WorkspaceFileItem, WorkspaceFilesParams, WorkspaceFilesResult,
    WorkspaceListParams, WorkspaceListResult, WorkspaceScanParams, WorkspaceScanResult,
    WorkspaceSummary, WorkspaceTargetValidateParams, WorkspaceTargetValidateResult, business_error,
    error, negotiate_protocol, parse_client_message, success,
};
use serde_json::{Value, json};
use std::fs;
use std::future::{Future, poll_fn};
use std::io;
use std::path::PathBuf;
use std::pin::pin;
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;

fn personal_method_removed(method: &str) -> bool {
    [
        "capture/",
        "cloudWorkspace/",
        "syncPlan/",
        "sync/",
        "syncExecution/",
        "conflict/",
        "employeeCredential/",
    ]
    .iter()
    .any(|prefix| method.starts_with(prefix))
        || matches!(
            method,
            "applicationCatalog/list"
                | "applicationCatalog/launch"
                | "aiBrowserPolicy/get"
                | "personalCapability/enterpriseBinding"
                | "personalCapability/enterpriseImport"
                | "personalCapability/enterpriseAuthorize"
                | "personalCapability/reviewPackage"
        )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    New,
    InitializeResponded,
    Ready,
    ShutdownPrepared,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteResult {
    pub response: Option<String>,
    pub should_exit: bool,
}

pub struct ConnectionRouter {
    phase: Phase,
    server_instance_id: String,
    connection_id: String,
    client_session_id: String,
    local_ipc_available: bool,
    services: Option<SharedAppServerServices>,
    source_client: Option<&'static str>,
    dedicated_search: bool,
    /// 首次用到时从服务里取一份，之后查索引进度不再经过服务主锁。
    index_board: Option<Arc<workspace_index::WorkspaceIndexBoard>>,
    lifecycle: Option<(AppServerLifecycle, LifecycleCaller)>,
}

pub type SharedAppServerServices = Arc<Mutex<AppServerServices>>;

pub struct AppServerServices {
    persistence: Option<SqlitePersistence>,
    data_dir: Option<PathBuf>,
    text_tickets: text_resource::TextResourceTickets,
    diff_tickets: text_diff::TextDiffTickets,
    hydration_tasks: file_hydration::HydrationTasks,
    search_cursors: unified_search::LocalSearchCursors,
    /// 后台索引进度。自带锁，读它不需要服务主锁。
    index_board: Arc<workspace_index::WorkspaceIndexBoard>,
    directory_reads: Arc<workspace_browse::DirectoryReads>,
}

impl AppServerServices {
    pub(crate) fn shutdown_blocking_reasons(&self) -> Result<Vec<String>, String> {
        self.persistence.as_ref().map_or_else(
            || Ok(vec![]),
            |persistence| {
                persistence
                    .personal_shutdown_blocking_reasons()
                    .map_err(|_| "app_server_writers_unconfirmed".into())
            },
        )
    }
    #[must_use]
    pub fn new(persistence: Option<SqlitePersistence>) -> Self {
        Self::with_data_dir(persistence, None)
    }

    #[must_use]
    pub fn with_data_dir(
        persistence: Option<SqlitePersistence>,
        data_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            persistence,
            data_dir,
            text_tickets: text_resource::TextResourceTickets::default(),
            diff_tickets: text_diff::TextDiffTickets::default(),
            hydration_tasks: file_hydration::HydrationTasks::default(),
            search_cursors: unified_search::LocalSearchCursors::default(),
            index_board: Arc::default(),
            directory_reads: Arc::default(),
        }
    }

    /// 后台索引进度登记处。连接和监听主管各持一份，之后读写都不经过服务主锁。
    #[must_use]
    pub fn index_board(&self) -> Arc<workspace_index::WorkspaceIndexBoard> {
        Arc::clone(&self.index_board)
    }

    fn ai_conversation_set_archived(
        &mut self,
        params: &AiConversationSetArchivedParams,
        now: i64,
    ) -> Result<AiConversationSetArchivedResult, ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let (state_revision, conversations, changed_conversation_ids) = persistence
            .set_ai_conversations_archived(
                params.expected_revision,
                &params.conversation_ids,
                params.archived,
                now,
            )
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("ai_state_write_failed"))
            })?;
        Ok(AiConversationSetArchivedResult {
            state_revision,
            conversations,
            changed_conversation_ids,
        })
    }

    fn ai_application(
        &mut self,
        params: &AiApplicationParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let value = serde_json::to_value(params)
            .map_err(|_| ServiceError::new("application_conversation_request_invalid"))?;
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .application_conversation(
                value["action"]
                    .as_str()
                    .ok_or_else(|| ServiceError::new("application_conversation_request_invalid"))?,
                &params.payload,
                now,
            )
            .map_err(|e| {
                ServiceError::new(
                    e.reason()
                        .unwrap_or("application_conversation_storage_failed"),
                )
            })
    }

    fn ai_thread_get(&mut self, params: &AiThreadGetParams) -> Result<Value, ServiceError> {
        self.persistence
            .as_ref()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .ai_thread_state(&params.conversation_id)
            .map_err(|e| ServiceError::new(e.reason().unwrap_or("ai_thread_read_failed")))
    }

    fn ai_thread_prepare(
        &mut self,
        params: &AiThreadPrepareParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let value = serde_json::to_value(params)
            .map_err(|_| ServiceError::new("ai_thread_request_invalid"))?;
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .prepare_ai_thread_operation(&value, now)
            .map_err(|e| ServiceError::new(e.reason().unwrap_or("ai_thread_write_failed")))
    }

    fn ai_thread_acknowledge(
        &mut self,
        params: &AiThreadAcknowledgeParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let value = serde_json::to_value(params)
            .map_err(|_| ServiceError::new("ai_thread_request_invalid"))?;
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .acknowledge_ai_thread_operation(&value, now)
            .map_err(|e| ServiceError::new(e.reason().unwrap_or("ai_thread_write_failed")))
    }

    fn ai_thread_reconcile(
        &mut self,
        params: &AiThreadAcknowledgeParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let mut value = serde_json::to_value(params)
            .map_err(|_| ServiceError::new("ai_thread_request_invalid"))?;
        value["reconciliation"] = json!(true);
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .acknowledge_ai_thread_operation(&value, now)
            .map_err(|e| ServiceError::new(e.reason().unwrap_or("ai_thread_write_failed")))
    }

    fn ai_thread_release(
        &mut self,
        params: &AiThreadReleaseParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let value = serde_json::to_value(params)
            .map_err(|_| ServiceError::new("ai_thread_request_invalid"))?;
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .release_ai_thread_writer(&value, now)
            .map_err(|e| ServiceError::new(e.reason().unwrap_or("ai_thread_write_failed")))
    }

    fn ai_thread_cleanup(
        &mut self,
        params: &AiThreadCleanupParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        let value = serde_json::to_value(params)
            .map_err(|_| ServiceError::new("ai_thread_request_invalid"))?;
        self.persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .record_ai_thread_cleanup(&value, now)
            .map_err(|error| ServiceError::new(error.reason().unwrap_or("ai_thread_write_failed")))
    }

    fn ai_state_get(&mut self, _: &AiStateGetParams) -> Result<AiStateGetResult, ServiceError> {
        let persistence = self
            .persistence
            .as_ref()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let (state_revision, state) = persistence
            .ai_runtime_state()
            .map_err(|error| ServiceError::new(error.reason().unwrap_or("ai_state_read_failed")))?;
        Ok(AiStateGetResult {
            state_revision,
            state,
        })
    }

    fn ai_state_replace(
        &mut self,
        params: &AiStateReplaceParams,
        now: i64,
    ) -> Result<AiStateReplaceResult, ServiceError> {
        if !params.state.is_object() {
            return Err(ServiceError::new("ai_state_invalid"));
        }
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let state_revision = persistence
            .replace_ai_runtime_state(params.expected_revision, &params.state, now)
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("ai_state_write_failed"))
            })?;
        Ok(AiStateReplaceResult { state_revision })
    }

    fn ai_state_apply(
        &mut self,
        params: &AiStateApplyParams,
        now: i64,
    ) -> Result<AiStateApplyResult, ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let state_revision = persistence
            .apply_ai_runtime_changes(
                params.expected_revision,
                &ai_state_changes(&params.delta),
                now,
            )
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("ai_state_write_failed"))
            })?;
        Ok(AiStateApplyResult { state_revision })
    }

    fn ai_work_task_candidates(
        &mut self,
        params: &AiWorkTaskCandidatesParams,
    ) -> Result<AiWorkTaskCandidatesResult, ServiceError> {
        if params.project_id.trim().is_empty() {
            return Err(ServiceError::new("project_required"));
        }
        let persistence = self
            .persistence
            .as_ref()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let tasks = persistence
            .ai_work_task_candidates(&params.project_id)
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("ai_work_candidates_read_failed"))
            })?
            .into_iter()
            .map(|task| AiWorkTaskCandidate {
                task_id: task.task_id,
                title: task.title,
                objective: task.objective,
                expected_output: task.expected_output,
                completion_criteria: task.completion_criteria,
                stage: task.stage,
                state_revision: task.state_revision,
            })
            .collect();
        Ok(AiWorkTaskCandidatesResult {
            project_id: params.project_id.clone(),
            tasks,
        })
    }

    fn ai_turn_finalize(
        &mut self,
        params: &AiTurnFinalizeParams,
        now: i64,
    ) -> Result<AiTurnFinalizeResult, ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let classification = &params.classification;
        let result = persistence
            .finalize_ai_turn(&AiTurnFinalizeCommand {
                ledger_request: Some(ai_business_ledger::prepare_ledger_request(&params.ledger)?),
                expected_revision: 0,
                changes: AiStateChanges::default(),
                conversation_id: params.conversation_id.clone(),
                run_id: params.run_id.clone(),
                project_id: params.project_id.clone(),
                previous_task_id: params.previous_task_id.clone(),
                agent_label: params.agent_label.clone(),
                new_task_id: params.new_task_id.clone(),
                classification: AiTurnClassificationRecord {
                    action: classification.action.clone(),
                    task_action: classification.task_action.clone(),
                    task_id: classification.task_id.clone(),
                    task_title: classification.task_title.clone(),
                    objective: classification.objective.clone(),
                    expected_output: classification.expected_output.clone(),
                    completion_criteria: classification.completion_criteria.clone(),
                    conversation_status: classification.conversation_status.clone(),
                    stage: classification.stage.clone(),
                    outcome_title: classification.outcome_title.clone(),
                    outcome_summary: classification.outcome_summary.clone(),
                    reason: classification.reason.clone(),
                    confidence: classification.confidence,
                    raw: classification.raw.clone(),
                },
                occurred_at: now,
            })
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("ai_turn_finalize_failed"))
            })?;
        Ok(AiTurnFinalizeResult {
            state_revision: result.state_revision,
            ledger_receipt: result
                .ledger_receipt
                .ok_or_else(|| ServiceError::new("ai_ledger_receipt_invalid"))?,
            task_id: result.task_id,
            task_title: result.task_title,
            stage: result.stage,
            replayed: result.replayed,
        })
    }

    fn open_preview(
        &mut self,
        params: &OpenPreviewParams,
        now: i64,
    ) -> Result<OpenPreviewResult, ServiceError> {
        if params.workspace_id.is_empty() || params.local_file_id.is_empty() {
            return Err(ServiceError::new("invalid_preview_request"));
        }
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("preview_persistence_unavailable"))?;
        let workspace_id = WorkspaceId::new(params.workspace_id.clone());
        let record = persistence
            .transact(|repositories| {
                repositories.preview_file(&workspace_id, &params.local_file_id)
            })
            .map_err(|_| ServiceError::new("preview_state_read_failed"))?
            .ok_or_else(|| ServiceError::new("preview_file_not_found"))?;
        preview::open_preview(
            &mut self.text_tickets,
            &record,
            params.source,
            format!("preview-ticket-{}", uuid::Uuid::new_v4()),
            now,
        )
        .map_err(|error| ServiceError::new(error.reason()))
    }

    fn read_binary_preview(
        &mut self,
        params: &contexthub_protocol::ReadBinaryPreviewParams,
        _now: i64,
    ) -> Result<contexthub_protocol::ReadBinaryPreviewResult, ServiceError> {
        let record = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("preview_persistence_unavailable"))?
            .transact(|repositories| {
                repositories.preview_file(
                    &WorkspaceId::new(params.workspace_id.clone()),
                    &params.local_file_id,
                )
            })
            .map_err(|_| ServiceError::new("preview_state_read_failed"))?
            .ok_or_else(|| ServiceError::new("preview_file_not_found"))?;
        preview_binary::read(&record).map_err(ServiceError::new)
    }

    fn compute_text_diff(
        &mut self,
        params: &ComputeTextDiffParams,
        now: i64,
    ) -> Result<ComputeTextDiffResult, ServiceError> {
        self.diff_tickets
            .compute(
                &mut self.text_tickets,
                format!("diff-ticket-{}", uuid::Uuid::new_v4()),
                params,
                now,
            )
            .map_err(|error| ServiceError::new(error.reason()))
    }

    fn read_text_diff(
        &mut self,
        params: &ReadTextDiffParams,
        now: i64,
    ) -> Result<ReadTextDiffResult, ServiceError> {
        self.diff_tickets
            .read(
                &mut self.text_tickets,
                &params.diff_ticket_id,
                params.cursor,
                params.max_rows,
                params.max_bytes,
                now,
            )
            .map_err(|error| ServiceError::new(error.reason()))
    }

    fn confirm_file(
        &mut self,
        params: &ConfirmFileParams,
        source_client: &str,
        now: i64,
    ) -> Result<ConfirmFileResult, ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("confirmation_persistence_unavailable"))?;
        confirmation::confirm_file(persistence, params, source_client, now)
            .map_err(|error| ServiceError::new(error.reason()))
    }

    fn confirmation_history(
        &mut self,
        params: &ConfirmationHistoryParams,
    ) -> Result<ConfirmationHistoryResult, ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("confirmation_persistence_unavailable"))?;
        confirmation::confirmation_history(persistence, params)
            .map_err(|error| ServiceError::new(error.reason()))
    }

    fn read_text_resource(
        &mut self,
        params: &ReadTextResourceParams,
        now: i64,
    ) -> Result<ReadTextResourceResult, ServiceError> {
        preview::read_preview(
            &mut self.text_tickets,
            &params.ticket_id,
            params.offset,
            params.max_bytes,
            now,
        )
        .map_err(|error| ServiceError::new(error.reason()))
    }

    /// 当前登记的工作区根目录，供文件监听决定要盯哪些目录。
    ///
    /// # Errors
    ///
    /// 持久化不可用或读取失败时返回错误。
    pub(crate) fn watched_workspace_roots(
        &mut self,
    ) -> Result<Vec<(String, String)>, ServiceError> {
        Ok(self
            .workspace_list(&WorkspaceListParams {})?
            .workspaces
            .into_iter()
            .filter(|workspace| workspace.workspace_kind == "local")
            .map(|workspace| (workspace.workspace_id, workspace.local_root))
            .collect())
    }

    /// 文件发生变化后重新扫描一个工作区。
    ///
    /// 设计上监听只是"需要重新检查"的信号，事实仍由扫描重新计算得出——
    /// 监听事件本身不足以判断内容变没变，也给不出 checksum。
    ///
    /// # Errors
    ///
    /// 扫描失败或工作区已不存在时返回错误。
    /// 增量重扫的第一段：读出比对所需的事实。
    ///
    /// 拆成两段是为了让最慢的那一步——走一遍目录——落在锁外。整个捕获里只有读取和
    /// 提交需要持锁；扫描占住服务主锁会让界面在每次文件变化时卡住整个扫描时长。
    ///
    /// # Errors
    ///
    /// 持久化不可用或工作区不存在时返回错误。
    pub(crate) fn prepare_watched_rescan(
        &mut self,
        workspace_id: &str,
    ) -> Result<workspace::IncrementalScanBasis, ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        workspace::prepare_incremental_scan(persistence, &WorkspaceId::new(workspace_id.to_owned()))
            .map_err(|error| ServiceError::new(workspace::index_failure_reason(&error)))
    }

    /// 增量重扫的第二段：把扫描结果提交成一次快照。
    ///
    /// # Errors
    ///
    /// 持久化不可用，或扫描期间有别的写入落库导致 generation 冲突时返回错误。
    pub(crate) fn commit_watched_rescan(
        &mut self,
        command: &workspace::CaptureLocalSnapshotCommand,
        findings: workspace::IncrementalScanFindings,
        is_last: bool,
    ) -> Result<(), ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        workspace::commit_incremental_batch(persistence, command, findings, is_last)
            .map_err(|error| ServiceError::new(workspace::index_failure_reason(&error)))?;
        Ok(())
    }

    /// 一次监听重扫用的指令。
    ///
    /// 扫描和提交必须共用同一份：观测里的 `proposed_local_file_id` 由 `snapshot_id` 拼出来，
    /// 而它会成为新文件的 `local_file_id`。两段各生成一次的话，先扫后提交之间 id 就对不上；
    /// 更糟的是若留空，两次扫描都会产出 `:entry:1`，第二次提交撞主键。
    pub(crate) fn watched_rescan_command(
        workspace_id: &str,
        now: i64,
    ) -> workspace::CaptureLocalSnapshotCommand {
        let id = uuid::Uuid::new_v4();
        workspace::CaptureLocalSnapshotCommand {
            workspace_id: WorkspaceId::new(workspace_id.to_owned()),
            snapshot_id: format!("watch-snapshot-{id}"),
            event_id: EventId::new(format!("watch-snapshot-event-{id}")),
            correlation_id: format!("watch-snapshot-correlation-{id}"),
            observed_at: now,
            completed_at: now,
        }
    }

    fn workspace_list(
        &mut self,
        _: &WorkspaceListParams,
    ) -> Result<WorkspaceListResult, ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let workspaces = persistence
            .transact(|repositories| {
                repositories.workspaces().and_then(|items| {
                    items
                        .into_iter()
                        .filter(|workspace| {
                            if workspace.workspace_kind.as_str() != "local" {
                                return true;
                            }
                            match fs::metadata(&workspace.local_root) {
                                Ok(metadata) => metadata.is_dir(),
                                Err(error) => !matches!(
                                    error.kind(),
                                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                                ),
                            }
                        })
                        .map(|workspace| {
                            let binding = repositories.platform_binding(&workspace.workspace_id)?;
                            Ok(WorkspaceSummary {
                                workspace_id: workspace.workspace_id.as_str().to_owned(),
                                workspace_kind: workspace.workspace_kind.as_str().to_owned(),
                                display_name: workspace.display_name,
                                local_root: workspace.local_root,
                                local_status: workspace.local_status.as_str().to_owned(),
                                state_revision: workspace.state_revision,
                                platform_bound: binding
                                    .as_ref()
                                    .is_some_and(|value| value.binding_state.as_str() == "bound"),
                                platform_revision: binding
                                    .as_ref()
                                    .and_then(|value| value.platform_snapshot_revision.clone()),
                                team_space_id: binding
                                    .as_ref()
                                    .and_then(|value| value.team_space_id.clone()),
                                managed_root: binding
                                    .as_ref()
                                    .and_then(|value| value.managed_root.clone()),
                                device_id: binding
                                    .as_ref()
                                    .and_then(|value| value.device_id.clone()),
                                baseline_revision: binding
                                    .as_ref()
                                    .and_then(|value| value.baseline_revision.clone()),
                                change_cursor: binding
                                    .as_ref()
                                    .and_then(|value| value.change_cursor.clone()),
                                permission_role: binding
                                    .as_ref()
                                    .and_then(|value| value.permission_role.clone()),
                                permission_revision: binding
                                    .and_then(|value| value.permission_revision),
                            })
                        })
                        .collect()
                })
            })
            .map_err(|_| ServiceError::new("workspace_list_failed"))?;
        Ok(WorkspaceListResult { workspaces })
    }

    fn workspace_create(
        &mut self,
        params: &WorkspaceCreateParams,
        source_client: &str,
        now: i64,
    ) -> Result<WorkspaceCreateResult, ServiceError> {
        if params.local_root.is_empty()
            || params.display_name.is_empty()
            || params.client_operation_id.is_empty()
        {
            return Err(ServiceError::new("invalid_workspace_create_params"));
        }
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let protected = persistence
            .database_path()
            .parent()
            .map(std::path::Path::to_path_buf)
            .into_iter()
            .collect::<Vec<_>>();
        let id = uuid::Uuid::new_v4();
        let result = workspace::create_preflighted_local_workspace(
            persistence,
            std::path::Path::new(&params.local_root),
            &protected,
            workspace::local_workspace_command(
                &format!("workspace-{id}"),
                &params.display_name,
                &format!("scan-task-{id}"),
                &format!("workspace-event-{id}"),
                &format!("workspace-correlation-{id}"),
                source_client,
                now,
            ),
        )
        .map_err(workspace_create_error)?;
        let workspace = result.business;
        let scan_task_id = persistence
            .workspace_scan_task_id(workspace.workspace_id.as_str())
            .map_err(|_| ServiceError::new("workspace_create_persistence_failed"))?;
        Ok(WorkspaceCreateResult {
            workspace: WorkspaceSummary {
                workspace_id: workspace.workspace_id.as_str().to_owned(),
                workspace_kind: workspace.workspace_kind.as_str().to_owned(),
                display_name: workspace.display_name,
                local_root: workspace.local_root,
                local_status: workspace.local_status.as_str().to_owned(),
                state_revision: workspace.state_revision,
                platform_bound: false,
                platform_revision: None,
                team_space_id: None,
                managed_root: None,
                device_id: None,
                baseline_revision: None,
                change_cursor: None,
                permission_role: None,
                permission_revision: None,
            },
            scan_task_id,
        })
    }

    fn workspace_target_validate(
        &mut self,
        params: &WorkspaceTargetValidateParams,
        _now: i64,
    ) -> Result<WorkspaceTargetValidateResult, ServiceError> {
        let workspace = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .transact(|repositories| {
                repositories.workspace(&WorkspaceId::new(&params.workspace_id))
            })
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("workspace_target_invalid"))
            })?
            .ok_or_else(|| ServiceError::new("workspace_not_found"))?;
        if workspace.workspace_kind.as_str() != "local" {
            return Err(ServiceError::new("personal_cloud_source_unavailable"));
        }
        Ok(WorkspaceTargetValidateResult {
            workspace_id: params.workspace_id.clone(),
            workspace_kind: workspace.workspace_kind.as_str().to_owned(),
            local_root: workspace.local_root,
            team_space_id: None,
            sync_eligible: false,
            permission_role: None,
        })
    }

    fn workspace_scan(
        &mut self,
        params: &WorkspaceScanParams,
        now: i64,
    ) -> Result<WorkspaceScanResult, ServiceError> {
        let workspace_id = WorkspaceId::new(params.workspace_id.clone());
        let workspace = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .transact(|repositories| repositories.workspace(&workspace_id))
            .map_err(|_| ServiceError::new("workspace_scan_state_failed"))?
            .ok_or_else(|| ServiceError::new("workspace_not_found"))?;
        if workspace.workspace_kind.as_str() != "local" {
            return Err(ServiceError::new("personal_cloud_source_unavailable"));
        }
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let id = uuid::Uuid::new_v4();
        let result = workspace::capture_full_local_snapshot(
            persistence,
            &workspace::CaptureLocalSnapshotCommand {
                workspace_id: workspace_id.clone(),
                snapshot_id: format!("snapshot-{id}"),
                event_id: EventId::new(format!("snapshot-event-{id}")),
                correlation_id: format!("snapshot-correlation-{id}"),
                observed_at: now,
                completed_at: now,
            },
            &contexthub_workspace_fs::WorkspaceCancellationToken::new(),
        )
        .map_err(|_| ServiceError::new("workspace_scan_failed"))?;
        let workspace = persistence
            .transact(|repositories| repositories.workspace(&workspace_id))
            .map_err(|_| ServiceError::new("workspace_scan_state_failed"))?
            .ok_or_else(|| ServiceError::new("workspace_not_found"))?;
        let observed_count = i64::try_from(
            result.business.retained_local_file_ids.len()
                + result.business.created_local_file_ids.len(),
        )
        .map_err(|_| ServiceError::new("workspace_scan_count_out_of_range"))?;
        Ok(WorkspaceScanResult {
            workspace_id: params.workspace_id.clone(),
            generation: workspace.state_revision,
            observed_count,
            state_revision: workspace.state_revision,
        })
    }

    fn workspace_files(
        &mut self,
        params: &WorkspaceFilesParams,
    ) -> Result<WorkspaceFilesResult, ServiceError> {
        const DEFAULT_FILE_PAGE_SIZE: u32 = 200;
        let limit = params.limit.unwrap_or(DEFAULT_FILE_PAGE_SIZE);
        if limit == 0 || limit > MAX_PAGE_SIZE {
            return Err(ServiceError::new("workspace_files_page_size_invalid"));
        }
        let workspace_id = WorkspaceId::new(params.workspace_id.clone());
        let workspace = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .transact(|repositories| repositories.workspace(&workspace_id))
            .map_err(|_| ServiceError::new("workspace_files_failed"))?
            .ok_or_else(|| ServiceError::new("workspace_not_found"))?;
        if workspace.workspace_kind.as_str() != "local" {
            return Err(ServiceError::new("personal_cloud_source_unavailable"));
        }
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        persistence
            .transact(|repositories| {
                repositories
                    .workspace_files_page(
                        &workspace_id,
                        params.cursor.as_deref(),
                        usize::try_from(limit).expect("u32 limit fits usize"),
                    )
                    .map(|page| WorkspaceFilesResult {
                        workspace_id: params.workspace_id.clone(),
                        workspace_revision: workspace.state_revision,
                        files: page
                            .files
                            .into_iter()
                            .map(|file| WorkspaceFileItem {
                                local_file_id: file.local_file_id,
                                relative_path: file.relative_path,
                                file_type: file.file_type,
                                presence_state: file.presence_state,
                                display_state: file.display_state,
                                baseline_relation: file.baseline_relation,
                                local_change_state: file.local_change_state,
                                confirmation_state: file.confirmation_state,
                                conflict_state: file.conflict_state,
                                state_revision: file.state_revision,
                                checksum_algorithm: file.checksum_algorithm,
                                checksum_value: file.checksum_value,
                                baseline_version_id: file.baseline_version_id,
                            })
                            .collect(),
                        next_cursor: page.next_cursor,
                        has_more: page.has_more,
                    })
            })
            .map_err(|_| ServiceError::new("workspace_files_failed"))
    }

    fn project_list(
        &mut self,
        params: &ProjectListParams,
    ) -> Result<ProjectListResult, ServiceError> {
        project_outcomes::list_projects(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_create(
        &mut self,
        params: &ProjectCreateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<ProjectCreateResult, ServiceError> {
        let persistence = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        project_outcomes::create_project(persistence, params, now)
            .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_attach_workspace(
        &mut self,
        params: &ProjectAttachWorkspaceParams,
        _source_client: &str,
        now: i64,
    ) -> Result<ProjectAttachWorkspaceResult, ServiceError> {
        project_outcomes::attach_project_workspace(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_delete(
        &mut self,
        params: &ProjectDeleteParams,
        _source_client: &str,
        now: i64,
    ) -> Result<ProjectDeleteResult, ServiceError> {
        project_outcomes::delete_project(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_detach_workspace(
        &mut self,
        params: &ProjectDetachWorkspaceParams,
        _source_client: &str,
        now: i64,
    ) -> Result<ProjectDetachWorkspaceResult, ServiceError> {
        project_outcomes::detach_project_workspace(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn workspace_delete(
        &mut self,
        params: &WorkspaceDeleteParams,
        _source_client: &str,
        now: i64,
    ) -> Result<WorkspaceDeleteResult, ServiceError> {
        project_outcomes::delete_workspace(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_get(&mut self, params: &ProjectGetParams) -> Result<ProjectGetResult, ServiceError> {
        project_outcomes::get_project(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_portfolio_list(
        &mut self,
        params: &ProjectPortfolioListParams,
    ) -> Result<ProjectPortfolioListResult, ServiceError> {
        project_portfolio::list_portfolio(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn deliverable_list(
        &mut self,
        params: &DeliverableListParams,
    ) -> Result<DeliverableListResult, ServiceError> {
        project_outcomes::list_deliverables(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn deliverable_create(
        &mut self,
        params: &DeliverableCreateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<DeliverableCreateResult, ServiceError> {
        project_outcomes::create_deliverable(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn deliverable_version_create(
        &mut self,
        params: &DeliverableVersionCreateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<DeliverableVersionCreateResult, ServiceError> {
        project_outcomes::create_deliverable_version(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_progress_get(
        &mut self,
        params: &ProjectProgressGetParams,
    ) -> Result<ProjectProgressGetResult, ServiceError> {
        project_progress::get_progress(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn my_work_get(&mut self, params: &MyWorkGetParams) -> Result<MyWorkGetResult, ServiceError> {
        my_work::get_my_work(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn global_search(
        &mut self,
        params: &GlobalSearchParams,
    ) -> Result<GlobalSearchResult, ServiceError> {
        global_search::query(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn attention_case_list(
        &mut self,
        params: &AttentionCaseListParams,
    ) -> Result<AttentionCaseListResult, ServiceError> {
        work_attention::list_cases(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_blocker_create(
        &mut self,
        params: &ProjectBlockerCreateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<ProjectBlockerCreateResult, ServiceError> {
        work_attention::create_blocker(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn decision_request_create(
        &mut self,
        params: &DecisionRequestCreateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<DecisionRequestCreateResult, ServiceError> {
        work_attention::create_decision_request(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_update(
        &mut self,
        params: &ProjectUpdateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<ProjectUpdateResult, ServiceError> {
        project_outcomes::update_project(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn agent_incident_report(
        &mut self,
        params: &AgentIncidentReportParams,
        _source_client: &str,
        now: i64,
    ) -> Result<AgentIncidentReportResult, ServiceError> {
        work_attention::report_agent_incident(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn attention_case_transition(
        &mut self,
        params: &AttentionCaseTransitionParams,
        _source_client: &str,
        now: i64,
    ) -> Result<AttentionCaseTransitionResult, ServiceError> {
        work_attention::transition_case(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn phase_plan_create(
        &mut self,
        params: &PhasePlanCreateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<PhasePlanCreateResult, ServiceError> {
        project_progress::create_phase_plan(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn phase_plan_activate(
        &mut self,
        params: &PhasePlanActivateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<PhasePlanActivateResult, ServiceError> {
        project_progress::activate_phase_plan(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn project_phase_transition(
        &mut self,
        params: &ProjectPhaseTransitionParams,
        _source_client: &str,
        now: i64,
    ) -> Result<ProjectPhaseTransitionResult, ServiceError> {
        project_progress::transition_phase(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn business_task_create(
        &mut self,
        params: &BusinessTaskCreateParams,
        _source_client: &str,
        now: i64,
    ) -> Result<BusinessTaskCreateResult, ServiceError> {
        project_progress::create_task(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn business_task_get(
        &mut self,
        params: &BusinessTaskGetParams,
    ) -> Result<BusinessTaskGetResult, ServiceError> {
        project_progress::get_task(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn business_task_transition(
        &mut self,
        params: &BusinessTaskTransitionParams,
        _source_client: &str,
        now: i64,
    ) -> Result<BusinessTaskTransitionResult, ServiceError> {
        project_progress::transition_task(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn business_task_move(
        &mut self,
        params: &BusinessTaskMoveParams,
        _source_client: &str,
        now: i64,
    ) -> Result<BusinessTaskMoveResult, ServiceError> {
        project_progress::move_task(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn deliverable_version_review(
        &mut self,
        params: &DeliverableVersionReviewParams,
        _source_client: &str,
        now: i64,
    ) -> Result<DeliverableVersionReviewResult, ServiceError> {
        project_outcomes::review_deliverable_version(
            self.persistence
                .as_mut()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?,
            params,
            now,
        )
        .map_err(|error| project_outcome_service_error(&error))
    }

    fn event_replay(
        &mut self,
        params: &EventReplayParams,
    ) -> Result<EventReplayResult, ServiceError> {
        let persistence = self
            .persistence
            .as_ref()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?;
        let page = params
            .after_sequence
            .map_or_else(
                || persistence.replay_global_available(params.limit as usize),
                |cursor| persistence.replay_global_after(cursor, params.limit as usize),
            )
            .map_err(|_| ServiceError::new("event_replay_failed"))?;
        Ok(EventReplayResult {
            events: page
                .events
                .into_iter()
                .map(|event| DurableEventItem {
                    sequence: event.global_sequence,
                    event_id: event.event_id.as_str().to_owned(),
                    event_type: event.event_type,
                    workspace_id: event.workspace_id.map(|value| value.as_str().to_owned()),
                    object_type: event.object_type,
                    object_id: event.object_id,
                    payload_json: event.payload_json,
                    correlation_id: event.correlation_id,
                    occurred_at: event.occurred_at,
                })
                .collect(),
            next_sequence: page.next_sequence,
            has_more: page.has_more,
        })
    }
}

#[derive(Debug)]
struct ServiceError {
    reason: &'static str,
}

/// 将所选目录的读取和持久化错误转换为界面原因。
fn workspace_create_error(error: workspace::CreateLocalWorkspaceError) -> ServiceError {
    use contexthub_workspace_fs::LocalWorkspaceRootErrorKind as Kind;
    let root = match error {
        workspace::CreateLocalWorkspaceError::Persistence(_) => {
            return ServiceError::new("workspace_create_persistence_failed");
        }
        workspace::CreateLocalWorkspaceError::Root(root) => root,
    };
    ServiceError::new(match root.kind() {
        Kind::RootUnavailable => "workspace_root_unavailable",
        Kind::RootNotDirectory => "workspace_root_not_directory",
        Kind::RootNonUnicode => "workspace_root_non_unicode",
        Kind::ProtectedRootUnavailable => "workspace_root_protected",
        Kind::EnvironmentConflict => "workspace_root_environment_conflict",
    })
}

impl ServiceError {
    const fn new(reason: &'static str) -> Self {
        Self { reason }
    }
}

fn project_outcome_service_error(
    error: &contexthub_persistence_sqlite::PersistenceError,
) -> ServiceError {
    ServiceError::new(error.reason().unwrap_or("project_outcome_operation_failed"))
}

impl ConnectionRouter {
    #[must_use]
    pub fn new(server_instance_id: String) -> Self {
        Self::with_context(
            server_instance_id,
            "conn_stdio_1".to_owned(),
            "cs_stdio_1".to_owned(),
            false,
        )
    }

    #[must_use]
    pub fn with_context(
        server_instance_id: String,
        connection_id: String,
        client_session_id: String,
        local_ipc_available: bool,
    ) -> Self {
        Self {
            phase: Phase::New,
            server_instance_id,
            connection_id,
            client_session_id,
            local_ipc_available,
            services: None,
            source_client: None,
            dedicated_search: false,
            index_board: None,
            lifecycle: None,
        }
    }

    #[must_use]
    pub fn with_services(
        server_instance_id: String,
        connection_id: String,
        client_session_id: String,
        local_ipc_available: bool,
        services: SharedAppServerServices,
    ) -> Self {
        Self {
            phase: Phase::New,
            server_instance_id,
            connection_id,
            client_session_id,
            local_ipc_available,
            services: Some(services),
            source_client: None,
            dedicated_search: false,
            index_board: None,
            lifecycle: None,
        }
    }

    #[must_use]
    pub fn with_lifecycle(
        mut self,
        lifecycle: AppServerLifecycle,
        caller: LifecycleCaller,
    ) -> Self {
        self.lifecycle = Some((lifecycle, caller));
        self
    }

    fn shutdown_decision(&self, commit: bool) -> Result<lifecycle::ShutdownDecision, String> {
        let (lifecycle, caller) = self.lifecycle.clone().unwrap_or_else(|| {
            (
                AppServerLifecycle::new(false),
                LifecycleCaller::Disconnected,
            )
        });
        lifecycle.shutdown_decision(caller, self.services.as_ref(), commit)
    }

    fn spawn_detached_call(
        &self,
        call: detached::DetachedCall,
        sender: tokio::sync::mpsc::UnboundedSender<String>,
    ) -> io::Result<()> {
        let activity = self
            .lifecycle
            .as_ref()
            .map(|(lifecycle, _)| lifecycle.begin_detached_request())
            .transpose()
            .map_err(io::Error::other)?;
        tokio::task::spawn_blocking(move || {
            let _activity = activity;
            let _ = sender.send(call());
        });
        Ok(())
    }

    #[must_use]
    pub fn handle_frame(&mut self, frame: Frame) -> RouteResult {
        limit_route_response(match frame {
            Frame::TooLarge => RouteResult {
                response: Some(error(
                    None,
                    BUSINESS_ERROR,
                    "Message too large",
                    Some(json!({
                        "reason": "message_too_large",
                        "retryable": false,
                        "maxMessageBytes": MAX_MESSAGE_BYTES
                    })),
                )),
                should_exit: false,
            },
            Frame::Message(bytes) => match parse_client_message(&bytes) {
                Ok(message) => self.route(message),
                Err(response) => RouteResult {
                    response: Some(response),
                    should_exit: false,
                },
            },
        })
    }

    fn is_dedicated_unified_search(&self, frame: &Frame) -> bool {
        if !self.dedicated_search || self.phase != Phase::Ready {
            return false;
        }
        let Frame::Message(bytes) = frame else {
            return false;
        };
        serde_json::from_slice::<Value>(bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("method")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .as_deref()
            == Some("unifiedSearch/query")
    }

    async fn handle_dedicated_unified_search(&mut self, frame: Frame) -> RouteResult {
        let Frame::Message(bytes) = frame else {
            return self.handle_frame(frame);
        };
        let request = match parse_client_message(&bytes) {
            Ok(ClientMessage::Request(request)) => request,
            Ok(message) => return self.route(message),
            Err(response) => {
                return RouteResult {
                    response: Some(response),
                    should_exit: false,
                };
            }
        };
        let Request { id, method, params } = request;
        if method != "unifiedSearch/query" {
            return RouteResult {
                response: Some(error(
                    Some(id),
                    METHOD_NOT_FOUND,
                    "Method not found",
                    Some(json!({ "reason": "method_not_found", "method": method })),
                )),
                should_exit: false,
            };
        }
        limit_route_response(RouteResult {
            response: Some(self.route_unified_search_async(id, params).await),
            should_exit: false,
        })
    }

    fn route(&mut self, message: ClientMessage) -> RouteResult {
        match message {
            ClientMessage::Request(request) => self.route_request(request),
            ClientMessage::Notification(notification) => {
                self.route_notification(&notification);
                RouteResult {
                    response: None,
                    should_exit: false,
                }
            }
            ClientMessage::Response(_) => RouteResult {
                response: None,
                should_exit: false,
            },
        }
    }

    fn personal_workspace_request_block(
        &self,
        method: &str,
        params: Option<&Value>,
    ) -> Option<&'static str> {
        if !(method.starts_with("workspace/")
            || method.starts_with("textDiff/")
            || matches!(
                method,
                "file/hydrate"
                    | "file/confirm"
                    | "preview/open"
                    | "preview/readBinary"
                    | "project/create"
                    | "project/attachWorkspace"
            ))
        {
            return None;
        }
        let params = params?;
        let ids = params
            .get("workspaceId")
            .and_then(Value::as_str)
            .into_iter()
            .chain(
                params
                    .get("workspaceIds")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str),
            )
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return None;
        }
        let services = self.services.as_ref()?;
        let Ok(mut services) = services.lock() else {
            return Some("workspace_read_failed");
        };
        let persistence = services.persistence.as_mut()?;
        for id in ids {
            match persistence.transact(|repositories| repositories.workspace(&WorkspaceId::new(id)))
            {
                Ok(Some(workspace)) if workspace.workspace_kind.as_str() != "local" => {
                    return Some("personal_cloud_source_unavailable");
                }
                Err(_) => return Some("workspace_read_failed"),
                _ => {}
            }
        }
        None
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the protocol allowlist stays explicit at the single routing boundary"
    )]
    fn route_request(&mut self, request: Request) -> RouteResult {
        let Request { id, method, params } = request;
        if let Some(reason) = self.personal_workspace_request_block(&method, params.as_ref()) {
            return RouteResult {
                response: Some(business_error(id, reason, "个人版不支持此来源", json!({}))),
                should_exit: false,
            };
        }
        let maintenance_control = matches!(
            method.as_str(),
            "initialize"
                | "ping"
                | "shutdown/prepare"
                | "shutdown/execute"
                | "aiLedger/get"
                | "aiLedger/apply"
                | "aiState/get"
                | "aiTurn/finalize"
                | "aiThread/get"
                | "aiThread/release"
                | "aiThread/acknowledge"
                | "aiThread/reconcile"
                | "modelConnection/list"
                | "modelExecution/list"
                | "modelUsage/finish"
                | "modelUsage/get"
                | "modelUsage/list"
                | "personalAutomation/bind"
                | "personalAutomation/finish"
                | "personalAutomation/listTriggers"
                | "personalAutomation/resolveInterrupted"
                | "personalMemory/authorization"
                | "personalPage/binding"
                | "personalPage/auditFinish"
                | "personalPage/auditList"
                | "personalApplicationData/get"
                | "personalApplicationData/list"
                | "personalApplicationData/cancel"
                | "personalApplicationData/rollback"
                | "personalApplicationData/resolveInterrupted"
                | "personalCapability/upgrade"
                | "personalPackage/importCommit"
                | "mcpExecution/registerProcess"
                | "mcpExecution/finish"
                | "mcpExecution/abortUnlaunched"
                | "mcpExecution/validate"
                | "mcpExecution/reportValidation"
                | "mcpExecution/list"
        ) || [
            "personalData/",
            "personalStorage/",
            "personalNamespaceStorage/",
            "personalMemoryReset/",
            "personalMaintenance/",
            "profileWriter/",
            "modelExecution/",
            "modelAuth/",
        ]
        .iter()
        .any(|prefix| method.starts_with(prefix))
            || (method == "aiApplication/execute"
                && params
                    .as_ref()
                    .and_then(|v| v.get("action"))
                    .and_then(Value::as_str)
                    .is_some_and(|action| {
                        matches!(
                            action,
                            "update" | "close" | "restartBlockers" | "read" | "recoverable" | "usage"
                        )
                    }))
            || (method == "aiHistory/execute"
                && params
                    .as_ref()
                    .and_then(|v| v.get("action"))
                    .and_then(Value::as_str)
                    == Some("handoff"));
        if !maintenance_control && let Some(services) = self.services.as_ref() {
            let blocked = services.lock().map_or(true, |services| {
                services.personal_profile_id().is_some_and(|profile| {
                    services
                        .persistence
                        .as_ref()
                        .is_none_or(|db| db.personal_maintenance_active(&profile).unwrap_or(true))
                })
            });
            if blocked {
                return RouteResult {
                    response: Some(business_error(
                        id,
                        "personal_data_maintenance_active",
                        "资料维护中",
                        json!({}),
                    )),
                    should_exit: false,
                };
            }
        }
        let response = match method.as_str() {
            "initialize" => self.initialize(id, params),
            "ping" => success(
                id,
                &json!({ "protocolVersion": contexthub_protocol::PROTOCOL_VERSION }),
            )
            .expect("ping result serializes"),
            "shutdown/prepare" => {
                match self.shutdown_decision(false) {
                    Ok(decision) => {
                        if decision.can_close { self.phase = Phase::ShutdownPrepared; }
                        success(id, &json!({"ready":decision.can_close,"continuing":decision.continuing,
                            "reasonCode":decision.reason_code,"serverInstanceId":self.server_instance_id,
                            "ipcClientCount":decision.ipc_client_count,"blockingReasons":decision.blocking_reasons}))
                            .expect("shutdown preparation result serializes")
                    }
                    Err(_) => business_error(id,"app_server_writers_unconfirmed","Shutdown state is unconfirmed",json!({})),
                }
            }
            "shutdown/execute" if self.phase == Phase::ShutdownPrepared => {
                let Ok(decision) = self.shutdown_decision(true) else {
                    return RouteResult {response:Some(business_error(id,"app_server_writers_unconfirmed","Shutdown state is unconfirmed",json!({}))),should_exit:false};
                };
                if !decision.can_close { self.phase = Phase::Ready; }
                return RouteResult {
                    response: Some(
                        success(id, &json!({ "accepted": decision.can_close,"continuing":decision.continuing,
                            "reasonCode":decision.reason_code,"serverInstanceId":self.server_instance_id,
                            "ipcClientCount":decision.ipc_client_count,"blockingReasons":decision.blocking_reasons}))
                            .expect("shutdown result serializes"),
                    ),
                    should_exit: decision.can_close,
                };
            }
            "shutdown/execute" => business_error(
                id,
                "shutdown_not_prepared",
                "Shutdown has not been prepared",
                json!({}),
            ),
            _ if self.phase != Phase::Ready => business_error(
                id,
                "not_initialized",
                "Connection is not initialized",
                json!({}),
            ),
            method if personal_method_removed(method) => business_error(
                id,
                "personal_feature_removed",
                "此功能不属于个人版",
                json!({}),
            ),
            "file/hydrate" => self.route_typed_service_call::<file_hydration::Params, _>(
                id,
                params,
                "File hydration failed",
                AppServerServices::hydrate_file,
            ),
            "preview/open" => self.open_preview(id, params),
            "textResource/read" => self.read_text_resource(id, params),
            "textDiff/compute" => self.compute_text_diff(id, params),
            "textDiff/read" => self.read_text_diff(id, params),
            "file/confirm" => self.confirm_file(id, params),
            "file/confirmationHistory" => self.confirmation_history(id, params),
            "workspace/list" => self.route_typed_service_call::<WorkspaceListParams, _>(
                id,
                params,
                "Workspace list failed",
                |services, value, _| services.workspace_list(value),
            ),
            "workspace/create" => self.route_client_service_call::<WorkspaceCreateParams, _>(
                id,
                params,
                "Workspace creation failed",
                AppServerServices::workspace_create,
            ),
            "workspace/createDirectory" => self
                .route_client_service_call::<WorkspaceDirectoryCreateParams, _>(
                    id,
                    params,
                    "Directory creation failed",
                    AppServerServices::workspace_directory_create,
                ),
            "preview/readBinary" => self
                .route_typed_service_call::<contexthub_protocol::ReadBinaryPreviewParams, _>(
                    id,
                    params,
                    "Binary preview read failed",
                    AppServerServices::read_binary_preview,
                ),
            "workspace/targetValidate" => self
                .route_typed_service_call::<WorkspaceTargetValidateParams, _>(
                    id,
                    params,
                    "Workspace target validation failed",
                    AppServerServices::workspace_target_validate,
                ),
            "workspace/scan" => self.route_typed_service_call::<WorkspaceScanParams, _>(
                id,
                params,
                "Workspace scan failed",
                AppServerServices::workspace_scan,
            ),
            "workspace/listDirectory" => self.route_workspace_list_directory(id, params),
            "workspace/indexRequest" => self.route_workspace_index_request(id, params),
            "workspace/indexStatus" => self.route_workspace_index_status(id, params),
            "workspace/indexPath" => self.route_workspace_index_path(id, params),
            "workspace/files" => self.route_typed_service_call::<WorkspaceFilesParams, _>(
                id,
                params,
                "Workspace files failed",
                |services, value, _| services.workspace_files(value),
            ),
            "skill/list" => self.route_typed_service_call::<SkillListParams, _>(
                id,
                params,
                "Skill list failed",
                |services, value, _| services.skill_list(value),
            ),
            "skill/runList" => self.route_typed_service_call::<SkillRunListParams, _>(
                id,
                params,
                "Skill installation status failed",
                |services, value, _| services.skill_run_list(value),
            ),
            "skill/runRetry" => self.route_typed_service_call::<SkillRunRetryParams, _>(
                id,
                params,
                "Skill installation retry failed",
                AppServerServices::skill_run_retry,
            ),
            "skill/get" => self.route_typed_service_call::<SkillGetParams, _>(
                id,
                params,
                "Skill read failed",
                |services, value, _| services.skill_get(value),
            ),
            "skill/save" => self.route_typed_service_call::<SkillSaveParams, _>(
                id,
                params,
                "Skill save failed",
                AppServerServices::skill_save,
            ),
            "skill/import" => self.route_typed_service_call::<SkillImportParams, _>(
                id,
                params,
                "Skill import failed",
                AppServerServices::skill_import,
            ),
            "skill/importDirectory" => self
                .route_typed_service_call::<SkillImportDirectoryParams, _>(
                    id,
                    params,
                    "Skill directory import failed",
                    AppServerServices::skill_import_directory,
                ),
            "skill/setState" => self.route_typed_service_call::<SkillSetStateParams, _>(
                id,
                params,
                "Skill state update failed",
                AppServerServices::skill_set_state,
            ),
            "skill/delete" => self.route_typed_service_call::<SkillDeleteParams, _>(
                id,
                params,
                "Skill deletion failed",
                AppServerServices::skill_delete,
            ),
            "skill/conversationCatalog" => self
                .route_typed_service_call::<SkillConversationCatalogParams, _>(
                    id,
                    params,
                    "Skill conversation catalog failed",
                    |services, value, _| services.skill_conversation_catalog(value),
                ),
            "skill/runtimeCatalog" if self.source_client == Some("desktop") => {
                self.route_typed_service_call::<SkillRuntimeCatalogParams, _>(
                    id,
                    params,
                    "Skill runtime catalog failed",
                    |services, value, _| services.skill_runtime_catalog(value),
                )
            }
            "skill/runtimeCatalog" => business_error(
                id,
                "skill_runtime_catalog_client_not_supported",
                "Client cannot read the skill runtime catalog",
                json!({}),
            ),
            "capabilityCreation/create" => self
                .route_typed_service_call::<CapabilityCreationCreateParams, _>(
                    id,
                    params,
                    "Capability creation project failed",
                    AppServerServices::capability_creation_create,
                ),
            "capabilityCreation/list" => self
                .route_typed_service_call::<CapabilityCreationListParams, _>(
                    id,
                    params,
                    "Capability creation project list failed",
                    AppServerServices::capability_creation_list,
                ),
            "capabilityCreation/get" => self
                .route_typed_service_call::<CapabilityCreationGetParams, _>(
                    id,
                    params,
                    "Capability creation project read failed",
                    AppServerServices::capability_creation_get,
                ),
            "capabilityCreation/preview" => self
                .route_typed_service_call::<CapabilityCreationPreviewParams, _>(
                    id,
                    params,
                    "Capability creation project preview failed",
                    AppServerServices::capability_creation_preview,
                ),
            "capabilityCreation/install" => business_error(
                id,
                "personal_application_data_legacy_install_disabled",
                "Legacy installation is disabled",
                json!({}),
            ),
            "personalPackage/importPrepare" => self.route_typed_service_call::<contexthub_protocol::PersonalPackageImportPrepareParams, _>(id, params, "Personal package import preparation failed", AppServerServices::personal_package_import_prepare),
            "personalPackage/importCommit" => self.route_typed_service_call::<contexthub_protocol::PersonalPackageImportCommitParams, _>(id, params, "Personal package import commit failed", AppServerServices::personal_package_import_commit),
            "personalPackage/importGet" => self.route_typed_service_call::<contexthub_protocol::PersonalPackageImportParams, _>(id, params, "Personal package import read failed", AppServerServices::personal_package_import_get),
            "personalPackage/importList" => self.route_typed_service_call::<contexthub_protocol::PersonalPackageImportListParams, _>(id, params, "Personal package import list failed", AppServerServices::personal_package_import_list),
            "personalPackage/importCancel" => self.route_typed_service_call::<contexthub_protocol::PersonalPackageImportCancelParams, _>(id, params, "Personal package import cancellation failed", AppServerServices::personal_package_import_cancel),
            "personalCapability/editor" => self
                .route_typed_service_call::<PersonalApplicationOpenParams, _>(
                    id,
                    params,
                    "Personal capability editor failed",
                    AppServerServices::personal_capability_editor,
                ),
            "personalCapability/build" => self
                .route_typed_service_call::<capability_creation::PersonalBuildParams, _>(
                    id,
                    params,
                    "Personal capability build failed",
                    AppServerServices::personal_capability_build,
                ),
            "personalCapability/upgrade" => self
                .route_typed_service_call::<capability_creation::PersonalUpgradeParams, _>(
                    id,
                    params,
                    "Personal capability upgrade failed",
                    AppServerServices::personal_capability_upgrade,
                ),
            "personalCapability/previewRuntime" => self
                .route_typed_service_call::<capability_creation::PersonalVersionParams, _>(
                    id,
                    params,
                    "Personal capability previewRuntime failed",
                    AppServerServices::personal_capability_preview_runtime,
                ),
            "personalCapability/selectFile" if self.source_client == Some("desktop") => {
                self.route_typed_service_call::<capability_creation::PersonalFileSelectParams, _>(
                    id,
                    params,
                    "Personal file select failed",
                    AppServerServices::personal_file_select,
                )
            }
            "personalCapability/consumeFile" if self.source_client == Some("desktop") => {
                self.route_typed_service_call::<capability_creation::PersonalFileAccessParams, _>(
                    id,
                    params,
                    "Personal file consume failed",
                    AppServerServices::personal_file_consume,
                )
            }
            "personalCapability/fileWritePreview" if self.source_client == Some("desktop") => {
                self.route_typed_service_call::<capability_creation::PersonalFileAccessParams, _>(
                    id,
                    params,
                    "Personal file write preview failed",
                    AppServerServices::personal_file_write_preview,
                )
            }
            "personalCapability/writeFile" if self.source_client == Some("desktop") => {
                self.route_typed_service_call::<capability_creation::PersonalFileAccessParams, _>(
                    id,
                    params,
                    "Personal file write failed",
                    AppServerServices::personal_file_write,
                )
            }
            "personalCapability/catalog" => self
                .route_typed_service_call::<CapabilityCreationListParams, _>(
                    id,
                    params,
                    "Personal catalog unavailable",
                    AppServerServices::personal_capability_catalog,
                ),
            "personalCapability/status" => self
                .route_typed_service_call::<PersonalApplicationOpenParams, _>(
                    id,
                    params,
                    "Personal capability state unavailable",
                    AppServerServices::personal_capability_status,
                ),
            "personalCapability/manage" => self
                .route_typed_service_call::<capability_creation::PersonalCapabilityManageParams, _>(
                    id,
                    params,
                    "Personal capability management failed",
                    AppServerServices::personal_capability_manage,
                ),
            "personalCapability/export" => self
                .route_typed_service_call::<PersonalApplicationOpenParams, _>(
                    id,
                    params,
                    "个人包导出失败",
                    AppServerServices::personal_package_export,
                ),
            "personalCapability/runtime" => self
                .route_typed_service_call::<PersonalApplicationOpenParams, _>(
                    id,
                    params,
                    "Personal capability runtime failed",
                    AppServerServices::personal_capability_runtime,
                ),
            "personalApplication/open" => self
                .route_typed_service_call::<PersonalApplicationOpenParams, _>(
                    id,
                    params,
                    "Personal application open failed",
                    AppServerServices::personal_application_open,
                ),
            "aiConversation/setArchived" => self
                .route_typed_service_call::<AiConversationSetArchivedParams, _>(
                    id,
                    params,
                    "Conversation archive failed",
                    AppServerServices::ai_conversation_set_archived,
                ),
            "aiApplication/execute" if self.source_client != Some("desktop") => business_error(
                id,
                "application_conversation_client_not_supported",
                "Client cannot access application conversations",
                json!({}),
            ),
            "aiApplication/execute" => self.route_typed_service_call::<AiApplicationParams, _>(
                id,
                params,
                "Application conversation operation failed",
                AppServerServices::ai_application,
            ),
            method if method.starts_with("aiThread/") && self.source_client != Some("desktop") => {
                business_error(
                    id,
                    "ai_thread_client_not_supported",
                    "Client cannot access the native thread journal",
                    json!({}),
                )
            }
            "aiThread/get" => self.route_typed_service_call::<AiThreadGetParams, _>(
                id,
                params,
                "AI thread read failed",
                |services, value, _| services.ai_thread_get(value),
            ),
            "aiThread/prepare" => self.route_typed_service_call::<AiThreadPrepareParams, _>(
                id,
                params,
                "AI thread preparation failed",
                AppServerServices::ai_thread_prepare,
            ),
            "aiThread/acknowledge" => self
                .route_typed_service_call::<AiThreadAcknowledgeParams, _>(
                    id,
                    params,
                    "AI thread receipt failed",
                    AppServerServices::ai_thread_acknowledge,
                ),
            "aiThread/reconcile" => self.route_typed_service_call::<AiThreadAcknowledgeParams, _>(
                id,
                params,
                "AI thread reconciliation failed",
                AppServerServices::ai_thread_reconcile,
            ),
            "aiThread/cleanup" => self.route_typed_service_call::<AiThreadCleanupParams, _>(
                id,
                params,
                "AI thread cleanup failed",
                AppServerServices::ai_thread_cleanup,
            ),
            "aiThread/release" => self.route_typed_service_call::<AiThreadReleaseParams, _>(
                id,
                params,
                "AI thread release failed",
                AppServerServices::ai_thread_release,
            ),
            "aiHistory/execute" => self
                .route_typed_service_call::<contexthub_protocol::AiHistoryParams, _>(
                    id,
                    params,
                    "AI history operation failed",
                    AppServerServices::ai_history,
                ),
            "personalAutomation/admit" => self.route_typed_service_call::<contexthub_protocol::PersonalAutomationAdmitParams, _>(
                id, params, "Automation admission failed", AppServerServices::personal_automation_admit,
            ),
            "personalAutomation/prepareExecution" => self.route_typed_service_call::<contexthub_protocol::PersonalAutomationPrepareExecutionParams, _>(
                id, params, "Automation model preparation failed", AppServerServices::personal_automation_prepare_execution,
            ),
            "personalAutomation/bind" => self.route_typed_service_call::<contexthub_protocol::PersonalAutomationBindParams, _>(
                id, params, "Automation run binding failed", AppServerServices::personal_automation_bind,
            ),
            "personalAutomation/finish" => self.route_typed_service_call::<contexthub_protocol::PersonalAutomationFinishParams, _>(
                id, params, "Automation settlement failed", AppServerServices::personal_automation_finish,
            ),
            "personalAutomation/listTriggers" => self.route_typed_service_call::<contexthub_protocol::PersonalAutomationListTriggersParams, _>(
                id, params, "Automation trigger history failed", AppServerServices::personal_automation_list,
            ),
            "personalAutomation/resolveInterrupted" => self.route_typed_service_call::<contexthub_protocol::PersonalAutomationResolveInterruptedParams, _>(
                id, params, "Automation interrupted claim check failed", AppServerServices::personal_automation_resolve,
            ),
            "modelUsage/begin" => self.route_typed_service_call::<contexthub_protocol::ModelUsageBeginParams, _>(
                id, params, "Model usage start failed", AppServerServices::model_usage_begin,
            ),
            "modelUsage/finish" => self.route_typed_service_call::<contexthub_protocol::ModelUsageFinishParams, _>(
                id, params, "Model usage finish failed", AppServerServices::model_usage_finish,
            ),
            "modelUsage/get" => self.route_typed_service_call::<contexthub_protocol::ModelUsageParams, _>(
                id, params, "Model usage read failed", AppServerServices::model_usage_get,
            ),
            "modelUsage/list" => self.route_typed_service_call::<contexthub_protocol::ModelUsageListParams, _>(
                id, params, "Model usage list failed", AppServerServices::model_usage_list,
            ),
            "personalPage/binding" => self.route_typed_service_call::<contexthub_protocol::PersonalPageBindingParams, _>(
                id, params, "Personal page binding failed", AppServerServices::personal_page_binding,
            ),
            "personalPage/auditPrepare" => self.route_typed_service_call::<contexthub_protocol::PersonalPageAuditPrepareParams, _>(
                id, params, "Personal page audit preparation failed", AppServerServices::personal_page_audit_prepare,
            ),
            "personalPage/auditFinish" => self.route_typed_service_call::<contexthub_protocol::PersonalPageAuditFinishParams, _>(
                id, params, "Personal page audit result failed", AppServerServices::personal_page_audit_finish,
            ),
            "personalPage/auditList" => self.route_typed_service_call::<contexthub_protocol::PersonalPageAuditListParams, _>(
                id, params, "Personal page audit list failed", AppServerServices::personal_page_audit_list,
            ),
            method if method.starts_with("personalApplicationData/") => {
                self.route_service_call(id,"Personal application data operation failed",|services,now|services.personal_application_data_rpc(method,params.unwrap_or_else(||json!({})),now))
            }
            method if method.starts_with("personalMemoryReset/") || method == "personalMemory/authorization" => {
                self.route_service_call(id,"Personal memory operation failed",|services,now|services.personal_memory_rpc(method,params.unwrap_or_else(||json!({})),&self.server_instance_id,now))
            }
            method if method.starts_with("personalNamespaceStorage/") => {
                self.route_service_call(id, "Personal namespace storage operation failed", |services, now| services.personal_namespace_storage_rpc(method,params.unwrap_or_else(||json!({})),&self.server_instance_id,now))
            }
            method if method.starts_with("personalStorage/") => {
                self.route_service_call(id, "Personal storage operation failed", |services, now| services.personal_storage_rpc(method,params.unwrap_or_else(||json!({})),&self.server_instance_id,now))
            }
            method if method.starts_with("personalData/") => {
                self.route_service_call(id, "Personal data operation failed", |services, now| services.personal_data_rpc(method,params.unwrap_or_else(||json!({})),&self.server_instance_id,now))
            }
            method if method.starts_with("profileWriter/") || method.starts_with("personalMaintenance/") => {
                self.route_service_call(id, "Personal writer operation failed", |services, now| services.profile_writer_rpc(method,params.unwrap_or_else(||json!({})),&self.server_instance_id,now))
            }
            method if ["mcpConnection/","mcpExecution/","mcpTool/","mcpCredentialCleanup/"].iter().any(|prefix|method.starts_with(prefix)) => {
                self.route_service_call(id, "Personal MCP operation failed", |services, now| services.mcp_connection_rpc(method,params.unwrap_or_else(||json!({})),&self.server_instance_id,now))
            }
            method
                if [
                    "modelConnection/",
                    "modelCredentialCleanup/",
                    "modelAuth/",
                    "modelCatalog/",
                    "modelDescriptor/",
                    "modelPurpose/",
                    "modelValidation/",
                    "modelExecution/",
                ]
                .iter()
                .any(|prefix| method.starts_with(prefix)) =>
            {
                self.route_service_call(id, "Personal model operation failed", |services, now| {
                    services.model_connection_rpc(
                        method,
                        params.unwrap_or_else(|| json!({})),
                        &self.server_instance_id,
                        now,
                    )
                })
            }
            "aiLedger/get" => self
                .route_typed_service_call::<contexthub_protocol::AiLedgerGetParams, _>(
                    id,
                    params,
                    "AI ledger read failed",
                    |services, _, _| services.ai_ledger_get(),
                ),
            "aiLedger/apply" => self
                .route_typed_service_call::<contexthub_protocol::AiLedgerApplyParams, _>(
                    id,
                    params,
                    "AI ledger write failed",
                    AppServerServices::ai_ledger_apply,
                ),
            "aiState/get" => self.route_typed_service_call::<AiStateGetParams, _>(
                id,
                params,
                "AI state read failed",
                |services, value, _| services.ai_state_get(value),
            ),
            "aiState/replace" => self.route_typed_service_call::<AiStateReplaceParams, _>(
                id,
                params,
                "AI state write failed",
                AppServerServices::ai_state_replace,
            ),
            "aiState/apply" => self.route_typed_service_call::<AiStateApplyParams, _>(
                id,
                params,
                "AI state write failed",
                AppServerServices::ai_state_apply,
            ),
            "aiWork/taskCandidates" => self
                .route_typed_service_call::<AiWorkTaskCandidatesParams, _>(
                    id,
                    params,
                    "AI work task candidates failed",
                    |services, value, _| services.ai_work_task_candidates(value),
                ),
            "aiTurn/finalize" => self.route_typed_service_call::<AiTurnFinalizeParams, _>(
                id,
                params,
                "AI turn finalization failed",
                AppServerServices::ai_turn_finalize,
            ),
            "project/list" => self.route_typed_service_call::<ProjectListParams, _>(
                id,
                params,
                "Project list failed",
                |services, value, _| services.project_list(value),
            ),
            "project/create" => self.route_client_service_call::<ProjectCreateParams, _>(
                id,
                params,
                "Project creation failed",
                AppServerServices::project_create,
            ),
            "project/update" => self.route_client_service_call::<ProjectUpdateParams, _>(
                id,
                params,
                "Project update failed",
                AppServerServices::project_update,
            ),
            "project/delete" => self.route_client_service_call::<ProjectDeleteParams, _>(
                id,
                params,
                "Project deletion failed",
                AppServerServices::project_delete,
            ),
            "workspace/delete" => self.route_client_service_call::<WorkspaceDeleteParams, _>(
                id,
                params,
                "Workspace deletion failed",
                AppServerServices::workspace_delete,
            ),
            "project/detachWorkspace" => self
                .route_client_service_call::<ProjectDetachWorkspaceParams, _>(
                    id,
                    params,
                    "Project workspace detach failed",
                    AppServerServices::project_detach_workspace,
                ),
            "project/attachWorkspace" => self
                .route_client_service_call::<ProjectAttachWorkspaceParams, _>(
                    id,
                    params,
                    "Project workspace attach failed",
                    AppServerServices::project_attach_workspace,
                ),
            "project/get" => self.route_typed_service_call::<ProjectGetParams, _>(
                id,
                params,
                "Project read failed",
                |services, value, _| services.project_get(value),
            ),
            "projectPortfolio/list" => self
                .route_typed_service_call::<ProjectPortfolioListParams, _>(
                    id,
                    params,
                    "Project portfolio read failed",
                    |services, value, _| services.project_portfolio_list(value),
                ),
            "deliverable/list" => self.route_typed_service_call::<DeliverableListParams, _>(
                id,
                params,
                "Deliverable list failed",
                |services, value, _| services.deliverable_list(value),
            ),
            "deliverable/create" => self.route_client_service_call::<DeliverableCreateParams, _>(
                id,
                params,
                "Deliverable creation failed",
                AppServerServices::deliverable_create,
            ),
            "deliverable/version/create" => self
                .route_client_service_call::<DeliverableVersionCreateParams, _>(
                    id,
                    params,
                    "Deliverable version creation failed",
                    AppServerServices::deliverable_version_create,
                ),
            "deliverable/version/review" => self
                .route_client_service_call::<DeliverableVersionReviewParams, _>(
                    id,
                    params,
                    "Deliverable version review failed",
                    AppServerServices::deliverable_version_review,
                ),
            "myWork/get" => self.route_typed_service_call::<MyWorkGetParams, _>(
                id,
                params,
                "My work projection read failed",
                |services, value, _| services.my_work_get(value),
            ),
            "globalSearch/query" => self.route_typed_service_call::<GlobalSearchParams, _>(
                id,
                params,
                "Global search failed",
                |services, value, _| services.global_search(value),
            ),
            "unifiedSearch/query" => self.route_unified_search(id, params),
            "attentionCase/list" => self.route_typed_service_call::<AttentionCaseListParams, _>(
                id,
                params,
                "Attention case list failed",
                |services, value, _| services.attention_case_list(value),
            ),
            "projectBlocker/create" => self
                .route_client_service_call::<ProjectBlockerCreateParams, _>(
                    id,
                    params,
                    "Project blocker creation failed",
                    AppServerServices::project_blocker_create,
                ),
            "decisionRequest/create" => self
                .route_client_service_call::<DecisionRequestCreateParams, _>(
                    id,
                    params,
                    "Decision request creation failed",
                    AppServerServices::decision_request_create,
                ),
            "agentIncident/report" => self
                .route_client_service_call::<AgentIncidentReportParams, _>(
                    id,
                    params,
                    "Agent incident report failed",
                    AppServerServices::agent_incident_report,
                ),
            "attentionCase/transition" => self
                .route_client_service_call::<AttentionCaseTransitionParams, _>(
                    id,
                    params,
                    "Attention case transition failed",
                    AppServerServices::attention_case_transition,
                ),
            "projectProgress/get" => self.route_typed_service_call::<ProjectProgressGetParams, _>(
                id,
                params,
                "Project progress read failed",
                |services, value, _| services.project_progress_get(value),
            ),
            "phasePlan/create" => self.route_client_service_call::<PhasePlanCreateParams, _>(
                id,
                params,
                "Phase plan creation failed",
                AppServerServices::phase_plan_create,
            ),
            "phasePlan/activate" => self.route_client_service_call::<PhasePlanActivateParams, _>(
                id,
                params,
                "Phase plan activation failed",
                AppServerServices::phase_plan_activate,
            ),
            "projectPhase/transition" => self
                .route_client_service_call::<ProjectPhaseTransitionParams, _>(
                    id,
                    params,
                    "Project phase transition failed",
                    AppServerServices::project_phase_transition,
                ),
            "businessTask/create" => self.route_client_service_call::<BusinessTaskCreateParams, _>(
                id,
                params,
                "Business task creation failed",
                AppServerServices::business_task_create,
            ),
            "businessTask/get" => self.route_typed_service_call::<BusinessTaskGetParams, _>(
                id,
                params,
                "Business task read failed",
                |services, value, _| services.business_task_get(value),
            ),
            "businessTask/transition" => self
                .route_client_service_call::<BusinessTaskTransitionParams, _>(
                    id,
                    params,
                    "Business task transition failed",
                    AppServerServices::business_task_transition,
                ),
            "businessTask/move" => self.route_client_service_call::<BusinessTaskMoveParams, _>(
                id,
                params,
                "Business task move failed",
                AppServerServices::business_task_move,
            ),
            "event/replay" => self.route_typed_service_call::<EventReplayParams, _>(
                id,
                params,
                "Event replay failed",
                |services, value, _| services.event_replay(value),
            ),
            _ => error(
                Some(id),
                METHOD_NOT_FOUND,
                "Method not found",
                Some(json!({ "reason": "method_not_found", "method": method })),
            ),
        };
        RouteResult {
            response: Some(response),
            should_exit: false,
        }
    }

    fn open_preview(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<OpenPreviewParams>(params) else {
            return invalid_params(id, "invalid_preview_open_params");
        };
        let Some(services) = &self.services else {
            return business_error(
                id,
                "preview_service_unavailable",
                "Preview service is unavailable",
                json!({}),
            );
        };
        let now = match unix_epoch_millis() {
            Ok(now) => now,
            Err(reason) => {
                return business_error(id, reason, "System time is unavailable", json!({}));
            }
        };
        match services
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .open_preview(&params, now)
        {
            Ok(result) => success(id, &result).expect("preview result serializes"),
            Err(error) => {
                business_error(id, error.reason, "Preview could not be opened", json!({}))
            }
        }
    }

    fn read_text_resource(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<ReadTextResourceParams>(params) else {
            return invalid_params(id, "invalid_text_resource_read_params");
        };
        let Some(services) = &self.services else {
            return business_error(
                id,
                "preview_service_unavailable",
                "Preview service is unavailable",
                json!({}),
            );
        };
        let now = match unix_epoch_millis() {
            Ok(now) => now,
            Err(reason) => {
                return business_error(id, reason, "System time is unavailable", json!({}));
            }
        };
        match services
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .read_text_resource(&params, now)
        {
            Ok(result) => success(id, &result).expect("text resource result serializes"),
            Err(error) => business_error(
                id,
                error.reason,
                "Text resource could not be read",
                json!({}),
            ),
        }
    }

    fn compute_text_diff(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<ComputeTextDiffParams>(params) else {
            return invalid_params(id, "invalid_text_diff_compute_params");
        };
        self.route_service_call(id, "Text diff could not be computed", |services, now| {
            services.compute_text_diff(&params, now)
        })
    }

    fn read_text_diff(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<ReadTextDiffParams>(params) else {
            return invalid_params(id, "invalid_text_diff_read_params");
        };
        self.route_service_call(id, "Text diff could not be read", |services, now| {
            services.read_text_diff(&params, now)
        })
    }

    fn confirm_file(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<ConfirmFileParams>(params) else {
            return invalid_params(id, "invalid_file_confirmation_params");
        };
        let Some(source_client) = self.source_client else {
            return business_error(
                id,
                "confirmation_client_not_supported",
                "Client cannot create file confirmations",
                json!({}),
            );
        };
        self.route_service_call(id, "File could not be confirmed", |services, now| {
            services.confirm_file(&params, source_client, now)
        })
    }

    fn confirmation_history(
        &mut self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<ConfirmationHistoryParams>(params) else {
            return invalid_params(id, "invalid_confirmation_history_params");
        };
        self.route_service_call(
            id,
            "Confirmation history could not be read",
            |services, _| services.confirmation_history(&params),
        )
    }

    fn route_service_call<T: serde::Serialize>(
        &self,
        id: contexthub_protocol::RequestId,
        message: &str,
        operation: impl FnOnce(&mut AppServerServices, i64) -> Result<T, ServiceError>,
    ) -> String {
        let Some(services) = &self.services else {
            return business_error(
                id,
                "app_service_unavailable",
                "Application service is unavailable",
                json!({}),
            );
        };
        let now = match unix_epoch_millis() {
            Ok(now) => now,
            Err(reason) => {
                return business_error(id, reason, "System time is unavailable", json!({}));
            }
        };
        match operation(
            &mut services
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            now,
        ) {
            Ok(result) => success(id, &result).expect("service result serializes"),
            Err(error) => business_error(id, error.reason, message, json!({})),
        }
    }

    fn route_typed_service_call<P, T>(
        &self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
        message: &str,
        operation: impl FnOnce(&mut AppServerServices, &P, i64) -> Result<T, ServiceError>,
    ) -> String
    where
        P: serde::de::DeserializeOwned,
        T: serde::Serialize,
    {
        let Some(params) = parse_params::<P>(params) else {
            return invalid_params(id, "invalid_method_params");
        };
        self.route_service_call(id, message, |services, now| {
            operation(services, &params, now)
        })
    }

    fn route_unified_search(
        &self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<UnifiedSearchParams>(params) else {
            return invalid_params(id, "invalid_method_params");
        };
        let Some(services) = &self.services else {
            return business_error(
                id,
                "app_service_unavailable",
                "Application service is unavailable",
                json!({}),
            );
        };
        let plan = services
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prepare_unified_search(&params);
        match plan {
            Ok(plan) => success(id, &unified_search::complete_unified_search(plan, &params))
                .expect("unified search result serializes"),
            Err(error) => business_error(id, error.reason, "Unified search failed", json!({})),
        }
    }

    async fn route_unified_search_async(
        &self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
    ) -> String {
        let Some(params) = parse_params::<UnifiedSearchParams>(params) else {
            return invalid_params(id, "invalid_method_params");
        };
        let Some(services) = self.services.clone() else {
            return business_error(
                id,
                "app_service_unavailable",
                "Application service is unavailable",
                json!({}),
            );
        };
        let result = tokio::task::spawn_blocking(move || {
            let plan = services
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .prepare_unified_search(&params)?;
            Ok::<_, ServiceError>(unified_search::complete_unified_search(plan, &params))
        })
        .await;
        match result {
            Ok(Ok(result)) => success(id, &result).expect("unified search result serializes"),
            Ok(Err(error)) => business_error(id, error.reason, "Unified search failed", json!({})),
            Err(_) => business_error(
                id,
                "local_search_worker_failed",
                "Unified search failed",
                json!({}),
            ),
        }
    }

    fn route_client_service_call<P, T>(
        &self,
        id: contexthub_protocol::RequestId,
        params: Option<Value>,
        message: &str,
        operation: impl FnOnce(&mut AppServerServices, &P, &str, i64) -> Result<T, ServiceError>,
    ) -> String
    where
        P: serde::de::DeserializeOwned,
        T: serde::Serialize,
    {
        let Some(params) = parse_params::<P>(params) else {
            return invalid_params(id, "invalid_method_params");
        };
        let Some(source_client) = self.source_client else {
            return business_error(
                id,
                "client_not_supported",
                "Client cannot perform this operation",
                json!({}),
            );
        };
        self.route_service_call(id, message, |services, now| {
            operation(services, &params, source_client, now)
        })
    }

    fn server_capabilities(&self) -> ServerCapabilities {
        ServerCapabilities {
            personal_data_version: Some(contexthub_protocol::PERSONAL_DATA_VERSION),
            model_connection_protocol: Some(contexthub_protocol::MODEL_CONNECTION_PROTOCOL),
            personal_profile_id: self.services.as_ref().and_then(|services| {
                services
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .personal_profile_id()
            }),
            local_ipc: self.local_ipc_available,
            event_replay: self.services.is_some(),
            interactive_requests: false,
            resource_tickets: self.services.is_some(),
            rich_text_diff: self.services.is_some(),
            file_confirmation: self.services.is_some() && self.source_client.is_some(),
            supported_checksum_algorithms: vec!["sha256".to_owned()],
        }
    }

    fn initialize(&mut self, id: contexthub_protocol::RequestId, params: Option<Value>) -> String {
        if self.phase != Phase::New {
            return business_error(
                id,
                "already_initialized",
                "Initialize can only be called once",
                json!({}),
            );
        }
        let Some(params) =
            params.and_then(|value| serde_json::from_value::<InitializeParams>(value).ok())
        else {
            return error(
                Some(id),
                INVALID_PARAMS,
                "Invalid params",
                Some(json!({ "reason": "invalid_initialize_params" })),
            );
        };
        let protocol_version = match negotiate_protocol(&params.protocol) {
            Ok(version) => version,
            Err("protocol_version_unsupported") => {
                return business_error(
                    id,
                    "protocol_version_unsupported",
                    "Protocol version is not supported",
                    json!({
                        "serverSupportedRange": { "minVersion": PROTOCOL_MIN_VERSION, "maxVersion": PROTOCOL_MAX_VERSION },
                        "minimumClientVersion": env!("CARGO_PKG_VERSION"),
                        "upgradeGuidance": "Upgrade the ContextHub client and bundled App Server together"
                    }),
                );
            }
            Err(reason) => {
                return error(
                    Some(id),
                    INVALID_PARAMS,
                    "Invalid params",
                    Some(json!({ "reason": reason })),
                );
            }
        };
        self.dedicated_search = params.client.name == "contexthub-desktop-search";
        self.source_client = match params.client.name.as_str() {
            "contexthub-desktop" => Some("desktop"),
            "contexthub-cli" => Some("cli"),
            _ => None,
        };
        self.phase = Phase::InitializeResponded;
        let result = InitializeResult {
            server: ServerInfo {
                name: "contexthub-app-server".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                instance_id: self.server_instance_id.clone(),
            },
            protocol_version,
            capabilities: self.server_capabilities(),
            session: SessionInfo {
                connection_id: self.connection_id.clone(),
                client_session_id: self.client_session_id.clone(),
            },
            limits: ProtocolLimits {
                max_message_bytes: u32::try_from(MAX_MESSAGE_BYTES)
                    .expect("message limit fits u32"),
                max_page_size: MAX_PAGE_SIZE,
            },
            resume: ResumeResult {
                requested_last_durable_event_sequence: params.resume.last_durable_event_sequence,
                replay_available: self.services.as_ref().is_some_and(|services| {
                    let services = services
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let Some(persistence) = services.persistence.as_ref() else {
                        return false;
                    };
                    params.resume.last_durable_event_sequence.map_or_else(
                        || persistence.replay_global_available(1).is_ok(),
                        |cursor| {
                            persistence
                                .replay_global_after(i64::from(cursor), 1)
                                .is_ok()
                        },
                    )
                }),
            },
        };
        success(id, &result).expect("initialize result serializes")
    }

    fn route_notification(&mut self, notification: &Notification) {
        if notification.method == "initialized" && self.phase == Phase::InitializeResponded {
            self.phase = Phase::Ready;
        }
    }
}

fn limit_route_response(mut result: RouteResult) -> RouteResult {
    let Some(response) = result.response.as_ref() else {
        return result;
    };
    if response.len() <= MAX_MESSAGE_BYTES {
        return result;
    }
    let id = serde_json::from_str::<Value>(response)
        .ok()
        .and_then(|value| value.get("id").cloned())
        .and_then(|value| serde_json::from_value(value).ok());
    result.response = Some(error(
        id,
        BUSINESS_ERROR,
        "Response too large",
        Some(json!({
            "reason": "response_too_large",
            "retryable": false,
            "maxMessageBytes": MAX_MESSAGE_BYTES
        })),
    ));
    result
}

fn parse_params<T: serde::de::DeserializeOwned>(params: Option<Value>) -> Option<T> {
    params.and_then(|value| serde_json::from_value(value).ok())
}

fn invalid_params(id: contexthub_protocol::RequestId, reason: &str) -> String {
    error(
        Some(id),
        INVALID_PARAMS,
        "Invalid params",
        Some(json!({ "reason": reason })),
    )
}

pub(crate) fn unix_epoch_millis() -> Result<i64, &'static str> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .ok_or("system_time_invalid")
}

/// 在任意异步字节流上运行一条 JSONL/JSON-RPC 连接。
///
/// # Errors
///
/// 读取、写入、刷新或全局关闭信号异常时返回 I/O 错误。
pub async fn serve_connection<R, W>(
    mut reader: R,
    mut writer: W,
    mut router: ConnectionRouter,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut decoder = contexthub_protocol::JsonlDecoder::default();
    let mut chunk = [0_u8; 8192];
    // 脱离请求通道执行的调用完成后，应答经这里交回本连接统一写出
    let (detached_tx, mut detached_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let mut detached_pending = 0_usize;
    loop {
        let outcome = {
            let mut read = pin!(reader.read(&mut chunk));
            let mut shutdown = pin!(shutdown_rx.changed());
            poll_fn(|context| {
                if let Poll::Ready(result) = shutdown.as_mut().poll(context) {
                    return Poll::Ready(ReadOutcome::Shutdown(result));
                }
                if let Poll::Ready(Some(response)) = detached_rx.poll_recv(context) {
                    return Poll::Ready(ReadOutcome::Detached(response));
                }
                read.as_mut().poll(context).map(ReadOutcome::Read)
            })
            .await
        };
        let count = match outcome {
            ReadOutcome::Detached(response) => {
                detached_pending -= 1;
                write_line(&mut writer, &response).await?;
                continue;
            }
            ReadOutcome::Read(result) => result?,
            ReadOutcome::Shutdown(changed) => {
                changed.map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "shutdown channel closed")
                })?;
                if *shutdown_rx.borrow() {
                    return Ok(());
                }
                continue;
            }
        };
        if count == 0 {
            // 对端已不再发送请求，仍把已受理请求的应答写完再结束
            while detached_pending > 0 {
                let Some(response) = detached_rx.recv().await else {
                    break;
                };
                detached_pending -= 1;
                write_line(&mut writer, &response).await?;
            }
            return Ok(());
        }
        for frame in decoder.push(&chunk[..count]) {
            if router.is_dedicated_unified_search(&frame) {
                let mut cancellation_probe = [0_u8; 1];
                let result = tokio::select! {
                    result = router.handle_dedicated_unified_search(frame) => result,
                    changed = shutdown_rx.changed() => {
                        changed.map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "shutdown channel closed"))?;
                        return Ok(());
                    }
                    disconnected = reader.read(&mut cancellation_probe) => {
                        match disconnected {
                            Ok(0) => return Ok(()),
                            Ok(_) => return Err(io::Error::new(io::ErrorKind::InvalidData, "dedicated search connection received an overlapping request")),
                            Err(error) if matches!(error.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset | io::ErrorKind::UnexpectedEof) => return Ok(()),
                            Err(error) => return Err(error),
                        }
                    }
                };
                if let Some(response) = result.response {
                    write_line(&mut writer, &response).await?;
                }
                continue;
            }
            if let Some(call) = router.detach(&frame) {
                router.spawn_detached_call(call, detached_tx.clone())?;
                detached_pending += 1;
                continue;
            }
            let dispatch = tokio::task::spawn_blocking(move || {
                let mut router = router;
                let result = router.handle_frame(frame);
                (router, result)
            })
            .await
            .map_err(|error| io::Error::other(format!("request routing task failed: {error}")))?;
            router = dispatch.0;
            let result = dispatch.1;
            let written = if let Some(response) = result.response {
                write_line(&mut writer, &response).await
            } else {
                Ok(())
            };
            if result.should_exit {
                shutdown_tx.send_replace(true);
                return written;
            }
            written?;
        }
    }
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, response: &str) -> io::Result<()> {
    writer.write_all(response.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}

enum ReadOutcome<T, S> {
    Read(T),
    Shutdown(S),
    Detached(String),
}

fn ai_entity_changes(delta: &AiStateEntityDelta) -> AiStateEntityChanges {
    AiStateEntityChanges {
        upserts: delta
            .upserts
            .iter()
            .map(|item| (item.position, item.value.clone()))
            .collect(),
        removed_ids: delta.removed_ids.clone(),
        total: delta.total,
    }
}

fn ai_state_changes(delta: &AiStateDelta) -> AiStateChanges {
    AiStateChanges {
        full: delta.full,
        payload: delta.payload.clone(),
        conversations: ai_entity_changes(&delta.conversations),
        messages: ai_entity_changes(&delta.messages),
        runs: ai_entity_changes(&delta.runs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{
        CaptureLocalSnapshotCommand, capture_full_local_snapshot,
        create_preflighted_local_workspace, local_workspace_command,
    };
    use contexthub_application::WorkspaceRecord;
    use contexthub_domain::{EventId, LocalWorkspaceStatus, WorkspaceKind};
    use contexthub_protocol::{Frame, RequestId};
    use contexthub_workspace_fs::WorkspaceCancellationToken;
    use std::fs;
    use std::path::Path;
    use tokio::io::{AsyncBufReadExt as _, BufReader};

    fn frame(value: &str) -> Frame {
        Frame::Message(value.as_bytes().to_vec())
    }

    struct EofObservedReader<R> {
        reader: R,
        eof: Option<tokio::sync::oneshot::Sender<()>>,
    }

    impl<R: AsyncRead + Unpin> AsyncRead for EofObservedReader<R> {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            context: &mut std::task::Context<'_>,
            buffer: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let before = buffer.filled().len();
            let result = std::pin::Pin::new(&mut self.reader).poll_read(context, buffer);
            if matches!(result, Poll::Ready(Ok(())))
                && buffer.filled().len() == before
                && let Some(eof) = self.eof.take()
            {
                let _ = eof.send(());
            }
            result
        }
    }

    #[test]
    fn historical_cloud_workspace_is_refused_before_detached_or_local_routes() {
        let root = std::env::temp_dir().join(format!(
            "cilai-personal-cloud-guard-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let mut persistence = SqlitePersistence::open(&root.join("state.db"), "0.1.0").unwrap();
        persistence
            .transact(|repositories| {
                repositories.insert_workspace(&WorkspaceRecord {
                    workspace_id: WorkspaceId::new("historical-cloud"),
                    workspace_kind: WorkspaceKind::CloudTeam,
                    display_name: "旧来源".into(),
                    local_root: root.join("unread-source").display().to_string(),
                    local_root_identity: None,
                    local_status: LocalWorkspaceStatus::Ready,
                    state_revision: 0,
                    last_local_scan_at: None,
                    created_at: 1,
                    updated_at: 1,
                })
            })
            .unwrap();
        let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
        let mut router = ConnectionRouter::with_services(
            "personal".into(),
            "connection".into(),
            "session".into(),
            false,
            services.clone(),
        );
        router.phase = Phase::Ready;
        router.source_client = Some("desktop");
        for method in [
            "workspace/scan",
            "workspace/files",
            "workspace/listDirectory",
            "workspace/targetValidate",
            "workspace/createDirectory",
            "preview/open",
            "preview/readBinary",
            "file/hydrate",
            "file/confirm",
            "textDiff/compute",
            "project/attachWorkspace",
        ] {
            let request = json!({"jsonrpc":"2.0", "id":1, "method":method, "params":{"workspaceId":"historical-cloud"}}).to_string();
            assert!(router.detach(&frame(&request)).is_none(), "{method}");
            let response = request_value(&mut router, &request);
            assert_eq!(
                response["error"]["data"]["reason"], "personal_cloud_source_unavailable",
                "{method}"
            );
        }
        assert!(
            services
                .lock()
                .unwrap()
                .persistence
                .as_mut()
                .unwrap()
                .transact(
                    |repositories| repositories.workspace(&WorkspaceId::new("historical-cloud"))
                )
                .unwrap()
                .is_some()
        );
        drop(router);
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_workspace_list_follows_disk_and_reuses_connected_directories() {
        let root =
            std::env::temp_dir().join(format!("ch-local-directories-{}", uuid::Uuid::new_v4()));
        let data = root.join("data");
        let parent = root.join("parent");
        let child = parent.join("child");
        let other = root.join("other");
        for directory in [&data, &child, &other] {
            fs::create_dir_all(directory).unwrap();
        }
        let persistence = SqlitePersistence::open(&data.join("state.db"), "0.1.0").unwrap();
        let mut services = AppServerServices::new(Some(persistence));
        let create = |services: &mut AppServerServices, path: &Path, operation: &str| {
            services
                .workspace_create(
                    &WorkspaceCreateParams {
                        local_root: path.to_str().unwrap().to_owned(),
                        display_name: operation.to_owned(),
                        client_operation_id: operation.to_owned(),
                    },
                    "desktop",
                    1,
                )
                .unwrap()
        };
        let first = create(&mut services, &parent, "parent");
        let nested = create(&mut services, &child, "child");
        let reused = create(&mut services, &parent, "parent-again");
        assert_eq!(first.workspace.workspace_id, reused.workspace.workspace_id);
        assert_eq!(first.scan_task_id, reused.scan_task_id);
        fs::rename(&parent, root.join("moved")).unwrap();
        let third = create(&mut services, &other, "other");
        let listed = services.workspace_list(&WorkspaceListParams {}).unwrap();
        assert_eq!(listed.workspaces.len(), 1);
        assert_eq!(
            listed.workspaces[0].workspace_id,
            third.workspace.workspace_id
        );
        let saved = services
            .persistence
            .as_mut()
            .unwrap()
            .transact(|repo| repo.workspaces())
            .unwrap();
        assert_eq!(saved.len(), 3);
        assert!(
            saved
                .iter()
                .any(|workspace| workspace.workspace_id.as_str() == nested.workspace.workspace_id)
        );
        fs::rename(root.join("moved"), &parent).unwrap();
        assert_eq!(
            services
                .workspace_list(&WorkspaceListParams {})
                .unwrap()
                .workspaces
                .len(),
            3
        );
        fs::rename(&parent, root.join("relocated")).unwrap();
        let relocated = create(&mut services, &root.join("relocated"), "relocated");
        assert_eq!(
            relocated.workspace.workspace_id,
            first.workspace.workspace_id
        );
        assert_eq!(relocated.scan_task_id, first.scan_task_id);
        assert_eq!(
            Path::new(&relocated.workspace.local_root),
            fs::canonicalize(root.join("relocated")).unwrap()
        );
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn blocked_directory_read_keeps_ping_and_project_requests_responsive() {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let root = std::env::temp_dir().join(format!("ch-browse-pause-{}", uuid::Uuid::new_v4()));
                let data = root.join("data");
                let workspace = root.join("workspace");
                fs::create_dir_all(&data).unwrap();
                fs::create_dir_all(&workspace).unwrap();
                fs::write(workspace.join("readme.md"), "contents").unwrap();
                let mut persistence = SqlitePersistence::open(&data.join("contexthub.db"), "0.1.0").unwrap();
                create_preflighted_local_workspace(
                    &mut persistence,
                    &workspace,
                    std::slice::from_ref(&data),
                    local_workspace_command("w-paused", "Paused", "scan", "event", "corr", "desktop", 1),
                ).unwrap();
                let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
                let mut router = ConnectionRouter::with_services(
                    "server-paused".into(), "connection-paused".into(), "session-paused".into(), true, Arc::clone(&services),
                );
                ready(&mut router);
                let (pause, mut entered) = crate::workspace_browse::pause_directory_read(workspace.canonicalize().unwrap().to_string_lossy().into_owned());
                let (client, server) = tokio::io::duplex(8_192);
                let (server_reader, server_writer) = tokio::io::split(server);
                let (eof_tx, eof_rx) = tokio::sync::oneshot::channel();
                let server_reader = EofObservedReader { reader: server_reader, eof: Some(eof_tx) };
                let (shutdown_tx, shutdown_rx) = watch::channel(false);
                let server_task = tokio::spawn(serve_connection(server_reader, server_writer, router, shutdown_tx, shutdown_rx));
                let (client_reader, mut client_writer) = tokio::io::split(client);
                let mut lines = BufReader::new(client_reader).lines();
                client_writer.write_all(br#"{"jsonrpc":"2.0","id":"directory","method":"workspace/listDirectory","params":{"workspaceId":"w-paused","relativePath":null}}
"#).await.unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(10), entered.recv()).await.unwrap().unwrap();
                client_writer.write_all(br#"{"jsonrpc":"2.0","id":"ping","method":"ping","params":{}}
{"jsonrpc":"2.0","id":"projects","method":"project/list","params":{"workspaceId":"w-paused"}}
"#).await.unwrap();
                for expected in ["ping", "projects"] {
                    let line = tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line()).await.unwrap().unwrap().unwrap();
                    let response: Value = serde_json::from_str(&line).unwrap();
                    assert_eq!(response["id"], expected);
                    assert!(response.get("error").is_none(), "{response}");
                    if expected == "projects" { assert_eq!(response["result"]["projects"], json!([])); }
                }
                client_writer.shutdown().await.unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(10), eof_rx).await.unwrap().unwrap();
                drop(client_writer);
                let mut other = ConnectionRouter::with_services("other-server".into(), "other-connection".into(), "other-session".into(), true, Arc::clone(&services));
                ready(&mut other);
                let duplicate = request_value(&mut other, r#"{"jsonrpc":"2.0","id":"duplicate","method":"workspace/listDirectory","params":{"workspaceId":"w-paused","relativePath":""}}"#);
                assert_eq!(duplicate["error"]["data"]["reason"], "workspace_directory_busy");
                drop(pause);
                let directory: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(directory["id"], "directory");
                assert_eq!(directory["result"]["entries"][0]["relativePath"], "readme.md");
                let after = request_value(&mut other, r#"{"jsonrpc":"2.0","id":"after","method":"workspace/listDirectory","params":{"workspaceId":"w-paused"}}"#);
                assert_eq!(after["result"]["entries"][0]["relativePath"], "readme.md");
                for (id, params, reason) in [
                    ("escape", json!({"workspaceId":"w-paused","relativePath":"../"}), "workspace_path_invalid"),
                    ("missing", json!({"workspaceId":"missing"}), "workspace_not_found"),
                ] {
                    let request = json!({"jsonrpc":"2.0","id":id,"method":"workspace/listDirectory","params":params});
                    let response = request_value(&mut other, &request.to_string());
                    assert_eq!(response["id"], id);
                    assert_eq!(response["error"]["data"]["reason"], reason);
                }
                drop(other);
                drop(lines);
                server_task.await.unwrap().unwrap();
                drop(services);
                fs::remove_dir_all(root).unwrap();
            });
    }

    #[test]
    fn disconnected_directory_read_keeps_its_lease_until_execution_finishes() {
        tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
            let root = std::env::temp_dir().join(format!("ch-browse-disconnect-{}", uuid::Uuid::new_v4()));
            let data = root.join("data");
            let workspace = root.join("workspace");
            fs::create_dir_all(&data).unwrap();
            fs::create_dir_all(&workspace).unwrap();
            fs::write(workspace.join("readme.md"), "contents").unwrap();
            let mut persistence = SqlitePersistence::open(&data.join("contexthub.db"), "0.1.0").unwrap();
            create_preflighted_local_workspace(&mut persistence, &workspace, std::slice::from_ref(&data), local_workspace_command("w-disconnected", "Disconnected", "scan", "event", "corr", "desktop", 1)).unwrap();
            let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
            let mut router = ConnectionRouter::with_services("disconnected-server".into(), "disconnected-connection".into(), "disconnected-session".into(), true, Arc::clone(&services));
            ready(&mut router);
            let (pause, mut entered) = crate::workspace_browse::pause_directory_read(workspace.canonicalize().unwrap().to_string_lossy().into_owned());
            let (mut client, server) = tokio::io::duplex(8_192);
            let (server_reader, server_writer) = tokio::io::split(server);
            let (eof_tx, eof_rx) = tokio::sync::oneshot::channel();
            let server_reader = EofObservedReader { reader: server_reader, eof: Some(eof_tx) };
            let (shutdown_tx, shutdown_rx) = watch::channel(false);
            let server_task = tokio::spawn(serve_connection(server_reader, server_writer, router, shutdown_tx, shutdown_rx));
            client.write_all(br#"{"jsonrpc":"2.0","id":"directory","method":"workspace/listDirectory","params":{"workspaceId":"w-disconnected"}}
"#).await.unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(10), entered.recv()).await.unwrap().unwrap();
            drop(client);
            tokio::time::timeout(std::time::Duration::from_secs(10), eof_rx).await.unwrap().unwrap();
            let mut other = ConnectionRouter::with_services("other-server".into(), "other-connection".into(), "other-session".into(), true, services);
            ready(&mut other);
            let request = r#"{"jsonrpc":"2.0","id":"after-disconnect","method":"workspace/listDirectory","params":{"workspaceId":"w-disconnected"}}"#;
            assert_eq!(request_value(&mut other, request)["error"]["data"]["reason"], "workspace_directory_busy");
            drop(pause);
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), server_task).await.unwrap().unwrap();
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
            let after = request_value(&mut other, request);
            assert_eq!(after["result"]["entries"][0]["relativePath"], "readme.md");
            drop(other);
            fs::remove_dir_all(root).unwrap();
        });
    }

    #[test]
    fn directory_read_capacity_rejects_extra_work_and_releases_after_completion() {
        tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
            let root = std::env::temp_dir().join(format!("ch-browse-capacity-{}", uuid::Uuid::new_v4()));
            let data = root.join("data");
            let workspace = root.join("workspace");
            fs::create_dir_all(&data).unwrap();
            for n in 1..=crate::workspace_browse::MAX_DIRECTORY_READS { fs::create_dir_all(workspace.join(format!("dir{n}"))).unwrap(); }
            let mut persistence = SqlitePersistence::open(&data.join("contexthub.db"), "0.1.0").unwrap();
            create_preflighted_local_workspace(&mut persistence, &workspace, std::slice::from_ref(&data), local_workspace_command("w-capacity", "Capacity", "scan", "event", "corr", "desktop", 1)).unwrap();
            let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
            let mut router = ConnectionRouter::with_services("capacity-server".into(), "capacity-connection".into(), "capacity-session".into(), true, services);
            ready(&mut router);
            let (pause, mut entered) = crate::workspace_browse::pause_directory_read(workspace.canonicalize().unwrap().to_string_lossy().into_owned());
            let (client, server) = tokio::io::duplex(8_192);
            let (server_reader, server_writer) = tokio::io::split(server);
            let (shutdown_tx, shutdown_rx) = watch::channel(false);
            let server_task = tokio::spawn(serve_connection(server_reader, server_writer, router, shutdown_tx, shutdown_rx));
            let (client_reader, mut client_writer) = tokio::io::split(client);
            let mut lines = BufReader::new(client_reader).lines();
            for n in 0..crate::workspace_browse::MAX_DIRECTORY_READS {
                let path = (n > 0).then(|| format!("dir{n}"));
                let request = json!({"jsonrpc":"2.0","id":format!("read{n}"),"method":"workspace/listDirectory","params":{"workspaceId":"w-capacity","relativePath":path}});
                client_writer.write_all(format!("{request}\n").as_bytes()).await.unwrap();
                tokio::time::timeout(std::time::Duration::from_secs(10), entered.recv()).await.unwrap().unwrap();
            }
            for (id, path, reason) in [("duplicate", "", "workspace_directory_busy"), ("capacity", "dir4", "workspace_directory_capacity_reached")] {
                let request = json!({"jsonrpc":"2.0","id":id,"method":"workspace/listDirectory","params":{"workspaceId":"w-capacity","relativePath":path}});
                client_writer.write_all(format!("{request}\n").as_bytes()).await.unwrap();
                let response: Value = serde_json::from_str(&tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line()).await.unwrap().unwrap().unwrap()).unwrap();
                assert_eq!(response["id"], id);
                assert_eq!(response["error"]["data"]["reason"], reason);
            }
            drop(pause);
            let mut completed = std::collections::HashSet::new();
            for _ in 0..crate::workspace_browse::MAX_DIRECTORY_READS {
                let response: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert!(response.get("result").is_some(), "{response}");
                assert!(completed.insert(response["id"].as_str().unwrap().to_owned()));
            }
            for (id, path, expected) in [("released", "dir4", None), ("failed", "missing", Some("workspace_target_unavailable")), ("failed-again", "missing", Some("workspace_target_unavailable"))] {
                let request = json!({"jsonrpc":"2.0","id":id,"method":"workspace/listDirectory","params":{"workspaceId":"w-capacity","relativePath":path}});
                client_writer.write_all(format!("{request}\n").as_bytes()).await.unwrap();
                let response: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
                assert_eq!(response["id"], id);
                if let Some(reason) = expected { assert_eq!(response["error"]["data"]["reason"], reason); }
                else { assert_eq!(response["result"]["entries"], json!([])); }
            }
            drop(client_writer);
            drop(lines);
            server_task.await.unwrap().unwrap();
            fs::remove_dir_all(root).unwrap();
        });
    }

    #[test]
    fn initialize_requires_initialized_notification_before_business_calls() {
        let mut router = ConnectionRouter::new("srv_test".into());
        let before = router.handle_frame(frame(
            r#"{"jsonrpc":"2.0","id":"r0","method":"workspace/list","params":{}}"#,
        ));
        assert!(before.response.unwrap().contains("not_initialized"));
        let initialized = router.handle_frame(frame(r#"{"jsonrpc":"2.0","id":"r1","method":"initialize","params":{"client":{"name":"test","version":"0.1.0","platform":"macos","arch":"arm64"},"protocol":{"minVersion":"1.0","maxVersion":"1.0"}}}"#));
        let response: Value = serde_json::from_str(&initialized.response.unwrap()).unwrap();
        assert_eq!(response["id"], "r1");
        assert_eq!(response["result"]["protocolVersion"], "1.0");
        assert!(
            router
                .handle_frame(frame(
                    r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#
                ))
                .response
                .is_none()
        );
        let after = router.handle_frame(frame(
            r#"{"jsonrpc":"2.0","id":"r2","method":"workspace/list","params":{}}"#,
        ));
        let after: Value = serde_json::from_str(&after.response.unwrap()).unwrap();
        assert_eq!(after["error"]["code"], BUSINESS_ERROR);
        assert_eq!(after["error"]["data"]["reason"], "app_service_unavailable");
    }

    #[test]
    fn incompatible_protocol_returns_stable_reason_and_range() {
        let mut router = ConnectionRouter::new("srv_test".into());
        let response = router.handle_frame(frame(r#"{"jsonrpc":"2.0","id":7,"method":"initialize","params":{"client":{"name":"test","version":"1","platform":"windows","arch":"x86_64"},"protocol":{"minVersion":"2.0","maxVersion":"2.1"}}}"#)).response.unwrap();
        let value: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["id"], 7);
        assert_eq!(
            value["error"]["data"]["reason"],
            "protocol_version_unsupported"
        );
        assert_eq!(
            value["error"]["data"]["serverSupportedRange"]["maxVersion"],
            "1.0"
        );
    }

    #[test]
    fn request_id_is_correlated() {
        let mut router = ConnectionRouter::new("srv_test".into());
        let response = router
            .handle_frame(frame(
                r#"{"jsonrpc":"2.0","id":"correlation","method":"ping"}"#,
            ))
            .response
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&response).unwrap()["id"],
            "correlation"
        );
        let _ = RequestId::Integer(1);
    }

    #[test]
    fn workspace_files_are_paged_and_each_response_stays_bounded() {
        let root = std::env::temp_dir().join(format!(
            "ch-files-page-{}-{}",
            std::process::id(),
            unix_epoch_millis().unwrap()
        ));
        let data = root.join("data");
        let workspace = root.join("workspace");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir(&workspace).unwrap();
        for index in 0..451 {
            fs::write(
                workspace.join(format!("document-{index:04}.md")),
                format!("# Document {index}\n"),
            )
            .unwrap();
        }
        let mut persistence =
            SqlitePersistence::open(&data.join("contexthub.db"), "0.1.0").unwrap();
        create_preflighted_local_workspace(
            &mut persistence,
            &workspace,
            std::slice::from_ref(&data),
            local_workspace_command(
                "w-files-page",
                "Paged files",
                "scan-files-page",
                "event-files-page",
                "corr-files-page",
                "desktop",
                1,
            ),
        )
        .unwrap();
        capture_full_local_snapshot(
            &mut persistence,
            &CaptureLocalSnapshotCommand {
                workspace_id: WorkspaceId::new("w-files-page"),
                snapshot_id: "snapshot-files-page".to_owned(),
                event_id: EventId::new("event-snapshot-files-page"),
                correlation_id: "correlation-files-page".to_owned(),
                observed_at: 2,
                completed_at: 3,
            },
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
        let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
        let mut router = ConnectionRouter::with_services(
            "server-files-page".to_owned(),
            "connection-files-page".to_owned(),
            "session-files-page".to_owned(),
            true,
            services,
        );
        ready(&mut router);

        let first_response = router
            .handle_frame(frame(
                r#"{"jsonrpc":"2.0","id":"page-1","method":"workspace/files","params":{"workspaceId":"w-files-page","cursor":null,"limit":200}}"#,
            ))
            .response
            .unwrap();
        assert!(first_response.len() <= MAX_MESSAGE_BYTES);
        let first: Value = serde_json::from_str(&first_response).unwrap();
        assert_eq!(first["result"]["files"].as_array().unwrap().len(), 200);
        let next_cursor = first["result"]["nextCursor"].as_str().unwrap();
        assert_eq!(first["result"]["hasMore"], true);

        let middle = request_value(
            &mut router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"page-2","method":"workspace/files","params":{{"workspaceId":"w-files-page","cursor":"{next_cursor}","limit":200}}}}"#
            ),
        );
        assert_eq!(middle["result"]["files"].as_array().unwrap().len(), 200);
        let next_cursor = middle["result"]["nextCursor"].as_str().unwrap();
        let last = request_value(
            &mut router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"page-3","method":"workspace/files","params":{{"workspaceId":"w-files-page","cursor":"{next_cursor}","limit":200}}}}"#
            ),
        );
        assert_eq!(last["result"]["files"].as_array().unwrap().len(), 51);
        assert!(last["result"]["nextCursor"].is_null());
        assert_eq!(last["result"]["hasMore"], false);

        let invalid = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"invalid-page","method":"workspace/files","params":{"workspaceId":"w-files-page","cursor":null,"limit":501}}"#,
        );
        assert_eq!(
            invalid["error"]["data"]["reason"],
            "workspace_files_page_size_invalid"
        );
        drop(router);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_outbound_response_becomes_a_correlated_protocol_error() {
        let response = success(
            RequestId::String("oversized".to_owned()),
            &"x".repeat(MAX_MESSAGE_BYTES),
        )
        .unwrap();
        let bounded = limit_route_response(RouteResult {
            response: Some(response),
            should_exit: false,
        });
        let value: Value = serde_json::from_str(&bounded.response.unwrap()).unwrap();
        assert_eq!(value["id"], "oversized");
        assert_eq!(value["error"]["data"]["reason"], "response_too_large");
        assert_eq!(value["error"]["data"]["maxMessageBytes"], MAX_MESSAGE_BYTES);
    }

    #[test]
    fn ai_state_protocol_round_trips_and_rejects_stale_revision() {
        let root = std::env::temp_dir().join(format!(
            "ch-ai-state-route-{}-{}",
            std::process::id(),
            unix_epoch_millis().unwrap()
        ));
        fs::create_dir_all(&root).unwrap();
        let persistence = SqlitePersistence::open(&root.join("contexthub.db"), "0.1.0").unwrap();
        let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
        let mut router = ConnectionRouter::with_services(
            "server-ai-state".to_owned(),
            "connection-ai-state".to_owned(),
            "session-ai-state".to_owned(),
            true,
            services,
        );
        ready(&mut router);

        let initial = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"get","method":"aiState/get","params":{}}"#,
        );
        assert_eq!(initial["result"]["stateRevision"], 0);
        let state = serde_json::json!({
            "agents": [], "conversations": [], "messages": [], "runs": [],
            "permissionRequests": [], "inboxReadItemIds": ["permission:1"],
            "automationRules": []
        });
        let replaced = request_value(
            &mut router,
            &serde_json::json!({
                "jsonrpc": "2.0", "id": "replace", "method": "aiState/replace",
                "params": { "expectedRevision": 0, "state": state.clone() }
            })
            .to_string(),
        );
        assert_eq!(replaced["result"]["stateRevision"], 1);

        let conflict = request_value(
            &mut router,
            &serde_json::json!({
                "jsonrpc": "2.0", "id": "conflict", "method": "aiState/replace",
                "params": { "expectedRevision": 0, "state": state }
            })
            .to_string(),
        );
        assert_eq!(
            conflict["error"]["data"]["reason"],
            "ai_state_revision_conflict"
        );
        drop(router);
        fs::remove_dir_all(root).unwrap();
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the acceptance test walks the whole browse, priority-index and request flow in order"
    )]
    #[test]
    fn 新连文件夹在索引建完之前可以逐层浏览并优先索引点开的文件() {
        let root = std::env::temp_dir().join(format!(
            "ch-browse-route-{}-{}",
            std::process::id(),
            unix_epoch_millis().unwrap()
        ));
        let data = root.join("data");
        let workspace = root.join("workspace");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(workspace.join("docs/deep")).unwrap();
        fs::create_dir_all(workspace.join(".git")).unwrap();
        fs::write(workspace.join("readme.md"), "# Root").unwrap();
        fs::write(workspace.join("docs/deep/plan.md"), "# Plan").unwrap();
        let mut persistence =
            SqlitePersistence::open(&data.join("contexthub.db"), "0.1.0").unwrap();
        create_preflighted_local_workspace(
            &mut persistence,
            &workspace,
            std::slice::from_ref(&data),
            local_workspace_command("w-browse", "Browse", "scan", "event", "corr", "desktop", 1),
        )
        .unwrap();
        let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
        let mut router = ConnectionRouter::with_services(
            "server-browse".to_owned(),
            "connection-browse".to_owned(),
            "session-browse".to_owned(),
            true,
            Arc::clone(&services),
        );
        ready(&mut router);

        let top = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"top","method":"workspace/listDirectory","params":{"workspaceId":"w-browse","relativePath":null}}"#,
        );
        let names = top["result"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["relativePath"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["docs", "readme.md"]);
        assert_eq!(top["result"]["entries"][0]["fileType"], "directory");

        let nested = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"nested","method":"workspace/listDirectory","params":{"workspaceId":"w-browse","relativePath":"docs/deep"}}"#,
        );
        assert_eq!(
            nested["result"]["entries"][0]["relativePath"],
            "docs/deep/plan.md"
        );
        assert_eq!(nested["result"]["entries"][0]["sizeBytes"], 6);

        let escape = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"escape","method":"workspace/listDirectory","params":{"workspaceId":"w-browse","relativePath":"../"}}"#,
        );
        assert_eq!(escape["error"]["data"]["reason"], "workspace_path_invalid");

        let indexed = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"index","method":"workspace/indexPath","params":{"workspaceId":"w-browse","relativePath":"docs/deep/plan.md"}}"#,
        );
        let file_id = indexed["result"]["localFileId"]
            .as_str()
            .unwrap()
            .to_owned();
        let opened = request_value(
            &mut router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"open","method":"preview/open","params":{{"workspaceId":"w-browse","localFileId":"{file_id}","source":"local"}}}}"#
            ),
        );
        assert_eq!(opened["result"]["availability"], "ready");

        // 只补录了一条：工作区不能因此被当成索引已建完。
        let listed = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"list","method":"workspace/list","params":{}}"#,
        );
        assert_ne!(listed["result"]["workspaces"][0]["localStatus"], "ready");

        // 这个连接背后没有监听主管：请求建索引要明确失败，而不是假装已排队。
        let requested = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"request","method":"workspace/indexRequest","params":{"workspaceId":"w-browse"}}"#,
        );
        assert_eq!(
            requested["error"]["data"]["reason"],
            "workspace_index_unavailable"
        );
        let status = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"status","method":"workspace/indexStatus","params":{"workspaceId":"w-browse"}}"#,
        );
        assert_eq!(status["result"]["phase"], "idle");

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        services
            .lock()
            .unwrap()
            .index_board()
            .attach_requests(sender);
        let requested = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"request-2","method":"workspace/indexRequest","params":{"workspaceId":"w-browse"}}"#,
        );
        assert_eq!(requested["result"]["workspaceId"], "w-browse");
        assert_eq!(receiver.try_recv().unwrap(), "w-browse");
        let status = request_value(
            &mut router,
            r#"{"jsonrpc":"2.0","id":"status-2","method":"workspace/indexStatus","params":{"workspaceId":"w-browse"}}"#,
        );
        assert_eq!(status["result"]["phase"], "queued");
        drop(router);
        drop(services);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preview_protocol_reads_local_markdown_and_keeps_source_explicit() {
        let root = std::env::temp_dir().join(format!(
            "ch-preview-route-{}-{}",
            std::process::id(),
            unix_epoch_millis().unwrap()
        ));
        let data = root.join("data");
        let workspace = root.join("workspace");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("note.md"), "# Routed preview").unwrap();
        let mut persistence =
            SqlitePersistence::open(&data.join("contexthub.db"), "0.1.0").unwrap();
        create_preflighted_local_workspace(
            &mut persistence,
            &workspace,
            std::slice::from_ref(&data),
            local_workspace_command(
                "w-preview",
                "Preview",
                "scan",
                "event",
                "corr",
                "desktop",
                1,
            ),
        )
        .unwrap();
        let snapshot = capture_full_local_snapshot(
            &mut persistence,
            &CaptureLocalSnapshotCommand {
                workspace_id: WorkspaceId::new("w-preview"),
                snapshot_id: "snapshot-preview".to_owned(),
                event_id: EventId::new("event-preview"),
                correlation_id: "correlation-preview".to_owned(),
                observed_at: 2,
                completed_at: 3,
            },
            &WorkspaceCancellationToken::new(),
        )
        .unwrap();
        let file_id = snapshot.business.created_local_file_ids[0].clone();
        let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
        let mut router = ConnectionRouter::with_services(
            "server-preview".to_owned(),
            "connection-preview".to_owned(),
            "session-preview".to_owned(),
            true,
            services,
        );
        ready(&mut router);

        let opened = request_value(
            &mut router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"open","method":"preview/open","params":{{"workspaceId":"w-preview","localFileId":"{file_id}","source":"local"}}}}"#
            ),
        );
        assert_eq!(opened["result"]["availability"], "ready");
        assert_eq!(opened["result"]["source"], "local");
        assert!(opened["result"]["sourceModifiedAt"].is_number());
        let ticket_id = opened["result"]["ticketId"].as_str().unwrap();
        let content_revision = opened["result"]["contentRevision"].as_str().unwrap();
        let ticket_uuid = ticket_id.strip_prefix("preview-ticket-").unwrap();
        uuid::Uuid::parse_str(ticket_uuid).unwrap();
        let read = request_value(
            &mut router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"read","method":"textResource/read","params":{{"ticketId":"{ticket_id}","offset":0,"maxBytes":256}}}}"#
            ),
        );
        assert_eq!(read["result"]["text"], "# Routed preview");
        assert_eq!(read["result"]["endOfResource"], true);

        assert_text_diff_protocol(&mut router, &file_id, ticket_id, content_revision);
        assert_local_confirmation(&mut router, &file_id, content_revision);

        let baseline = request_value(
            &mut router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"baseline","method":"preview/open","params":{{"workspaceId":"w-preview","localFileId":"{file_id}","source":"baseline"}}}}"#
            ),
        );
        assert_eq!(baseline["result"]["availability"], "ready");
        assert_eq!(baseline["result"]["source"], "baseline");
        assert_eq!(baseline["result"]["contentRevision"], content_revision);
        let baseline_ticket = baseline["result"]["ticketId"].as_str().unwrap();
        let baseline_read = request_value(
            &mut router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"baseline-read","method":"textResource/read","params":{{"ticketId":"{baseline_ticket}","offset":0,"maxBytes":256}}}}"#
            ),
        );
        assert_eq!(baseline_read["result"]["text"], "# Routed preview");
        assert_eq!(baseline_read["result"]["endOfResource"], true);
        drop(router);
        fs::remove_dir_all(root).unwrap();
    }

    fn assert_text_diff_protocol(
        router: &mut ConnectionRouter,
        file_id: &str,
        first_ticket_id: &str,
        first_revision: &str,
    ) {
        let reopened = request_value(
            router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"reopen","method":"preview/open","params":{{"workspaceId":"w-preview","localFileId":"{file_id}","source":"local"}}}}"#
            ),
        );
        let second_ticket_id = reopened["result"]["ticketId"].as_str().unwrap();
        let second_revision = reopened["result"]["contentRevision"].as_str().unwrap();
        assert_ne!(first_ticket_id, second_ticket_id);
        uuid::Uuid::parse_str(second_ticket_id.strip_prefix("preview-ticket-").unwrap()).unwrap();
        let computed = request_value(
            router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"diff","method":"textDiff/compute","params":{{"left":{{"ticketId":"{first_ticket_id}","source":"local","contentRevision":"{first_revision}","label":"note.md","version":null,"modifiedAt":1}},"right":{{"ticketId":"{second_ticket_id}","source":"local","contentRevision":"{second_revision}","label":"note.md","version":null,"modifiedAt":1}}}}}}"#
            ),
        );
        assert_eq!(computed["result"]["summary"]["unchangedLines"], 1);
        assert_eq!(computed["result"]["summary"]["changeBlocks"], 0);
        let diff_ticket_id = computed["result"]["diffTicketId"].as_str().unwrap();
        uuid::Uuid::parse_str(diff_ticket_id.strip_prefix("diff-ticket-").unwrap()).unwrap();
        let diff = request_value(
            router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"diff-read","method":"textDiff/read","params":{{"diffTicketId":"{diff_ticket_id}","cursor":0,"maxRows":200,"maxBytes":262144}}}}"#
            ),
        );
        assert_eq!(diff["result"]["rows"][0]["kind"], "equal");
        assert!(diff["result"]["nextCursor"].is_null());
    }

    fn assert_local_confirmation(
        router: &mut ConnectionRouter,
        file_id: &str,
        content_revision: &str,
    ) {
        let request = format!(
            r#"{{"jsonrpc":"2.0","id":"confirm","method":"file/confirm","params":{{"workspaceId":"w-preview","localFileId":"{file_id}","expectedChecksumAlgorithm":"sha256","expectedChecksumValue":"{content_revision}","expectedBaselineVersionId":null,"clientOperationId":"desktop-confirm-protocol-test"}}}}"#
        );
        let confirmed = request_value(router, &request);
        assert_eq!(confirmed["result"]["checksumValue"], content_revision);
        assert_eq!(confirmed["result"]["replayed"], false);
        let replay = request_value(router, &request);
        assert_eq!(
            replay["result"]["confirmationId"],
            confirmed["result"]["confirmationId"]
        );
        assert_eq!(replay["result"]["replayed"], true);

        let history = request_value(
            router,
            &format!(
                r#"{{"jsonrpc":"2.0","id":"confirmation-history","method":"file/confirmationHistory","params":{{"workspaceId":"w-preview","localFileId":"{file_id}"}}}}"#
            ),
        );
        assert_eq!(history["result"]["items"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn report_authorization_reaches_persistence_through_typed_desktop_rpc() {
        let root =
            std::env::temp_dir().join(format!("report-authorization-rpc-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let persistence = SqlitePersistence::open(&root.join("state.db"), "0.1.0").unwrap();
        let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
        let mut router = ConnectionRouter::with_services(
            "server".into(),
            "connection".into(),
            "session".into(),
            true,
            services,
        );
        ready(&mut router);
        let execute = |router: &mut ConnectionRouter, action: &str, payload: Value| {
            request_value(router, &json!({"jsonrpc":"2.0","id":action,"method":"aiApplication/execute","params":{"action":action,"payload":payload}}).to_string())
        };
        let id = format!("model_report_{}", "a".repeat(64));
        let app = "com.cilai.environmental.policy";
        let legacy = "b".repeat(64);
        let opened = execute(
            &mut router,
            "open",
            json!({"sessionId":id,"instanceId":id,"applicationId":app,"authorizationDigest":legacy}),
        );
        assert!(opened.get("error").is_none(), "{opened}");
        let reserved = execute(
            &mut router,
            "reserve",
            json!({"sessionId":id,"clientMessageId":"original-message","contentDigest":"original-digest","runId":"original-run","snapshot":{"runId":"original-run","applicationId":app,"modelId":"model","status":"completed"},"context":{},"attachments":[]}),
        );
        assert!(reserved.get("error").is_none(), "{reserved}");
        let mut payload = json!({"sessionId":id,"instanceId":id,"applicationId":app,"authorizationDigest":format!("report-v1:{}", "c".repeat(64)),"legacyAuthorizationDigest":legacy});
        let migrated = execute(&mut router, "openReport", payload.clone());
        assert!(migrated.get("error").is_none(), "{migrated}");
        assert_eq!(migrated["result"]["messages"][0]["runId"], "original-run");
        payload["legacyAuthorizationDigest"] = json!("d".repeat(64));
        let upgraded = execute(&mut router, "openReport", payload.clone());
        assert_eq!(
            upgraded["result"]["messages"],
            migrated["result"]["messages"]
        );
        payload["authorizationDigest"] = json!(format!("report-v1:{}", "e".repeat(64)));
        let denied = execute(&mut router, "openReport", payload.clone());
        assert_eq!(
            denied["error"]["data"]["reason"],
            "application_conversation_restore_denied"
        );
        router.source_client = Some("cli");
        let denied = execute(&mut router, "openReport", payload);
        assert_eq!(
            denied["error"]["data"]["reason"],
            "application_conversation_client_not_supported"
        );
        drop(router);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ordinary_page_actions_reach_persistence_through_typed_desktop_rpc() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!("page-actions-rpc-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let persistence = SqlitePersistence::open(&root.join("state.db"), "0.1.0").unwrap();
        let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
        let mut router = ConnectionRouter::with_services(
            "server".into(),
            "connection".into(),
            "session".into(),
            true,
            services,
        );
        ready(&mut router);
        let execute = |router: &mut ConnectionRouter, action: &str, payload: Value| {
            request_value(router, &json!({"jsonrpc":"2.0","id":action,"method":"aiApplication/execute","params":{"action":action,"payload":payload}}).to_string())
        };
        let instance = "synthetic-page-instance";
        let application = "com.cilai.environmental.policy";
        let authorization = format!("page-v1:{}", "a".repeat(64));
        let session = format!("model_page_{:x}", Sha256::digest(authorization.as_bytes()));
        let legacy = format!("model_session_{:x}", Sha256::digest(instance.as_bytes()));
        let opened = execute(
            &mut router,
            "openPage",
            json!({"sessionId":session,"legacySessionId":legacy,"instanceId":instance,"applicationId":application,"authorizationDigest":authorization,"legacyAuthorizationDigest":"original-authorization"}),
        );
        assert!(opened.get("error").is_none(), "{opened}");
        assert_eq!(opened["result"]["session"]["sessionId"], session);
        let listed = execute(
            &mut router,
            "listInstance",
            json!({"instanceId":instance,"applicationId":application}),
        );
        assert_eq!(listed["result"]["sessionIds"], json!([session]));
        let other_application = execute(
            &mut router,
            "listInstance",
            json!({"instanceId":instance,"applicationId":"another-application"}),
        );
        assert_eq!(other_application["result"]["sessionIds"], json!([]));
        let idle = execute(&mut router, "restartBlockers", json!({}));
        assert_eq!(idle["result"]["blockers"], json!([]));
        let unknown = execute(&mut router, "unsupportedActionForAcceptance", json!({}));
        assert_eq!(unknown["error"]["code"], -32602);
        router.source_client = Some("cli");
        for action in ["openPage", "listInstance", "restartBlockers"] {
            let rejected = execute(&mut router, action, json!({}));
            assert_eq!(
                rejected["error"]["data"]["reason"],
                "application_conversation_client_not_supported"
            );
        }
        drop(router);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn application_restart_blockers_track_real_messages_through_typed_desktop_rpc() {
        let root =
            std::env::temp_dir().join(format!("restart-blockers-rpc-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let persistence = SqlitePersistence::open(&root.join("state.db"), "0.1.0").unwrap();
        let services = Arc::new(Mutex::new(AppServerServices::new(Some(persistence))));
        let mut router = ConnectionRouter::with_services(
            "server".into(),
            "connection".into(),
            "session".into(),
            true,
            services,
        );
        ready(&mut router);
        let execute = |router: &mut ConnectionRouter, action: &str, payload: Value| {
            request_value(router, &json!({"jsonrpc":"2.0","id":action,"method":"aiApplication/execute","params":{"action":action,"payload":payload}}).to_string())
        };
        let opened = execute(
            &mut router,
            "open",
            json!({"sessionId":"synthetic-page","instanceId":"synthetic-instance","applicationId":"app","authorizationDigest":"authorization"}),
        );
        assert!(opened.get("error").is_none(), "{opened}");
        let mut payload = json!({"sessionId":"synthetic-page","clientMessageId":"synthetic-message","contentDigest":"synthetic-digest","runId":"synthetic-run","snapshot":{"runId":"synthetic-run","applicationId":"app","modelId":"model","status":"running"},"context":{},"attachments":[]});
        let reserved = execute(&mut router, "reserve", payload.clone());
        assert!(reserved.get("error").is_none(), "{reserved}");
        let active = execute(&mut router, "restartBlockers", json!({}));
        assert_eq!(
            active["result"]["blockers"],
            json!(["application_model_run_active"])
        );
        payload["snapshot"]["status"] = json!("completed");
        let completed = execute(&mut router, "update", payload);
        assert!(completed.get("error").is_none(), "{completed}");
        let idle = execute(&mut router, "restartBlockers", json!({}));
        assert_eq!(idle["result"]["blockers"], json!([]));
        let stored = execute(&mut router, "read", json!({"sessionId":"synthetic-page"}));
        assert_eq!(
            stored["result"]["messages"][0]["snapshot"]["status"],
            "completed"
        );
        drop(router);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_thread_journal_rejects_non_desktop_clients() {
        let mut router = ConnectionRouter::with_context(
            "server".to_owned(),
            "connection".to_owned(),
            "session".to_owned(),
            true,
        );
        router.phase = Phase::Ready;
        router.source_client = Some("cli");
        for method in [
            "aiThread/get",
            "aiThread/prepare",
            "aiThread/acknowledge",
            "aiThread/reconcile",
            "aiThread/release",
            "aiThread/cleanup",
        ] {
            let response = request_value(
                &mut router,
                &json!({"jsonrpc":"2.0","id":"denied","method":method,"params":{}}).to_string(),
            );
            assert_eq!(
                response["error"]["data"]["reason"],
                "ai_thread_client_not_supported"
            );
        }
    }

    #[test]
    fn removed_enterprise_and_capture_methods_are_refused_for_every_client() {
        for client in ["cli", "desktop"] {
            let mut router = ConnectionRouter::with_context(
                "server".into(),
                "connection".into(),
                "session".into(),
                true,
            );
            router.phase = Phase::Ready;
            router.source_client = Some(client);
            for method in [
                "capture/execute",
                "capture/ingest",
                "cloudWorkspace/createTeamSpace",
                "cloudWorkspace/attach",
                "cloudWorkspace/fileAction",
                "cloudWorkspace/material",
                "cloudWorkspace/preview",
                "cloudWorkspace/deleteItem",
                "cloudWorkspace/start",
                "cloudWorkspace/status",
                "cloudWorkspace/teamSpaces",
                "syncPlan/create",
                "syncPlan/get",
                "sync/execute",
                "syncExecution/get",
                "conflict/list",
                "conflict/resolve",
                "applicationCatalog/list",
                "applicationCatalog/launch",
                "aiBrowserPolicy/get",
                "employeeCredential/get",
                "employeeCredential/save",
                "employeeCredential/delete",
            ] {
                let request =
                    json!({"jsonrpc":"2.0", "id":method, "method":method, "params":{}}).to_string();
                assert!(router.detach(&frame(&request)).is_none());
                let response = request_value(&mut router, &request);
                assert_eq!(response["id"], method);
                assert_eq!(
                    response["error"]["data"]["reason"], "personal_feature_removed",
                    "{client}: {method}"
                );
            }
            assert_eq!(
                request_value(
                    &mut router,
                    r#"{"jsonrpc":"2.0","id":"ping","method":"ping"}"#
                )["result"]["protocolVersion"],
                "1.0"
            );
        }
    }

    fn ready(router: &mut ConnectionRouter) {
        let initialized = router.handle_frame(frame(r#"{"jsonrpc":"2.0","id":"init","method":"initialize","params":{"client":{"name":"contexthub-desktop","version":"1","platform":"macos","arch":"arm64"},"protocol":{"minVersion":"1.0","maxVersion":"1.0"}}}"#));
        let initialized: Value = serde_json::from_str(&initialized.response.unwrap()).unwrap();
        assert_eq!(
            initialized["result"]["capabilities"]["resourceTickets"],
            true
        );
        assert_eq!(initialized["result"]["capabilities"]["eventReplay"], true);
        assert_eq!(initialized["result"]["resume"]["replayAvailable"], true);
        let _ = router.handle_frame(frame(
            r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
        ));
    }

    fn request_value(router: &mut ConnectionRouter, request: &str) -> Value {
        serde_json::from_str(
            &router
                .handle_frame(frame(request))
                .response
                .expect("request returns a response"),
        )
        .unwrap()
    }
}
