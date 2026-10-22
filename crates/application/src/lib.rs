#![doc = "`ContextHub` 应用用例、端口和事务边界。"]

mod global_search;
mod my_work;
mod project_portfolio;
mod project_progress;
mod work_attention;

pub use global_search::*;
pub use my_work::*;
pub use project_portfolio::*;
pub use project_progress::*;
pub use work_attention::*;

use contexthub_domain::{
    AcceptanceDecision, AccountId, DeliverableStatus, DeliverableVersionStatus, EventId,
    FileStateInput, FileStateOutput, LocalWorkspaceStatus, ProjectStatus, TaskAttemptId,
    TaskAttemptState, TaskId, TaskState, WorkspaceId, WorkspaceKind, WorkspacePlatformBindingState,
    compute_file_state,
};
use contexthub_manifest::{EncodedManifestSet, ManifestError, ManifestSet};

#[must_use]
pub fn workspace_label(workspace_id: &WorkspaceId) -> String {
    format!("workspace:{}", workspace_id.as_str())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRecord {
    pub workspace_id: WorkspaceId,
    pub workspace_kind: WorkspaceKind,
    pub display_name: String,
    pub local_root: String,
    pub local_root_identity: Option<String>,
    pub local_status: LocalWorkspaceStatus,
    pub state_revision: i64,
    pub last_local_scan_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRecord {
    pub project_id: String,
    pub display_name: String,
    pub goal: String,
    pub status: ProjectStatus,
    pub primary_workspace_id: Option<WorkspaceId>,
    pub state_revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliverableRecord {
    pub deliverable_id: String,
    pub project_id: String,
    pub title: String,
    pub deliverable_type: String,
    pub owner_label: String,
    pub acceptance_criteria: String,
    pub status: DeliverableStatus,
    pub current_accepted_version_id: Option<String>,
    pub state_revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliverableFileBindingRecord {
    pub version_id: String,
    pub local_file_id: String,
    pub workspace_id: WorkspaceId,
    pub relative_path: String,
    pub file_revision: i64,
    pub checksum_algorithm: String,
    pub checksum_value: String,
    pub binding_role: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliverableCarrierRecord {
    pub carrier_id: String,
    pub version_id: String,
    pub carrier_type: String,
    pub carrier_role: String,
    pub label: String,
    pub reference: String,
    pub version_label: String,
    pub source_project_id: Option<String>,
    pub source_deliverable_id: Option<String>,
    pub source_version_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliverableVersionRecord {
    pub version_id: String,
    pub deliverable_id: String,
    pub version_no: i64,
    pub status: DeliverableVersionStatus,
    pub summary: String,
    pub created_at: i64,
    pub accepted_at: Option<i64>,
    pub rejected_at: Option<i64>,
    pub bindings: Vec<DeliverableFileBindingRecord>,
    pub carriers: Vec<DeliverableCarrierRecord>,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliverableCarrierInput {
    pub carrier_id: String,
    pub carrier_type: String,
    pub carrier_role: String,
    pub label: String,
    pub reference: String,
    pub version_label: String,
    pub source_version_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliverableReviewRecord {
    pub acceptance_id: String,
    pub deliverable: DeliverableRecord,
    pub version: DeliverableVersionRecord,
    pub decision: AcceptanceDecision,
    pub reviewer_label: String,
    pub comment: String,
    pub created_at: i64,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteProjectCommand {
    pub project_id: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

/// 删除项目时被移除的过程信息计数，用于向用户回报删除范围。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeletedProjectSummary {
    pub project_id: String,
    pub released_workspace_ids: Vec<WorkspaceId>,
    pub removed_phases: i64,
    pub removed_tasks: i64,
    pub removed_deliverables: i64,
    pub removed_attention_cases: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeleteWorkspaceCommand {
    pub workspace_id: WorkspaceId,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub occurred_at: i64,
}

/// 删除工作区时被清除的记录数，用于向用户回报影响范围。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeletedWorkspaceSummary {
    pub workspace_id: String,
    pub removed_files: i64,
    pub removed_events: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetachProjectWorkspaceCommand {
    pub project_id: String,
    pub workspace_id: WorkspaceId,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttachProjectWorkspaceCommand {
    pub project_id: String,
    pub workspace_id: WorkspaceId,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateProjectCommand {
    pub project_id: String,
    pub display_name: String,
    pub goal: String,
    pub primary_workspace_id: Option<WorkspaceId>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateProjectCommand {
    pub project_id: String,
    pub display_name: String,
    pub goal: String,
    pub expected_revision: i64,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateDeliverableCommand {
    pub deliverable_id: String,
    pub project_id: String,
    pub title: String,
    pub deliverable_type: String,
    pub owner_label: String,
    pub acceptance_criteria: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateDeliverableVersionCommand {
    pub version_id: String,
    pub deliverable_id: String,
    pub summary: String,
    pub local_file_ids: Vec<String>,
    pub carriers: Vec<DeliverableCarrierInput>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewDeliverableVersionCommand {
    pub acceptance_id: String,
    pub deliverable_id: String,
    pub version_id: String,
    pub decision: AcceptanceDecision,
    pub reviewer_label: String,
    pub comment: String,
    pub expected_deliverable_revision: i64,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceFileRecord {
    pub local_file_id: String,
    pub relative_path: String,
    pub normalized_relative_path: String,
    pub file_identity: Option<String>,
    pub file_type: String,
    pub size_bytes: Option<i64>,
    pub modified_at_fs: Option<i64>,
    pub presence_state: String,
    pub display_state: Option<String>,
    /// 本地基线相对平台最新已发布版本的关系。纯本地工作区为 `platform_absent`。
    pub baseline_relation: Option<String>,
    pub local_change_state: Option<String>,
    pub confirmation_state: Option<String>,
    pub conflict_state: Option<String>,
    pub state_revision: Option<i64>,
    pub checksum_algorithm: Option<String>,
    pub checksum_value: Option<String>,
    pub baseline_version_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceFilePage {
    pub files: Vec<WorkspaceFileRecord>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspacePlatformBindingRecord {
    pub workspace_id: WorkspaceId,
    pub account_id: AccountId,
    pub organization_id: Option<String>,
    pub platform_workspace_id: Option<String>,
    pub project_id: String,
    pub platform_root: String,
    pub binding_state: WorkspacePlatformBindingState,
    pub platform_snapshot_revision: Option<String>,
    pub last_platform_check_at: Option<i64>,
    pub team_space_id: Option<String>,
    pub managed_root: Option<String>,
    pub device_id: Option<String>,
    pub baseline_revision: Option<String>,
    pub change_cursor: Option<String>,
    pub permission_role: Option<String>,
    pub permission_revision: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotKind {
    Full,
    Incremental,
}

impl SnapshotKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Incremental => "incremental",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalFileObservation {
    pub proposed_local_file_id: String,
    pub relative_path: String,
    pub normalized_relative_path: String,
    pub file_identity: Option<String>,
    pub file_type: String,
    pub size_bytes: Option<i64>,
    pub modified_at_fs: Option<i64>,
    pub checksum_algorithm: Option<String>,
    pub checksum_value: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalScanSnapshot {
    pub snapshot_id: String,
    pub workspace_id: WorkspaceId,
    pub generation: i64,
    pub snapshot_kind: SnapshotKind,
    pub observed_at: i64,
    pub completed_at: i64,
    pub issue_count: i64,
    pub event_id: EventId,
    pub correlation_id: String,
    pub observations: Vec<LocalFileObservation>,
    pub removed_normalized_paths: Vec<String>,
    /// 这次快照是否来自一遍完整的目录遍历。只有完整遍历才能把工作区标为就绪；
    /// 单个文件的优先索引只补录那一条，不代表整棵树已经收齐。
    pub covers_whole_tree: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSnapshotCommitResult {
    pub snapshot_id: String,
    pub retained_local_file_ids: Vec<String>,
    pub created_local_file_ids: Vec<String>,
    pub missing_local_file_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreviewFileRecord {
    pub workspace_id: WorkspaceId,
    pub local_file_id: String,
    pub workspace_root: String,
    pub relative_path: String,
    pub materialization_state: String,
    pub file_type: String,
    pub size_bytes: Option<i64>,
    pub modified_at_fs: Option<i64>,
    pub checksum_algorithm: Option<String>,
    pub checksum_value: Option<String>,
    pub local_change_state: Option<String>,
    pub baseline_text_content: Option<String>,
    pub baseline_content_revision: Option<String>,
    pub baseline_confirmed_at: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmationCandidate {
    pub workspace_id: WorkspaceId,
    pub workspace_is_cloud: bool,
    pub local_file_id: String,
    pub workspace_root: String,
    pub relative_path: String,
    pub materialization_state: String,
    pub file_type: String,
    pub scan_state: String,
    pub size_bytes: Option<i64>,
    pub modified_at_fs: Option<i64>,
    pub local_file_revision: i64,
    pub baseline_version_id: Option<String>,
    pub file_state_revision: Option<i64>,
    pub confirmation_allowed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateFileConfirmationCommand {
    pub confirmation_id: String,
    pub workspace_id: WorkspaceId,
    pub local_file_id: String,
    pub expected_local_file_revision: i64,
    pub expected_file_state_revision: Option<i64>,
    pub expected_checksum_algorithm: String,
    pub expected_checksum_value: String,
    pub expected_baseline_version_id: Option<String>,
    pub verified_size_bytes: i64,
    pub verified_modified_at_fs: i64,
    pub confirmed_text_content: Option<String>,
    pub confirmed_by_identity_id: AccountId,
    pub source_client: String,
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub confirmed_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileConfirmationRecord {
    pub confirmation_id: String,
    pub workspace_id: WorkspaceId,
    pub local_file_id: String,
    pub checksum_algorithm: String,
    pub checksum_value: String,
    pub baseline_version_id: Option<String>,
    pub confirmed_by: String,
    pub confirmed_at: i64,
    pub replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileConfirmationHistoryRecord {
    pub confirmation_id: String,
    pub checksum_algorithm: String,
    pub checksum_value: String,
    pub baseline_version_id: Option<String>,
    pub confirmed_by: String,
    pub confirmed_at: i64,
    pub invalidated_at: Option<i64>,
    pub invalidation_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecomputeFileStateCommand {
    pub workspace_id: WorkspaceId,
    pub local_file_id: String,
    pub expected_state_revision: Option<i64>,
    pub input: FileStateInput,
    pub confirmation_invalidation_id: String,
    pub confirmation_invalidation_event_id: EventId,
    pub derived_conflict_id: String,
    pub event_id: EventId,
    pub correlation_id: String,
    pub computed_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedFileState {
    pub local_file_id: String,
    pub output: FileStateOutput,
    pub state_revision: i64,
    pub invalidated_confirmation: Option<ConfirmationInvalidationRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmationInvalidationRecord {
    pub invalidation_id: String,
    pub confirmation_id: String,
    pub workspace_id: WorkspaceId,
    pub local_file_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvalidateIdentityConfirmationsCommand {
    pub identity_id: AccountId,
    pub invalidation_id_prefix: String,
    pub event_id_prefix: String,
    pub correlation_id: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceExportRecord {
    pub workspace_id: WorkspaceId,
    pub desired_revision: i64,
    pub exported_revision: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectionWriteExpectation {
    pub exported_revision: i64,
    pub manifest_checksum: Option<String>,
    pub allow_incomplete_recovery: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceProjectionSnapshot {
    pub workspace_id: WorkspaceId,
    pub workspace_root: String,
    pub desired_revision: i64,
    pub expectation: ProjectionWriteExpectation,
    pub manifests: ManifestSet,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionExportOutcome {
    Exported,
    AlreadyCurrent,
}

#[derive(Debug)]
pub enum ExportWorkspaceProjectionError<RepositoryError, WriterError> {
    Repository(RepositoryError),
    Manifest(ManifestError),
    Writer(WriterError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateLocalWorkspaceCommand {
    pub workspace_id: WorkspaceId,
    pub display_name: String,
    pub canonical_local_root: String,
    pub local_root_identity: Option<String>,
    pub scan_task_id: TaskId,
    pub event_id: EventId,
    pub correlation_id: String,
    pub created_by_client: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewDurableEvent {
    pub event_id: EventId,
    pub workspace_id: Option<WorkspaceId>,
    pub event_type: String,
    pub object_type: Option<String>,
    pub object_id: Option<String>,
    pub payload_json: String,
    pub correlation_id: Option<String>,
    pub occurred_at: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredDurableEvent {
    pub global_sequence: i64,
    pub workspace_sequence: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableEventRecord {
    pub global_sequence: i64,
    pub event_id: EventId,
    pub workspace_id: Option<WorkspaceId>,
    pub workspace_sequence: Option<i64>,
    pub event_type: String,
    pub object_type: Option<String>,
    pub object_id: Option<String>,
    pub payload_json: String,
    pub correlation_id: Option<String>,
    pub occurred_at: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventReplayPage {
    pub events: Vec<DurableEventRecord>,
    pub next_sequence: i64,
    pub has_more: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRecord {
    pub task_id: TaskId,
    pub task_type: String,
    pub workspace_id: Option<WorkspaceId>,
    pub state: TaskState,
    pub progress_completed: i64,
    pub progress_total: Option<i64>,
    pub current_stage: Option<String>,
    pub cancellable: bool,
    pub payload_json: String,
    pub result_ref_type: Option<String>,
    pub result_ref_id: Option<String>,
    pub correlation_id: String,
    pub created_by_client: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub heartbeat_at: Option<i64>,
    pub error_reason: Option<String>,
    pub error_detail_json: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskAttemptRecord {
    pub attempt_id: TaskAttemptId,
    pub task_id: TaskId,
    pub attempt_no: i64,
    pub server_instance_id: String,
    pub state: TaskAttemptState,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub error_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskProgress {
    pub task_id: TaskId,
    pub progress_completed: i64,
    pub progress_total: Option<i64>,
    pub current_stage: Option<String>,
    pub heartbeat_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskTransition {
    pub task_id: TaskId,
    pub expected_state: TaskState,
    pub target_state: TaskState,
    pub occurred_at: i64,
    pub result_ref_type: Option<String>,
    pub result_ref_id: Option<String>,
    pub error_reason: Option<String>,
    pub error_detail_json: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRecoveryPolicyError {
    task_type: String,
}

impl TaskRecoveryPolicyError {
    #[must_use]
    pub fn task_type(&self) -> &str {
        &self.task_type
    }
}

impl std::fmt::Display for TaskRecoveryPolicyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "running task type '{}' has no startup recovery policy",
            self.task_type
        )
    }
}

impl std::error::Error for TaskRecoveryPolicyError {}

/// 为启动时发现的任务生成显式恢复迁移；无需变更的非运行任务返回 `None`。
///
/// # Errors
///
/// running 任务类型没有冻结的恢复策略时返回错误，调用方必须阻止服务进入 ready。
pub fn startup_recovery_transition(
    task: &TaskRecord,
    occurred_at: i64,
) -> Result<Option<TaskTransition>, TaskRecoveryPolicyError> {
    if task.state != TaskState::Running {
        return Ok(None);
    }
    let (target_state, reason) = match task.task_type.as_str() {
        "scan" | "diff" | "preview" | "materialize" => {
            (TaskState::Interrupted, "app_server_restarted")
        }
        "sync" => (
            TaskState::OutcomeUnknown,
            "app_server_restarted_external_result_unconfirmed",
        ),
        _ => {
            return Err(TaskRecoveryPolicyError {
                task_type: task.task_type.clone(),
            });
        }
    };
    Ok(Some(TaskTransition {
        task_id: task.task_id.clone(),
        expected_state: TaskState::Running,
        target_state,
        occurred_at,
        result_ref_type: None,
        result_ref_id: None,
        error_reason: Some(reason.to_owned()),
        error_detail_json: None,
    }))
}

pub trait TaskRepository {
    type Error;

    /// # Errors
    ///
    /// 任务不是合法的初始状态、违反持久化约束或无法写入时返回错误。
    fn insert_task(&mut self, task: &TaskRecord) -> Result<(), Self::Error>;

    /// # Errors
    ///
    /// 任务数据无法读取或不能转换为领域状态时返回错误。
    fn task(&mut self, task_id: &TaskId) -> Result<Option<TaskRecord>, Self::Error>;

    /// 返回所有需要启动恢复审计的非确定终态任务，顺序必须稳定。
    ///
    /// # Errors
    ///
    /// 任务数据无法读取或不能转换为领域状态时返回错误。
    fn nonterminal_tasks(&mut self) -> Result<Vec<TaskRecord>, Self::Error>;

    /// # Errors
    ///
    /// 任务不存在、不能开始或 attempt 无法创建时返回错误。
    fn start_task(
        &mut self,
        task_id: &TaskId,
        attempt_id: &TaskAttemptId,
        server_instance_id: &str,
        started_at: i64,
    ) -> Result<(TaskRecord, TaskAttemptRecord), Self::Error>;

    /// # Errors
    ///
    /// 任务不在运行状态、进度倒退、超过总量或无法写入时返回错误。
    fn update_task_progress(&mut self, progress: &TaskProgress) -> Result<TaskRecord, Self::Error>;

    /// # Errors
    ///
    /// expected state 不匹配、状态转换非法、不可取消任务被取消或无法写入时返回错误。
    fn transition_task(&mut self, transition: &TaskTransition) -> Result<TaskRecord, Self::Error>;
}

pub trait DurableEventReader {
    type Error;

    /// 从当前可用窗口的第一条事件开始返回一页，供没有历史游标的新客户端初始化。
    ///
    /// # Errors
    ///
    /// 页大小非法或持久化数据无法读取时返回错误。
    fn replay_global_available(&self, limit: usize) -> Result<EventReplayPage, Self::Error>;

    /// # Errors
    ///
    /// 页大小非法、游标不在可重放窗口或持久化数据无法读取时返回错误。
    fn replay_global_after(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> Result<EventReplayPage, Self::Error>;

    /// # Errors
    ///
    /// 工作区不存在、页大小非法、游标不在可重放窗口或数据无法读取时返回错误。
    fn replay_workspace_after(
        &self,
        workspace_id: &WorkspaceId,
        after_sequence: i64,
        limit: usize,
    ) -> Result<EventReplayPage, Self::Error>;

    /// # Errors
    ///
    /// 持久化游标无法读取时返回错误。
    fn client_cursor(
        &self,
        client_identity: &str,
        account_id: &AccountId,
    ) -> Result<Option<i64>, Self::Error>;
}

pub trait ClientEventCursorRepository {
    type Error;

    /// 确认客户端已处理到指定全局 sequence，返回实际保存的单调游标。
    ///
    /// # Errors
    ///
    /// 客户端身份为空、账号不存在、sequence 超前或当前事务无法写入时返回错误。
    fn acknowledge_event_cursor(
        &mut self,
        client_identity: &str,
        account_id: &AccountId,
        sequence: i64,
        updated_at: i64,
    ) -> Result<i64, Self::Error>;
}

pub trait WorkspaceRepository {
    type Error;

    /// # Errors
    ///
    /// 记录违反业务约束或当前事务无法写入时返回错误。
    fn insert_workspace(&mut self, workspace: &WorkspaceRecord) -> Result<(), Self::Error>;
    /// # Errors
    ///
    /// 工作区不存在、投影已存在或当前事务无法写入时返回错误。
    fn initialize_workspace_export(
        &mut self,
        export: &WorkspaceExportRecord,
    ) -> Result<(), Self::Error>;
    /// # Errors
    ///
    /// 当前事务无法读取或持久化数据不能转换为领域状态时返回错误。
    fn workspace(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<WorkspaceRecord>, Self::Error>;
    /// 返回全部本地工作区，按创建时间和 ID 稳定排序。
    ///
    /// # Errors
    ///
    /// 当前事务无法读取或持久化数据不能转换为领域状态时返回错误。
    fn workspaces(&mut self) -> Result<Vec<WorkspaceRecord>, Self::Error>;
    /// # Errors
    /// 工作区不存在或文件状态无法读取时返回错误。
    fn workspace_files(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<WorkspaceFileRecord>, Self::Error>;
    /// 只返回当前还在磁盘上的文件记录。
    ///
    /// 增量比对只关心它们，而历史上标为缺失的旧记录可能是它们的好几倍。
    ///
    /// # Errors
    /// 工作区不存在或文件状态无法读取时返回错误。
    fn present_workspace_files(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<WorkspaceFileRecord>, Self::Error>;
    /// 只返回在指定快照里首次出现的文件记录。
    ///
    /// 首次索引分批提交时，每批只需为本批新增的记录初始化状态；按工作区全量读取会让
    /// 每一批都比上一批更慢。
    ///
    /// # Errors
    /// 工作区不存在或文件状态无法读取时返回错误。
    fn workspace_files_first_seen_in(
        &mut self,
        workspace_id: &WorkspaceId,
        snapshot_id: &str,
    ) -> Result<Vec<WorkspaceFileRecord>, Self::Error>;
    /// 返回稳定排序的工作区文件页；`limit` 必须大于零。
    ///
    /// # Errors
    ///
    /// 工作区不存在、分页参数非法或文件状态无法读取时返回错误。
    fn workspace_files_page(
        &mut self,
        workspace_id: &WorkspaceId,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<WorkspaceFilePage, Self::Error>;

    /// # Errors
    ///
    /// 当前事务无法读取或绑定状态无法转换时返回错误。
    fn platform_binding(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<WorkspacePlatformBindingRecord>, Self::Error>;

    /// # Errors
    ///
    /// 工作区不存在、修订不匹配或当前事务无法写入时返回错误。
    fn update_workspace_status(
        &mut self,
        workspace_id: &WorkspaceId,
        expected_revision: i64,
        status: LocalWorkspaceStatus,
        updated_at: i64,
    ) -> Result<WorkspaceRecord, Self::Error>;
}

pub trait FileSnapshotRepository {
    type Error;

    /// 返回工作区下一次可提交的本地扫描代次。
    ///
    /// # Errors
    ///
    /// 工作区不存在或当前事务无法读取时返回错误。
    fn next_local_snapshot_generation(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<i64, Self::Error>;

    /// 原子提交一次完整或增量本地扫描观察；完整快照中未出现的当前文件转为 missing。
    ///
    /// # Errors
    ///
    /// 快照代次、路径、身份或持久化约束冲突时返回错误并回滚全部变更。
    fn commit_local_snapshot(
        &mut self,
        snapshot: &LocalScanSnapshot,
    ) -> Result<LocalSnapshotCommitResult, Self::Error>;
}

pub trait PreviewRepository {
    type Error;

    /// 返回安全预览签发所需的当前工作区与文件观察，不包含文件正文。
    ///
    /// # Errors
    ///
    /// 当前事务无法读取或持久化记录不满足类型约束时返回错误。
    fn preview_file(
        &mut self,
        workspace_id: &WorkspaceId,
        local_file_id: &str,
    ) -> Result<Option<PreviewFileRecord>, Self::Error>;
}

pub trait ConfirmationRepository {
    type Error;

    /// 返回强校验和确认事务需要绑定的当前数据库事实，不读取文件正文。
    ///
    /// # Errors
    ///
    /// 当前事务无法读取或记录关系损坏时返回错误。
    fn confirmation_candidate(
        &mut self,
        workspace_id: &WorkspaceId,
        local_file_id: &str,
    ) -> Result<Option<ConfirmationCandidate>, Self::Error>;

    /// 按客户端操作键查询已提交确认；相同键绑定不同指纹时必须拒绝。
    ///
    /// # Errors
    ///
    /// 幂等键冲突或读取失败时返回错误。
    fn file_confirmation_for_operation(
        &mut self,
        client_operation_id: &str,
        request_fingerprint: &str,
    ) -> Result<Option<FileConfirmationRecord>, Self::Error>;

    /// 在重新核验所有期望事实后创建或幂等重放文件确认。
    ///
    /// # Errors
    ///
    /// 文件、基线、修订、能力或幂等指纹变化，以及写入失败时返回错误。
    fn create_file_confirmation(
        &mut self,
        command: &CreateFileConfirmationCommand,
    ) -> Result<FileConfirmationRecord, Self::Error>;

    /// 返回指定文件的全部确认审计记录，按确认时间倒序排列。
    ///
    /// # Errors
    ///
    /// 文件不属于工作区、确认历史损坏或当前事务读取失败时返回错误。
    fn file_confirmation_history(
        &mut self,
        workspace_id: &WorkspaceId,
        local_file_id: &str,
    ) -> Result<Vec<FileConfirmationHistoryRecord>, Self::Error>;

    /// 在身份已不再有效时原子失效其全部当前确认，并推进相关工作区投影修订。
    ///
    /// # Errors
    ///
    /// 身份仍有效、命令不合法、历史约束冲突或写入失败时返回错误。
    fn invalidate_confirmations_for_identity(
        &mut self,
        command: &InvalidateIdentityConfirmationsCommand,
    ) -> Result<Vec<ConfirmationInvalidationRecord>, Self::Error>;
}

pub trait FileStateRepository {
    type Error;

    /// 原子持久化领域层计算结果，并按结果使旧确认失效、建立或结束派生冲突。
    ///
    /// # Errors
    ///
    /// 文件不存在、状态修订不匹配、输入事实与当前有效记录冲突或写入失败时返回错误。
    fn persist_computed_file_state(
        &mut self,
        command: &RecomputeFileStateCommand,
        output: &FileStateOutput,
    ) -> Result<PersistedFileState, Self::Error>;
}

pub trait ProjectionRepository {
    type Error;

    /// 读取某一 desired revision 对应的完整一致投影快照。
    ///
    /// # Errors
    ///
    /// 工作区或导出状态不存在、关系数据损坏，或当前事务无法读取时返回错误。
    fn projection_snapshot(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceProjectionSnapshot, Self::Error>;

    /// 仅在 desired revision 仍与导出快照一致时登记成功。
    ///
    /// # Errors
    ///
    /// 导出期间业务事实已经推进、工作区不存在或写入失败时返回错误。
    fn mark_projection_succeeded(
        &mut self,
        workspace_id: &WorkspaceId,
        expected_desired_revision: i64,
        manifest_checksum: &str,
        updated_at: i64,
    ) -> Result<(), Self::Error>;

    /// 导出失败只更新投影状态，不回滚此前已经提交的业务事实。
    ///
    /// # Errors
    ///
    /// 导出期间业务事实已经推进、工作区不存在或写入失败时返回错误。
    fn mark_projection_failed(
        &mut self,
        workspace_id: &WorkspaceId,
        expected_desired_revision: i64,
        reason: &str,
        conflict: bool,
        updated_at: i64,
    ) -> Result<(), Self::Error>;
}

pub trait ProjectionWriter {
    type Error: std::fmt::Display;

    /// # Errors
    ///
    /// 目标目录不安全、现有投影与期望不一致或原子写入失败时返回错误。
    fn write_projection(
        &mut self,
        workspace_root: &str,
        manifests: &EncodedManifestSet,
        expectation: &ProjectionWriteExpectation,
    ) -> Result<ProjectionExportOutcome, Self::Error>;

    fn is_conflict(error: &Self::Error) -> bool;
}

/// 从 `SQLite` 一致快照导出便携投影，再用独立事务登记结果。
///
/// # Errors
///
/// 快照读取、Schema 编码、文件系统写入或结果登记失败时返回分层错误；文件系统失败不会伪报业务事务失败。
pub fn export_workspace_projection<Unit, Writer>(
    unit_of_work: &mut Unit,
    writer: &mut Writer,
    workspace_id: &WorkspaceId,
    generated_at: i64,
) -> Result<ProjectionExportOutcome, ExportWorkspaceProjectionError<Unit::Error, Writer::Error>>
where
    Unit: UnitOfWork,
    Writer: ProjectionWriter,
{
    let snapshot = unit_of_work
        .transact(|repositories| repositories.projection_snapshot(workspace_id))
        .map_err(ExportWorkspaceProjectionError::Repository)?;
    let encoded = snapshot
        .manifests
        .encode(generated_at)
        .map_err(ExportWorkspaceProjectionError::Manifest)?;
    match writer.write_projection(&snapshot.workspace_root, &encoded, &snapshot.expectation) {
        Ok(outcome) => {
            unit_of_work
                .transact(|repositories| {
                    repositories.mark_projection_succeeded(
                        &snapshot.workspace_id,
                        snapshot.desired_revision,
                        &encoded.manifest_checksum,
                        generated_at,
                    )
                })
                .map_err(ExportWorkspaceProjectionError::Repository)?;
            Ok(outcome)
        }
        Err(error) => {
            let conflict = Writer::is_conflict(&error);
            let reason = error.to_string();
            unit_of_work
                .transact(|repositories| {
                    repositories.mark_projection_failed(
                        &snapshot.workspace_id,
                        snapshot.desired_revision,
                        &reason,
                        conflict,
                        generated_at,
                    )
                })
                .map_err(ExportWorkspaceProjectionError::Repository)?;
            Err(ExportWorkspaceProjectionError::Writer(error))
        }
    }
}

/// 使用唯一领域规则计算文件状态，并与确认失效、冲突记录和耐久事件原子提交。
///
/// # Errors
///
/// 命令字段无效、乐观修订冲突或 Repository 写入失败时返回错误。
pub fn recompute_file_state<Repositories>(
    repositories: &mut Repositories,
    command: &RecomputeFileStateCommand,
) -> Result<PersistedFileState, <Repositories as FileStateRepository>::Error>
where
    Repositories: FileStateRepository
        + DurableEventRepository<Error = <Repositories as FileStateRepository>::Error>,
{
    let output = compute_file_state(&command.input);
    let persisted = repositories.persist_computed_file_state(command, &output)?;
    if let Some(invalidation) = &persisted.invalidated_confirmation {
        repositories.append_event(&NewDurableEvent {
            event_id: command.confirmation_invalidation_event_id.clone(),
            workspace_id: Some(command.workspace_id.clone()),
            event_type: "file.confirmation_invalidated".to_owned(),
            object_type: Some("file_confirmation".to_owned()),
            object_id: Some(invalidation.confirmation_id.clone()),
            payload_json: format!(
                "{{\"local_file_id\":\"{}\",\"invalidation_id\":\"{}\",\"reason\":\"{}\"}}",
                invalidation.local_file_id, invalidation.invalidation_id, invalidation.reason
            ),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.computed_at,
            created_at: command.computed_at,
        })?;
    }
    repositories.append_event(&NewDurableEvent {
        event_id: command.event_id.clone(),
        workspace_id: Some(command.workspace_id.clone()),
        event_type: "file.state_changed".to_owned(),
        object_type: Some("local_file".to_owned()),
        object_id: Some(command.local_file_id.clone()),
        payload_json: format!(
            "{{\"state_revision\":{},\"display_state\":\"{}\",\"conflict_state\":\"{}\"}}",
            persisted.state_revision,
            output.display_state.as_str(),
            output.conflict_state.as_str()
        ),
        correlation_id: Some(command.correlation_id.clone()),
        occurred_at: command.computed_at,
        created_at: command.computed_at,
    })?;
    Ok(persisted)
}

/// 失效已停用身份创建的全部当前确认，并为每项追加独立耐久事件。
///
/// # Errors
///
/// 身份仍有效、命令非法或任一确认、投影、事件写入失败时整笔事务回滚。
pub fn invalidate_identity_confirmations<Repositories>(
    repositories: &mut Repositories,
    command: &InvalidateIdentityConfirmationsCommand,
) -> Result<Vec<ConfirmationInvalidationRecord>, <Repositories as ConfirmationRepository>::Error>
where
    Repositories: ConfirmationRepository
        + DurableEventRepository<Error = <Repositories as ConfirmationRepository>::Error>,
{
    let invalidations = repositories.invalidate_confirmations_for_identity(command)?;
    for invalidation in &invalidations {
        repositories.append_event(&NewDurableEvent {
            event_id: EventId::new(format!(
                "{}{}",
                command.event_id_prefix, invalidation.confirmation_id
            )),
            workspace_id: Some(invalidation.workspace_id.clone()),
            event_type: "file.confirmation_invalidated".to_owned(),
            object_type: Some("file_confirmation".to_owned()),
            object_id: Some(invalidation.confirmation_id.clone()),
            payload_json: format!(
                "{{\"local_file_id\":\"{}\",\"invalidation_id\":\"{}\",\"reason\":\"{}\"}}",
                invalidation.local_file_id, invalidation.invalidation_id, invalidation.reason
            ),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.occurred_at,
            created_at: command.occurred_at,
        })?;
    }
    Ok(invalidations)
}

/// 创建绑定文件 checksum、基线、身份和客户端操作键的人工确认，并原子追加耐久事件。
///
/// # Errors
///
/// 当前事实不匹配、幂等键冲突或仓储写入失败时返回错误。
pub fn create_file_confirmation<Repositories>(
    repositories: &mut Repositories,
    command: &CreateFileConfirmationCommand,
) -> Result<FileConfirmationRecord, <Repositories as ConfirmationRepository>::Error>
where
    Repositories: ConfirmationRepository
        + DurableEventRepository<Error = <Repositories as ConfirmationRepository>::Error>,
{
    let confirmation = repositories.create_file_confirmation(command)?;
    if !confirmation.replayed {
        repositories.append_event(&NewDurableEvent {
            event_id: command.event_id.clone(),
            workspace_id: Some(command.workspace_id.clone()),
            event_type: "file.confirmed".to_owned(),
            object_type: Some("file_confirmation".to_owned()),
            object_id: Some(confirmation.confirmation_id.clone()),
            payload_json: format!(
                "{{\"local_file_id\":\"{}\",\"checksum_algorithm\":\"{}\",\"checksum_value\":\"{}\"}}",
                confirmation.local_file_id,
                confirmation.checksum_algorithm,
                confirmation.checksum_value
            ),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.confirmed_at,
            created_at: command.confirmed_at,
        })?;
    }
    Ok(confirmation)
}

pub trait DurableEventRepository {
    type Error;

    /// # Errors
    ///
    /// 事件违反唯一性或引用约束，或当前事务无法写入时返回错误。
    fn append_event(&mut self, event: &NewDurableEvent) -> Result<StoredDurableEvent, Self::Error>;
}

pub trait ProjectOutcomeRepository {
    type Error;

    /// 按工作区稳定读取显式绑定的项目。
    ///
    /// # Errors
    ///
    /// 数据无法读取或领域状态无法转换时返回错误。
    fn projects_for_workspace(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ProjectRecord>, Self::Error>;

    /// # Errors
    ///
    /// 工作区不存在、幂等冲突或记录无法写入时返回错误。
    fn insert_project(
        &mut self,
        command: &CreateProjectCommand,
    ) -> Result<ProjectRecord, Self::Error>;

    /// # Errors
    ///
    /// 项目不存在、修订不匹配、幂等冲突或记录无法写入时返回错误。
    fn update_project(
        &mut self,
        command: &UpdateProjectCommand,
    ) -> Result<ProjectRecord, Self::Error>;

    /// 把工作空间关联到项目；同一工作空间可同时关联多个项目。
    ///
    /// 重复绑定同一对项目与工作区视为重放并返回原项目，不产生第二条记录。
    ///
    /// # Errors
    ///
    /// 项目或工作区不存在，或记录无法写入时返回错误。
    fn attach_project_workspace(
        &mut self,
        command: &AttachProjectWorkspaceCommand,
    ) -> Result<ProjectRecord, Self::Error>;

    /// 解除当前项目与文件夹的关联，保留其他项目的关联。
    ///
    /// 当前导航默认文件夹移出后由仓储原子选择替代项；没有替代项时项目保持为空。
    /// 重复移出同一个文件夹视为重放。
    ///
    /// # Errors
    ///
    /// 项目不存在或关联无法更新时返回错误。
    fn detach_project_workspace(
        &mut self,
        command: &DetachProjectWorkspaceCommand,
    ) -> Result<ProjectRecord, Self::Error>;

    /// 删除一个不属于任何项目的工作区，连同它的文件索引、快照与变更历史。
    ///
    /// 磁盘上的目录与文件不受影响。仍被交付物版本引用的工作区不允许删除。
    ///
    /// # Errors
    ///
    /// 工作区不存在、仍归属某个项目、或其文件仍被交付物引用时返回错误。
    fn delete_workspace(
        &mut self,
        command: &DeleteWorkspaceCommand,
    ) -> Result<DeletedWorkspaceSummary, Self::Error>;

    /// 删除项目及其全部过程信息（阶段、任务、交付物元数据、关注事项与投影），
    /// 并释放其持有的工作区。工作区本身与磁盘上的文件都不会被删除。
    ///
    /// # Errors
    ///
    /// 项目不存在或记录无法删除时返回错误。
    fn delete_project(
        &mut self,
        command: &DeleteProjectCommand,
    ) -> Result<DeletedProjectSummary, Self::Error>;

    /// 按项目身份稳定读取单个项目，不依赖调用方先知道其所属工作区。
    ///
    /// # Errors
    ///
    /// 项目不存在或数据无法读取时返回错误。
    fn project_by_id(&mut self, project_id: &str) -> Result<ProjectRecord, Self::Error>;

    /// 读取项目当前绑定的全部工作区。
    ///
    /// # Errors
    ///
    /// 项目不存在或数据无法读取时返回错误。
    fn workspaces_for_project(&mut self, project_id: &str)
    -> Result<Vec<WorkspaceId>, Self::Error>;

    /// # Errors
    ///
    /// 项目不存在或数据无法读取时返回错误。
    fn deliverables_for_project(
        &mut self,
        project_id: &str,
    ) -> Result<Vec<DeliverableRecord>, Self::Error>;

    /// # Errors
    ///
    /// 项目不可编辑、幂等冲突或记录无法写入时返回错误。
    fn insert_deliverable(
        &mut self,
        command: &CreateDeliverableCommand,
    ) -> Result<DeliverableRecord, Self::Error>;

    /// # Errors
    ///
    /// 交付物不存在或数据无法读取时返回错误。
    fn versions_for_deliverable(
        &mut self,
        deliverable_id: &str,
    ) -> Result<Vec<DeliverableVersionRecord>, Self::Error>;

    /// # Errors
    ///
    /// 绑定文件不属于项目、不是当前可用真实文件、幂等冲突或事务失败时返回错误。
    fn insert_deliverable_version(
        &mut self,
        command: &CreateDeliverableVersionCommand,
    ) -> Result<DeliverableVersionRecord, Self::Error>;

    /// # Errors
    ///
    /// 候选版本或修订不匹配、幂等冲突或事务失败时返回错误。
    fn review_deliverable_version(
        &mut self,
        command: &ReviewDeliverableVersionCommand,
    ) -> Result<DeliverableReviewRecord, Self::Error>;
}

pub trait RepositoryTransaction:
    WorkspaceRepository<Error = Self::TransactionError>
    + FileSnapshotRepository<Error = Self::TransactionError>
    + PreviewRepository<Error = Self::TransactionError>
    + ConfirmationRepository<Error = Self::TransactionError>
    + FileStateRepository<Error = Self::TransactionError>
    + ProjectionRepository<Error = Self::TransactionError>
    + DurableEventRepository<Error = Self::TransactionError>
    + ProjectOutcomeRepository<Error = Self::TransactionError>
    + ProjectProgressRepository<Error = Self::TransactionError>
    + MyWorkRepository<Error = Self::TransactionError>
    + ProjectPortfolioRepository<Error = Self::TransactionError>
    + AttentionCaseRepository<Error = Self::TransactionError>
    + ClientEventCursorRepository<Error = Self::TransactionError>
    + TaskRepository<Error = Self::TransactionError>
{
    type TransactionError;
}

/// 创建显式项目；本地工作区是可选关系，并原子发布项目事件。
///
/// # Errors
///
/// 工作区归属、幂等或持久化约束不满足时返回错误。
pub fn create_project<Repositories>(
    repositories: &mut Repositories,
    command: &CreateProjectCommand,
) -> Result<ProjectRecord, <Repositories as ProjectOutcomeRepository>::Error>
where
    Repositories: ProjectOutcomeRepository
        + DurableEventRepository<Error = <Repositories as ProjectOutcomeRepository>::Error>,
{
    let project = repositories.insert_project(command)?;
    if !project.replayed {
        repositories.append_event(&NewDurableEvent {
            event_id: command.event_id.clone(),
            workspace_id: command.primary_workspace_id.clone(),
            event_type: "project.created".to_owned(),
            object_type: Some("project".to_owned()),
            object_id: Some(project.project_id.clone()),
            payload_json: format!("{{\"display_name\":\"{}\"}}", project.display_name),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.occurred_at,
            created_at: command.occurred_at,
        })?;
    }
    Ok(project)
}

/// 修改项目的名称与目标并原子发布事件。
///
/// 项目是抽象容器，改名不触碰磁盘上的任何文件，也不改变它持有的文件夹。
///
/// # Errors
///
/// 项目不存在、修订与调用方所见不一致或记录无法写入时返回错误。
pub fn update_project<Repositories>(
    repositories: &mut Repositories,
    command: &UpdateProjectCommand,
) -> Result<ProjectRecord, <Repositories as ProjectOutcomeRepository>::Error>
where
    Repositories: ProjectOutcomeRepository
        + DurableEventRepository<Error = <Repositories as ProjectOutcomeRepository>::Error>,
{
    let project = repositories.update_project(command)?;
    {
        repositories.append_event(&NewDurableEvent {
            event_id: command.event_id.clone(),
            workspace_id: None,
            event_type: "project.updated".to_owned(),
            object_type: Some("project".to_owned()),
            object_id: Some(project.project_id.clone()),
            payload_json: format!("{{\"display_name\":\"{}\"}}", project.display_name),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.occurred_at,
            created_at: command.occurred_at,
        })?;
    }
    Ok(project)
}

/// 删除项目及其过程信息并原子发布事件。
///
/// 变更历史中的既有事件流水不会被删除：它是审计记录，删除项目不等于该项目从未存在过。
///
/// # Errors
///
/// 项目不存在或删除约束不满足时返回错误。
pub fn delete_project<Repositories>(
    repositories: &mut Repositories,
    command: &DeleteProjectCommand,
) -> Result<DeletedProjectSummary, <Repositories as ProjectOutcomeRepository>::Error>
where
    Repositories: ProjectOutcomeRepository
        + DurableEventRepository<Error = <Repositories as ProjectOutcomeRepository>::Error>,
{
    let summary = repositories.delete_project(command)?;
    repositories.append_event(&NewDurableEvent {
        event_id: command.event_id.clone(),
        workspace_id: summary.released_workspace_ids.first().cloned(),
        event_type: "project.deleted".to_owned(),
        object_type: Some("project".to_owned()),
        object_id: Some(summary.project_id.clone()),
        payload_json: format!(
            "{{\"removed_phases\":{},\"removed_tasks\":{},\"removed_deliverables\":{}}}",
            summary.removed_phases, summary.removed_tasks, summary.removed_deliverables
        ),
        correlation_id: Some(command.correlation_id.clone()),
        occurred_at: command.occurred_at,
        created_at: command.occurred_at,
    })?;
    Ok(summary)
}

/// 删除一个不属于任何项目的工作区，并原子记录一条全局删除事件。
///
/// 该工作区自身的事件流水随之删除（用户已明确同意），因此这条记录不引用它，
/// 只保留「发生过一次删除」这一事实。
///
/// # Errors
///
/// 工作区不存在、仍归属项目或仍被交付物引用时返回错误。
pub fn delete_workspace<Repositories>(
    repositories: &mut Repositories,
    command: &DeleteWorkspaceCommand,
    event_id: EventId,
) -> Result<DeletedWorkspaceSummary, <Repositories as ProjectOutcomeRepository>::Error>
where
    Repositories: ProjectOutcomeRepository
        + DurableEventRepository<Error = <Repositories as ProjectOutcomeRepository>::Error>,
{
    let summary = repositories.delete_workspace(command)?;
    repositories.append_event(&NewDurableEvent {
        event_id,
        workspace_id: None,
        event_type: "workspace.deleted".to_owned(),
        object_type: Some("workspace".to_owned()),
        object_id: Some(summary.workspace_id.clone()),
        payload_json: format!(
            "{{\"removed_files\":{},\"removed_events\":{}}}",
            summary.removed_files, summary.removed_events
        ),
        correlation_id: None,
        occurred_at: command.occurred_at,
        created_at: command.occurred_at,
    })?;
    Ok(summary)
}

/// 把一个文件夹移出项目并原子发布事件。
///
/// # Errors
///
/// 项目或归属约束不满足时返回错误。
pub fn detach_project_workspace<Repositories>(
    repositories: &mut Repositories,
    command: &DetachProjectWorkspaceCommand,
) -> Result<ProjectRecord, <Repositories as ProjectOutcomeRepository>::Error>
where
    Repositories: ProjectOutcomeRepository
        + DurableEventRepository<Error = <Repositories as ProjectOutcomeRepository>::Error>,
{
    let project = repositories.detach_project_workspace(command)?;
    if !project.replayed {
        repositories.append_event(&NewDurableEvent {
            event_id: command.event_id.clone(),
            workspace_id: Some(command.workspace_id.clone()),
            event_type: "project.workspace_detached".to_owned(),
            object_type: Some("project".to_owned()),
            object_id: Some(project.project_id.clone()),
            payload_json: format!("{{\"workspace_id\":\"{}\"}}", command.workspace_id.as_str()),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.occurred_at,
            created_at: command.occurred_at,
        })?;
    }
    Ok(project)
}

/// 为已有项目追加一个工作区并原子发布事件。
///
/// # Errors
///
/// 项目或工作区归属约束不满足时返回错误。
pub fn attach_project_workspace<Repositories>(
    repositories: &mut Repositories,
    command: &AttachProjectWorkspaceCommand,
) -> Result<ProjectRecord, <Repositories as ProjectOutcomeRepository>::Error>
where
    Repositories: ProjectOutcomeRepository
        + DurableEventRepository<Error = <Repositories as ProjectOutcomeRepository>::Error>,
{
    let project = repositories.attach_project_workspace(command)?;
    if !project.replayed {
        repositories.append_event(&NewDurableEvent {
            event_id: command.event_id.clone(),
            workspace_id: Some(command.workspace_id.clone()),
            event_type: "project.workspace_attached".to_owned(),
            object_type: Some("project".to_owned()),
            object_id: Some(project.project_id.clone()),
            payload_json: format!("{{\"workspace_id\":\"{}\"}}", command.workspace_id.as_str()),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.occurred_at,
            created_at: command.occurred_at,
        })?;
    }
    Ok(project)
}

/// 创建项目交付物并原子发布事件。
///
/// # Errors
///
/// 项目状态、幂等或持久化约束不满足时返回错误。
pub fn create_deliverable<Repositories>(
    repositories: &mut Repositories,
    command: &CreateDeliverableCommand,
) -> Result<DeliverableRecord, <Repositories as ProjectOutcomeRepository>::Error>
where
    Repositories: ProjectOutcomeRepository
        + DurableEventRepository<Error = <Repositories as ProjectOutcomeRepository>::Error>,
{
    let deliverable = repositories.insert_deliverable(command)?;
    if !deliverable.replayed {
        repositories.append_event(&NewDurableEvent {
            event_id: command.event_id.clone(),
            workspace_id: None,
            event_type: "deliverable.created".to_owned(),
            object_type: Some("deliverable".to_owned()),
            object_id: Some(deliverable.deliverable_id.clone()),
            payload_json: format!("{{\"project_id\":\"{}\"}}", deliverable.project_id),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.occurred_at,
            created_at: command.occurred_at,
        })?;
    }
    Ok(deliverable)
}

/// 从当前真实文件事实冻结候选版本并原子发布事件。
///
/// # Errors
///
/// 文件绑定、幂等或持久化约束不满足时返回错误。
pub fn create_deliverable_version<Repositories>(
    repositories: &mut Repositories,
    command: &CreateDeliverableVersionCommand,
) -> Result<DeliverableVersionRecord, <Repositories as ProjectOutcomeRepository>::Error>
where
    Repositories: ProjectOutcomeRepository
        + DurableEventRepository<Error = <Repositories as ProjectOutcomeRepository>::Error>,
{
    let version = repositories.insert_deliverable_version(command)?;
    if !version.replayed {
        repositories.append_event(&NewDurableEvent {
            event_id: command.event_id.clone(),
            workspace_id: version
                .bindings
                .first()
                .map(|binding| binding.workspace_id.clone()),
            event_type: "deliverable.version.created".to_owned(),
            object_type: Some("deliverable_version".to_owned()),
            object_id: Some(version.version_id.clone()),
            payload_json: format!(
                "{{\"deliverable_id\":\"{}\",\"version_no\":{},\"file_count\":{}}}",
                version.deliverable_id,
                version.version_no,
                version.bindings.len()
            ),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.occurred_at,
            created_at: command.occurred_at,
        })?;
    }
    Ok(version)
}

/// 审阅候选版本并原子切换正式版本或记录退回。
///
/// # Errors
///
/// 状态、并发修订、幂等或持久化约束不满足时返回错误。
pub fn review_deliverable_version<Repositories>(
    repositories: &mut Repositories,
    command: &ReviewDeliverableVersionCommand,
) -> Result<DeliverableReviewRecord, <Repositories as ProjectOutcomeRepository>::Error>
where
    Repositories: ProjectOutcomeRepository
        + DurableEventRepository<Error = <Repositories as ProjectOutcomeRepository>::Error>,
{
    let review = repositories.review_deliverable_version(command)?;
    if !review.replayed {
        repositories.append_event(&NewDurableEvent {
            event_id: command.event_id.clone(),
            workspace_id: review
                .version
                .bindings
                .first()
                .map(|binding| binding.workspace_id.clone()),
            event_type: format!("deliverable.version.{}", review.decision.as_str()),
            object_type: Some("deliverable_version".to_owned()),
            object_id: Some(review.version.version_id.clone()),
            payload_json: format!(
                "{{\"deliverable_id\":\"{}\",\"state_revision\":{}}}",
                review.deliverable.deliverable_id, review.deliverable.state_revision
            ),
            correlation_id: Some(command.correlation_id.clone()),
            occurred_at: command.occurred_at,
            created_at: command.occurred_at,
        })?;
    }
    Ok(review)
}

pub trait UnitOfWork {
    type Error;
    type Transaction<'a>: RepositoryTransaction<TransactionError = Self::Error>
    where
        Self: 'a;

    /// # Errors
    ///
    /// 开启或提交事务失败，或操作闭包返回错误时回滚并返回该错误。
    fn transact<T, Operation>(&mut self, operation: Operation) -> Result<T, Self::Error>
    where
        Operation: FnOnce(&mut Self::Transaction<'_>) -> Result<T, Self::Error>;
}

/// 在一个事务中创建仅本地工作区、投影状态、首次扫描任务和 Durable Event。
///
/// 该用例不读取账号或平台项目；平台连接必须通过独立绑定端口建立。
///
/// # Errors
///
/// 任一记录违反约束或事务无法写入时返回仓储错误，调用方必须整体回滚。
pub fn create_local_workspace<Repositories>(
    repositories: &mut Repositories,
    command: &CreateLocalWorkspaceCommand,
) -> Result<WorkspaceRecord, Repositories::TransactionError>
where
    Repositories: RepositoryTransaction,
{
    let workspace = WorkspaceRecord {
        workspace_id: command.workspace_id.clone(),
        workspace_kind: WorkspaceKind::Local,
        display_name: command.display_name.clone(),
        local_root: command.canonical_local_root.clone(),
        local_root_identity: command.local_root_identity.clone(),
        local_status: LocalWorkspaceStatus::Scanning,
        state_revision: 0,
        last_local_scan_at: None,
        created_at: command.occurred_at,
        updated_at: command.occurred_at,
    };
    repositories.insert_workspace(&workspace)?;
    repositories.initialize_workspace_export(&WorkspaceExportRecord {
        workspace_id: command.workspace_id.clone(),
        desired_revision: 1,
        exported_revision: 0,
        updated_at: command.occurred_at,
    })?;
    repositories.insert_task(&TaskRecord {
        task_id: command.scan_task_id.clone(),
        task_type: "scan".to_owned(),
        workspace_id: Some(command.workspace_id.clone()),
        state: TaskState::Queued,
        progress_completed: 0,
        progress_total: None,
        current_stage: Some("workspace_created".to_owned()),
        cancellable: true,
        payload_json: "{}".to_owned(),
        result_ref_type: None,
        result_ref_id: None,
        correlation_id: command.correlation_id.clone(),
        created_by_client: command.created_by_client.clone(),
        created_at: command.occurred_at,
        started_at: None,
        completed_at: None,
        heartbeat_at: None,
        error_reason: None,
        error_detail_json: None,
    })?;
    repositories.append_event(&NewDurableEvent {
        event_id: command.event_id.clone(),
        workspace_id: Some(command.workspace_id.clone()),
        event_type: "workspace.created".to_owned(),
        object_type: Some("workspace".to_owned()),
        object_id: Some(command.workspace_id.as_str().to_owned()),
        payload_json: "{\"mode\":\"local\"}".to_owned(),
        correlation_id: Some(command.correlation_id.clone()),
        occurred_at: command.occurred_at,
        created_at: command.occurred_at,
    })?;
    Ok(workspace)
}

#[cfg(test)]
mod tests {
    use super::{TaskRecord, startup_recovery_transition};
    use contexthub_domain::{TaskId, TaskState};

    #[test]
    fn startup_recovery_policy_distinguishes_local_and_external_tasks() {
        let scan = task("scan", TaskState::Running);
        let scan_transition = startup_recovery_transition(&scan, 10).unwrap().unwrap();
        assert_eq!(scan_transition.target_state, TaskState::Interrupted);
        assert_eq!(
            scan_transition.error_reason.as_deref(),
            Some("app_server_restarted")
        );

        let sync = task("sync", TaskState::Running);
        let sync_transition = startup_recovery_transition(&sync, 11).unwrap().unwrap();
        assert_eq!(sync_transition.target_state, TaskState::OutcomeUnknown);
        assert!(
            startup_recovery_transition(&task("scan", TaskState::Queued), 12)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unknown_running_task_type_has_no_implicit_recovery_fallback() {
        let error =
            startup_recovery_transition(&task("unknown", TaskState::Running), 10).unwrap_err();
        assert_eq!(error.task_type(), "unknown");
    }

    fn task(task_type: &str, state: TaskState) -> TaskRecord {
        TaskRecord {
            task_id: TaskId::new("task-1"),
            task_type: task_type.to_owned(),
            workspace_id: None,
            state,
            progress_completed: 0,
            progress_total: None,
            current_stage: None,
            cancellable: true,
            payload_json: "{}".to_owned(),
            result_ref_type: None,
            result_ref_id: None,
            correlation_id: "correlation-1".to_owned(),
            created_by_client: "desktop".to_owned(),
            created_at: 1,
            started_at: (state != TaskState::Queued).then_some(2),
            completed_at: None,
            heartbeat_at: (state == TaskState::Running).then_some(2),
            error_reason: None,
            error_detail_json: None,
        }
    }
}
