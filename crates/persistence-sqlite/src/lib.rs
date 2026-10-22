#![doc = "`SQLite` 仓储与迁移适配器；仅允许 `Local App Server` 组合使用。"]

mod ai_business_ledger;
mod ai_history;
mod ai_memory_legacy;
mod ai_state;
mod ai_threads;
mod application_conversations;
mod application_handoff;
pub mod harness_home_migration;
mod mcp_connections;
mod model_connections;
mod model_usage;
mod personal_automation;
mod restore_abort;
mod shutdown;
mod text_integrity;
mod text_recovery;
pub use ai_state::{AiStateChanges, AiStateEntityChanges};
mod capability_creation;
mod capability_lifecycle;
mod enterprise_capabilities;
mod personal_application_data;
mod personal_data;
mod personal_page_audit;
pub use personal_application_data::{PersonalApplicationDataRecord, PersonalApplicationDataSlot};
mod personal_files;
pub use personal_data::materialize_native_database;
mod maintenance_operations;
mod personal_memory;
mod personal_memory_reset;
mod personal_namespace_storage;
mod personal_package_imports;
mod personal_storage;
pub use personal_memory_reset::{PersonalMemoryResetRecord, PersonalMemoryResetScopeAuthorization};
pub use personal_namespace_storage::{
    PersonalNamespaceActionAuthorization, PersonalNamespaceStorageRecord,
};
pub mod profile_writers;
pub use capability_creation::{
    CapabilityCreationBuildRecord, CapabilityCreationCreateCommand,
    CapabilityCreationDeliveryRecord, CapabilityCreationInstallCommand,
    CapabilityCreationOperationRecord, CapabilityCreationProjectRecord,
    PersonalCapabilityInstallationRecord, PersonalCapabilityManageCommand,
};
pub use capability_lifecycle::PersonalCapabilityVersion;
pub use maintenance_operations::personal_data_schema_version;
pub use personal_package_imports::{PersonalPackageImportCommand, PersonalPackageImportRecord};
pub use personal_storage::{
    PersonalStoragePathReference, PersonalStorageRecord, PersonalStorageWorkspaceReference,
};
mod skill_installations;
pub use skill_installations::SkillInstallationRecord;
mod file_content_index;
mod global_search;
pub use file_content_index::FileContentIndex;
mod incident_recovery;
mod my_work;
mod project_portfolio;
mod project_progress;
mod work_attention;

use contexthub_application::{
    AttachProjectWorkspaceCommand, ClientEventCursorRepository, ConfirmationCandidate,
    ConfirmationInvalidationRecord, ConfirmationRepository, CreateDeliverableCommand,
    CreateDeliverableVersionCommand, CreateFileConfirmationCommand, CreateProjectCommand,
    DeleteProjectCommand, DeleteWorkspaceCommand, DeletedProjectSummary, DeletedWorkspaceSummary,
    DeliverableCarrierRecord, DeliverableFileBindingRecord, DeliverableRecord,
    DeliverableReviewRecord, DeliverableVersionRecord, DetachProjectWorkspaceCommand,
    DurableEventReader, DurableEventRecord, DurableEventRepository, EventReplayPage,
    FileConfirmationHistoryRecord, FileConfirmationRecord, FileSnapshotRepository,
    FileStateRepository, InvalidateIdentityConfirmationsCommand, LocalScanSnapshot,
    LocalSnapshotCommitResult, NewDurableEvent, PersistedFileState, PreviewFileRecord,
    PreviewRepository, ProjectOutcomeRepository, ProjectRecord, ProjectionRepository,
    ProjectionWriteExpectation, RecomputeFileStateCommand, RepositoryTransaction,
    ReviewDeliverableVersionCommand, SnapshotKind, StoredDurableEvent, TaskAttemptRecord,
    TaskProgress, TaskRecord, TaskRepository, TaskTransition, UnitOfWork, UpdateProjectCommand,
    WorkspaceExportRecord, WorkspaceFilePage, WorkspaceFileRecord, WorkspacePlatformBindingRecord,
    WorkspaceProjectionSnapshot, WorkspaceRecord, WorkspaceRepository,
};
use contexthub_domain::{
    AcceptanceDecision, AccountId, ConfirmationState, ConflictState, DeliverableStatus,
    DeliverableVersionStatus, EventId, FileOperation, FileStateOutput, LocalWorkspaceStatus,
    ProjectStatus, TaskAttemptId, TaskAttemptState, TaskId, TaskState, WorkspaceId,
    WorkspacePlatformBindingState,
};
use contexthub_manifest::{
    BaselineEntry, BaselineManifest, Checksum, ConfirmationEntry, ConfirmationManifest,
    MANIFEST_SCHEMA_VERSION, ManifestSet, WorkspaceManifest,
};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior, params,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "create_schema_migrations",
        sql: include_str!("../../../migrations/0001_create_schema_migrations.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 2,
        name: "create_m0_core_schema",
        sql: include_str!("../../../migrations/0002_create_m0_core_schema.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 3,
        name: "split_local_workspace_and_platform_binding",
        sql: include_str!(
            "../../../migrations/0003_split_local_workspace_and_platform_binding.sql"
        ),
        rebuilds_foreign_key_parent: true,
    },
    Migration {
        version: 4,
        name: "add_file_and_platform_snapshots",
        sql: include_str!("../../../migrations/0004_add_file_and_platform_snapshots.sql"),
        rebuilds_foreign_key_parent: true,
    },
    Migration {
        version: 5,
        name: "add_local_confirmation_identity_and_idempotency",
        sql: include_str!(
            "../../../migrations/0005_add_local_confirmation_identity_and_idempotency.sql"
        ),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 6,
        name: "add_confirmation_identity_invalidation",
        sql: include_str!("../../../migrations/0006_add_confirmation_identity_invalidation.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 7,
        name: "freeze_sync_plan_and_operation_idempotency",
        sql: include_str!(
            "../../../migrations/0007_freeze_sync_plan_and_operation_idempotency.sql"
        ),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 8,
        name: "strengthen_conflict_resolution_idempotency",
        sql: include_str!(
            "../../../migrations/0008_strengthen_conflict_resolution_idempotency.sql"
        ),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 9,
        name: "add_workspace_file_order_index",
        sql: include_str!("../../../migrations/0009_add_workspace_file_order_index.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 10,
        name: "add_project_outcomes",
        sql: include_str!("../../../migrations/0010_add_project_outcomes.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 11,
        name: "add_project_progress",
        sql: include_str!("../../../migrations/0011_add_project_progress.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 12,
        name: "add_my_work_projection",
        sql: include_str!("../../../migrations/0012_add_my_work_projection.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 13,
        name: "add_attention_cases",
        sql: include_str!("../../../migrations/0013_add_attention_cases.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 14,
        name: "add_ai_runtime_state",
        sql: include_str!("../../../migrations/0014_add_ai_runtime_state.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 15,
        name: "add_business_task_attempts",
        sql: include_str!("../../../migrations/0015_add_business_task_attempts.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 16,
        name: "add_business_task_moves",
        sql: include_str!("../../../migrations/0016_add_business_task_moves.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 17,
        name: "add_deliverable_version_carriers",
        sql: include_str!("../../../migrations/0017_add_deliverable_version_carriers.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 18,
        name: "add_structured_decision_cards",
        sql: include_str!("../../../migrations/0018_add_structured_decision_cards.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 19,
        name: "add_structured_decision_actions",
        sql: include_str!("../../../migrations/0019_add_structured_decision_actions.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 20,
        name: "allow_projects_without_workspaces",
        sql: include_str!("../../../migrations/0020_allow_projects_without_workspaces.sql"),
        rebuilds_foreign_key_parent: true,
    },
    Migration {
        version: 21,
        name: "add_file_confirmation_text_snapshots",
        sql: include_str!("../../../migrations/0021_add_file_confirmation_text_snapshots.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 22,
        name: "add_ai_conversation_work_classification",
        sql: include_str!("../../../migrations/0022_add_ai_conversation_work_classification.sql"),
        rebuilds_foreign_key_parent: true,
    },
    Migration {
        version: 23,
        name: "focus_workbench_on_business_actions",
        sql: include_str!("../../../migrations/0023_focus_workbench_on_business_actions.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 24,
        name: "separate_task_responsibility_from_ai_model",
        sql: include_str!(
            "../../../migrations/0024_separate_task_responsibility_from_ai_model.sql"
        ),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 25,
        name: "add_conversation_completion_and_outcomes",
        sql: include_str!("../../../migrations/0025_add_conversation_completion_and_outcomes.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 26,
        name: "resolve_frontend_inferred_run_incidents",
        sql: include_str!("../../../migrations/0026_resolve_frontend_inferred_run_incidents.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 27,
        name: "add_cloud_team_workspaces",
        sql: include_str!("../../../migrations/0027_add_cloud_team_workspaces.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 28,
        name: "add_cloud_workspace_remote_binding",
        sql: include_str!("../../../migrations/0028_add_cloud_workspace_remote_binding.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 29,
        name: "add_cloud_workspace_baseline_transfer",
        sql: include_str!("../../../migrations/0029_add_cloud_workspace_baseline_transfer.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 30,
        name: "add_cloud_workspace_pending_operation_index",
        sql: include_str!(
            "../../../migrations/0030_add_cloud_workspace_pending_operation_index.sql"
        ),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 31,
        name: "add_user_skills",
        sql: include_str!("../../../migrations/0031_add_user_skills.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 32,
        name: "scope_file_confirmation_to_cloud_workspaces",
        sql: include_str!(
            "../../../migrations/0032_scope_file_confirmation_to_cloud_workspaces.sql"
        ),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 33,
        name: "add_direct_cloud_workspaces",
        sql: include_str!("../../../migrations/0033_add_direct_cloud_workspaces.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 34,
        name: "separate_ai_conversation_metadata",
        sql: include_str!("../../../migrations/0034_separate_ai_conversation_metadata.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 35,
        name: "add_ai_skill_installations",
        sql: include_str!("../../../migrations/0035_add_ai_skill_installations.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 36,
        name: "index_foreign_keys_for_workspace_delete",
        sql: include_str!("../../../migrations/0036_index_foreign_keys_for_workspace_delete.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 37,
        name: "index_foreign_keys_for_delete_paths",
        sql: include_str!("../../../migrations/0037_index_foreign_keys_for_delete_paths.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 38,
        name: "separate_ai_messages_and_runs",
        sql: include_str!("../../../migrations/0038_separate_ai_messages_and_runs.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 39,
        name: "deduplicate_ai_message_application_contexts",
        sql: include_str!(
            "../../../migrations/0039_deduplicate_ai_message_application_contexts.sql"
        ),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 40,
        name: "add_ai_thread_journal",
        sql: include_str!("../../../migrations/0040_add_ai_thread_journal.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 41,
        name: "project_failed_work_classifications",
        sql: include_str!("../../../migrations/0041_project_failed_work_classifications.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 42,
        name: "project_failed_tool_calls",
        sql: include_str!("../../../migrations/0042_project_failed_tool_calls.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 43,
        name: "add_employee_service_credentials",
        sql: include_str!("../../../migrations/0043_add_employee_service_credentials.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 44,
        name: "add_shared_file_content_index",
        sql: include_str!("../../../migrations/0044_add_shared_file_content_index.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 45,
        name: "allow_shared_project_workspaces",
        sql: include_str!("../../../migrations/0045_allow_shared_project_workspaces.sql"),
        rebuilds_foreign_key_parent: true,
    },
    Migration {
        version: 46,
        name: "add_capability_creation_projects",
        sql: include_str!("../../../migrations/0046_add_capability_creation_projects.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 47,
        name: "add_capability_source_packages",
        sql: include_str!("../../../migrations/0047_add_capability_source_packages.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 48,
        name: "add_personal_capability_installations",
        sql: include_str!("../../../migrations/0048_add_personal_capability_installations.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 49,
        name: "separate_classification_incidents_from_business_work",
        sql: include_str!(
            "../../../migrations/0049_separate_classification_incidents_from_business_work.sql"
        ),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 50,
        name: "add_personal_capability_management",
        sql: include_str!("../../../migrations/0050_add_personal_capability_management.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 51,
        name: "add_personal_capability_versions",
        sql: include_str!("../../../migrations/0051_add_personal_capability_versions.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 52,
        name: "add_personal_file_grants",
        sql: include_str!("../../../migrations/0052_add_personal_file_grants.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 53,
        name: "add_enterprise_capability_bindings",
        sql: include_str!("../../../migrations/0053_add_enterprise_capability_bindings.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 54,
        name: "retire_cloud_workspace_mirrors",
        sql: include_str!("../../../migrations/0054_retire_cloud_workspace_mirrors.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 55,
        name: "add_ai_business_ledger",
        sql: include_str!("../../../migrations/0055_add_ai_business_ledger.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 56,
        name: "add_application_thread_scopes",
        sql: include_str!("../../../migrations/0056_add_application_thread_scopes.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 57,
        name: "add_application_conversation_journal",
        sql: include_str!("../../../migrations/0057_add_application_conversation_journal.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 58,
        name: "add_parked_native_writer",
        sql: include_str!("../../../migrations/0058_add_parked_native_writer.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 59,
        name: "add_workbench_context_imports",
        sql: include_str!("../../../migrations/0059_add_workbench_context_imports.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 60,
        name: "add_capture_transfers",
        sql: include_str!("../../../migrations/0060_add_capture_transfers.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 61,
        name: "add_capture_workflow",
        sql: include_str!("../../../migrations/0061_add_capture_workflow.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 62,
        name: "add_capture_automation",
        sql: include_str!("../../../migrations/0062_add_capture_automation.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 63,
        name: "add_capture_hardware_library",
        sql: include_str!("../../../migrations/0063_add_capture_hardware_library.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 64,
        name: "freeze_capture_execution",
        sql: include_str!("../../../migrations/0064_freeze_capture_execution.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 65,
        name: "add_capture_device_results",
        sql: include_str!("../../../migrations/0065_add_capture_device_results.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 66,
        name: "add_capture_analysis_submissions",
        sql: include_str!("../../../migrations/0066_add_capture_analysis_submissions.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 67,
        name: "add_capture_text_result_bindings",
        sql: include_str!("../../../migrations/0067_add_capture_text_result_bindings.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 68,
        name: "add_personal_model_profile_fence",
        sql: include_str!("../../../migrations/0068_add_personal_model_profile_fence.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 69,
        name: "add_personal_package_imports",
        sql: include_str!("../../../migrations/0069_add_personal_package_imports.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 70,
        name: "add_personal_data_operations",
        sql: include_str!("../../../migrations/0070_add_personal_data_operations.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 71,
        name: "add_personal_mcp_records",
        sql: include_str!("../../../migrations/0071_add_personal_mcp_records.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 72,
        name: "add_personal_writer_fences",
        sql: include_str!("../../../migrations/0072_add_personal_writer_fences.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 73,
        name: "add_personal_storage_and_maintenance_registry",
        sql: include_str!(
            "../../../migrations/0073_add_personal_storage_and_maintenance_registry.sql"
        ),
        rebuilds_foreign_key_parent: true,
    },
    Migration {
        version: 74,
        name: "add_personal_memory_reset_operations",
        sql: include_str!("../../../migrations/0074_add_personal_memory_reset_operations.sql"),
        rebuilds_foreign_key_parent: true,
    },
    Migration {
        version: 75,
        name: "add_personal_application_data_slots",
        sql: include_str!("../../../migrations/0075_add_personal_application_data_slots.sql"),
        rebuilds_foreign_key_parent: false,
    },
    Migration {
        version: 76,
        name: "add_personal_namespace_storage",
        sql: include_str!("../../../migrations/0076_add_personal_namespace_storage.sql"),
        rebuilds_foreign_key_parent: false,
    },
];

#[derive(Clone, Copy)]
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
    rebuilds_foreign_key_parent: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteRuntimeConfiguration {
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub synchronous: i64,
    pub busy_timeout_ms: i64,
    pub temp_store: i64,
    pub trusted_schema: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiWorkTaskRecord {
    pub task_id: String,
    pub title: String,
    pub objective: String,
    pub expected_output: String,
    pub completion_criteria: String,
    pub stage: String,
    pub state_revision: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AiTurnClassificationRecord {
    pub action: String,
    pub task_action: String,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub objective: Option<String>,
    pub expected_output: Option<String>,
    pub completion_criteria: Option<String>,
    pub conversation_status: Option<String>,
    pub stage: Option<String>,
    pub outcome_title: Option<String>,
    pub outcome_summary: Option<String>,
    pub reason: String,
    pub confidence: f64,
    pub raw: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AiTurnFinalizeCommand {
    pub ledger_request: Option<serde_json::Value>,
    pub expected_revision: i64,
    pub changes: AiStateChanges,
    pub conversation_id: String,
    pub run_id: String,
    pub project_id: Option<String>,
    pub previous_task_id: Option<String>,
    pub agent_label: String,
    pub new_task_id: Option<String>,
    pub classification: AiTurnClassificationRecord,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiTurnFinalizeRecord {
    pub ledger_receipt: Option<serde_json::Value>,
    pub state_revision: i64,
    pub task_id: Option<String>,
    pub task_title: Option<String>,
    pub stage: Option<String>,
    /// 同一回合此前已提交过，本次没有写入状态。
    pub replayed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceErrorKind {
    Configuration,
    Database,
    Migration,
    Conflict,
    ReplayUnavailable,
}

pub struct SqliteRepositoryTransaction<'connection> {
    pub(crate) transaction: Transaction<'connection>,
}

impl UnitOfWork for SqlitePersistence {
    type Error = PersistenceError;
    type Transaction<'a> = SqliteRepositoryTransaction<'a>;

    fn transact<T, Operation>(&mut self, operation: Operation) -> Result<T, Self::Error>
    where
        Operation: FnOnce(&mut Self::Transaction<'_>) -> Result<T, Self::Error>,
    {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("start repository transaction"))?;
        let mut repositories = SqliteRepositoryTransaction { transaction };
        let output = operation(&mut repositories)?;
        repositories
            .transaction
            .commit()
            .map_err(database_error("commit repository transaction"))?;
        Ok(output)
    }
}

impl RepositoryTransaction for SqliteRepositoryTransaction<'_> {
    type TransactionError = PersistenceError;
}

impl PreviewRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn preview_file(
        &mut self,
        workspace_id: &WorkspaceId,
        local_file_id: &str,
    ) -> Result<Option<PreviewFileRecord>, Self::Error> {
        self.transaction
            .query_row(
                "SELECT w.local_root, lf.relative_path, lf.materialization_state, lf.file_type, lf.size_bytes, lf.modified_at_fs, lf.local_checksum_algorithm, lf.local_checksum_value, fs.local_change_state,
                        (SELECT snapshot.text_content FROM file_confirmation_text_snapshots snapshot JOIN file_confirmations confirmation ON confirmation.confirmation_id=snapshot.confirmation_id WHERE snapshot.workspace_id=lf.workspace_id AND snapshot.local_file_id=lf.local_file_id ORDER BY confirmation.created_at DESC, confirmation.confirmation_id DESC LIMIT 1),
                        (SELECT snapshot.content_revision FROM file_confirmation_text_snapshots snapshot JOIN file_confirmations confirmation ON confirmation.confirmation_id=snapshot.confirmation_id WHERE snapshot.workspace_id=lf.workspace_id AND snapshot.local_file_id=lf.local_file_id ORDER BY confirmation.created_at DESC, confirmation.confirmation_id DESC LIMIT 1),
                        (SELECT snapshot.created_at FROM file_confirmation_text_snapshots snapshot JOIN file_confirmations confirmation ON confirmation.confirmation_id=snapshot.confirmation_id WHERE snapshot.workspace_id=lf.workspace_id AND snapshot.local_file_id=lf.local_file_id ORDER BY confirmation.created_at DESC, confirmation.confirmation_id DESC LIMIT 1)
                 FROM local_files lf JOIN workspaces w ON w.workspace_id=lf.workspace_id LEFT JOIN file_states fs ON fs.local_file_id=lf.local_file_id WHERE lf.workspace_id=?1 AND lf.local_file_id=?2",
                params![workspace_id.as_str(), local_file_id],
                |row| {
                    Ok(PreviewFileRecord {
                        workspace_id: workspace_id.clone(),
                        local_file_id: local_file_id.to_owned(),
                        workspace_root: row.get(0)?,
                        relative_path: row.get(1)?,
                        materialization_state: row.get(2)?,
                        file_type: row.get(3)?,
                        size_bytes: row.get(4)?,
                        modified_at_fs: row.get(5)?,
                        checksum_algorithm: row.get(6)?,
                        checksum_value: row.get(7)?,
                        local_change_state: row.get(8)?,
                        baseline_text_content: row.get(9)?,
                        baseline_content_revision: row.get(10)?,
                        baseline_confirmed_at: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(database_error("read preview file"))
    }
}

impl ConfirmationRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn confirmation_candidate(
        &mut self,
        workspace_id: &WorkspaceId,
        local_file_id: &str,
    ) -> Result<Option<ConfirmationCandidate>, Self::Error> {
        self.transaction
            .query_row(
                "SELECT w.local_root, lf.relative_path, lf.materialization_state, lf.file_type, lf.scan_state, lf.size_bytes, lf.modified_at_fs, lf.state_revision, fs.baseline_version_id, fs.state_revision, fs.capability_bits, w.workspace_kind FROM local_files lf JOIN workspaces w ON w.workspace_id=lf.workspace_id LEFT JOIN file_states fs ON fs.local_file_id=lf.local_file_id WHERE lf.workspace_id=?1 AND lf.local_file_id=?2",
                params![workspace_id.as_str(), local_file_id],
                |row| {
                    let capability_bits = row.get::<_, Option<i64>>(10)?;
                    let workspace_is_cloud = row.get::<_, String>(11)? == "cloud_team";
                    Ok(ConfirmationCandidate {
                        workspace_id: workspace_id.clone(),
                        workspace_is_cloud,
                        local_file_id: local_file_id.to_owned(),
                        workspace_root: row.get(0)?,
                        relative_path: row.get(1)?,
                        materialization_state: row.get(2)?,
                        file_type: row.get(3)?,
                        scan_state: row.get(4)?,
                        size_bytes: row.get(5)?,
                        modified_at_fs: row.get(6)?,
                        local_file_revision: row.get(7)?,
                        baseline_version_id: row.get(8)?,
                        file_state_revision: row.get(9)?,
                        confirmation_allowed: !workspace_is_cloud || capability_bits.is_some_and(|bits| {
                                bits & i64::from(FileOperation::Confirm.bit()) != 0
                            }),
                    })
                },
            )
            .optional()
            .map_err(database_error("read confirmation candidate"))
    }

    fn create_file_confirmation(
        &mut self,
        command: &CreateFileConfirmationCommand,
    ) -> Result<FileConfirmationRecord, Self::Error> {
        validate_confirmation_command(command)?;
        if let Some(replayed) = read_confirmation_by_operation(
            &self.transaction,
            &command.client_operation_id,
            &command.request_fingerprint,
        )? {
            return Ok(replayed);
        }
        let candidate = self
            .confirmation_candidate(&command.workspace_id, &command.local_file_id)?
            .ok_or_else(|| confirmation_conflict("confirmation_file_not_found"))?;
        validate_confirmation_candidate(&candidate, command)?;
        let identity_valid = self
            .transaction
            .query_row(
                "SELECT identity_kind='local' AND account_status='active' FROM accounts WHERE account_id=?1",
                [command.confirmed_by_identity_id.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .map_err(database_error("validate local confirmation identity"))?;
        if identity_valid != Some(true) {
            return Err(confirmation_conflict("confirmation_identity_unavailable"));
        }
        if let Some(existing) = read_matching_active_confirmation(&self.transaction, command)? {
            return Ok(existing);
        }
        let changed = self
            .transaction
            .execute(
                "UPDATE local_files SET size_bytes=?1, modified_at_fs=?2, local_checksum_algorithm=?3, local_checksum_value=?4, state_revision=state_revision+1, updated_at=?5 WHERE workspace_id=?6 AND local_file_id=?7 AND state_revision=?8 AND size_bytes=?1 AND modified_at_fs=?2",
                params![command.verified_size_bytes, command.verified_modified_at_fs, command.expected_checksum_algorithm, command.expected_checksum_value, command.confirmed_at, command.workspace_id.as_str(), command.local_file_id, command.expected_local_file_revision],
            )
            .map_err(database_error("bind verified confirmation checksum"))?;
        if changed != 1 {
            return Err(confirmation_conflict("content_changed_during_confirmation"));
        }
        self.transaction
            .execute(
                "INSERT INTO file_confirmations(confirmation_id, workspace_id, local_file_id, confirmed_by_account_id, confirmed_checksum_algorithm, confirmed_checksum_value, baseline_version_id, target_scope, result, source_client, created_at, client_operation_id, request_fingerprint, confirmed_file_revision, workspace_kind_at_confirmation) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'content', 'confirmed', ?8, ?9, ?10, ?11, ?12, ?13)",
                params![command.confirmation_id, command.workspace_id.as_str(), command.local_file_id, command.confirmed_by_identity_id.as_str(), command.expected_checksum_algorithm, command.expected_checksum_value, command.expected_baseline_version_id, command.source_client, command.confirmed_at, command.client_operation_id, command.request_fingerprint, command.expected_local_file_revision + 1, if candidate.workspace_is_cloud { "cloud_team" } else { "local" },
                ],
            )
            .map_err(database_error("insert file confirmation"))?;
        if let Some(content) = &command.confirmed_text_content {
            self.transaction
                .execute(
                    "INSERT INTO file_confirmation_text_snapshots(confirmation_id, workspace_id, local_file_id, content_revision, text_content, content_bytes, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![command.confirmation_id, command.workspace_id.as_str(), command.local_file_id, command.expected_checksum_value, content, i64::try_from(content.len()).map_err(|_| configuration_error("confirmation text snapshot exceeds platform size"))?, command.confirmed_at],
                )
                .map_err(database_error("insert confirmation text snapshot"))?;
        }
        persist_confirmed_state(&self.transaction, &candidate, command)?;
        mark_projection_pending(
            &self.transaction,
            &command.workspace_id,
            command.confirmed_at,
        )?;
        read_confirmation(&self.transaction, &command.confirmation_id, false)
    }

    fn file_confirmation_for_operation(
        &mut self,
        client_operation_id: &str,
        request_fingerprint: &str,
    ) -> Result<Option<FileConfirmationRecord>, Self::Error> {
        read_confirmation_by_operation(&self.transaction, client_operation_id, request_fingerprint)
    }

    fn file_confirmation_history(
        &mut self,
        workspace_id: &WorkspaceId,
        local_file_id: &str,
    ) -> Result<Vec<FileConfirmationHistoryRecord>, Self::Error> {
        let belongs_to_workspace: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM local_files WHERE workspace_id=?1 AND local_file_id=?2)",
                params![workspace_id.as_str(), local_file_id],
                |row| row.get(0),
            )
            .map_err(database_error("validate confirmation history file"))?;
        if !belongs_to_workspace {
            return Err(confirmation_conflict("confirmation_file_not_found"));
        }
        let mut statement = self
            .transaction
            .prepare(
                "SELECT fc.confirmation_id, fc.confirmed_checksum_algorithm, fc.confirmed_checksum_value, fc.baseline_version_id, a.display_name, fc.created_at, fc.invalidated_at, fc.invalidation_reason FROM file_confirmations fc JOIN accounts a ON a.account_id=fc.confirmed_by_account_id WHERE fc.workspace_id=?1 AND fc.local_file_id=?2 ORDER BY fc.created_at DESC, fc.confirmation_id DESC",
            )
            .map_err(database_error("prepare confirmation history"))?;
        let rows = statement
            .query_map(params![workspace_id.as_str(), local_file_id], |row| {
                Ok(FileConfirmationHistoryRecord {
                    confirmation_id: row.get(0)?,
                    checksum_algorithm: row.get(1)?,
                    checksum_value: row.get(2)?,
                    baseline_version_id: row.get(3)?,
                    confirmed_by: row.get(4)?,
                    confirmed_at: row.get(5)?,
                    invalidated_at: row.get(6)?,
                    invalidation_reason: row.get(7)?,
                })
            })
            .map_err(database_error("query confirmation history"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read confirmation history row"))
    }

    fn invalidate_confirmations_for_identity(
        &mut self,
        command: &InvalidateIdentityConfirmationsCommand,
    ) -> Result<Vec<ConfirmationInvalidationRecord>, Self::Error> {
        validate_identity_invalidation_command(command)?;
        let status = self
            .transaction
            .query_row(
                "SELECT account_status FROM accounts WHERE account_id=?1",
                [command.identity_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read invalidated confirmation identity"))?;
        if status.as_deref() != Some("disabled") {
            return Err(confirmation_conflict("confirmation_identity_still_active"));
        }
        let confirmations =
            read_active_identity_confirmations(&self.transaction, command.identity_id.as_str())?;
        let mut invalidations = Vec::with_capacity(confirmations.len());
        let mut workspaces = HashSet::new();
        for (confirmation_id, workspace_id, local_file_id, previous_checksum) in confirmations {
            let invalidation_id = format!("{}{}", command.invalidation_id_prefix, confirmation_id);
            self.transaction
                .execute(
                    "UPDATE file_confirmations SET invalidated_at=?1, invalidation_reason='identity_changed' WHERE confirmation_id=?2 AND invalidated_at IS NULL",
                    params![command.occurred_at, confirmation_id],
                )
                .map_err(database_error("invalidate confirmation identity"))?;
            self.transaction
                .execute(
                    "INSERT INTO confirmation_invalidations(invalidation_id, confirmation_id, reason, previous_checksum, occurred_at, correlation_id) VALUES (?1, ?2, 'identity_changed', ?3, ?4, ?5)",
                    params![invalidation_id, confirmation_id, previous_checksum, command.occurred_at, command.correlation_id],
                )
                .map_err(database_error("record identity confirmation invalidation"))?;
            workspaces.insert(workspace_id.clone());
            invalidations.push(ConfirmationInvalidationRecord {
                invalidation_id,
                confirmation_id,
                workspace_id: WorkspaceId::new(workspace_id),
                local_file_id,
                reason: "identity_changed".to_owned(),
            });
        }
        for workspace_id in workspaces {
            mark_projection_pending(
                &self.transaction,
                &WorkspaceId::new(workspace_id),
                command.occurred_at,
            )?;
        }
        Ok(invalidations)
    }
}

fn validate_identity_invalidation_command(
    command: &InvalidateIdentityConfirmationsCommand,
) -> Result<(), PersistenceError> {
    if command.identity_id.as_str().is_empty()
        || command.invalidation_id_prefix.is_empty()
        || command.event_id_prefix.is_empty()
        || command.correlation_id.is_empty()
        || command.occurred_at < 0
    {
        return Err(configuration_error(
            "identity confirmation invalidation command is invalid",
        ));
    }
    Ok(())
}

fn read_active_identity_confirmations(
    transaction: &Transaction<'_>,
    identity_id: &str,
) -> Result<Vec<(String, String, String, String)>, PersistenceError> {
    let mut statement = transaction
        .prepare(
            "SELECT confirmation_id, workspace_id, local_file_id, confirmed_checksum_value FROM file_confirmations WHERE confirmed_by_account_id=?1 AND result='confirmed' AND invalidated_at IS NULL ORDER BY confirmation_id",
        )
        .map_err(database_error("prepare identity confirmations"))?;
    let rows = statement
        .query_map([identity_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(database_error("query identity confirmations"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read identity confirmation row"))
}

fn validate_confirmation_command(
    command: &CreateFileConfirmationCommand,
) -> Result<(), PersistenceError> {
    if command.confirmation_id.is_empty()
        || command.workspace_id.as_str().is_empty()
        || command.local_file_id.is_empty()
        || command.expected_local_file_revision < 0
        || command.expected_checksum_algorithm != "sha256"
        || command.expected_checksum_value.len() != 64
        || !command
            .expected_checksum_value
            .bytes()
            .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
        || command.verified_size_bytes < 0
        || command.verified_modified_at_fs < 0
        || command
            .confirmed_text_content
            .as_ref()
            .is_some_and(|content| content.len() > 4 * 1024 * 1024)
        || !matches!(command.source_client.as_str(), "desktop" | "cli")
        || command.client_operation_id.is_empty()
        || command.client_operation_id.len() > 128
        || command.request_fingerprint.len() != 64
        || command.confirmed_at < 0
    {
        return Err(configuration_error("file confirmation command is invalid"));
    }
    Ok(())
}

fn validate_confirmation_candidate(
    candidate: &ConfirmationCandidate,
    command: &CreateFileConfirmationCommand,
) -> Result<(), PersistenceError> {
    if candidate.materialization_state != "materialized"
        || candidate.file_type != "file"
        || candidate.scan_state != "current"
    {
        return Err(confirmation_conflict("confirmation_file_not_ready"));
    }
    if !candidate.confirmation_allowed {
        return Err(confirmation_conflict("confirmation_not_allowed"));
    }
    if candidate.local_file_revision != command.expected_local_file_revision
        || candidate.file_state_revision != command.expected_file_state_revision
    {
        return Err(confirmation_conflict("confirmation_state_changed"));
    }
    if candidate.baseline_version_id != command.expected_baseline_version_id {
        return Err(confirmation_conflict("confirmation_baseline_changed"));
    }
    if candidate.size_bytes != Some(command.verified_size_bytes)
        || candidate.modified_at_fs != Some(command.verified_modified_at_fs)
    {
        return Err(confirmation_conflict("content_changed_during_confirmation"));
    }
    Ok(())
}

fn read_confirmation_by_operation(
    transaction: &Transaction<'_>,
    client_operation_id: &str,
    request_fingerprint: &str,
) -> Result<Option<FileConfirmationRecord>, PersistenceError> {
    let existing = transaction
        .query_row(
            "SELECT confirmation_id, request_fingerprint FROM file_confirmations WHERE client_operation_id=?1",
            [client_operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error("read confirmation idempotency key"))?;
    let Some((confirmation_id, existing_fingerprint)) = existing else {
        return Ok(None);
    };
    if existing_fingerprint != request_fingerprint {
        return Err(confirmation_conflict("idempotency_conflict"));
    }
    read_confirmation(transaction, &confirmation_id, true).map(Some)
}

fn read_matching_active_confirmation(
    transaction: &Transaction<'_>,
    command: &CreateFileConfirmationCommand,
) -> Result<Option<FileConfirmationRecord>, PersistenceError> {
    let confirmation_id = transaction
        .query_row(
            "SELECT confirmation_id FROM file_confirmations WHERE workspace_id=?1 AND local_file_id=?2 AND confirmed_checksum_algorithm=?3 AND confirmed_checksum_value=?4 AND baseline_version_id IS ?5 AND result='confirmed' AND invalidated_at IS NULL AND workspace_kind_at_confirmation=(SELECT workspace_kind FROM workspaces WHERE workspace_id=?1)",
            params![command.workspace_id.as_str(), command.local_file_id, command.expected_checksum_algorithm, command.expected_checksum_value, command.expected_baseline_version_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error("read matching active confirmation"))?;
    confirmation_id
        .map(|confirmation_id| read_confirmation(transaction, &confirmation_id, true))
        .transpose()
}

fn read_confirmation(
    transaction: &Transaction<'_>,
    confirmation_id: &str,
    replayed: bool,
) -> Result<FileConfirmationRecord, PersistenceError> {
    transaction
        .query_row(
            "SELECT fc.confirmation_id, fc.workspace_id, fc.local_file_id, fc.confirmed_checksum_algorithm, fc.confirmed_checksum_value, fc.baseline_version_id, a.display_name, fc.created_at FROM file_confirmations fc JOIN accounts a ON a.account_id=fc.confirmed_by_account_id WHERE fc.confirmation_id=?1",
            [confirmation_id],
            |row| {
                Ok(FileConfirmationRecord {
                    confirmation_id: row.get(0)?,
                    workspace_id: WorkspaceId::new(row.get::<_, String>(1)?),
                    local_file_id: row.get(2)?,
                    checksum_algorithm: row.get(3)?,
                    checksum_value: row.get(4)?,
                    baseline_version_id: row.get(5)?,
                    confirmed_by: row.get(6)?,
                    confirmed_at: row.get(7)?,
                    replayed,
                })
            },
        )
        .map_err(database_error("read file confirmation"))
}

fn persist_confirmed_state(
    transaction: &Transaction<'_>,
    candidate: &ConfirmationCandidate,
    command: &CreateFileConfirmationCommand,
) -> Result<(), PersistenceError> {
    let confirm_bit = i64::from(FileOperation::Confirm.bit());
    if let Some(expected_revision) = candidate.file_state_revision {
        let current_reasons = transaction
            .query_row(
                "SELECT capability_reasons_json FROM file_states WHERE local_file_id=?1",
                [command.local_file_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .map_err(database_error("read confirmation capability reasons"))?;
        let reasons = confirmed_capability_reasons(&current_reasons)?;
        let changed = transaction
            .execute(
                "UPDATE file_states SET confirmation_state='confirmed', display_state='confirmed_ready', capability_bits=capability_bits & ~?1, capability_reasons_json=?2, computed_at=?3, state_revision=state_revision+1 WHERE local_file_id=?4 AND state_revision=?5 AND baseline_version_id IS ?6",
                params![confirm_bit, reasons, command.confirmed_at, command.local_file_id, expected_revision, command.expected_baseline_version_id],
            )
            .map_err(database_error("mark file state confirmed"))?;
        if changed != 1 {
            return Err(confirmation_conflict("confirmation_state_changed"));
        }
    } else {
        let capability_bits = i64::from(
            FileOperation::Preview.bit()
                | FileOperation::DiscardLocalChange.bit()
                | FileOperation::OpenExternalTool.bit(),
        );
        transaction
            .execute(
                "INSERT INTO file_states(local_file_id, baseline_version_id, baseline_relation, local_change_state, confirmation_state, submission_state, conflict_state, display_state, capability_bits, capability_reasons_json, computed_at, state_revision) VALUES (?1, ?2, 'platform_absent', 'added', 'confirmed', 'none', 'none', 'confirmed_ready', ?3, ?4, ?5, 0)",
                params![command.local_file_id, command.expected_baseline_version_id, capability_bits, local_confirmed_capability_reasons(), command.confirmed_at],
            )
            .map_err(database_error("insert confirmed local file state"))?;
    }
    Ok(())
}

fn confirmed_capability_reasons(current: &str) -> Result<String, PersistenceError> {
    let mut reasons = serde_json::from_str::<Vec<serde_json::Value>>(current)
        .map_err(|_| configuration_error("capability reasons JSON is invalid"))?;
    reasons.retain(|item| {
        item.get("operation").and_then(serde_json::Value::as_str) != Some("confirm")
    });
    reasons.push(serde_json::json!({
        "operation": "confirm",
        "reason": "confirmation_not_required"
    }));
    serde_json::to_string(&reasons)
        .map_err(|_| configuration_error("capability reasons JSON cannot be encoded"))
}

fn local_confirmed_capability_reasons() -> &'static str {
    r#"[{"operation":"pull","reason":"platform_has_no_update"},{"operation":"confirm","reason":"confirmation_not_required"},{"operation":"prepare_submission","reason":"platform_unavailable"},{"operation":"submit","reason":"platform_unavailable"},{"operation":"review","reason":"review_not_pending"},{"operation":"resolve_conflict","reason":"no_conflict"}]"#
}

/// 读取工作区文件记录。`present_only` 只取当前还在磁盘上的那些。
///
/// 增量比对只关心当前存在的记录，而历史上标为缺失的旧记录可能是它们的好几倍——
/// 本机实测 49,081 条里有 26,732 条是缺失的。把它们一起读出来，只是白白拉长
/// 持锁时间。
/// 读取工作区文件记录时的范围。
enum WorkspaceFileScope<'a> {
    All,
    PresentOnly,
    /// 只取在这次快照里首次出现的记录。分批写入首次索引时每批只需处理本批新增的
    /// 记录，按工作区全量读会让每一批都比上一批更慢。
    FirstSeenIn(&'a str),
}

fn workspace_files_filtered(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    scope: &WorkspaceFileScope<'_>,
) -> Result<Vec<WorkspaceFileRecord>, PersistenceError> {
    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
            [workspace_id.as_str()],
            |row| row.get(0),
        )
        .map_err(database_error("validate workspace file list"))?;
    if !exists {
        return Err(conflict_error("workspace_not_found"));
    }
    // 按快照取时条件要单独写成等值：写成"参数为空或相等"，查询规划就用不上
    // first_seen_snapshot_id 的索引，只能把整个工作区的记录过一遍。
    let (filter, argument) = match scope {
        WorkspaceFileScope::All => ("", None),
        WorkspaceFileScope::PresentOnly => (" AND lf.materialization_state='materialized'", None),
        WorkspaceFileScope::FirstSeenIn(snapshot_id) => {
            (" AND lf.first_seen_snapshot_id=?2", Some(*snapshot_id))
        }
    };
    let mut statement = transaction.prepare(&format!(
        "SELECT lf.local_file_id, lf.relative_path, lf.normalized_relative_path, lf.file_identity, lf.file_type, lf.size_bytes, lf.modified_at_fs, CASE lf.materialization_state WHEN 'materialized' THEN 'present' WHEN 'missing' THEN 'missing' ELSE lf.materialization_state END, fs.display_state, fs.baseline_relation, fs.local_change_state, fs.confirmation_state, fs.conflict_state, fs.state_revision, lf.local_checksum_algorithm, lf.local_checksum_value, fs.baseline_version_id FROM local_files lf LEFT JOIN file_states fs ON fs.local_file_id=lf.local_file_id WHERE lf.workspace_id=?1{filter} ORDER BY lf.normalized_relative_path, lf.local_file_id",
    )).map_err(database_error("prepare workspace files"))?;
    let bind = |row: &rusqlite::Row<'_>| -> rusqlite::Result<WorkspaceFileRecord> {
        Ok(WorkspaceFileRecord {
            local_file_id: row.get(0)?,
            relative_path: row.get(1)?,
            normalized_relative_path: row.get(2)?,
            file_identity: row.get(3)?,
            file_type: row.get(4)?,
            size_bytes: row.get(5)?,
            modified_at_fs: row.get(6)?,
            presence_state: row.get(7)?,
            display_state: row.get(8)?,
            baseline_relation: row.get(9)?,
            local_change_state: row.get(10)?,
            confirmation_state: row.get(11)?,
            conflict_state: row.get(12)?,
            state_revision: row.get(13)?,
            checksum_algorithm: row.get(14)?,
            checksum_value: row.get(15)?,
            baseline_version_id: row.get(16)?,
        })
    };
    let rows = match argument {
        Some(snapshot_id) => statement.query_map(params![workspace_id.as_str(), snapshot_id], bind),
        None => statement.query_map(params![workspace_id.as_str()], bind),
    };
    rows.map_err(database_error("query workspace files"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read workspace file"))
}

impl WorkspaceRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn insert_workspace(&mut self, workspace: &WorkspaceRecord) -> Result<(), Self::Error> {
        self.transaction
            .execute(
                "INSERT INTO workspaces(workspace_id, display_name, local_root, local_root_identity, local_status, state_revision, last_local_scan_at, created_at, updated_at, workspace_kind) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    workspace.workspace_id.as_str(),
                    workspace.display_name,
                    workspace.local_root,
                    workspace.local_root_identity,
                    workspace.local_status.as_str(),
                    workspace.state_revision,
                    workspace.last_local_scan_at,
                    workspace.created_at,
                    workspace.updated_at,
                    workspace.workspace_kind.as_str(),
                ],
            )
            .map_err(database_error("insert workspace"))?;
        Ok(())
    }

    fn initialize_workspace_export(
        &mut self,
        export: &WorkspaceExportRecord,
    ) -> Result<(), Self::Error> {
        let state = if export.desired_revision == export.exported_revision {
            "current"
        } else {
            "pending"
        };
        self.transaction
            .execute(
                "INSERT INTO workspace_exports(workspace_id, desired_revision, exported_revision, export_state, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![export.workspace_id.as_str(), export.desired_revision, export.exported_revision, state, export.updated_at],
            )
            .map_err(database_error("initialize workspace export"))?;
        Ok(())
    }

    fn workspace(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<WorkspaceRecord>, Self::Error> {
        let record = self
            .transaction
            .query_row(
                "SELECT workspace_id, display_name, local_root, local_root_identity, local_status, state_revision, last_local_scan_at, created_at, updated_at, workspace_kind FROM workspaces WHERE workspace_id=?1",
                [workspace_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error("read workspace"))?;
        record
            .map(|record| {
                let local_status =
                    LocalWorkspaceStatus::try_from(record.4.as_str()).map_err(|()| {
                        PersistenceError::new(
                            PersistenceErrorKind::Database,
                            "database contains an unknown workspace status",
                        )
                    })?;
                let workspace_kind = contexthub_domain::WorkspaceKind::try_from(record.9.as_str())
                    .map_err(|()| {
                        PersistenceError::new(
                            PersistenceErrorKind::Database,
                            "database contains an unknown workspace kind",
                        )
                    })?;
                Ok(WorkspaceRecord {
                    workspace_id: WorkspaceId::new(record.0),
                    workspace_kind,
                    display_name: record.1,
                    local_root: record.2,
                    local_root_identity: record.3,
                    local_status,
                    state_revision: record.5,
                    last_local_scan_at: record.6,
                    created_at: record.7,
                    updated_at: record.8,
                })
            })
            .transpose()
    }

    fn workspaces(&mut self) -> Result<Vec<WorkspaceRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare("SELECT workspace_id FROM workspaces ORDER BY created_at, workspace_id")
            .map_err(database_error("prepare workspace list"))?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error("read workspace list"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("collect workspace list"))?;
        drop(statement);
        ids.into_iter()
            .map(|id| {
                self.workspace(&WorkspaceId::new(id))?.ok_or_else(|| {
                    PersistenceError::new(
                        PersistenceErrorKind::Database,
                        "workspace disappeared while listing transaction",
                    )
                })
            })
            .collect()
    }

    fn workspace_files(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<WorkspaceFileRecord>, Self::Error> {
        workspace_files_filtered(&self.transaction, workspace_id, &WorkspaceFileScope::All)
    }

    fn present_workspace_files(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<WorkspaceFileRecord>, Self::Error> {
        workspace_files_filtered(
            &self.transaction,
            workspace_id,
            &WorkspaceFileScope::PresentOnly,
        )
    }

    fn workspace_files_first_seen_in(
        &mut self,
        workspace_id: &WorkspaceId,
        snapshot_id: &str,
    ) -> Result<Vec<WorkspaceFileRecord>, Self::Error> {
        workspace_files_filtered(
            &self.transaction,
            workspace_id,
            &WorkspaceFileScope::FirstSeenIn(snapshot_id),
        )
    }

    fn workspace_files_page(
        &mut self,
        workspace_id: &WorkspaceId,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<WorkspaceFilePage, Self::Error> {
        if limit == 0 {
            return Err(conflict_error("workspace_file_page_limit_invalid"));
        }
        let exists: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("validate workspace file page"))?;
        if !exists {
            return Err(conflict_error("workspace_not_found"));
        }
        let canonical_path = "lf.local_file_id=(SELECT candidate.local_file_id FROM local_files candidate WHERE candidate.workspace_id=lf.workspace_id AND candidate.normalized_relative_path=lf.normalized_relative_path AND candidate.materialization_state<>'missing' ORDER BY candidate.updated_at DESC, candidate.local_file_id DESC LIMIT 1)";
        let cursor_path = cursor
            .map(|local_file_id| {
                self.transaction
                    .query_row(
                        &format!("SELECT lf.normalized_relative_path FROM local_files lf WHERE lf.workspace_id=?1 AND lf.local_file_id=?2 AND {canonical_path}"),
                        params![workspace_id.as_str(), local_file_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(database_error("resolve workspace file page cursor"))?
                    .ok_or_else(|| conflict_error("workspace_file_page_cursor_invalid"))
            })
            .transpose()?;
        let query_limit = i64::try_from(limit.saturating_add(1))
            .map_err(|_| conflict_error("workspace_file_page_limit_invalid"))?;
        let select = "SELECT lf.local_file_id, lf.relative_path, lf.normalized_relative_path, lf.file_identity, lf.file_type, lf.size_bytes, lf.modified_at_fs, CASE lf.materialization_state WHEN 'materialized' THEN 'present' WHEN 'missing' THEN 'missing' ELSE lf.materialization_state END, fs.display_state, fs.baseline_relation, fs.local_change_state, fs.confirmation_state, fs.conflict_state, fs.state_revision, lf.local_checksum_algorithm, lf.local_checksum_value, fs.baseline_version_id FROM local_files lf LEFT JOIN file_states fs ON fs.local_file_id=lf.local_file_id";
        // The database intentionally retains earlier and missing file identities for history.
        // The current tree exposes only the newest materialized identity for each normalized path.
        let row_to_file = |row: &Row<'_>| {
            Ok(WorkspaceFileRecord {
                local_file_id: row.get(0)?,
                relative_path: row.get(1)?,
                normalized_relative_path: row.get(2)?,
                file_identity: row.get(3)?,
                file_type: row.get(4)?,
                size_bytes: row.get(5)?,
                modified_at_fs: row.get(6)?,
                presence_state: row.get(7)?,
                display_state: row.get(8)?,
                baseline_relation: row.get(9)?,
                local_change_state: row.get(10)?,
                confirmation_state: row.get(11)?,
                conflict_state: row.get(12)?,
                state_revision: row.get(13)?,
                checksum_algorithm: row.get(14)?,
                checksum_value: row.get(15)?,
                baseline_version_id: row.get(16)?,
            })
        };
        let mut files = if let (Some(cursor_path), Some(cursor)) = (cursor_path, cursor) {
            let mut statement = self
                .transaction
                .prepare(&format!("{select} WHERE lf.workspace_id=?1 AND {canonical_path} AND (lf.normalized_relative_path, lf.local_file_id) > (?2, ?3) ORDER BY lf.normalized_relative_path, lf.local_file_id LIMIT ?4"))
                .map_err(database_error("prepare workspace file page"))?;
            statement
                .query_map(
                    params![workspace_id.as_str(), cursor_path, cursor, query_limit],
                    row_to_file,
                )
                .map_err(database_error("query workspace file page"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error("read workspace file page"))?
        } else {
            let mut statement = self
                .transaction
                .prepare(&format!("{select} WHERE lf.workspace_id=?1 AND {canonical_path} ORDER BY lf.normalized_relative_path, lf.local_file_id LIMIT ?2"))
                .map_err(database_error("prepare workspace file page"))?;
            statement
                .query_map(params![workspace_id.as_str(), query_limit], row_to_file)
                .map_err(database_error("query workspace file page"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error("read workspace file page"))?
        };
        let has_more = files.len() > limit;
        files.truncate(limit);
        let next_cursor = has_more
            .then(|| files.last().map(|file| file.local_file_id.clone()))
            .flatten();
        Ok(WorkspaceFilePage {
            files,
            next_cursor,
            has_more,
        })
    }

    fn platform_binding(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<WorkspacePlatformBindingRecord>, Self::Error> {
        let record = self
            .transaction
            .query_row(
                "SELECT account_id, organization_id, platform_workspace_id, project_id, platform_root, binding_state, platform_snapshot_revision, last_platform_check_at, created_at, updated_at, team_space_id, managed_root, device_id, baseline_revision, change_cursor, permission_role, permission_revision FROM workspace_platform_bindings WHERE workspace_id=?1",
                [workspace_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, Option<String>>(6)?, row.get::<_, Option<i64>>(7)?, row.get::<_, i64>(8)?, row.get::<_, i64>(9)?, row.get::<_, Option<String>>(10)?, row.get::<_, Option<String>>(11)?, row.get::<_, Option<String>>(12)?, row.get::<_, Option<String>>(13)?, row.get::<_, Option<String>>(14)?, row.get::<_, Option<String>>(15)?, row.get::<_, Option<String>>(16)?)),
            )
            .optional()
            .map_err(database_error("read workspace platform binding"))?;
        record
            .map(|record| {
                let binding_state = WorkspacePlatformBindingState::try_from(record.5.as_str())
                    .map_err(|()| {
                        PersistenceError::new(
                            PersistenceErrorKind::Database,
                            "database contains an unknown workspace platform binding state",
                        )
                    })?;
                Ok(WorkspacePlatformBindingRecord {
                    workspace_id: workspace_id.clone(),
                    account_id: AccountId::new(record.0),
                    organization_id: record.1,
                    platform_workspace_id: record.2,
                    project_id: record.3,
                    platform_root: record.4,
                    binding_state,
                    platform_snapshot_revision: record.6,
                    last_platform_check_at: record.7,
                    team_space_id: record.10,
                    managed_root: record.11,
                    device_id: record.12,
                    baseline_revision: record.13,
                    change_cursor: record.14,
                    permission_role: record.15,
                    permission_revision: record.16,
                    created_at: record.8,
                    updated_at: record.9,
                })
            })
            .transpose()
    }

    fn update_workspace_status(
        &mut self,
        workspace_id: &WorkspaceId,
        expected_revision: i64,
        status: LocalWorkspaceStatus,
        updated_at: i64,
    ) -> Result<WorkspaceRecord, Self::Error> {
        let changed = self
            .transaction
            .execute(
                "UPDATE workspaces SET local_status=?1, state_revision=state_revision+1, updated_at=?2 WHERE workspace_id=?3 AND state_revision=?4",
                params![status.as_str(), updated_at, workspace_id.as_str(), expected_revision],
            )
            .map_err(database_error("update workspace status"))?;
        if changed != 1 {
            return Err(PersistenceError::new(
                PersistenceErrorKind::Conflict,
                "workspace state revision does not match expected revision",
            ));
        }
        self.workspace(workspace_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "updated workspace disappeared inside transaction",
            )
        })
    }
}

#[derive(Clone)]
struct CurrentLocalFile {
    local_file_id: String,
    normalized_relative_path: String,
    file_identity: Option<String>,
}

impl FileSnapshotRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn next_local_snapshot_generation(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<i64, Self::Error> {
        let exists: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("check local snapshot workspace"))?;
        if !exists {
            return Err(conflict_error("local snapshot workspace does not exist"));
        }
        self.transaction
            .query_row(
                "SELECT COALESCE(MAX(generation), 0) + 1 FROM local_scan_snapshots WHERE workspace_id=?1",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("read next local snapshot generation"))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "identity resolution, path staging, observations and the durable event form one auditable transaction boundary"
    )]
    fn commit_local_snapshot(
        &mut self,
        snapshot: &LocalScanSnapshot,
    ) -> Result<LocalSnapshotCommitResult, Self::Error> {
        validate_local_snapshot(snapshot)?;
        let workspace_exists: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [snapshot.workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("check local snapshot workspace"))?;
        if !workspace_exists {
            return Err(conflict_error("local snapshot workspace does not exist"));
        }
        let expected_generation: i64 = self
            .transaction
            .query_row(
                "SELECT COALESCE(MAX(generation), 0) + 1 FROM local_scan_snapshots WHERE workspace_id=?1",
                [snapshot.workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("allocate local snapshot generation"))?;
        if snapshot.generation != expected_generation {
            return Err(conflict_error(
                "local snapshot generation is not the next generation",
            ));
        }

        let current_files = read_current_local_files(&self.transaction, &snapshot.workspace_id)?;
        let path_to_id = current_files
            .iter()
            .map(|file| {
                (
                    file.normalized_relative_path.clone(),
                    file.local_file_id.clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        let identity_to_id = unique_identity_map(&current_files);
        let mut used_existing = HashSet::new();
        let mut assignments = Vec::with_capacity(snapshot.observations.len());
        for observation in &snapshot.observations {
            let identity_match = observation
                .file_identity
                .as_ref()
                .and_then(|identity| identity_to_id.get(identity))
                .filter(|local_file_id| !used_existing.contains(*local_file_id))
                .cloned();
            let path_match = path_to_id
                .get(&observation.normalized_relative_path)
                .filter(|local_file_id| !used_existing.contains(*local_file_id))
                .and_then(|local_file_id| {
                    let current = current_files
                        .iter()
                        .find(|file| file.local_file_id == *local_file_id)?;
                    let identity_changed = current.file_identity.is_some()
                        && observation.file_identity.is_some()
                        && current.file_identity != observation.file_identity;
                    (!identity_changed).then(|| local_file_id.clone())
                });
            let retained_id = identity_match.or(path_match);
            if let Some(local_file_id) = &retained_id {
                used_existing.insert(local_file_id.clone());
            }
            assignments.push((
                retained_id
                    .clone()
                    .unwrap_or_else(|| observation.proposed_local_file_id.clone()),
                retained_id.is_some(),
            ));
        }

        self.transaction
            .execute(
                "INSERT INTO local_scan_snapshots(snapshot_id, workspace_id, snapshot_kind, generation, observed_at, completed_at, entry_count, issue_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![snapshot.snapshot_id, snapshot.workspace_id.as_str(), snapshot.snapshot_kind.as_str(), snapshot.generation, snapshot.observed_at, snapshot.completed_at, i64::try_from(snapshot.observations.len()).map_err(|_| configuration_error("local snapshot entry count exceeds database range"))?, snapshot.issue_count],
            )
            .map_err(database_error("insert local scan snapshot"))?;

        let mut missing_ids = HashSet::new();
        if snapshot.snapshot_kind == SnapshotKind::Full {
            for current in &current_files {
                missing_ids.insert(current.local_file_id.clone());
            }
        } else {
            for removed_path in &snapshot.removed_normalized_paths {
                if let Some(local_file_id) = path_to_id.get(removed_path) {
                    missing_ids.insert(local_file_id.clone());
                }
            }
            for (observation, (assigned_id, retained)) in
                snapshot.observations.iter().zip(&assignments)
            {
                if *retained {
                    missing_ids.insert(assigned_id.clone());
                }
                if let Some(displaced_id) = path_to_id.get(&observation.normalized_relative_path)
                    && displaced_id != assigned_id
                {
                    missing_ids.insert(displaced_id.clone());
                }
            }
        }
        for local_file_id in &missing_ids {
            self.transaction
                .execute(
                    "UPDATE local_files SET materialization_state='missing', missing_since_snapshot_id=?1, observed_at=?2, updated_at=?3, state_revision=state_revision+1 WHERE local_file_id=?4 AND workspace_id=?5 AND materialization_state<>'missing'",
                    params![snapshot.snapshot_id, snapshot.observed_at, snapshot.completed_at, local_file_id, snapshot.workspace_id.as_str()],
                )
                .map_err(database_error("stage missing local file"))?;
        }

        let mut retained_ids = Vec::new();
        let mut created_ids = Vec::new();
        let mut observed_path_to_id = HashMap::new();
        for (observation, (local_file_id, retained)) in
            snapshot.observations.iter().zip(&assignments)
        {
            if *retained {
                let changed = self
                    .transaction
                    .execute(
                        "UPDATE local_files SET relative_path=?1, normalized_relative_path=?2, file_identity=?3, materialization_state='materialized', file_type=?4, size_bytes=?5, modified_at_fs=?6, observed_at=?7, local_checksum_algorithm=?8, local_checksum_value=?9, scan_state='current', last_seen_snapshot_id=?10, missing_since_snapshot_id=NULL, state_revision=state_revision+1, updated_at=?11 WHERE local_file_id=?12 AND workspace_id=?13",
                        params![observation.relative_path, observation.normalized_relative_path, observation.file_identity, observation.file_type, observation.size_bytes, observation.modified_at_fs, snapshot.observed_at, observation.checksum_algorithm, observation.checksum_value, snapshot.snapshot_id, snapshot.completed_at, local_file_id, snapshot.workspace_id.as_str()],
                    )
                    .map_err(database_error("update retained local file"))?;
                if changed != 1 {
                    return Err(conflict_error("retained local file no longer exists"));
                }
                retained_ids.push(local_file_id.clone());
                missing_ids.remove(local_file_id);
            } else {
                self.transaction
                    .execute(
                        "INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, file_identity, materialization_state, file_type, size_bytes, modified_at_fs, observed_at, local_checksum_algorithm, local_checksum_value, scan_state, first_seen_snapshot_id, last_seen_snapshot_id, state_revision, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, 'materialized', ?6, ?7, ?8, ?9, ?10, ?11, 'current', ?12, ?12, 0, ?9, ?13)",
                        params![local_file_id, snapshot.workspace_id.as_str(), observation.relative_path, observation.normalized_relative_path, observation.file_identity, observation.file_type, observation.size_bytes, observation.modified_at_fs, snapshot.observed_at, observation.checksum_algorithm, observation.checksum_value, snapshot.snapshot_id, snapshot.completed_at],
                    )
                    .map_err(database_error("insert observed local file"))?;
                created_ids.push(local_file_id.clone());
            }
            self.transaction
                .execute(
                    "INSERT INTO local_file_observations(snapshot_id, local_file_id, relative_path, normalized_relative_path, file_identity, file_type, size_bytes, modified_at_fs, checksum_algorithm, checksum_value, observed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![snapshot.snapshot_id, local_file_id, observation.relative_path, observation.normalized_relative_path, observation.file_identity, observation.file_type, observation.size_bytes, observation.modified_at_fs, observation.checksum_algorithm, observation.checksum_value, snapshot.observed_at],
                )
                .map_err(database_error("insert local file observation"))?;
            observed_path_to_id.insert(
                observation.normalized_relative_path.clone(),
                local_file_id.clone(),
            );
        }
        for (observation, (local_file_id, _)) in snapshot.observations.iter().zip(&assignments) {
            let parent_id = observation
                .normalized_relative_path
                .rsplit_once('/')
                .and_then(|(parent_path, _)| observed_path_to_id.get(parent_path));
            self.transaction
                .execute(
                    "UPDATE local_files SET parent_local_file_id=?1 WHERE local_file_id=?2",
                    params![parent_id, local_file_id],
                )
                .map_err(database_error("update local file parent"))?;
        }
        self.transaction
            .execute(
                "UPDATE workspaces SET local_status=CASE WHEN ?3 THEN 'ready' ELSE local_status END, last_local_scan_at=CASE WHEN ?3 THEN ?1 ELSE last_local_scan_at END, state_revision=state_revision+1, updated_at=?1 WHERE workspace_id=?2",
                params![snapshot.completed_at, snapshot.workspace_id.as_str(), snapshot.covers_whole_tree],
            )
            .map_err(database_error("update workspace after local snapshot"))?;
        let invalidations = invalidate_changed_local_confirmations(
            &self.transaction,
            &snapshot.workspace_id,
            &snapshot.snapshot_id,
            &snapshot.correlation_id,
            snapshot.completed_at,
        )?;
        for invalidation in invalidations {
            self.append_event(&NewDurableEvent {
                event_id: EventId::new(format!(
                    "{}:confirmation:{}",
                    snapshot.event_id.as_str(),
                    invalidation.confirmation_id
                )),
                workspace_id: Some(snapshot.workspace_id.clone()),
                event_type: "file.confirmation_invalidated".to_owned(),
                object_type: Some("file_confirmation".to_owned()),
                object_id: Some(invalidation.confirmation_id.clone()),
                payload_json: format!(
                    "{{\"local_file_id\":\"{}\",\"invalidation_id\":\"{}\",\"reason\":\"local_content_changed\"}}",
                    invalidation.local_file_id, invalidation.invalidation_id
                ),
                correlation_id: Some(snapshot.correlation_id.clone()),
                occurred_at: snapshot.completed_at,
                created_at: snapshot.completed_at,
            })?;
        }
        self.append_event(&NewDurableEvent {
            event_id: snapshot.event_id.clone(),
            workspace_id: Some(snapshot.workspace_id.clone()),
            event_type: "workspace.local_snapshot_committed".to_owned(),
            object_type: Some("local_scan_snapshot".to_owned()),
            object_id: Some(snapshot.snapshot_id.clone()),
            payload_json: format!(
                "{{\"generation\":{},\"entry_count\":{},\"missing_count\":{}}}",
                snapshot.generation,
                snapshot.observations.len(),
                missing_ids.len()
            ),
            correlation_id: Some(snapshot.correlation_id.clone()),
            occurred_at: snapshot.completed_at,
            created_at: snapshot.completed_at,
        })?;
        mark_projection_pending(
            &self.transaction,
            &snapshot.workspace_id,
            snapshot.completed_at,
        )?;
        prune_superseded_observations(
            &self.transaction,
            &snapshot.workspace_id,
            &snapshot.snapshot_id,
        )?;
        retained_ids.sort();
        created_ids.sort();
        let mut missing_local_file_ids = missing_ids.into_iter().collect::<Vec<_>>();
        missing_local_file_ids.sort();
        Ok(LocalSnapshotCommitResult {
            snapshot_id: snapshot.snapshot_id.clone(),
            retained_local_file_ids: retained_ids,
            created_local_file_ids: created_ids,
            missing_local_file_ids,
        })
    }
}

/// 每次扫描都会写一份完整的观测快照。当前的文件事实存在 `local_files` 里，快照只是
/// 提交过程中的中间产物——产品代码从来没有读过 `local_file_observations`，唯一的
/// SELECT 在测试里。
///
/// 手动扫描年代这只是慢性膨胀；接上文件监听之后它变成了急性的：实测两万多文件的工作区
/// 每次扫描写下 22,347 行，库在一小时里从 166 MB 涨到 3.3 GB。
///
/// 保留最近十份。数据模型文档的"可以清理"和"不能自动清理"两份清单里都没有这张表，
/// 所以这里不做激进删除：十份足够回看最近的扫描轨迹，同时把增长封住——真正的修复是
/// 让监听不再自我触发，这里只是给最坏情况兜底。
fn prune_superseded_observations(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    current_snapshot_id: &str,
) -> Result<(), PersistenceError> {
    transaction
        .execute(
            "DELETE FROM local_file_observations WHERE snapshot_id IN (
                 SELECT snapshot_id FROM local_scan_snapshots
                 WHERE workspace_id=?1 AND snapshot_id<>?2
                 ORDER BY generation DESC LIMIT -1 OFFSET 9
             )",
            params![workspace_id.as_str(), current_snapshot_id],
        )
        .map_err(database_error("prune superseded local file observations"))?;
    Ok(())
}

fn invalidate_changed_local_confirmations(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    snapshot_id: &str,
    correlation_id: &str,
    occurred_at: i64,
) -> Result<Vec<ConfirmationInvalidationRecord>, PersistenceError> {
    let mut statement = transaction
        .prepare(
            "SELECT fc.confirmation_id, fc.local_file_id, fc.confirmed_checksum_value, lf.local_checksum_value FROM file_confirmations fc JOIN local_files lf ON lf.local_file_id=fc.local_file_id WHERE fc.workspace_id=?1 AND fc.result='confirmed' AND fc.invalidated_at IS NULL AND (lf.materialization_state<>'materialized' OR lf.local_checksum_value IS NULL OR lf.local_checksum_value<>fc.confirmed_checksum_value) ORDER BY fc.confirmation_id",
        )
        .map_err(database_error("prepare changed local confirmations"))?;
    let rows = statement
        .query_map([workspace_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(database_error("query changed local confirmations"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read changed local confirmation row"))?;
    drop(statement);
    let mut result = Vec::with_capacity(rows.len());
    for (confirmation_id, local_file_id, previous_checksum, current_checksum) in rows {
        let invalidation_id = format!("invalidation:{snapshot_id}:{confirmation_id}");
        transaction
            .execute(
                "UPDATE file_confirmations SET invalidated_at=?1, invalidation_reason='local_content_changed' WHERE confirmation_id=?2 AND invalidated_at IS NULL",
                params![occurred_at, confirmation_id],
            )
            .map_err(database_error("invalidate changed local confirmation"))?;
        transaction
            .execute(
                "INSERT INTO confirmation_invalidations(invalidation_id, confirmation_id, reason, previous_checksum, current_checksum, occurred_at, correlation_id) VALUES (?1, ?2, 'local_content_changed', ?3, ?4, ?5, ?6)",
                params![invalidation_id, confirmation_id, previous_checksum, current_checksum, occurred_at, correlation_id],
            )
            .map_err(database_error("record changed local confirmation"))?;
        let confirm_bit = i64::from(FileOperation::Confirm.bit());
        let mutation_bits =
            i64::from(FileOperation::PrepareSubmission.bit() | FileOperation::Submit.bit());
        transaction
            .execute(
                "UPDATE file_states SET confirmation_state='invalidated_content_changed', display_state=CASE WHEN local_change_state='deleted_candidate' THEN 'local_missing' ELSE 'confirmation_required' END, capability_bits=(capability_bits | ?1) & ~?2, computed_at=?3, state_revision=state_revision+1 WHERE local_file_id=?4",
                params![confirm_bit, mutation_bits, occurred_at, local_file_id],
            )
            .map_err(database_error("update state for changed local confirmation"))?;
        result.push(ConfirmationInvalidationRecord {
            invalidation_id,
            confirmation_id,
            workspace_id: workspace_id.clone(),
            local_file_id,
            reason: "local_content_changed".to_owned(),
        });
    }
    Ok(result)
}

fn read_current_local_files(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
) -> Result<Vec<CurrentLocalFile>, PersistenceError> {
    let mut statement = transaction
        .prepare(
            "SELECT local_file_id, normalized_relative_path, file_identity FROM local_files WHERE workspace_id=?1 AND materialization_state<>'missing' ORDER BY local_file_id",
        )
        .map_err(database_error("prepare current local file query"))?;
    statement
        .query_map([workspace_id.as_str()], |row| {
            Ok(CurrentLocalFile {
                local_file_id: row.get(0)?,
                normalized_relative_path: row.get(1)?,
                file_identity: row.get(2)?,
            })
        })
        .map_err(database_error("query current local files"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read current local files"))
}

fn unique_identity_map(current_files: &[CurrentLocalFile]) -> HashMap<String, String> {
    let mut identities = HashMap::new();
    let mut ambiguous = HashSet::new();
    for file in current_files {
        if let Some(identity) = &file.file_identity
            && identities
                .insert(identity.clone(), file.local_file_id.clone())
                .is_some()
        {
            ambiguous.insert(identity.clone());
        }
    }
    identities.retain(|identity, _| !ambiguous.contains(identity));
    identities
}

fn validate_local_snapshot(snapshot: &LocalScanSnapshot) -> Result<(), PersistenceError> {
    if snapshot.snapshot_id.is_empty()
        || snapshot.workspace_id.as_str().is_empty()
        || snapshot.generation <= 0
        || snapshot.observed_at < 0
        || snapshot.completed_at < snapshot.observed_at
        || snapshot.issue_count < 0
        || snapshot.event_id.as_str().is_empty()
        || snapshot.correlation_id.is_empty()
        || (snapshot.snapshot_kind == SnapshotKind::Full && snapshot.issue_count != 0)
        || (snapshot.snapshot_kind == SnapshotKind::Full
            && !snapshot.removed_normalized_paths.is_empty())
        || (snapshot.snapshot_kind == SnapshotKind::Full && !snapshot.covers_whole_tree)
        || (snapshot.issue_count != 0 && !snapshot.removed_normalized_paths.is_empty())
    {
        return Err(configuration_error("local snapshot fields are invalid"));
    }
    let mut paths = HashSet::new();
    let mut proposed_ids = HashSet::new();
    for observation in &snapshot.observations {
        if observation.proposed_local_file_id.is_empty()
            || observation.relative_path.is_empty()
            || observation.normalized_relative_path.is_empty()
            || !paths.insert(observation.normalized_relative_path.as_str())
            || !proposed_ids.insert(observation.proposed_local_file_id.as_str())
            || !matches!(
                observation.file_type.as_str(),
                "file" | "directory" | "symlink"
            )
            || observation.size_bytes.is_some_and(|value| value < 0)
            || observation.modified_at_fs.is_some_and(|value| value < 0)
            || observation.checksum_algorithm.is_some() != observation.checksum_value.is_some()
        {
            return Err(configuration_error("local file observation is invalid"));
        }
    }
    let mut removed_paths = HashSet::new();
    for removed_path in &snapshot.removed_normalized_paths {
        if removed_path.is_empty()
            || paths.contains(removed_path.as_str())
            || !removed_paths.insert(removed_path.as_str())
        {
            return Err(configuration_error(
                "local snapshot removed path is invalid",
            ));
        }
    }
    Ok(())
}

impl FileStateRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn persist_computed_file_state(
        &mut self,
        command: &RecomputeFileStateCommand,
        output: &FileStateOutput,
    ) -> Result<PersistedFileState, Self::Error> {
        validate_file_state_command(command)?;
        let local_file_workspace = self
            .transaction
            .query_row(
                "SELECT workspace_id FROM local_files WHERE local_file_id=?1",
                [command.local_file_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read computed file workspace"))?;
        if local_file_workspace.as_deref() != Some(command.workspace_id.as_str()) {
            return Err(conflict_error(
                "computed file does not belong to the requested workspace",
            ));
        }
        let current_revision = self
            .transaction
            .query_row(
                "SELECT state_revision FROM file_states WHERE local_file_id=?1",
                [command.local_file_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(database_error("read current file state revision"))?;
        if current_revision != command.expected_state_revision {
            return Err(conflict_error(
                "file state revision does not match expected revision",
            ));
        }

        let invalidated_confirmation =
            reconcile_confirmation(&self.transaction, command, output.confirmation_state)?;
        reconcile_conflict(&self.transaction, command, output.conflict_state)?;
        let next_revision = current_revision.map_or(0, |revision| revision + 1);
        let capability_reasons_json = capability_reasons_json(output);
        match current_revision {
            None => {
                self.transaction
                    .execute(
                        "INSERT INTO file_states(local_file_id, baseline_version_id, baseline_checksum_algorithm, baseline_checksum_value, baseline_relation, local_change_state, confirmation_state, submission_state, conflict_state, display_state, capability_bits, capability_reasons_json, computed_at, state_revision) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0)",
                        params![command.local_file_id, command.input.baseline_version_id, command.input.baseline_checksum_algorithm, command.input.baseline_checksum_value, output.baseline_relation.as_str(), output.local_change_state.as_str(), output.confirmation_state.as_str(), output.submission_state.as_str(), output.conflict_state.as_str(), output.display_state.as_str(), i64::from(output.capabilities.bits), capability_reasons_json, command.computed_at],
                    )
                    .map_err(database_error("insert computed file state"))?;
            }
            Some(expected_revision) => {
                let changed = self
                    .transaction
                    .execute(
                        "UPDATE file_states SET baseline_version_id=?1, baseline_checksum_algorithm=?2, baseline_checksum_value=?3, baseline_relation=?4, local_change_state=?5, confirmation_state=?6, submission_state=?7, conflict_state=?8, display_state=?9, capability_bits=?10, capability_reasons_json=?11, computed_at=?12, state_revision=state_revision+1 WHERE local_file_id=?13 AND state_revision=?14",
                        params![command.input.baseline_version_id, command.input.baseline_checksum_algorithm, command.input.baseline_checksum_value, output.baseline_relation.as_str(), output.local_change_state.as_str(), output.confirmation_state.as_str(), output.submission_state.as_str(), output.conflict_state.as_str(), output.display_state.as_str(), i64::from(output.capabilities.bits), capability_reasons_json, command.computed_at, command.local_file_id, expected_revision],
                    )
                    .map_err(database_error("update computed file state"))?;
                if changed != 1 {
                    return Err(conflict_error(
                        "file state changed before computed state update",
                    ));
                }
            }
        }
        mark_projection_pending(
            &self.transaction,
            &command.workspace_id,
            command.computed_at,
        )?;
        Ok(PersistedFileState {
            local_file_id: command.local_file_id.clone(),
            output: output.clone(),
            state_revision: next_revision,
            invalidated_confirmation,
        })
    }
}

impl ProjectionRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn projection_snapshot(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceProjectionSnapshot, Self::Error> {
        let header = self
            .transaction
            .query_row(
                "SELECT w.local_root, w.created_at, e.desired_revision, e.exported_revision, e.manifest_checksum, e.export_state, b.project_id, b.platform_root FROM workspaces w JOIN workspace_exports e ON e.workspace_id=w.workspace_id LEFT JOIN workspace_platform_bindings b ON b.workspace_id=w.workspace_id AND b.binding_state <> 'disconnected' WHERE w.workspace_id=?1",
                [workspace_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error("read projection header"))?
            .ok_or_else(|| conflict_error("workspace projection state does not exist"))?;
        let (
            local_root,
            created_at,
            desired,
            exported,
            checksum,
            export_state,
            project_id,
            platform_root,
        ) = header;
        let environment = if platform_root.is_some() {
            "platform".to_owned()
        } else {
            "local".to_owned()
        };
        if desired < exported {
            return Err(configuration_error(
                "workspace projection revision is inconsistent",
            ));
        }
        let baseline = read_baseline_manifest(&self.transaction, workspace_id, desired)?;
        let confirmation = read_confirmation_manifest(&self.transaction, workspace_id, desired)?;
        Ok(WorkspaceProjectionSnapshot {
            workspace_id: workspace_id.clone(),
            workspace_root: local_root,
            desired_revision: desired,
            expectation: ProjectionWriteExpectation {
                exported_revision: exported,
                manifest_checksum: checksum,
                allow_incomplete_recovery: export_state == "failed",
            },
            manifests: ManifestSet {
                workspace: WorkspaceManifest {
                    schema_version: MANIFEST_SCHEMA_VERSION,
                    database_export_revision: desired,
                    workspace_id: workspace_id.as_str().to_owned(),
                    project_id,
                    platform_root,
                    local_binding_id: workspace_id.as_str().to_owned(),
                    environment,
                    created_at,
                },
                baseline,
                confirmation,
            },
        })
    }

    fn mark_projection_succeeded(
        &mut self,
        workspace_id: &WorkspaceId,
        expected_desired_revision: i64,
        manifest_checksum: &str,
        updated_at: i64,
    ) -> Result<(), Self::Error> {
        if manifest_checksum.is_empty() || expected_desired_revision < 0 || updated_at < 0 {
            return Err(configuration_error("projection success result is invalid"));
        }
        let changed = self
            .transaction
            .execute(
                "UPDATE workspace_exports SET exported_revision=desired_revision, manifest_checksum=?1, export_state='current', last_error_reason=NULL, updated_at=?2 WHERE workspace_id=?3 AND desired_revision=?4",
                params![manifest_checksum, updated_at, workspace_id.as_str(), expected_desired_revision],
            )
            .map_err(database_error("mark projection succeeded"))?;
        if changed != 1 {
            return Err(conflict_error(
                "projection desired revision changed before success was recorded",
            ));
        }
        Ok(())
    }

    fn mark_projection_failed(
        &mut self,
        workspace_id: &WorkspaceId,
        expected_desired_revision: i64,
        reason: &str,
        conflict: bool,
        updated_at: i64,
    ) -> Result<(), Self::Error> {
        if reason.is_empty() || expected_desired_revision < 0 || updated_at < 0 {
            return Err(configuration_error("projection failure result is invalid"));
        }
        let changed = self
            .transaction
            .execute(
                "UPDATE workspace_exports SET export_state=?1, last_error_reason=?2, updated_at=?3 WHERE workspace_id=?4 AND desired_revision=?5",
                params![if conflict { "conflict" } else { "failed" }, reason, updated_at, workspace_id.as_str(), expected_desired_revision],
            )
            .map_err(database_error("mark projection failed"))?;
        if changed != 1 {
            return Err(conflict_error(
                "projection desired revision changed before failure was recorded",
            ));
        }
        Ok(())
    }
}

fn read_baseline_manifest(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    revision: i64,
) -> Result<BaselineManifest, PersistenceError> {
    let mut statement = transaction
        .prepare(
            "SELECT lf.document_id, lf.relative_path, fs.baseline_version_id, fs.baseline_checksum_algorithm, fs.baseline_checksum_value, lf.updated_at FROM local_files lf JOIN file_states fs ON fs.local_file_id=lf.local_file_id WHERE lf.workspace_id=?1 AND lf.materialization_state='materialized' AND fs.baseline_version_id IS NOT NULL ORDER BY lf.normalized_relative_path, lf.local_file_id",
        )
        .map_err(database_error("prepare baseline projection"))?;
    let rows = statement
        .query_map([workspace_id.as_str()], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(database_error("query baseline projection"))?;
    let mut files = Vec::new();
    for row in rows {
        let (document_id, relative_path, version_id, algorithm, value, materialized_at) =
            row.map_err(database_error("read baseline projection row"))?;
        let (Some(document_id), Some(algorithm), Some(value)) = (document_id, algorithm, value)
        else {
            return Err(configuration_error(
                "baseline projection contains incomplete identity or checksum",
            ));
        };
        files.push(BaselineEntry {
            document_id,
            relative_path,
            baseline_version_id: version_id,
            checksum: Checksum { algorithm, value },
            materialized_at: Some(materialized_at),
        });
    }
    Ok(BaselineManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        database_export_revision: revision,
        files,
    })
}

fn read_confirmation_manifest(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    revision: i64,
) -> Result<ConfirmationManifest, PersistenceError> {
    let mut statement = transaction
        .prepare(
            "SELECT lf.relative_path, fc.document_id, fc.confirmation_id, fc.confirmed_checksum_algorithm, fc.confirmed_checksum_value, fc.baseline_version_id, fc.confirmed_by_account_id, fc.created_at FROM file_confirmations fc JOIN local_files lf ON lf.local_file_id=fc.local_file_id WHERE fc.workspace_id=?1 AND fc.result='confirmed' AND fc.invalidated_at IS NULL AND fc.workspace_kind_at_confirmation=(SELECT workspace_kind FROM workspaces WHERE workspace_id=?1) ORDER BY lf.normalized_relative_path, fc.confirmation_id",
        )
        .map_err(database_error("prepare confirmation projection"))?;
    let rows = statement
        .query_map([workspace_id.as_str()], |row| {
            Ok(ConfirmationEntry {
                relative_path: row.get(0)?,
                document_id: row.get(1)?,
                confirmation_id: row.get(2)?,
                confirmed_checksum: Checksum {
                    algorithm: row.get(3)?,
                    value: row.get(4)?,
                },
                baseline_version_id: row.get(5)?,
                confirmed_by: row.get(6)?,
                confirmed_at: row.get(7)?,
            })
        })
        .map_err(database_error("query confirmation projection"))?;
    let confirmations = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read confirmation projection row"))?;
    Ok(ConfirmationManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        database_export_revision: revision,
        confirmations,
    })
}

fn mark_projection_pending(
    transaction: &Transaction<'_>,
    workspace_id: &WorkspaceId,
    updated_at: i64,
) -> Result<(), PersistenceError> {
    if updated_at < 0 {
        return Err(configuration_error("projection pending time is invalid"));
    }
    let changed = transaction
        .execute(
            "UPDATE workspace_exports SET desired_revision=desired_revision+1, export_state=CASE WHEN export_state='conflict' THEN 'conflict' ELSE 'pending' END, last_error_reason=CASE WHEN export_state='conflict' THEN last_error_reason ELSE NULL END, updated_at=?1 WHERE workspace_id=?2",
            params![updated_at, workspace_id.as_str()],
        )
        .map_err(database_error("mark workspace projection pending"))?;
    if changed != 1 {
        return Err(conflict_error(
            "workspace projection state does not exist for changed business facts",
        ));
    }
    Ok(())
}

fn validate_file_state_command(
    command: &RecomputeFileStateCommand,
) -> Result<(), PersistenceError> {
    if command.workspace_id.as_str().is_empty()
        || command.local_file_id.is_empty()
        || command
            .expected_state_revision
            .is_some_and(|revision| revision < 0)
        || command.event_id.as_str().is_empty()
        || command
            .confirmation_invalidation_event_id
            .as_str()
            .is_empty()
        || command.correlation_id.is_empty()
        || command.computed_at < 0
    {
        return Err(configuration_error(
            "computed file state command is invalid",
        ));
    }
    Ok(())
}

fn reconcile_confirmation(
    transaction: &Transaction<'_>,
    command: &RecomputeFileStateCommand,
    confirmation_state: ConfirmationState,
) -> Result<Option<ConfirmationInvalidationRecord>, PersistenceError> {
    let reason = match confirmation_state {
        ConfirmationState::InvalidatedContentChanged => Some("local_content_changed"),
        ConfirmationState::InvalidatedBaselineChanged => Some("baseline_changed"),
        ConfirmationState::BlockedByConflict => Some("conflict_detected"),
        ConfirmationState::BlockedByPermission => Some("permission_changed"),
        _ => None,
    };
    let Some(reason) = reason else {
        return Ok(None);
    };
    let active = transaction
        .query_row(
            "SELECT confirmation_id, confirmed_checksum_value FROM file_confirmations WHERE local_file_id=?1 AND invalidated_at IS NULL AND result='confirmed'",
            [command.local_file_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error("read active confirmation for reconciliation"))?;
    let Some((confirmation_id, previous_checksum)) = active else {
        return Ok(None);
    };
    if command.confirmation_invalidation_id.is_empty() {
        return Err(configuration_error(
            "confirmation invalidation requires an identity",
        ));
    }
    transaction
        .execute(
            "UPDATE file_confirmations SET invalidated_at=?1, invalidation_reason=?2 WHERE confirmation_id=?3 AND invalidated_at IS NULL",
            params![command.computed_at, reason, confirmation_id],
        )
        .map_err(database_error("invalidate active confirmation"))?;
    transaction
        .execute(
            "INSERT INTO confirmation_invalidations(invalidation_id, confirmation_id, reason, previous_checksum, current_checksum, occurred_at, correlation_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![command.confirmation_invalidation_id, confirmation_id, reason, previous_checksum, command.input.local_checksum_value, command.computed_at, command.correlation_id],
        )
        .map_err(database_error("record confirmation invalidation"))?;
    Ok(Some(ConfirmationInvalidationRecord {
        invalidation_id: command.confirmation_invalidation_id.clone(),
        confirmation_id,
        workspace_id: command.workspace_id.clone(),
        local_file_id: command.local_file_id.clone(),
        reason: reason.to_owned(),
    }))
}

fn reconcile_conflict(
    transaction: &Transaction<'_>,
    command: &RecomputeFileStateCommand,
    conflict_state: ConflictState,
) -> Result<(), PersistenceError> {
    let active = transaction
        .query_row(
            "SELECT conflict_id, conflict_type FROM conflicts WHERE local_file_id=?1 AND state IN ('detected', 'in_progress')",
            [command.local_file_id.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error("read active conflict for reconciliation"))?;
    match (&active, command.input.active_conflict) {
        (None, Some(input)) if input.blocks_mutation() => {
            return Err(conflict_error(
                "computed state references an active conflict that does not exist",
            ));
        }
        (Some(_), None) => {
            return Err(conflict_error(
                "computed state omitted the current active conflict",
            ));
        }
        (Some((_, active_type)), Some(input))
            if input != ConflictState::Unresolved && active_type != input.as_str() =>
        {
            return Err(conflict_error(
                "computed state active conflict does not match the database",
            ));
        }
        _ => {}
    }
    if !conflict_state.blocks_mutation() {
        if let Some((conflict_id, _)) = active {
            transaction
                .execute(
                    "UPDATE conflicts SET state='superseded', resolved_at=?1, state_revision=state_revision+1 WHERE conflict_id=?2",
                    params![command.computed_at, conflict_id],
                )
                .map_err(database_error("supersede cleared conflict"))?;
        }
        return Ok(());
    }
    if conflict_state == ConflictState::Unresolved {
        return active.map_or_else(
            || {
                Err(conflict_error(
                    "unresolved state requires an active conflict",
                ))
            },
            |_| Ok(()),
        );
    }
    let conflict_type = conflict_state.as_str();
    if active
        .as_ref()
        .is_some_and(|(_, active_type)| active_type == conflict_type)
    {
        return Ok(());
    }
    if let Some((conflict_id, _)) = active {
        transaction
            .execute(
                "UPDATE conflicts SET state='superseded', resolved_at=?1, state_revision=state_revision+1 WHERE conflict_id=?2",
                params![command.computed_at, conflict_id],
            )
            .map_err(database_error("supersede changed conflict"))?;
    }
    if command.derived_conflict_id.is_empty() {
        return Err(configuration_error("derived conflict requires an identity"));
    }
    transaction
        .execute(
            "INSERT INTO conflicts(conflict_id, workspace_id, local_file_id, conflict_type, state, baseline_version_id, platform_version_id, baseline_checksum, platform_checksum, local_checksum, detected_at, state_revision) VALUES (?1, ?2, ?3, ?4, 'detected', ?5, ?6, ?7, ?8, ?9, ?10, 0)",
            params![command.derived_conflict_id, command.workspace_id.as_str(), command.local_file_id, conflict_type, command.input.baseline_version_id, command.input.latest_published_version_id, command.input.baseline_checksum_value, command.input.latest_published_checksum_value, command.input.local_checksum_value, command.computed_at],
        )
        .map_err(database_error("insert derived conflict"))?;
    Ok(())
}

fn capability_reasons_json(output: &FileStateOutput) -> String {
    let mut json = String::from("[");
    for (index, block) in output.capabilities.blocks.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        write!(
            &mut json,
            "{{\"operation\":\"{}\",\"reason\":\"{}\"}}",
            block.operation.as_str(),
            block.reason.as_str()
        )
        .expect("writing fixed capability values to String cannot fail");
    }
    json.push(']');
    json
}

impl TaskRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn insert_task(&mut self, task: &TaskRecord) -> Result<(), Self::Error> {
        validate_new_task(task)?;
        self.transaction
            .execute(
                "INSERT INTO tasks(task_id, task_type, workspace_id, state, progress_completed, progress_total, current_stage, cancellable, payload_json, result_ref_type, result_ref_id, correlation_id, created_by_client, created_at, started_at, completed_at, heartbeat_at, error_reason, error_detail_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                params![
                    task.task_id.as_str(),
                    task.task_type,
                    task.workspace_id.as_ref().map(WorkspaceId::as_str),
                    task.state.as_str(),
                    task.progress_completed,
                    task.progress_total,
                    task.current_stage,
                    task.cancellable,
                    task.payload_json,
                    task.result_ref_type,
                    task.result_ref_id,
                    task.correlation_id,
                    task.created_by_client,
                    task.created_at,
                    task.started_at,
                    task.completed_at,
                    task.heartbeat_at,
                    task.error_reason,
                    task.error_detail_json,
                ],
            )
            .map_err(database_error("insert task"))?;
        Ok(())
    }

    fn task(&mut self, task_id: &TaskId) -> Result<Option<TaskRecord>, Self::Error> {
        self.transaction
            .query_row(
                "SELECT task_id, task_type, workspace_id, state, progress_completed, progress_total, current_stage, cancellable, payload_json, result_ref_type, result_ref_id, correlation_id, created_by_client, created_at, started_at, completed_at, heartbeat_at, error_reason, error_detail_json FROM tasks WHERE task_id=?1",
                [task_id.as_str()],
                read_task,
            )
            .optional()
            .map_err(database_error("read task"))
    }

    fn nonterminal_tasks(&mut self) -> Result<Vec<TaskRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT task_id, task_type, workspace_id, state, progress_completed, progress_total, current_stage, cancellable, payload_json, result_ref_type, result_ref_id, correlation_id, created_by_client, created_at, started_at, completed_at, heartbeat_at, error_reason, error_detail_json FROM tasks WHERE state IN ('queued', 'running', 'waiting_for_user', 'interrupted', 'outcome_unknown') ORDER BY created_at, task_id",
            )
            .map_err(database_error("prepare nonterminal task recovery query"))?;
        statement
            .query_map([], read_task)
            .map_err(database_error("query nonterminal tasks"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read nonterminal tasks"))
    }

    fn start_task(
        &mut self,
        task_id: &TaskId,
        attempt_id: &TaskAttemptId,
        server_instance_id: &str,
        started_at: i64,
    ) -> Result<(TaskRecord, TaskAttemptRecord), Self::Error> {
        if task_id.as_str().is_empty()
            || attempt_id.as_str().is_empty()
            || server_instance_id.is_empty()
            || started_at < 0
        {
            return Err(configuration_error("task start fields are invalid"));
        }
        let task = self.task(task_id)?.ok_or_else(|| {
            PersistenceError::new(PersistenceErrorKind::Conflict, "task does not exist")
        })?;
        if !matches!(task.state, TaskState::Queued | TaskState::Interrupted) {
            return Err(conflict_error("task cannot start from its current state"));
        }
        let has_running_attempt: bool = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_attempts WHERE task_id=?1 AND state='running')",
                [task_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("check active task attempt"))?;
        if has_running_attempt {
            return Err(conflict_error("task already has a running attempt"));
        }
        let attempt_no: i64 = self
            .transaction
            .query_row(
                "SELECT COALESCE(MAX(attempt_no), 0) + 1 FROM task_attempts WHERE task_id=?1",
                [task_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("allocate task attempt number"))?;
        let changed = self
            .transaction
            .execute(
                "UPDATE tasks SET state='running', started_at=COALESCE(started_at, ?1), completed_at=NULL, heartbeat_at=?1, error_reason=NULL, error_detail_json=NULL WHERE task_id=?2 AND state=?3",
                params![started_at, task_id.as_str(), task.state.as_str()],
            )
            .map_err(database_error("start task"))?;
        if changed != 1 {
            return Err(conflict_error("task state changed before start"));
        }
        self.transaction
            .execute(
                "INSERT INTO task_attempts(attempt_id, task_id, attempt_no, server_instance_id, state, started_at) VALUES (?1, ?2, ?3, ?4, 'running', ?5)",
                params![attempt_id.as_str(), task_id.as_str(), attempt_no, server_instance_id, started_at],
            )
            .map_err(database_error("insert task attempt"))?;
        let started = self.task(task_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "started task disappeared inside transaction",
            )
        })?;
        Ok((
            started,
            TaskAttemptRecord {
                attempt_id: attempt_id.clone(),
                task_id: task_id.clone(),
                attempt_no,
                server_instance_id: server_instance_id.to_owned(),
                state: TaskAttemptState::Running,
                started_at,
                ended_at: None,
                error_reason: None,
            },
        ))
    }

    fn update_task_progress(&mut self, progress: &TaskProgress) -> Result<TaskRecord, Self::Error> {
        if progress.progress_completed < 0
            || progress.progress_total.is_some_and(|total| total < 0)
            || progress
                .progress_total
                .is_some_and(|total| progress.progress_completed > total)
            || progress.heartbeat_at < 0
        {
            return Err(configuration_error("task progress fields are invalid"));
        }
        let task = self.task(&progress.task_id)?.ok_or_else(|| {
            PersistenceError::new(PersistenceErrorKind::Conflict, "task does not exist")
        })?;
        if task.state != TaskState::Running {
            return Err(conflict_error(
                "task progress can only change while running",
            ));
        }
        if progress.progress_completed < task.progress_completed {
            return Err(conflict_error("task progress must not move backwards"));
        }
        let effective_total = match (task.progress_total, progress.progress_total) {
            (None, incoming) => incoming,
            (Some(current), None) => Some(current),
            (Some(current), Some(incoming)) if current == incoming => Some(current),
            (Some(_), Some(_)) => return Err(conflict_error("task progress total is immutable")),
        };
        if effective_total.is_some_and(|total| progress.progress_completed > total) {
            return Err(conflict_error("task progress exceeds its total"));
        }
        let changed = self
            .transaction
            .execute(
                "UPDATE tasks SET progress_completed=?1, progress_total=?2, current_stage=?3, heartbeat_at=?4 WHERE task_id=?5 AND state='running' AND progress_completed<=?1",
                params![progress.progress_completed, effective_total, progress.current_stage, progress.heartbeat_at, progress.task_id.as_str()],
            )
            .map_err(database_error("update task progress"))?;
        if changed != 1 {
            return Err(conflict_error(
                "task state or progress changed before update",
            ));
        }
        self.task(&progress.task_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "updated task disappeared inside transaction",
            )
        })
    }

    fn transition_task(&mut self, transition: &TaskTransition) -> Result<TaskRecord, Self::Error> {
        validate_task_transition(transition)?;
        let task = self.task(&transition.task_id)?.ok_or_else(|| {
            PersistenceError::new(PersistenceErrorKind::Conflict, "task does not exist")
        })?;
        if task.state != transition.expected_state {
            return Err(conflict_error("task state does not match expected state"));
        }
        if !task.state.can_transition_to(transition.target_state) {
            return Err(conflict_error("task state transition is not allowed"));
        }
        if transition.target_state == TaskState::Running
            && transition.expected_state == TaskState::Interrupted
        {
            return Err(conflict_error(
                "an interrupted task must create a new attempt through start_task",
            ));
        }
        if transition.target_state == TaskState::Cancelled && !task.cancellable {
            return Err(conflict_error("task is not cancellable"));
        }
        let completes = matches!(
            transition.target_state,
            TaskState::Succeeded
                | TaskState::Failed
                | TaskState::Cancelled
                | TaskState::Interrupted
                | TaskState::OutcomeUnknown
        );
        let changed = self
            .transaction
            .execute(
                "UPDATE tasks SET state=?1, completed_at=?2, heartbeat_at=?3, result_ref_type=?4, result_ref_id=?5, error_reason=?6, error_detail_json=?7 WHERE task_id=?8 AND state=?9",
                params![
                    transition.target_state.as_str(),
                    completes.then_some(transition.occurred_at),
                    transition.occurred_at,
                    transition.result_ref_type,
                    transition.result_ref_id,
                    transition.error_reason,
                    transition.error_detail_json,
                    transition.task_id.as_str(),
                    transition.expected_state.as_str(),
                ],
            )
            .map_err(database_error("transition task"))?;
        if changed != 1 {
            return Err(conflict_error("task state changed before transition"));
        }
        if completes
            && !matches!(
                transition.expected_state,
                TaskState::Queued | TaskState::OutcomeUnknown
            )
        {
            let attempt_state = match transition.target_state {
                TaskState::Succeeded => TaskAttemptState::Succeeded,
                TaskState::Failed => TaskAttemptState::Failed,
                TaskState::Cancelled | TaskState::Interrupted | TaskState::OutcomeUnknown => {
                    TaskAttemptState::Interrupted
                }
                _ => unreachable!("completing transition has a terminal attempt state"),
            };
            let attempts = self
                .transaction
                .execute(
                    "UPDATE task_attempts SET state=?1, ended_at=?2, error_reason=?3 WHERE task_id=?4 AND state='running'",
                    params![attempt_state.as_str(), transition.occurred_at, transition.error_reason, transition.task_id.as_str()],
                )
                .map_err(database_error("finish task attempt"))?;
            if attempts != 1 {
                return Err(PersistenceError::new(
                    PersistenceErrorKind::Database,
                    "running task does not have exactly one active attempt",
                ));
            }
        }
        self.task(&transition.task_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "transitioned task disappeared inside transaction",
            )
        })
    }
}

fn validate_new_task(task: &TaskRecord) -> Result<(), PersistenceError> {
    let valid_progress = task.progress_completed >= 0
        && task
            .progress_total
            .is_none_or(|total| total >= 0 && task.progress_completed <= total);
    if task.task_id.as_str().is_empty()
        || task.task_type.is_empty()
        || task.correlation_id.is_empty()
        || task.created_at < 0
        || !valid_progress
    {
        return Err(configuration_error("new task fields are invalid"));
    }
    if task.state != TaskState::Queued
        || task.started_at.is_some()
        || task.completed_at.is_some()
        || task.heartbeat_at.is_some()
        || task.result_ref_type.is_some()
        || task.result_ref_id.is_some()
        || task.error_reason.is_some()
        || task.error_detail_json.is_some()
    {
        return Err(configuration_error(
            "new task must be queued without runtime or result fields",
        ));
    }
    Ok(())
}

fn validate_task_transition(transition: &TaskTransition) -> Result<(), PersistenceError> {
    if transition.task_id.as_str().is_empty() || transition.occurred_at < 0 {
        return Err(configuration_error("task transition fields are invalid"));
    }
    if transition.result_ref_type.is_some() != transition.result_ref_id.is_some() {
        return Err(configuration_error(
            "task result reference type and id must be provided together",
        ));
    }
    if transition.target_state == TaskState::Succeeded {
        if transition.error_reason.is_some() || transition.error_detail_json.is_some() {
            return Err(configuration_error(
                "succeeded task transition must not contain an error",
            ));
        }
    } else if transition.result_ref_type.is_some() {
        return Err(configuration_error(
            "only a succeeded task transition may contain a result reference",
        ));
    }
    if matches!(
        transition.target_state,
        TaskState::Failed
            | TaskState::Cancelled
            | TaskState::Interrupted
            | TaskState::OutcomeUnknown
    ) && transition
        .error_reason
        .as_ref()
        .is_none_or(String::is_empty)
    {
        return Err(configuration_error(
            "failed, cancelled, interrupted, or outcome-unknown transition requires a reason",
        ));
    }
    if matches!(
        transition.target_state,
        TaskState::Running | TaskState::WaitingForUser
    ) && (transition.error_reason.is_some() || transition.error_detail_json.is_some())
    {
        return Err(configuration_error(
            "non-completing task transition must not contain an error",
        ));
    }
    Ok(())
}

fn read_task(row: &Row<'_>) -> rusqlite::Result<TaskRecord> {
    let raw_state = row.get::<_, String>(3)?;
    let state = TaskState::try_from(raw_state.as_str()).map_err(|()| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unknown task state",
            )),
        )
    })?;
    Ok(TaskRecord {
        task_id: TaskId::new(row.get::<_, String>(0)?),
        task_type: row.get(1)?,
        workspace_id: row.get::<_, Option<String>>(2)?.map(WorkspaceId::new),
        state,
        progress_completed: row.get(4)?,
        progress_total: row.get(5)?,
        current_stage: row.get(6)?,
        cancellable: row.get(7)?,
        payload_json: row.get(8)?,
        result_ref_type: row.get(9)?,
        result_ref_id: row.get(10)?,
        correlation_id: row.get(11)?,
        created_by_client: row.get(12)?,
        created_at: row.get(13)?,
        started_at: row.get(14)?,
        completed_at: row.get(15)?,
        heartbeat_at: row.get(16)?,
        error_reason: row.get(17)?,
        error_detail_json: row.get(18)?,
    })
}

fn configuration_error(message: &'static str) -> PersistenceError {
    PersistenceError::new(PersistenceErrorKind::Configuration, message)
}

fn conflict_error(message: &'static str) -> PersistenceError {
    PersistenceError::new(PersistenceErrorKind::Conflict, message)
}

fn confirmation_conflict(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}

impl ProjectOutcomeRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn projects_for_workspace(
        &mut self,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<ProjectRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT p.project_id, p.display_name, p.goal, p.status, p.state_revision, p.created_at, p.updated_at, primary_link.workspace_id FROM projects p JOIN project_workspaces requested_link ON requested_link.project_id=p.project_id LEFT JOIN project_workspaces primary_link ON primary_link.project_id=p.project_id AND primary_link.role='primary' WHERE requested_link.workspace_id=?1 ORDER BY p.created_at, p.project_id",
            )
            .map_err(database_error("prepare projects for workspace"))?;
        let rows = statement
            .query_map([workspace_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(database_error("query projects for workspace"))?;
        rows.map(|row| {
            let (
                project_id,
                display_name,
                goal,
                status,
                state_revision,
                created_at,
                updated_at,
                primary_workspace_id,
            ) = row.map_err(database_error("read project"))?;
            Ok(ProjectRecord {
                project_id,
                display_name,
                goal,
                status: ProjectStatus::try_from(status.as_str())
                    .map_err(|()| project_outcome_state_error("invalid_project_status"))?,
                primary_workspace_id: primary_workspace_id.map(WorkspaceId::new),
                state_revision,
                created_at,
                updated_at,
                replayed: false,
            })
        })
        .collect()
    }

    fn project_by_id(&mut self, project_id: &str) -> Result<ProjectRecord, Self::Error> {
        let exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
                [project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check project exists"))?;
        if !exists {
            return Err(configuration_error("project_not_found"));
        }
        read_project(&self.transaction, project_id, false)
    }

    fn workspaces_for_project(
        &mut self,
        project_id: &str,
    ) -> Result<Vec<WorkspaceId>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT workspace_id FROM project_workspaces WHERE project_id=?1 ORDER BY role, workspace_id",
            )
            .map_err(database_error("prepare workspaces for project"))?;
        let rows = statement
            .query_map([project_id], |row| row.get::<_, String>(0))
            .map_err(database_error("query workspaces for project"))?;
        rows.map(|row| {
            row.map(WorkspaceId::new)
                .map_err(database_error("read project workspace"))
        })
        .collect()
    }

    fn insert_project(
        &mut self,
        command: &CreateProjectCommand,
    ) -> Result<ProjectRecord, Self::Error> {
        if let Some((project_id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT project_id, request_fingerprint FROM projects WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read project idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(project_outcome_conflict("project_idempotency_conflict"));
            }
            return read_project(&self.transaction, &project_id, true);
        }
        if let Some(workspace_id) = &command.primary_workspace_id {
            let workspace_exists = self
                .transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                    [workspace_id.as_str()],
                    |row| row.get::<_, bool>(0),
                )
                .map_err(database_error("check project workspace"))?;
            if !workspace_exists {
                return Err(project_outcome_conflict("project_workspace_not_found"));
            }
        }
        self.transaction
            .execute(
                "INSERT INTO projects(project_id, display_name, goal, status, state_revision, client_operation_id, request_fingerprint, created_at, updated_at) VALUES (?1, ?2, ?3, 'active', 0, ?4, ?5, ?6, ?6)",
                params![
                    command.project_id,
                    command.display_name,
                    command.goal,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert project"))?;
        if let Some(workspace_id) = &command.primary_workspace_id {
            self.transaction
                .execute(
                    "INSERT INTO project_workspaces(project_id, workspace_id, role, created_at) VALUES (?1, ?2, 'primary', ?3)",
                    params![command.project_id, workspace_id.as_str(), command.occurred_at],
                )
                .map_err(database_error("insert project workspace"))?;
        }
        read_project(&self.transaction, &command.project_id, false)
    }

    fn update_project(
        &mut self,
        command: &UpdateProjectCommand,
    ) -> Result<ProjectRecord, Self::Error> {
        let current: Option<i64> = self
            .transaction
            .query_row(
                "SELECT state_revision FROM projects WHERE project_id=?1",
                [&command.project_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error("read project revision"))?;
        let Some(current) = current else {
            return Err(project_outcome_conflict("project_not_found"));
        };
        // The editor was showing a particular revision; refusing a stale one is what keeps a
        // second editor from overwriting the first without either of them noticing.
        if current != command.expected_revision {
            return Err(project_outcome_conflict("project_revision_conflict"));
        }
        self.transaction
            .execute(
                "UPDATE projects SET display_name=?2, goal=?3, state_revision=state_revision+1, updated_at=?4 WHERE project_id=?1",
                params![
                    command.project_id,
                    command.display_name,
                    command.goal,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("update project"))?;
        read_project(&self.transaction, &command.project_id, false)
    }

    fn attach_project_workspace(
        &mut self,
        command: &AttachProjectWorkspaceCommand,
    ) -> Result<ProjectRecord, Self::Error> {
        let project_exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
                [&command.project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check attach target project"))?;
        if !project_exists {
            return Err(project_outcome_conflict("project_not_found"));
        }
        let workspace_exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [command.workspace_id.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check attach workspace"))?;
        if !workspace_exists {
            return Err(project_outcome_conflict("project_workspace_not_found"));
        }
        let attached = self.transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_workspaces WHERE project_id=?1 AND workspace_id=?2)",
            params![command.project_id, command.workspace_id.as_str()],
            |row| row.get::<_, bool>(0),
        ).map_err(database_error("read workspace project binding"))?;
        if attached {
            return read_project(&self.transaction, &command.project_id, true);
        }
        let role = if self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM project_workspaces WHERE project_id=?1)",
                [&command.project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check project workspace count"))?
        {
            "supporting"
        } else {
            "primary"
        };
        self.transaction
            .execute(
                "INSERT INTO project_workspaces(project_id, workspace_id, role, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    command.project_id,
                    command.workspace_id.as_str(),
                    role,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert supporting project workspace"))?;
        if role == "primary" {
            self.transaction
                .execute(
                    "UPDATE my_work_items SET primary_workspace_id=?2 WHERE project_id=?1",
                    params![command.project_id, command.workspace_id.as_str()],
                )
                .map_err(database_error("refresh project work item workspace"))?;
        }
        read_project(&self.transaction, &command.project_id, false)
    }

    fn delete_workspace(
        &mut self,
        command: &DeleteWorkspaceCommand,
    ) -> Result<DeletedWorkspaceSummary, Self::Error> {
        let id = command.workspace_id.as_str();
        let exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check workspace before delete"))?;
        if !exists {
            return Err(project_outcome_conflict("workspace_not_found"));
        }
        let still_bound = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM project_workspaces WHERE workspace_id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check workspace project binding"))?;
        if still_bound {
            return Err(project_outcome_conflict("workspace_still_in_project"));
        }
        // Files here may still be the carrier of an accepted deliverable version; removing them
        // would silently damage another project's acceptance record.
        let referenced = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM deliverable_file_bindings WHERE workspace_id=?1)",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check deliverable references"))?;
        if referenced {
            return Err(project_outcome_conflict(
                "workspace_referenced_by_deliverable",
            ));
        }

        let removed_files: i64 = self
            .transaction
            .query_row(
                "SELECT COUNT(*) FROM local_files WHERE workspace_id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(database_error("count workspace files"))?;
        let removed_events: i64 = self
            .transaction
            .query_row(
                "SELECT COUNT(*) FROM durable_events WHERE workspace_id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(database_error("count workspace events"))?;

        // local_files references itself and documents, so a strict leaf-first order is fragile.
        // Deferring foreign keys moves the check to commit: a genuine violation still fails, but
        // the statement order inside the transaction no longer has to be perfect.
        self.transaction
            .execute_batch("PRAGMA defer_foreign_keys = ON")
            .map_err(database_error("defer foreign keys"))?;
        for statement in [
            "DELETE FROM sync_plan_items WHERE plan_id IN (SELECT plan_id FROM sync_plans WHERE workspace_id=?1)",
            "DELETE FROM sync_executions WHERE plan_id IN (SELECT plan_id FROM sync_plans WHERE workspace_id=?1)",
            "DELETE FROM sync_plans WHERE workspace_id=?1",
            "DELETE FROM task_attempts WHERE task_id IN (SELECT task_id FROM tasks WHERE workspace_id=?1)",
            "DELETE FROM tasks WHERE workspace_id=?1",
            "DELETE FROM checksum_cache WHERE workspace_id=?1",
            "DELETE FROM conflicts WHERE workspace_id=?1",
            "DELETE FROM file_confirmation_text_snapshots WHERE workspace_id=?1",
            "DELETE FROM file_confirmations WHERE workspace_id=?1",
            "DELETE FROM file_states WHERE local_file_id IN (SELECT local_file_id FROM local_files WHERE workspace_id=?1)",
            "DELETE FROM local_file_observations WHERE snapshot_id IN (SELECT snapshot_id FROM local_scan_snapshots WHERE workspace_id=?1)",
            "DELETE FROM document_versions WHERE document_id IN (SELECT document_id FROM documents WHERE workspace_id=?1)",
            "DELETE FROM local_files WHERE workspace_id=?1",
            "DELETE FROM documents WHERE workspace_id=?1",
            "DELETE FROM local_scan_snapshots WHERE workspace_id=?1",
            "DELETE FROM platform_snapshots WHERE workspace_id=?1",
            "DELETE FROM workspace_platform_bindings WHERE workspace_id=?1",
            "DELETE FROM workspace_exports WHERE workspace_id=?1",
            "DELETE FROM my_work_items WHERE primary_workspace_id=?1",
            "DELETE FROM durable_events WHERE workspace_id=?1",
            "DELETE FROM workspaces WHERE workspace_id=?1",
        ] {
            self.transaction
                .execute(statement, [id])
                .map_err(database_error("delete workspace graph"))?;
        }

        Ok(DeletedWorkspaceSummary {
            workspace_id: id.to_owned(),
            removed_files,
            removed_events,
        })
    }

    fn detach_project_workspace(
        &mut self,
        command: &DetachProjectWorkspaceCommand,
    ) -> Result<ProjectRecord, Self::Error> {
        let project_exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
                [&command.project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check detach target project"))?;
        if !project_exists {
            return Err(project_outcome_conflict("project_not_found"));
        }
        let binding = self
            .transaction
            .query_row(
                "SELECT role FROM project_workspaces WHERE project_id=?1 AND workspace_id=?2",
                params![command.project_id, command.workspace_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read workspace binding for detach"))?;
        let Some(role) = binding else {
            // Already unbound: treat as a replay so a retried request is not an error.
            return read_project(&self.transaction, &command.project_id, true);
        };
        self.transaction
            .execute(
                "DELETE FROM project_workspaces WHERE project_id=?1 AND workspace_id=?2",
                params![command.project_id, command.workspace_id.as_str()],
            )
            .map_err(database_error("delete project workspace binding"))?;
        if role == "primary" {
            let replacement = self
                .transaction
                .query_row(
                    "SELECT workspace_id FROM project_workspaces WHERE project_id=?1 ORDER BY created_at, workspace_id LIMIT 1",
                    [&command.project_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(database_error("read replacement project workspace"))?;
            if let Some(replacement) = &replacement {
                self.transaction
                    .execute(
                        "UPDATE project_workspaces SET role='primary' WHERE project_id=?1 AND workspace_id=?2",
                        params![command.project_id, replacement],
                    )
                    .map_err(database_error("promote replacement project workspace"))?;
            }
            self.transaction
                .execute(
                    "UPDATE my_work_items SET primary_workspace_id=?2 WHERE project_id=?1",
                    params![command.project_id, replacement],
                )
                .map_err(database_error("refresh project work item workspace"))?;
        }
        read_project(&self.transaction, &command.project_id, false)
    }

    fn delete_project(
        &mut self,
        command: &DeleteProjectCommand,
    ) -> Result<DeletedProjectSummary, Self::Error> {
        let project_exists = self
            .transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
                [&command.project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("check project before delete"))?;
        if !project_exists {
            return Err(project_outcome_conflict("project_not_found"));
        }

        let count_of = |label: &'static str, sql: &str| -> Result<i64, PersistenceError> {
            self.transaction
                .query_row(sql, [&command.project_id], |row| row.get::<_, i64>(0))
                .map_err(database_error(label))
        };
        let removed_phases = count_of(
            "count project phases",
            "SELECT COUNT(*) FROM project_phases WHERE project_id=?1",
        )?;
        let removed_tasks = count_of(
            "count project tasks",
            "SELECT COUNT(*) FROM business_tasks WHERE project_id=?1",
        )?;
        let removed_deliverables = count_of(
            "count project deliverables",
            "SELECT COUNT(*) FROM deliverables WHERE project_id=?1",
        )?;
        let removed_attention_cases = count_of(
            "count project attention cases",
            "SELECT COUNT(*) FROM attention_cases WHERE project_id=?1",
        )?;

        let mut released = self
            .transaction
            .prepare("SELECT workspace_id FROM project_workspaces WHERE project_id=?1")
            .map_err(database_error("prepare released workspaces"))?;
        let released_workspace_ids = released
            .query_map([&command.project_id], |row| row.get::<_, String>(0))
            .map_err(database_error("read released workspaces"))?
            .collect::<Result<Vec<String>, _>>()
            .map_err(database_error("collect released workspaces"))?
            .into_iter()
            .map(WorkspaceId::new)
            .collect::<Vec<WorkspaceId>>();
        drop(released);

        // Deleted leaf-first: every foreign key in this graph is ON DELETE RESTRICT, so a wrong
        // order is refused by the database rather than silently cascading.
        for statement in [
            "DELETE FROM my_work_items WHERE project_id=?1",
            "DELETE FROM attention_case_transitions WHERE attention_case_id IN (SELECT attention_case_id FROM attention_cases WHERE project_id=?1)",
            "DELETE FROM attention_cases WHERE project_id=?1",
            "DELETE FROM business_task_moves WHERE business_task_id IN (SELECT business_task_id FROM business_tasks WHERE project_id=?1)",
            "DELETE FROM business_task_transitions WHERE business_task_id IN (SELECT business_task_id FROM business_tasks WHERE project_id=?1)",
            "DELETE FROM business_tasks WHERE project_id=?1",
            "DELETE FROM phase_transitions WHERE phase_id IN (SELECT phase_id FROM project_phases WHERE project_id=?1)",
            "DELETE FROM phase_dependencies WHERE plan_id IN (SELECT plan_id FROM phase_plans WHERE project_id=?1)",
            "DELETE FROM project_phases WHERE project_id=?1",
            "DELETE FROM phase_plans WHERE project_id=?1",
            "DELETE FROM deliverable_acceptances WHERE deliverable_id IN (SELECT deliverable_id FROM deliverables WHERE project_id=?1)",
            "DELETE FROM deliverable_file_bindings WHERE version_id IN (SELECT version_id FROM deliverable_versions WHERE deliverable_id IN (SELECT deliverable_id FROM deliverables WHERE project_id=?1))",
            "DELETE FROM deliverable_version_carriers WHERE version_id IN (SELECT version_id FROM deliverable_versions WHERE deliverable_id IN (SELECT deliverable_id FROM deliverables WHERE project_id=?1))",
            "DELETE FROM deliverable_version_carriers WHERE source_project_id=?1",
            "DELETE FROM deliverable_versions WHERE deliverable_id IN (SELECT deliverable_id FROM deliverables WHERE project_id=?1)",
            "DELETE FROM deliverables WHERE project_id=?1",
            "DELETE FROM project_workspaces WHERE project_id=?1",
            "DELETE FROM projects WHERE project_id=?1",
        ] {
            self.transaction
                .execute(statement, [&command.project_id])
                .map_err(database_error("delete project graph"))?;
        }

        Ok(DeletedProjectSummary {
            project_id: command.project_id.clone(),
            released_workspace_ids,
            removed_phases,
            removed_tasks,
            removed_deliverables,
            removed_attention_cases,
        })
    }

    fn deliverables_for_project(
        &mut self,
        project_id: &str,
    ) -> Result<Vec<DeliverableRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT deliverable_id FROM deliverables WHERE project_id=?1 ORDER BY updated_at DESC, deliverable_id",
            )
            .map_err(database_error("prepare project deliverables"))?;
        let ids = statement
            .query_map([project_id], |row| row.get::<_, String>(0))
            .map_err(database_error("query project deliverables"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read project deliverable id"))?;
        ids.into_iter()
            .map(|id| read_deliverable(&self.transaction, &id, false))
            .collect()
    }

    fn insert_deliverable(
        &mut self,
        command: &CreateDeliverableCommand,
    ) -> Result<DeliverableRecord, Self::Error> {
        if let Some((deliverable_id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT deliverable_id, request_fingerprint FROM deliverables WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read deliverable idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(project_outcome_conflict("deliverable_idempotency_conflict"));
            }
            return read_deliverable(&self.transaction, &deliverable_id, true);
        }
        let project_status = self
            .transaction
            .query_row(
                "SELECT status FROM projects WHERE project_id=?1",
                [&command.project_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read deliverable project"))?
            .ok_or_else(|| project_outcome_conflict("project_not_found"))?;
        if project_status != "active" {
            return Err(project_outcome_conflict("project_not_editable"));
        }
        self.transaction
            .execute(
                "INSERT INTO deliverables(deliverable_id, project_id, title, deliverable_type, owner_label, acceptance_criteria, status, current_accepted_version_id, state_revision, client_operation_id, request_fingerprint, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'draft', NULL, 0, ?7, ?8, ?9, ?9)",
                params![
                    command.deliverable_id,
                    command.project_id,
                    command.title,
                    command.deliverable_type,
                    command.owner_label,
                    command.acceptance_criteria,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert deliverable"))?;
        self.transaction
            .execute(
                "UPDATE projects SET state_revision=state_revision+1, updated_at=?2 WHERE project_id=?1",
                params![command.project_id, command.occurred_at],
            )
            .map_err(database_error("advance project revision for deliverable"))?;
        read_deliverable(&self.transaction, &command.deliverable_id, false)
    }

    fn versions_for_deliverable(
        &mut self,
        deliverable_id: &str,
    ) -> Result<Vec<DeliverableVersionRecord>, Self::Error> {
        let mut statement = self
            .transaction
            .prepare(
                "SELECT version_id FROM deliverable_versions WHERE deliverable_id=?1 ORDER BY version_no DESC",
            )
            .map_err(database_error("prepare deliverable versions"))?;
        let ids = statement
            .query_map([deliverable_id], |row| row.get::<_, String>(0))
            .map_err(database_error("query deliverable versions"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read deliverable version id"))?;
        ids.into_iter()
            .map(|id| read_deliverable_version(&self.transaction, &id, false))
            .collect()
    }

    fn insert_deliverable_version(
        &mut self,
        command: &CreateDeliverableVersionCommand,
    ) -> Result<DeliverableVersionRecord, Self::Error> {
        validate_deliverable_version_carrier_inputs(command)?;
        if let Some((version_id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT version_id, request_fingerprint FROM deliverable_versions WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read deliverable version idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(project_outcome_conflict("deliverable_version_idempotency_conflict"));
            }
            return read_deliverable_version(&self.transaction, &version_id, true);
        }
        let project_id = self
            .transaction
            .query_row(
                "SELECT project_id FROM deliverables WHERE deliverable_id=?1",
                [&command.deliverable_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read version deliverable"))?
            .ok_or_else(|| project_outcome_conflict("deliverable_not_found"))?;
        let version_no = self
            .transaction
            .query_row(
                "SELECT COALESCE(MAX(version_no), 0) + 1 FROM deliverable_versions WHERE deliverable_id=?1",
                [&command.deliverable_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(database_error("allocate deliverable version"))?;
        self.transaction
            .execute(
                "INSERT INTO deliverable_versions(version_id, deliverable_id, version_no, status, summary, client_operation_id, request_fingerprint, created_at) VALUES (?1, ?2, ?3, 'candidate', ?4, ?5, ?6, ?7)",
                params![
                    command.version_id,
                    command.deliverable_id,
                    version_no,
                    command.summary,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert deliverable version"))?;
        insert_deliverable_file_bindings(&self.transaction, command, &project_id)?;
        insert_deliverable_version_carriers(&self.transaction, command, &project_id)?;
        self.transaction
            .execute(
                "UPDATE deliverables SET status='candidate', state_revision=state_revision+1, updated_at=?2 WHERE deliverable_id=?1",
                params![command.deliverable_id, command.occurred_at],
            )
            .map_err(database_error("advance deliverable candidate"))?;
        read_deliverable_version(&self.transaction, &command.version_id, false)
    }

    fn review_deliverable_version(
        &mut self,
        command: &ReviewDeliverableVersionCommand,
    ) -> Result<DeliverableReviewRecord, Self::Error> {
        if let Some((acceptance_id, fingerprint)) = self
            .transaction
            .query_row(
                "SELECT acceptance_id, request_fingerprint FROM deliverable_acceptances WHERE client_operation_id=?1",
                [&command.client_operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error("read deliverable review idempotency"))?
        {
            if fingerprint != command.request_fingerprint {
                return Err(project_outcome_conflict("deliverable_review_idempotency_conflict"));
            }
            return read_deliverable_review(&self.transaction, &acceptance_id, true);
        }
        let revision = self
            .transaction
            .query_row(
                "SELECT state_revision FROM deliverables WHERE deliverable_id=?1",
                [&command.deliverable_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(database_error("read review deliverable revision"))?
            .ok_or_else(|| project_outcome_conflict("deliverable_not_found"))?;
        if revision != command.expected_deliverable_revision {
            return Err(project_outcome_conflict("deliverable_revision_conflict"));
        }
        let version_status = self
            .transaction
            .query_row(
                "SELECT status FROM deliverable_versions WHERE version_id=?1 AND deliverable_id=?2",
                params![command.version_id, command.deliverable_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read reviewed version"))?
            .ok_or_else(|| project_outcome_conflict("deliverable_version_not_found"))?;
        if version_status != "candidate" {
            return Err(project_outcome_conflict(
                "deliverable_version_not_candidate",
            ));
        }
        self.transaction
            .execute(
                "INSERT INTO deliverable_acceptances(acceptance_id, deliverable_id, version_id, decision, reviewer_label, comment, client_operation_id, request_fingerprint, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    command.acceptance_id,
                    command.deliverable_id,
                    command.version_id,
                    command.decision.as_str(),
                    command.reviewer_label,
                    command.comment,
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert deliverable review"))?;
        match command.decision {
            AcceptanceDecision::Accepted => {
                self.transaction
                    .execute(
                        "UPDATE deliverable_versions SET status='superseded' WHERE deliverable_id=?1 AND status='accepted'",
                        [&command.deliverable_id],
                    )
                    .map_err(database_error("supersede accepted deliverable version"))?;
                self.transaction
                    .execute(
                        "UPDATE deliverable_versions SET status='accepted', accepted_at=?2 WHERE version_id=?1",
                        params![command.version_id, command.occurred_at],
                    )
                    .map_err(database_error("accept deliverable version"))?;
                self.transaction
                    .execute(
                        "UPDATE deliverables SET status='accepted', current_accepted_version_id=?2, state_revision=state_revision+1, updated_at=?3 WHERE deliverable_id=?1",
                        params![command.deliverable_id, command.version_id, command.occurred_at],
                    )
                    .map_err(database_error("set accepted deliverable version"))?;
            }
            AcceptanceDecision::Rejected => {
                self.transaction
                    .execute(
                        "UPDATE deliverable_versions SET status='rejected', rejected_at=?2 WHERE version_id=?1",
                        params![command.version_id, command.occurred_at],
                    )
                    .map_err(database_error("reject deliverable version"))?;
                self.transaction
                    .execute(
                        "UPDATE deliverables SET status='revision_needed', state_revision=state_revision+1, updated_at=?2 WHERE deliverable_id=?1",
                        params![command.deliverable_id, command.occurred_at],
                    )
                    .map_err(database_error("set deliverable revision needed"))?;
            }
        }
        read_deliverable_review(&self.transaction, &command.acceptance_id, false)
    }
}

fn validate_deliverable_version_carrier_inputs(
    command: &CreateDeliverableVersionCommand,
) -> Result<(), PersistenceError> {
    if command.local_file_ids.is_empty() && command.carriers.is_empty() {
        return Err(project_outcome_conflict(
            "deliverable_version_carrier_required",
        ));
    }
    let mut unique_ids = command.local_file_ids.clone();
    unique_ids.sort();
    unique_ids.dedup();
    if unique_ids.len() != command.local_file_ids.len() {
        return Err(project_outcome_conflict(
            "deliverable_version_files_duplicated",
        ));
    }
    let mut carrier_ids = HashSet::new();
    for carrier in &command.carriers {
        if !carrier_ids.insert(&carrier.carrier_id)
            || !matches!(
                carrier.carrier_type.as_str(),
                "directory" | "commit" | "external" | "project_version"
            )
            || !matches!(carrier.carrier_role.as_str(), "primary" | "supporting")
            || carrier.label.trim().is_empty()
            || carrier.reference.trim().is_empty()
            || carrier.version_label.trim().is_empty()
            || (carrier.carrier_type == "project_version") != carrier.source_version_id.is_some()
        {
            return Err(project_outcome_conflict(
                "invalid_deliverable_version_carrier",
            ));
        }
    }
    Ok(())
}

fn insert_deliverable_version_carriers(
    transaction: &Transaction<'_>,
    command: &CreateDeliverableVersionCommand,
    project_id: &str,
) -> Result<(), PersistenceError> {
    for carrier in &command.carriers {
        let (source_project_id, source_deliverable_id, source_version_id) = if carrier.carrier_type
            == "project_version"
        {
            let source = transaction
                    .query_row(
                        "SELECT d.project_id,v.deliverable_id,v.version_id
                         FROM deliverable_versions v
                         JOIN deliverables d ON d.deliverable_id=v.deliverable_id
                         WHERE v.version_id=?1 AND v.status IN ('accepted','superseded') AND d.project_id<>?2",
                        params![carrier.source_version_id, project_id],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
                    )
                    .optional()
                    .map_err(database_error("read referenced project version"))?
                    .ok_or_else(|| project_outcome_conflict("project_version_reference_invalid"))?;
            (Some(source.0), Some(source.1), Some(source.2))
        } else {
            if carrier.carrier_type == "directory" {
                let valid: bool = transaction
                        .query_row(
                            "SELECT EXISTS(
                                SELECT 1 FROM local_files file
                                JOIN project_workspaces workspace ON workspace.workspace_id=file.workspace_id
                                WHERE file.local_file_id=?1 AND workspace.project_id=?2
                                  AND file.file_type='directory' AND file.presence_state='present'
                            )",
                            params![carrier.reference, project_id],
                            |row| row.get(0),
                        )
                        .map_err(database_error("validate deliverable directory carrier"))?;
                if !valid {
                    return Err(project_outcome_conflict(
                        "deliverable_directory_not_available",
                    ));
                }
            }
            (None, None, None)
        };
        transaction
            .execute(
                "INSERT INTO deliverable_version_carriers(carrier_id,version_id,carrier_type,carrier_role,label,reference,version_label,source_project_id,source_deliverable_id,source_version_id,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    carrier.carrier_id,
                    command.version_id,
                    carrier.carrier_type,
                    carrier.carrier_role,
                    carrier.label.trim(),
                    carrier.reference.trim(),
                    carrier.version_label.trim(),
                    source_project_id,
                    source_deliverable_id,
                    source_version_id,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert deliverable version carrier"))?;
    }
    Ok(())
}

fn insert_deliverable_file_bindings(
    transaction: &Transaction<'_>,
    command: &CreateDeliverableVersionCommand,
    project_id: &str,
) -> Result<(), PersistenceError> {
    for (index, local_file_id) in command.local_file_ids.iter().enumerate() {
        let file = transaction
            .query_row(
                "SELECT lf.workspace_id, lf.relative_path, lf.state_revision, lf.local_checksum_algorithm, lf.local_checksum_value FROM local_files lf JOIN project_workspaces pw ON pw.workspace_id=lf.workspace_id WHERE lf.local_file_id=?1 AND pw.project_id=?2 AND lf.file_type='file' AND lf.materialization_state='materialized' AND lf.scan_state='current'",
                params![local_file_id, project_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error("read candidate version file"))?
            .ok_or_else(|| project_outcome_conflict("deliverable_version_file_not_available"))?;
        let checksum_algorithm = file
            .3
            .ok_or_else(|| project_outcome_conflict("deliverable_version_file_checksum_missing"))?;
        let checksum_value = file
            .4
            .ok_or_else(|| project_outcome_conflict("deliverable_version_file_checksum_missing"))?;
        transaction
            .execute(
                "INSERT INTO deliverable_file_bindings(version_id, local_file_id, workspace_id, relative_path_at_binding, file_revision_at_binding, checksum_algorithm, checksum_value, binding_role, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    command.version_id,
                    local_file_id,
                    file.0,
                    file.1,
                    file.2,
                    checksum_algorithm,
                    checksum_value,
                    if index == 0 { "primary" } else { "supporting" },
                    command.occurred_at,
                ],
            )
            .map_err(database_error("bind deliverable version file"))?;
    }
    Ok(())
}

fn read_project(
    transaction: &Transaction<'_>,
    project_id: &str,
    replayed: bool,
) -> Result<ProjectRecord, PersistenceError> {
    let row = transaction
        .query_row(
            "SELECT p.display_name, p.goal, p.status, p.state_revision, p.created_at, p.updated_at, pw.workspace_id FROM projects p LEFT JOIN project_workspaces pw ON pw.project_id=p.project_id AND pw.role='primary' WHERE p.project_id=?1",
            [project_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .map_err(database_error("read project"))?;
    Ok(ProjectRecord {
        project_id: project_id.to_owned(),
        display_name: row.0,
        goal: row.1,
        status: ProjectStatus::try_from(row.2.as_str())
            .map_err(|()| project_outcome_state_error("invalid_project_status"))?,
        state_revision: row.3,
        created_at: row.4,
        updated_at: row.5,
        primary_workspace_id: row.6.map(WorkspaceId::new),
        replayed,
    })
}

fn read_deliverable(
    transaction: &Transaction<'_>,
    deliverable_id: &str,
    replayed: bool,
) -> Result<DeliverableRecord, PersistenceError> {
    let row = transaction
        .query_row(
            "SELECT project_id, title, deliverable_type, owner_label, acceptance_criteria, status, current_accepted_version_id, state_revision, created_at, updated_at FROM deliverables WHERE deliverable_id=?1",
            [deliverable_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?, row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?, row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?, row.get::<_, i64>(9)?,
                ))
            },
        )
        .map_err(database_error("read deliverable"))?;
    Ok(DeliverableRecord {
        deliverable_id: deliverable_id.to_owned(),
        project_id: row.0,
        title: row.1,
        deliverable_type: row.2,
        owner_label: row.3,
        acceptance_criteria: row.4,
        status: DeliverableStatus::try_from(row.5.as_str())
            .map_err(|()| project_outcome_state_error("invalid_deliverable_status"))?,
        current_accepted_version_id: row.6,
        state_revision: row.7,
        created_at: row.8,
        updated_at: row.9,
        replayed,
    })
}

fn read_deliverable_version(
    transaction: &Transaction<'_>,
    version_id: &str,
    replayed: bool,
) -> Result<DeliverableVersionRecord, PersistenceError> {
    let row = transaction
        .query_row(
            "SELECT deliverable_id, version_no, status, summary, created_at, accepted_at, rejected_at FROM deliverable_versions WHERE version_id=?1",
            [version_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?, row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .map_err(database_error("read deliverable version"))?;
    let mut statement = transaction
        .prepare(
            "SELECT local_file_id, workspace_id, relative_path_at_binding, file_revision_at_binding, checksum_algorithm, checksum_value, binding_role FROM deliverable_file_bindings WHERE version_id=?1 ORDER BY CASE binding_role WHEN 'primary' THEN 0 ELSE 1 END, relative_path_at_binding, local_file_id",
        )
        .map_err(database_error("prepare deliverable bindings"))?;
    let bindings = statement
        .query_map([version_id], |binding| {
            Ok(DeliverableFileBindingRecord {
                version_id: version_id.to_owned(),
                local_file_id: binding.get(0)?,
                workspace_id: WorkspaceId::new(binding.get::<_, String>(1)?),
                relative_path: binding.get(2)?,
                file_revision: binding.get(3)?,
                checksum_algorithm: binding.get(4)?,
                checksum_value: binding.get(5)?,
                binding_role: binding.get(6)?,
            })
        })
        .map_err(database_error("query deliverable bindings"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read deliverable bindings"))?;
    let mut carrier_statement = transaction
        .prepare(
            "SELECT carrier_id,carrier_type,carrier_role,label,reference,version_label,source_project_id,source_deliverable_id,source_version_id
             FROM deliverable_version_carriers WHERE version_id=?1
             ORDER BY CASE carrier_role WHEN 'primary' THEN 0 ELSE 1 END,carrier_type,carrier_id",
        )
        .map_err(database_error("prepare deliverable carriers"))?;
    let carriers = carrier_statement
        .query_map([version_id], |carrier| {
            Ok(DeliverableCarrierRecord {
                carrier_id: carrier.get(0)?,
                version_id: version_id.to_owned(),
                carrier_type: carrier.get(1)?,
                carrier_role: carrier.get(2)?,
                label: carrier.get(3)?,
                reference: carrier.get(4)?,
                version_label: carrier.get(5)?,
                source_project_id: carrier.get(6)?,
                source_deliverable_id: carrier.get(7)?,
                source_version_id: carrier.get(8)?,
            })
        })
        .map_err(database_error("query deliverable carriers"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read deliverable carriers"))?;
    Ok(DeliverableVersionRecord {
        version_id: version_id.to_owned(),
        deliverable_id: row.0,
        version_no: row.1,
        status: DeliverableVersionStatus::try_from(row.2.as_str())
            .map_err(|()| project_outcome_state_error("invalid_deliverable_version_status"))?,
        summary: row.3,
        created_at: row.4,
        accepted_at: row.5,
        rejected_at: row.6,
        bindings,
        carriers,
        replayed,
    })
}

fn read_deliverable_review(
    transaction: &Transaction<'_>,
    acceptance_id: &str,
    replayed: bool,
) -> Result<DeliverableReviewRecord, PersistenceError> {
    let row = transaction
        .query_row(
            "SELECT deliverable_id, version_id, decision, reviewer_label, comment, created_at FROM deliverable_acceptances WHERE acceptance_id=?1",
            [acceptance_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?, row.get::<_, i64>(5)?,
                ))
            },
        )
        .map_err(database_error("read deliverable review"))?;
    Ok(DeliverableReviewRecord {
        acceptance_id: acceptance_id.to_owned(),
        deliverable: read_deliverable(transaction, &row.0, false)?,
        version: read_deliverable_version(transaction, &row.1, false)?,
        decision: AcceptanceDecision::try_from(row.2.as_str())
            .map_err(|()| project_outcome_state_error("invalid_acceptance_decision"))?,
        reviewer_label: row.3,
        comment: row.4,
        created_at: row.5,
        replayed,
    })
}

fn project_outcome_conflict(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}

fn project_outcome_state_error(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Database, reason, reason)
}

impl DurableEventRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn append_event(&mut self, event: &NewDurableEvent) -> Result<StoredDurableEvent, Self::Error> {
        let workspace_sequence = if let Some(workspace_id) = &event.workspace_id {
            Some(
                self.transaction
                    .query_row(
                        "SELECT COALESCE(MAX(workspace_sequence), 0) + 1 FROM durable_events WHERE workspace_id=?1",
                        [workspace_id.as_str()],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(database_error("allocate workspace event sequence"))?,
            )
        } else {
            None
        };
        self.transaction
            .execute(
                "INSERT INTO durable_events(event_id, workspace_id, workspace_sequence, event_type, object_type, object_id, payload_json, correlation_id, occurred_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    event.event_id.as_str(),
                    event.workspace_id.as_ref().map(WorkspaceId::as_str),
                    workspace_sequence,
                    event.event_type,
                    event.object_type,
                    event.object_id,
                    event.payload_json,
                    event.correlation_id,
                    event.occurred_at,
                    event.created_at,
                ],
            )
            .map_err(database_error("append durable event"))?;
        Ok(StoredDurableEvent {
            global_sequence: self.transaction.last_insert_rowid(),
            workspace_sequence,
        })
    }
}

impl ClientEventCursorRepository for SqliteRepositoryTransaction<'_> {
    type Error = PersistenceError;

    fn acknowledge_event_cursor(
        &mut self,
        client_identity: &str,
        account_id: &AccountId,
        sequence: i64,
        updated_at: i64,
    ) -> Result<i64, Self::Error> {
        if client_identity.is_empty() || sequence < 0 || updated_at < 0 {
            return Err(PersistenceError::new(
                PersistenceErrorKind::Configuration,
                "event cursor identity must not be empty and values must not be negative",
            ));
        }
        let latest: i64 = self
            .transaction
            .query_row(
                "SELECT COALESCE(MAX(global_sequence), 0) FROM durable_events",
                [],
                |row| row.get(0),
            )
            .map_err(database_error("read latest durable event sequence"))?;
        if sequence > latest {
            return Err(PersistenceError::new(
                PersistenceErrorKind::Conflict,
                "client event cursor is ahead of the durable event store",
            ));
        }
        self.transaction
            .execute(
                "INSERT INTO client_event_cursors(client_identity, account_id, last_ack_sequence, updated_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(client_identity, account_id) DO UPDATE SET last_ack_sequence=MAX(client_event_cursors.last_ack_sequence, excluded.last_ack_sequence), updated_at=CASE WHEN excluded.last_ack_sequence > client_event_cursors.last_ack_sequence THEN excluded.updated_at ELSE client_event_cursors.updated_at END",
                params![client_identity, account_id.as_str(), sequence, updated_at],
            )
            .map_err(database_error("acknowledge durable event cursor"))?;
        self.transaction
            .query_row(
                "SELECT last_ack_sequence FROM client_event_cursors WHERE client_identity=?1 AND account_id=?2",
                params![client_identity, account_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("read acknowledged durable event cursor"))
    }
}

impl DurableEventReader for SqlitePersistence {
    type Error = PersistenceError;

    fn replay_global_available(&self, limit: usize) -> Result<EventReplayPage, Self::Error> {
        let earliest = event_window(&self.connection, None)?.0;
        self.replay_global_after(earliest.map_or(0, |value| value.saturating_sub(1)), limit)
    }

    fn replay_global_after(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> Result<EventReplayPage, Self::Error> {
        validate_replay_request(after_sequence, limit)?;
        validate_replay_window(after_sequence, event_window(&self.connection, None)?)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT global_sequence, event_id, workspace_id, workspace_sequence, event_type, object_type, object_id, payload_json, correlation_id, occurred_at, created_at FROM durable_events WHERE global_sequence>?1 ORDER BY global_sequence LIMIT ?2",
            )
            .map_err(database_error("prepare global durable event replay"))?;
        let events = statement
            .query_map(
                params![after_sequence, replay_query_limit(limit)?],
                read_event,
            )
            .map_err(database_error("query global durable event replay"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read global durable event replay"))?;
        Ok(replay_page(events, after_sequence, limit, false))
    }

    fn replay_workspace_after(
        &self,
        workspace_id: &WorkspaceId,
        after_sequence: i64,
        limit: usize,
    ) -> Result<EventReplayPage, Self::Error> {
        validate_replay_request(after_sequence, limit)?;
        let exists: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE workspace_id=?1)",
                [workspace_id.as_str()],
                |row| row.get(0),
            )
            .map_err(database_error("validate replay workspace"))?;
        if !exists {
            return Err(PersistenceError::new(
                PersistenceErrorKind::Conflict,
                "event replay workspace does not exist",
            ));
        }
        validate_replay_window(
            after_sequence,
            event_window(&self.connection, Some(workspace_id))?,
        )?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT global_sequence, event_id, workspace_id, workspace_sequence, event_type, object_type, object_id, payload_json, correlation_id, occurred_at, created_at FROM durable_events WHERE workspace_id=?1 AND workspace_sequence>?2 ORDER BY workspace_sequence LIMIT ?3",
            )
            .map_err(database_error("prepare workspace durable event replay"))?;
        let events = statement
            .query_map(
                params![
                    workspace_id.as_str(),
                    after_sequence,
                    replay_query_limit(limit)?
                ],
                read_event,
            )
            .map_err(database_error("query workspace durable event replay"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read workspace durable event replay"))?;
        Ok(replay_page(events, after_sequence, limit, true))
    }

    fn client_cursor(
        &self,
        client_identity: &str,
        account_id: &AccountId,
    ) -> Result<Option<i64>, Self::Error> {
        if client_identity.is_empty() {
            return Err(PersistenceError::new(
                PersistenceErrorKind::Configuration,
                "event cursor identity must not be empty",
            ));
        }
        self.connection
            .query_row(
                "SELECT last_ack_sequence FROM client_event_cursors WHERE client_identity=?1 AND account_id=?2",
                params![client_identity, account_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error("read client durable event cursor"))
    }
}

fn validate_replay_request(after_sequence: i64, limit: usize) -> Result<(), PersistenceError> {
    if after_sequence < 0 || !(1..=500).contains(&limit) {
        return Err(PersistenceError::new(
            PersistenceErrorKind::Configuration,
            "event replay requires a non-negative cursor and page size from 1 through 500",
        ));
    }
    Ok(())
}

fn replay_query_limit(limit: usize) -> Result<i64, PersistenceError> {
    i64::try_from(limit + 1).map_err(|_| {
        PersistenceError::new(
            PersistenceErrorKind::Configuration,
            "event replay page size exceeds SQLite INTEGER",
        )
    })
}

fn event_window(
    connection: &Connection,
    workspace_id: Option<&WorkspaceId>,
) -> Result<(Option<i64>, Option<i64>), PersistenceError> {
    if let Some(workspace_id) = workspace_id {
        connection
            .query_row(
                "SELECT (SELECT workspace_sequence FROM durable_events WHERE workspace_id=?1 ORDER BY workspace_sequence ASC LIMIT 1), (SELECT workspace_sequence FROM durable_events WHERE workspace_id=?1 ORDER BY workspace_sequence DESC LIMIT 1)",
                [workspace_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(database_error("read workspace event replay window"))
    } else {
        connection
            .query_row(
                "SELECT (SELECT global_sequence FROM durable_events ORDER BY global_sequence ASC LIMIT 1), (SELECT global_sequence FROM durable_events ORDER BY global_sequence DESC LIMIT 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(database_error("read global event replay window"))
    }
}

fn validate_replay_window(
    after_sequence: i64,
    (earliest, latest): (Option<i64>, Option<i64>),
) -> Result<(), PersistenceError> {
    let (Some(earliest), Some(latest)) = (earliest, latest) else {
        return if after_sequence == 0 {
            Ok(())
        } else {
            Err(PersistenceError::new(
                PersistenceErrorKind::ReplayUnavailable,
                "event replay cursor is ahead of an empty durable event store",
            ))
        };
    };
    if after_sequence > latest || after_sequence < earliest.saturating_sub(1) {
        return Err(PersistenceError::new(
            PersistenceErrorKind::ReplayUnavailable,
            format!(
                "event replay cursor {after_sequence} is outside available window {}..={latest}",
                earliest.saturating_sub(1)
            ),
        ));
    }
    Ok(())
}

fn replay_page(
    mut events: Vec<DurableEventRecord>,
    after_sequence: i64,
    limit: usize,
    workspace_scope: bool,
) -> EventReplayPage {
    let has_more = events.len() > limit;
    if has_more {
        events.pop();
    }
    let next_sequence = events.last().map_or(after_sequence, |event| {
        if workspace_scope {
            event
                .workspace_sequence
                .expect("workspace replay only returns workspace events")
        } else {
            event.global_sequence
        }
    });
    EventReplayPage {
        events,
        next_sequence,
        has_more,
    }
}

fn read_event(row: &Row<'_>) -> rusqlite::Result<DurableEventRecord> {
    Ok(DurableEventRecord {
        global_sequence: row.get(0)?,
        event_id: EventId::new(row.get::<_, String>(1)?),
        workspace_id: row.get::<_, Option<String>>(2)?.map(WorkspaceId::new),
        workspace_sequence: row.get(3)?,
        event_type: row.get(4)?,
        object_type: row.get(5)?,
        object_id: row.get(6)?,
        payload_json: row.get(7)?,
        correlation_id: row.get(8)?,
        occurred_at: row.get(9)?,
        created_at: row.get(10)?,
    })
}

#[derive(Debug)]
pub struct PersistenceError {
    kind: PersistenceErrorKind,
    message: String,
    reason: Option<&'static str>,
}

impl PersistenceError {
    #[must_use]
    pub fn kind(&self) -> PersistenceErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn reason(&self) -> Option<&'static str> {
        self.reason
    }

    fn new(kind: PersistenceErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            reason: None,
        }
    }

    #[must_use]
    pub fn with_reason(
        kind: PersistenceErrorKind,
        reason: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            reason: Some(reason),
        }
    }
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PersistenceError {}

#[derive(Debug)]
pub struct SqlitePersistence {
    connection: Connection,
    database_path: PathBuf,
    file_content_index: Option<std::sync::Arc<FileContentIndex>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSkillRecord {
    pub skill_id: String,
    pub name: String,
    pub normalized_name: String,
    pub description: String,
    pub managed_relative_path: String,
    pub enabled: bool,
    pub automatic: bool,
    pub state_revision: i64,
    pub content_digest: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSkillOperationRecord {
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub operation_kind: String,
    pub skill_id: String,
    pub result_revision: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserSkillSaveCommand {
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub skill_id: String,
    pub name: String,
    pub normalized_name: String,
    pub description: String,
    pub managed_relative_path: String,
    pub enabled: bool,
    pub automatic: bool,
    pub expected_revision: Option<i64>,
    pub content_digest: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloudWorkspaceOperationRecord {
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub operation_kind: String,
    pub project_id: String,
    pub source_workspace_id: Option<String>,
    pub team_space_id: Option<String>,
    pub display_name: String,
    pub managed_root: String,
    pub device_id: String,
    pub state: String,
    pub failure_reason: Option<String>,
    pub workspace_id: Option<String>,
    pub remote_binding_id: Option<String>,
    pub expected_manifest_digest: Option<String>,
    pub expected_object_count: Option<u64>,
    pub baseline_revision: Option<String>,
    pub remote_stage: Option<String>,
    pub remote_operation_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloudWorkspaceTransferEntry {
    pub local_relative_path: String,
    pub remote_relative_path: String,
    pub entry_kind: String,
    pub size_bytes: u64,
    pub checksum_sha256: String,
    pub remote_node_id: Option<String>,
    pub upload_id: Option<String>,
    pub transfer_state: String,
    pub remote_operation_id: Option<String>,
}

impl SqlitePersistence {
    /// 打开业务写连接、应用固定运行配置并执行所有嵌入式前向迁移。
    ///
    /// # Errors
    ///
    /// 路径不安全、SQLite 配置无法生效、迁移历史不一致或迁移执行失败时返回错误。
    pub fn open(database_path: &Path, app_version: &str) -> Result<Self, PersistenceError> {
        if app_version.is_empty() {
            return Err(PersistenceError::new(
                PersistenceErrorKind::Configuration,
                "App Server version must not be empty",
            ));
        }
        validate_database_path(database_path)?;
        let connection =
            Connection::open(database_path).map_err(database_error("open database"))?;
        secure_database_file(database_path)?;
        configure_connection(&connection)?;
        let mut persistence = Self {
            connection,
            database_path: database_path.to_owned(),
            file_content_index: None,
        };
        persistence.apply_migrations(app_version)?;
        persistence.runtime_configuration()?;
        let transaction = persistence
            .connection
            .transaction()
            .map_err(database_error("begin incident recovery"))?;
        incident_recovery::reconcile(&transaction)?;
        transaction
            .commit()
            .map_err(database_error("commit incident recovery"))?;
        Ok(persistence)
    }

    /// 员工重新选中同一实际目录时，将登记路径更新到当前磁盘位置。
    ///
    /// # Errors
    /// 记录已变化或数据库写入失败时返回错误。
    pub fn relocate_local_workspace_root(
        &mut self,
        workspace: &WorkspaceRecord,
        canonical_root: &str,
        occurred_at: i64,
    ) -> Result<WorkspaceRecord, PersistenceError> {
        self.transact(|repositories| {
            let changed = repositories.transaction.execute(
                "UPDATE workspaces SET local_root=?1,state_revision=state_revision+1,updated_at=?2 WHERE workspace_id=?3 AND state_revision=?4 AND workspace_kind='local'",
                params![canonical_root, occurred_at, workspace.workspace_id.as_str(), workspace.state_revision],
            ).map_err(database_error("update local workspace root"))?;
            if changed != 1 {
                return Err(PersistenceError::with_reason(PersistenceErrorKind::Conflict,
                    "workspace_root_changed", "local workspace changed before relocation"));
            }
            repositories.workspace(&workspace.workspace_id)?
                .ok_or_else(|| configuration_error("relocated workspace missing"))
        })
    }

    /// 读取工作空间首次登记的扫描任务，重复连接复用同一任务引用。
    ///
    /// # Errors
    /// 任务缺失或数据库读取失败时返回错误。
    pub fn workspace_scan_task_id(&self, workspace_id: &str) -> Result<String, PersistenceError> {
        self.connection.query_row(
            "SELECT task_id FROM tasks WHERE workspace_id=?1 AND task_type='scan' ORDER BY created_at, task_id LIMIT 1",
            [workspace_id], |row| row.get(0),
        ).map_err(database_error("read workspace scan task"))
    }

    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// # Errors
    /// Returns a persistence error when the skill list cannot be read.
    pub fn list_user_skills(&self) -> Result<Vec<UserSkillRecord>, PersistenceError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT skill_id,name,normalized_name,description,managed_relative_path,enabled,automatic,state_revision,content_digest,created_at,updated_at
                 FROM user_skills ORDER BY updated_at DESC, skill_id ASC",
            )
            .map_err(database_error("prepare user skill list"))?;
        statement
            .query_map([], row_to_user_skill)
            .map_err(database_error("query user skill list"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read user skill list"))
    }

    /// # Errors
    /// Returns a persistence error when the skill record cannot be read.
    pub fn user_skill(&self, skill_id: &str) -> Result<Option<UserSkillRecord>, PersistenceError> {
        self.connection
            .query_row(
                "SELECT skill_id,name,normalized_name,description,managed_relative_path,enabled,automatic,state_revision,content_digest,created_at,updated_at
                 FROM user_skills WHERE skill_id=?1",
                [skill_id],
                row_to_user_skill,
            )
            .optional()
            .map_err(database_error("read user skill"))
    }

    /// # Errors
    /// Returns a persistence error when the skill record cannot be read.
    pub fn user_skill_by_normalized_name(
        &self,
        normalized_name: &str,
    ) -> Result<Option<UserSkillRecord>, PersistenceError> {
        self.connection
            .query_row(
                "SELECT skill_id,name,normalized_name,description,managed_relative_path,enabled,automatic,state_revision,content_digest,created_at,updated_at
                 FROM user_skills WHERE normalized_name=?1",
                [normalized_name],
                row_to_user_skill,
            )
            .optional()
            .map_err(database_error("read user skill by name"))
    }

    /// # Errors
    /// Returns a persistence error when the operation record cannot be read.
    pub fn user_skill_operation(
        &self,
        client_operation_id: &str,
    ) -> Result<Option<UserSkillOperationRecord>, PersistenceError> {
        self.connection
            .query_row(
                "SELECT client_operation_id,request_fingerprint,operation_kind,skill_id,result_revision
                 FROM user_skill_operations WHERE client_operation_id=?1",
                [client_operation_id],
                |row| {
                    Ok(UserSkillOperationRecord {
                        client_operation_id: row.get(0)?,
                        request_fingerprint: row.get(1)?,
                        operation_kind: row.get(2)?,
                        skill_id: row.get(3)?,
                        result_revision: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(database_error("read user skill operation"))
    }

    /// # Errors
    /// Returns a conflict for reused operation IDs or invalid revisions, or a persistence error.
    pub fn save_user_skill(
        &mut self,
        command: &UserSkillSaveCommand,
    ) -> Result<UserSkillRecord, PersistenceError> {
        if let Some(operation) = self.user_skill_operation(&command.client_operation_id)? {
            if operation.request_fingerprint != command.request_fingerprint
                || operation.operation_kind != "save"
                || operation.skill_id != command.skill_id
            {
                return Err(PersistenceError::with_reason(
                    PersistenceErrorKind::Conflict,
                    "operation_id_reused",
                    "user skill operation id was reused with different parameters",
                ));
            }
            return self.user_skill(&operation.skill_id)?.ok_or_else(|| {
                PersistenceError::with_reason(
                    PersistenceErrorKind::Conflict,
                    "skill_not_found",
                    "saved user skill no longer exists",
                )
            });
        }

        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin user skill save"))?;
        let next_revision = if let Some(expected_revision) = command.expected_revision {
            let changed = transaction
                .execute(
                    "UPDATE user_skills
                     SET name=?1,normalized_name=?2,description=?3,managed_relative_path=?4,enabled=?5,automatic=?6,state_revision=state_revision+1,content_digest=?7,updated_at=?8
                     WHERE skill_id=?9 AND state_revision=?10",
                    params![
                        command.name,
                        command.normalized_name,
                        command.description,
                        command.managed_relative_path,
                        command.enabled,
                        command.automatic,
                        command.content_digest,
                        command.occurred_at,
                        command.skill_id,
                        expected_revision,
                    ],
                )
                .map_err(database_error("update user skill"))?;
            if changed == 0 {
                return Err(PersistenceError::with_reason(
                    PersistenceErrorKind::Conflict,
                    "skill_revision_conflict",
                    "user skill revision changed",
                ));
            }
            expected_revision + 1
        } else {
            transaction
                .execute(
                    "INSERT INTO user_skills(skill_id,name,normalized_name,description,managed_relative_path,enabled,automatic,state_revision,content_digest,created_at,updated_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8,?9,?9)",
                    params![
                        command.skill_id,
                        command.name,
                        command.normalized_name,
                        command.description,
                        command.managed_relative_path,
                        command.enabled,
                        command.automatic,
                        command.content_digest,
                        command.occurred_at,
                    ],
                )
                .map_err(database_error("insert user skill"))?;
            1
        };
        transaction
            .execute(
                "INSERT INTO user_skill_operations(client_operation_id,request_fingerprint,operation_kind,skill_id,result_revision,created_at)
                 VALUES (?1,?2,'save',?3,?4,?5)",
                params![
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.skill_id,
                    next_revision,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("record user skill save"))?;
        transaction
            .commit()
            .map_err(database_error("commit user skill save"))?;
        self.user_skill(&command.skill_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "user skill disappeared after save",
            )
        })
    }

    /// # Errors
    /// Returns a conflict for reused operation IDs or invalid revisions, or a persistence error.
    #[expect(
        clippy::too_many_arguments,
        reason = "the state transition keeps its optimistic and idempotency facts explicit"
    )]
    pub fn set_user_skill_state(
        &mut self,
        skill_id: &str,
        enabled: bool,
        automatic: bool,
        expected_revision: i64,
        client_operation_id: &str,
        request_fingerprint: &str,
        occurred_at: i64,
    ) -> Result<UserSkillRecord, PersistenceError> {
        if let Some(operation) = self.user_skill_operation(client_operation_id)? {
            if operation.request_fingerprint != request_fingerprint
                || operation.operation_kind != "set_state"
                || operation.skill_id != skill_id
            {
                return Err(PersistenceError::with_reason(
                    PersistenceErrorKind::Conflict,
                    "operation_id_reused",
                    "user skill operation id was reused with different parameters",
                ));
            }
            return self.user_skill(&operation.skill_id)?.ok_or_else(|| {
                PersistenceError::with_reason(
                    PersistenceErrorKind::Conflict,
                    "skill_not_found",
                    "updated user skill no longer exists",
                )
            });
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin user skill state update"))?;
        let changed = transaction
            .execute(
                "UPDATE user_skills SET enabled=?1,automatic=?2,state_revision=state_revision+1,updated_at=?3
                 WHERE skill_id=?4 AND state_revision=?5",
                params![enabled, automatic, occurred_at, skill_id, expected_revision],
            )
            .map_err(database_error("update user skill state"))?;
        if changed == 0 {
            return Err(PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "skill_revision_conflict",
                "user skill revision changed or skill does not exist",
            ));
        }
        let next_revision = expected_revision + 1;
        transaction
            .execute(
                "INSERT INTO user_skill_operations(client_operation_id,request_fingerprint,operation_kind,skill_id,result_revision,created_at)
                 VALUES (?1,?2,'set_state',?3,?4,?5)",
                params![client_operation_id, request_fingerprint, skill_id, next_revision, occurred_at],
            )
            .map_err(database_error("record user skill state update"))?;
        transaction
            .commit()
            .map_err(database_error("commit user skill state update"))?;
        self.user_skill(skill_id)?.ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Database,
                "user skill disappeared after state update",
            )
        })
    }

    /// # Errors
    /// Returns a conflict for reused operation IDs or invalid revisions, or a persistence error.
    pub fn delete_user_skill(
        &mut self,
        skill_id: &str,
        expected_revision: i64,
        client_operation_id: &str,
        request_fingerprint: &str,
        occurred_at: i64,
    ) -> Result<bool, PersistenceError> {
        if let Some(operation) = self.user_skill_operation(client_operation_id)? {
            if operation.request_fingerprint != request_fingerprint
                || operation.operation_kind != "delete"
                || operation.skill_id != skill_id
            {
                return Err(PersistenceError::with_reason(
                    PersistenceErrorKind::Conflict,
                    "operation_id_reused",
                    "user skill operation id was reused with different parameters",
                ));
            }
            return Ok(true);
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin user skill delete"))?;
        let changed = transaction
            .execute(
                "DELETE FROM user_skills WHERE skill_id=?1 AND state_revision=?2",
                params![skill_id, expected_revision],
            )
            .map_err(database_error("delete user skill"))?;
        if changed == 0 {
            return Err(PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "skill_revision_conflict",
                "user skill revision changed or skill does not exist",
            ));
        }
        transaction
            .execute(
                "INSERT INTO user_skill_operations(client_operation_id,request_fingerprint,operation_kind,skill_id,result_revision,created_at)
                 VALUES (?1,?2,'delete',?3,NULL,?4)",
                params![client_operation_id, request_fingerprint, skill_id, occurred_at],
            )
            .map_err(database_error("record user skill delete"))?;
        transaction
            .commit()
            .map_err(database_error("commit user skill delete"))?;
        Ok(true)
    }

    /// 读取指定操作号对应的云端工作空间操作。
    ///
    /// # Errors
    ///
    /// 数据库查询失败，或持久化数值无法转换为领域类型时返回错误。
    pub fn cloud_workspace_operation(
        &self,
        client_operation_id: &str,
    ) -> Result<Option<CloudWorkspaceOperationRecord>, PersistenceError> {
        self.connection
            .query_row(
                "SELECT client_operation_id,request_fingerprint,operation_kind,project_id,source_workspace_id,team_space_id,display_name,managed_root,device_id,state,failure_reason,workspace_id,remote_binding_id,expected_manifest_digest,expected_object_count,baseline_revision,remote_stage,remote_operation_id,created_at,updated_at FROM cloud_workspace_operations WHERE client_operation_id=?1",
                [client_operation_id],
                |row| Ok(CloudWorkspaceOperationRecord {
                    client_operation_id: row.get(0)?,
                    request_fingerprint: row.get(1)?,
                    operation_kind: row.get(2)?,
                    project_id: row.get(3)?,
                    source_workspace_id: row.get(4)?,
                    team_space_id: row.get(5)?,
                    display_name: row.get(6)?,
                    managed_root: row.get(7)?,
                    device_id: row.get(8)?,
                    state: row.get(9)?,
                    failure_reason: row.get(10)?,
                    workspace_id: row.get(11)?,
                    remote_binding_id: row.get(12)?,
                    expected_manifest_digest: row.get(13)?,
                    expected_object_count: read_optional_u64(row, 14)?,
                    baseline_revision: row.get(15)?,
                    remote_stage: row.get(16)?,
                    remote_operation_id: row.get(17)?,
                    created_at: row.get(18)?,
                    updated_at: row.get(19)?,
                }),
            )
            .optional()
            .map_err(database_error("read cloud workspace operation"))
    }

    /// 读取指定工作空间最近一次云端操作。
    ///
    /// # Errors
    ///
    /// 数据库查询失败，或操作记录无法解析时返回错误。
    pub fn cloud_workspace_operation_for_workspace(
        &self,
        workspace_id: &str,
    ) -> Result<Option<CloudWorkspaceOperationRecord>, PersistenceError> {
        let operation_id = self
            .connection
            .query_row(
                "SELECT client_operation_id FROM cloud_workspace_operations
                  WHERE workspace_id=?1 ORDER BY updated_at DESC LIMIT 1",
                [workspace_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read cloud workspace operation id"))?;
        operation_id
            .map(|operation_id| self.cloud_workspace_operation(&operation_id))
            .transpose()
            .map(Option::flatten)
    }

    /// 读取项目当前尚未结束的云端工作空间操作。
    ///
    /// # Errors
    ///
    /// 数据库查询失败，或操作记录无法解析时返回错误。
    pub fn pending_cloud_workspace_operation_for_project(
        &self,
        project_id: &str,
    ) -> Result<Option<CloudWorkspaceOperationRecord>, PersistenceError> {
        let operation_id = self
            .connection
            .query_row(
                "SELECT client_operation_id FROM cloud_workspace_operations
                  WHERE project_id=?1
                    AND state IN ('prepared','creating_remote','pulling_remote','uploading_initial')
                  ORDER BY created_at LIMIT 1",
                [project_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error("read pending cloud workspace operation"))?;
        operation_id
            .map(|operation_id| self.cloud_workspace_operation(&operation_id))
            .transpose()
            .map(Option::flatten)
    }

    /// 读取下一个尚未完成的基线传输项。
    ///
    /// # Errors
    ///
    /// 数据库查询失败，或文件大小无法转换为无符号数值时返回错误。
    pub fn next_cloud_workspace_transfer_entry(
        &self,
        client_operation_id: &str,
    ) -> Result<Option<CloudWorkspaceTransferEntry>, PersistenceError> {
        self.connection
            .query_row(
                "SELECT local_relative_path,remote_relative_path,entry_kind,size_bytes,
                        checksum_sha256,remote_node_id,upload_id,transfer_state,remote_operation_id
                   FROM cloud_workspace_transfer_entries
                  WHERE client_operation_id=?1 AND transfer_state!='completed'
                  ORDER BY CASE entry_kind WHEN 'folder' THEN 0 ELSE 1 END, remote_relative_path
                  LIMIT 1",
                [client_operation_id],
                |row| {
                    Ok(CloudWorkspaceTransferEntry {
                        local_relative_path: row.get(0)?,
                        remote_relative_path: row.get(1)?,
                        entry_kind: row.get(2)?,
                        size_bytes: read_u64(row, 3)?,
                        checksum_sha256: row.get(4)?,
                        remote_node_id: row.get(5)?,
                        upload_id: row.get(6)?,
                        transfer_state: row.get(7)?,
                        remote_operation_id: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(database_error("read cloud workspace transfer entry"))
    }

    /// 统计指定云端操作的基线传输项数量。
    ///
    /// # Errors
    ///
    /// 数据库查询失败，或数量无法转换为无符号数值时返回错误。
    pub fn cloud_workspace_transfer_entry_count(
        &self,
        client_operation_id: &str,
    ) -> Result<u64, PersistenceError> {
        self.connection
            .query_row(
                "SELECT count(*) FROM cloud_workspace_transfer_entries WHERE client_operation_id=?1",
                [client_operation_id],
                |row| read_u64(row, 0),
            )
            .map_err(database_error("count cloud workspace transfer entries"))
    }

    /// 返回当前连接实际生效的 `SQLite` 运行参数。
    ///
    /// # Errors
    ///
    /// 任一 PRAGMA 无法查询或返回非预期类型时返回错误。
    pub fn runtime_configuration(&self) -> Result<SqliteRuntimeConfiguration, PersistenceError> {
        runtime_configuration(&self.connection)
    }

    /// 返回项目中可供当前对话继续绑定或重新激活的任务。
    ///
    /// # Errors
    ///
    /// 项目不存在或数据库不可读时返回错误。
    pub fn ai_work_task_candidates(
        &self,
        project_id: &str,
    ) -> Result<Vec<AiWorkTaskRecord>, PersistenceError> {
        let project_exists = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM projects WHERE project_id=?1)",
                [project_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("validate AI work project"))?;
        if !project_exists {
            return Err(PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "project_not_found",
                "AI work project does not exist",
            ));
        }
        let mut statement = self
            .connection
            .prepare(
                "SELECT business_task_id,title,objective,expected_output,completion_criteria,
                        CASE WHEN status IN ('candidate','rework') THEN 'todo' ELSE status END,
                        state_revision
                 FROM business_tasks
                 WHERE project_id=?1 AND status <> 'terminated'
                 ORDER BY updated_at DESC,business_task_id",
            )
            .map_err(database_error("prepare AI work task candidates"))?;
        statement
            .query_map([project_id], |row| {
                Ok(AiWorkTaskRecord {
                    task_id: row.get(0)?,
                    title: row.get(1)?,
                    objective: row.get(2)?,
                    expected_output: row.get(3)?,
                    completion_criteria: row.get(4)?,
                    stage: row.get(5)?,
                    state_revision: row.get(6)?,
                })
            })
            .map_err(database_error("query AI work task candidates"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read AI work task candidates"))
    }

    /// 在一个 `SQLite` 事务中同时提交 AI 回复、任务绑定和通用工作阶段。
    ///
    /// # Errors
    ///
    /// 结构化分类不合法、任务归属不匹配、修订冲突或数据库写入失败时返回错误。
    #[expect(
        clippy::too_many_lines,
        reason = "原子提交必须在同一事务边界内完成所有校验与写入"
    )]
    pub fn finalize_ai_turn(
        &mut self,
        command: &AiTurnFinalizeCommand,
    ) -> Result<AiTurnFinalizeRecord, PersistenceError> {
        validate_ai_turn_command(command)?;
        let raw_json = serde_json::to_string(&command.classification.raw).map_err(|error| {
            PersistenceError::new(
                PersistenceErrorKind::Configuration,
                format!("encode AI work classification: {error}"),
            )
        })?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("begin AI turn finalization"))?;

        if let Some((conversation_id, existing_raw, task_id, task_title, stage)) = transaction
            .query_row(
                "SELECT classification.conversation_id,classification.raw_json,classification.business_task_id,task.title,classification.stage
                 FROM conversation_work_classifications classification
                 LEFT JOIN business_tasks task ON task.business_task_id=classification.business_task_id
                 WHERE classification.run_id=?1",
                [&command.run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error("read AI turn finalization replay"))?
        {
            if conversation_id != command.conversation_id || existing_raw != raw_json {
                return Err(ai_turn_conflict("ai_turn_idempotency_conflict"));
            }
            let ledger_receipt = if let Some(request) = &command.ledger_request {
                let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM ai_ledger_commands WHERE command_id=?1)",[request["commandId"].as_str().ok_or_else(|| ai_turn_conflict("ai_ledger_request_invalid"))?],|row|row.get(0)).map_err(database_error("read finalized ledger command"))?;
                if !exists { return Err(ai_turn_conflict("ai_turn_idempotency_conflict")); }
                Some(ai_business_ledger::apply_batch(&transaction,request,command.occurred_at)?)
            } else { None };
            let state_revision = if command.ledger_request.is_some() { 0 } else { transaction
                .query_row(
                    "SELECT state_revision FROM ai_runtime_revision WHERE state_key='default'",
                    [],
                    |row| row.get(0),
                )
                .map_err(database_error("read replayed AI state revision"))? };
            transaction
                .commit()
                .map_err(database_error("commit replayed AI turn"))?;
            return Ok(AiTurnFinalizeRecord {
                ledger_receipt,
                state_revision,
                task_id,
                task_title,
                stage,
                replayed: true,
            });
        }

        let (next_revision, ledger_receipt) = if let Some(request) = &command.ledger_request {
            (
                0,
                Some(ai_business_ledger::apply_batch(
                    &transaction,
                    request,
                    command.occurred_at,
                )?),
            )
        } else {
            let active: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM ai_storage_activation WHERE storage_key='native')",
                    [],
                    |row| row.get(0),
                )
                .map_err(database_error("read AI storage mode"))?;
            if active {
                return Err(ai_turn_conflict("ai_legacy_storage_retired"));
            }
            let revision = ai_state::advance_revision(&transaction, command.expected_revision)?;
            ai_state::apply_changes(&transaction, &command.changes, command.occurred_at)?;
            (revision, None)
        };

        let mut resolved_task_id = None;
        let mut resolved_task_title = None;
        let mut resolved_stage = None;
        if command.classification.action != "unclassified" {
            let project_id = command
                .project_id
                .as_deref()
                .ok_or_else(|| ai_turn_conflict("ai_work_project_required"))?;
            let project_active = transaction
                .query_row(
                    "SELECT status='active' FROM projects WHERE project_id=?1",
                    [project_id],
                    |row| row.get::<_, bool>(0),
                )
                .optional()
                .map_err(database_error("validate classified AI project"))?
                .ok_or_else(|| ai_turn_conflict("project_not_found"))?;
            if !project_active {
                return Err(ai_turn_conflict("project_not_editable"));
            }

            if command.classification.task_action == "create" {
                let task_id = command
                    .new_task_id
                    .as_deref()
                    .ok_or_else(|| ai_turn_conflict("ai_work_new_task_id_required"))?;
                let title = required_ai_text(
                    command.classification.task_title.as_deref(),
                    "ai_work_task_title_required",
                )?;
                let objective = required_ai_text(
                    command.classification.objective.as_deref(),
                    "ai_work_task_objective_required",
                )?;
                let expected_output = required_ai_text(
                    command.classification.expected_output.as_deref(),
                    "ai_work_expected_output_required",
                )?;
                let completion_criteria = required_ai_text(
                    command.classification.completion_criteria.as_deref(),
                    "ai_work_completion_criteria_required",
                )?;
                let duplicate_task_id = {
                    let mut statement = transaction
                        .prepare(
                            "SELECT business_task_id,title,objective,expected_output,completion_criteria
                             FROM business_tasks
                             WHERE project_id=?1 AND status <> 'terminated'",
                        )
                        .map_err(database_error("prepare AI task duplicate check"))?;
                    let candidates = statement
                        .query_map([project_id], |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, String>(4)?,
                            ))
                        })
                        .map_err(database_error("query AI task duplicate check"))?;
                    let mut duplicate = None;
                    for candidate in candidates {
                        let (
                            candidate_id,
                            candidate_title,
                            candidate_objective,
                            candidate_output,
                            candidate_criteria,
                        ) = candidate.map_err(database_error("read AI task duplicate check"))?;
                        if ai_task_contract_matches(
                            (
                                &candidate_title,
                                &candidate_objective,
                                &candidate_output,
                                &candidate_criteria,
                            ),
                            (title, objective, expected_output, completion_criteria),
                        ) {
                            duplicate = Some(candidate_id);
                            break;
                        }
                    }
                    duplicate
                };
                if duplicate_task_id.is_some() {
                    return Err(ai_turn_conflict("ai_work_task_duplicate"));
                }
                let fingerprint = sha256_string(&raw_json);
                transaction
                    .execute(
                        "INSERT INTO business_tasks(
                            business_task_id,project_id,phase_id,title,objective,input_context,
                            expected_output,completion_criteria,assignee_type,assignee_label,
                            priority,due_at,status,state_revision,client_operation_id,
                            request_fingerprint,created_at,updated_at,attempt_no
                         ) VALUES (?1,?2,NULL,?3,?4,'',?5,?6,'collaboration','AI 对话协作','medium',NULL,?7,0,?8,?9,?10,?10,1)",
                        params![
                            task_id,
                            project_id,
                            title,
                            objective,
                            expected_output,
                            completion_criteria,
                            "todo",
                            format!("ai-task-create:{}", command.run_id),
                            fingerprint,
                            command.occurred_at,
                        ],
                    )
                    .map_err(database_error("create AI classified task"))?;
                resolved_task_id = Some(task_id.to_owned());
                resolved_task_title = Some(title.to_owned());
            } else {
                let task_id = command
                    .classification
                    .task_id
                    .as_deref()
                    .ok_or_else(|| ai_turn_conflict("ai_work_task_id_required"))?;
                let (task_project_id, title, current_status, attempt_no) = transaction
                    .query_row(
                        "SELECT project_id,title,status,attempt_no FROM business_tasks WHERE business_task_id=?1",
                        [task_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, i64>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(database_error("read AI classified task"))?
                    .ok_or_else(|| ai_turn_conflict("ai_work_task_not_found"))?;
                if task_project_id != project_id {
                    return Err(ai_turn_conflict("ai_work_task_project_mismatch"));
                }
                if command.classification.task_title.as_deref().map(str::trim)
                    != Some(title.as_str())
                {
                    return Err(ai_turn_conflict("ai_work_task_title_mismatch"));
                }
                if current_status == "terminated" {
                    return Err(ai_turn_conflict("ai_work_task_terminal"));
                }
                let _ = attempt_no;
                resolved_task_id = Some(task_id.to_owned());
                resolved_task_title = Some(title);
            }

            let task_id = resolved_task_id
                .as_deref()
                .ok_or_else(|| ai_turn_conflict("ai_work_task_resolution_failed"))?;
            transaction
                .execute(
                    "INSERT INTO conversation_task_bindings(
                        conversation_id,project_id,business_task_id,last_run_id,source_revision,created_at,updated_at
                     ) VALUES (?1,?2,?3,?4,0,?5,?5)
                     ON CONFLICT(conversation_id) DO UPDATE SET
                        project_id=excluded.project_id,
                        business_task_id=excluded.business_task_id,
                        last_run_id=excluded.last_run_id,
                        source_revision=conversation_task_bindings.source_revision+1,
                        updated_at=excluded.updated_at",
                    params![
                        command.conversation_id,
                        project_id,
                        task_id,
                        command.run_id,
                        command.occurred_at,
                    ],
                )
                .map_err(database_error("bind conversation to classified task"))?;
        }

        transaction
            .execute(
                "INSERT INTO conversation_work_classifications(
                    classification_id,run_id,conversation_id,project_id,business_task_id,
                    classification_action,task_action,conversation_status,stage,outcome_title,
                    outcome_summary,reason,confidence,raw_json,created_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                params![
                    format!("classification:{}", command.run_id),
                    command.run_id,
                    command.conversation_id,
                    command.project_id,
                    resolved_task_id,
                    command.classification.action,
                    command.classification.task_action,
                    command.classification.conversation_status,
                    command.classification.stage,
                    command.classification.outcome_title,
                    command.classification.outcome_summary,
                    command.classification.reason.trim(),
                    command.classification.confidence,
                    raw_json,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("record AI work classification"))?;

        reconcile_tool_failure_incident(&transaction, command, resolved_task_id.as_deref())?;

        if let Some(task_id) = resolved_task_id.as_deref() {
            resolved_stage = Some(recompute_ai_task_status(
                &transaction,
                task_id,
                &command.run_id,
                command.agent_label.trim(),
                command.classification.reason.trim(),
                command.occurred_at,
            )?);
        }

        let event_payload = serde_json::json!({
            "conversationId": command.conversation_id,
            "runId": command.run_id,
            "projectId": command.project_id,
            "taskId": resolved_task_id,
            "stage": resolved_stage,
            "conversationStatus": command.classification.conversation_status,
            "outcomeTitle": command.classification.outcome_title,
            "action": command.classification.action,
        });
        transaction
            .execute(
                "INSERT INTO durable_events(
                    event_id,workspace_id,workspace_sequence,event_type,object_type,object_id,
                    payload_json,correlation_id,occurred_at,created_at
                 ) VALUES (?1,NULL,NULL,'conversation.work_classified','conversation',?2,?3,?4,?5,?5)",
                params![
                    format!("event:ai-work:{}", command.run_id),
                    command.conversation_id,
                    event_payload.to_string(),
                    format!("correlation:ai-work:{}", command.run_id),
                    command.occurred_at,
                ],
            )
            .map_err(database_error("append AI work classification event"))?;
        incident_recovery::reconcile(&transaction)?;

        transaction
            .commit()
            .map_err(database_error("commit AI turn finalization"))?;
        Ok(AiTurnFinalizeRecord {
            ledger_receipt,
            state_revision: next_revision,
            task_id: resolved_task_id,
            task_title: resolved_task_title,
            stage: resolved_stage,
            replayed: false,
        })
    }

    fn apply_migrations(&mut self, app_version: &str) -> Result<(), PersistenceError> {
        let table_exists = schema_migrations_exists(&self.connection)?;
        if !table_exists && user_schema_object_count(&self.connection)? != 0 {
            return Err(PersistenceError::new(
                PersistenceErrorKind::Migration,
                "database contains schema objects without a migration history",
            ));
        }
        let applied = if table_exists {
            load_applied_migrations(&self.connection)?
        } else {
            Vec::new()
        };
        validate_applied_history(&applied)?;
        if !applied.is_empty() && !applied.iter().any(|migration| migration.version >= 57) {
            let directory = self
                .database_path
                .parent()
                .ok_or_else(|| configuration_error("database parent missing"))?
                .join("schema-backups");
            std::fs::create_dir_all(&directory).map_err(|_| {
                migration_error("create application journal backup directory")(
                    rusqlite::Error::InvalidPath(directory.clone()),
                )
            })?;
            let path = directory.join(format!(
                "before-application-journal-57-{}.sqlite",
                unix_epoch_millis()?
            ));
            self.connection
                .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
                .map_err(migration_error(
                    "backup database before application journal migration",
                ))?;
        }

        for migration in MIGRATIONS {
            let checksum = migration_checksum(migration.sql);
            if let Some(existing) = applied
                .iter()
                .find(|item| item.version == migration.version)
            {
                if existing.name != migration.name || existing.checksum != checksum {
                    return Err(PersistenceError::new(
                        PersistenceErrorKind::Migration,
                        format!(
                            "applied migration {} does not match embedded name and checksum",
                            migration.version
                        ),
                    ));
                }
                continue;
            }
            self.apply_migration(migration, &checksum, app_version)?;
        }
        Ok(())
    }

    fn apply_migration(
        &mut self,
        migration: &Migration,
        checksum: &str,
        app_version: &str,
    ) -> Result<(), PersistenceError> {
        if migration.rebuilds_foreign_key_parent {
            self.connection
                .pragma_update(None, "foreign_keys", false)
                .map_err(migration_error(
                    "disable foreign keys for parent table rebuild",
                ))?;
        }
        let result = (|| {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(migration_error("start migration transaction"))?;
            transaction
                .execute_batch(migration.sql)
                .map_err(migration_error("execute migration SQL"))?;
            if migration.rebuilds_foreign_key_parent {
                let violations: i64 = transaction
                    .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                        row.get(0)
                    })
                    .map_err(migration_error("validate rebuilt foreign keys"))?;
                if violations != 0 {
                    return Err(PersistenceError::new(
                        PersistenceErrorKind::Migration,
                        "rebuilt workspace schema contains foreign key violations",
                    ));
                }
            }
            transaction
                .execute(
                    "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![migration.version, migration.name, checksum, unix_epoch_millis()?, app_version],
                )
                .map_err(migration_error("record applied migration"))?;
            transaction
                .commit()
                .map_err(migration_error("commit migration"))
        })();
        if migration.rebuilds_foreign_key_parent {
            let restore = self
                .connection
                .pragma_update(None, "foreign_keys", true)
                .map_err(migration_error(
                    "restore foreign keys after parent table rebuild",
                ));
            if result.is_ok() {
                restore?;
            }
        }
        result
    }
}

/// 尝试取得并立即释放 `SQLite` 写事务，用于陈旧实例恢复前确认没有活动数据库写者。
///
/// # Errors
///
/// 数据库无法只写打开、正在被写者占用、已损坏或写者状态无法确定时返回错误。
pub fn validate_no_active_writer(database_path: &Path) -> Result<(), PersistenceError> {
    if !database_path.exists() {
        return Ok(());
    }
    validate_database_path(database_path)?;
    let connection = Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(database_error("open database for writer validation"))?;
    connection
        .busy_timeout(Duration::ZERO)
        .map_err(database_error("disable writer validation timeout"))?;
    connection
        .execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
        .map_err(database_error("validate database writer ownership"))
}

fn configure_connection(connection: &Connection) -> Result<(), PersistenceError> {
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(database_error("enable WAL journal mode"))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(PersistenceError::new(
            PersistenceErrorKind::Configuration,
            format!("SQLite refused WAL journal mode: {journal_mode}"),
        ));
    }
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(database_error("enable foreign keys"))?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(database_error("set synchronous mode"))?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(database_error("set busy timeout"))?;
    connection
        .pragma_update(None, "temp_store", "MEMORY")
        .map_err(database_error("set temp store"))?;
    connection
        .pragma_update(None, "trusted_schema", false)
        .map_err(database_error("disable trusted schema"))
}

fn runtime_configuration(
    connection: &Connection,
) -> Result<SqliteRuntimeConfiguration, PersistenceError> {
    Ok(SqliteRuntimeConfiguration {
        journal_mode: pragma_string(connection, "journal_mode")?,
        foreign_keys: pragma_i64(connection, "foreign_keys")? == 1,
        synchronous: pragma_i64(connection, "synchronous")?,
        busy_timeout_ms: pragma_i64(connection, "busy_timeout")?,
        temp_store: pragma_i64(connection, "temp_store")?,
        trusted_schema: pragma_i64(connection, "trusted_schema")? == 1,
    })
}

fn pragma_string(connection: &Connection, name: &str) -> Result<String, PersistenceError> {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(database_error("read SQLite text configuration"))
}

fn pragma_i64(connection: &Connection, name: &str) -> Result<i64, PersistenceError> {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(database_error("read SQLite integer configuration"))
}

fn schema_migrations_exists(connection: &Connection) -> Result<bool, PersistenceError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'schema_migrations')",
            [],
            |row| row.get(0),
        )
        .map_err(migration_error("inspect migration table"))
}

fn user_schema_object_count(connection: &Connection) -> Result<i64, PersistenceError> {
    connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(migration_error("inspect existing database schema"))
}

#[derive(Debug)]
struct AppliedMigration {
    version: i64,
    name: String,
    checksum: String,
}

fn load_applied_migrations(
    connection: &Connection,
) -> Result<Vec<AppliedMigration>, PersistenceError> {
    let mut statement = connection
        .prepare("SELECT version, name, checksum FROM schema_migrations ORDER BY version")
        .map_err(migration_error("prepare migration history query"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(AppliedMigration {
                version: row.get(0)?,
                name: row.get(1)?,
                checksum: row.get(2)?,
            })
        })
        .map_err(migration_error("query migration history"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(migration_error("read migration history"))
}

fn validate_applied_history(applied: &[AppliedMigration]) -> Result<(), PersistenceError> {
    if applied.len() > MIGRATIONS.len() {
        return Err(PersistenceError::new(
            PersistenceErrorKind::Migration,
            "database migration history is newer than this App Server",
        ));
    }
    for (index, existing) in applied.iter().enumerate() {
        let expected = MIGRATIONS.get(index).ok_or_else(|| {
            PersistenceError::new(
                PersistenceErrorKind::Migration,
                "database contains an unknown migration version",
            )
        })?;
        if existing.version != expected.version {
            return Err(PersistenceError::new(
                PersistenceErrorKind::Migration,
                "database migration history is not contiguous",
            ));
        }
    }
    Ok(())
}

fn migration_checksum(sql: &str) -> String {
    let digest = Sha256::digest(sql.as_bytes());
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn unix_epoch_millis() -> Result<i64, PersistenceError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            PersistenceError::new(PersistenceErrorKind::Configuration, error.to_string())
        })?
        .as_millis();
    i64::try_from(millis).map_err(|_| {
        PersistenceError::new(
            PersistenceErrorKind::Configuration,
            "system time exceeds SQLite INTEGER milliseconds",
        )
    })
}

fn validate_database_path(database_path: &Path) -> Result<(), PersistenceError> {
    let parent = database_path.parent().ok_or_else(|| {
        PersistenceError::new(
            PersistenceErrorKind::Configuration,
            "database path has no parent directory",
        )
    })?;
    if !parent.is_dir() {
        return Err(PersistenceError::new(
            PersistenceErrorKind::Configuration,
            "database parent directory does not exist",
        ));
    }
    match fs::symlink_metadata(database_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(PersistenceError::new(
                PersistenceErrorKind::Configuration,
                "database path must be a regular file and must not be a symlink",
            ))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PersistenceError::new(
            PersistenceErrorKind::Configuration,
            format!("inspect database path: {error}"),
        )),
    }
}

fn secure_database_file(database_path: &Path) -> Result<(), PersistenceError> {
    contexthub_platform_adapter::security::restrict_existing_path(database_path).map_err(|error| {
        PersistenceError::new(
            PersistenceErrorKind::Configuration,
            format!("set database permissions: {error}"),
        )
    })
}

fn database_error(operation: &'static str) -> impl FnOnce(rusqlite::Error) -> PersistenceError {
    move |error| {
        PersistenceError::new(
            PersistenceErrorKind::Database,
            format!("{operation}: {error}"),
        )
    }
}

fn read_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn row_to_user_skill(row: &Row<'_>) -> rusqlite::Result<UserSkillRecord> {
    Ok(UserSkillRecord {
        skill_id: row.get(0)?,
        name: row.get(1)?,
        normalized_name: row.get(2)?,
        description: row.get(3)?,
        managed_relative_path: row.get(4)?,
        enabled: row.get(5)?,
        automatic: row.get(6)?,
        state_revision: row.get(7)?,
        content_digest: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn read_optional_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
        })
        .transpose()
}

fn recompute_ai_task_status(
    transaction: &Transaction<'_>,
    task_id: &str,
    run_id: &str,
    agent_label: &str,
    reason: &str,
    occurred_at: i64,
) -> Result<String, PersistenceError> {
    let (current_status, attempt_no) = transaction
        .query_row(
            "SELECT status,attempt_no FROM business_tasks WHERE business_task_id=?1",
            [task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(database_error("read AI task before aggregation"))?;
    if current_status == "terminated" {
        return Err(ai_turn_conflict("ai_work_task_terminal"));
    }
    let target_status = transaction
        .query_row(
            "SELECT CASE
                WHEN SUM(CASE WHEN latest.conversation_status='active' THEN 1 ELSE 0 END)=0 THEN 'completed'
                WHEN SUM(CASE WHEN latest.conversation_status='active' AND latest.stage='decision' THEN 1 ELSE 0 END)>0 THEN 'decision'
                WHEN SUM(CASE WHEN latest.conversation_status='active' AND latest.stage='review' THEN 1 ELSE 0 END)>0 THEN 'review'
                WHEN SUM(CASE WHEN latest.conversation_status='active' AND latest.stage='in_progress' THEN 1 ELSE 0 END)>0 THEN 'in_progress'
                ELSE 'todo'
             END
             FROM conversation_work_classifications latest
             WHERE latest.business_task_id=?1
               AND NOT EXISTS(
                   SELECT 1 FROM conversation_work_classifications newer
                   WHERE newer.business_task_id=latest.business_task_id
                     AND newer.conversation_id=latest.conversation_id
                     AND (
                         newer.created_at>latest.created_at OR
                         (newer.created_at=latest.created_at AND newer.classification_id>latest.classification_id)
                     )
               )",
            [task_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(database_error("aggregate AI task status"))?;
    let next_attempt_no = if current_status == "completed" {
        attempt_no.saturating_add(1)
    } else {
        attempt_no
    };
    if current_status != target_status {
        let transition_id = if current_status == "completed" {
            format!("ai-task-reactivate:{run_id}")
        } else {
            format!("ai-task-transition:{run_id}")
        };
        transaction
            .execute(
                "INSERT INTO business_task_transitions(
                    transition_id,business_task_id,from_status,to_status,actor_label,
                    comment,client_operation_id,request_fingerprint,created_at,attempt_no
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    transition_id,
                    task_id,
                    current_status,
                    target_status,
                    agent_label,
                    reason,
                    format!("ai-task-status:{run_id}"),
                    sha256_string(&format!("{task_id}:{run_id}:{target_status}")),
                    occurred_at,
                    next_attempt_no,
                ],
            )
            .map_err(database_error("record aggregated AI task transition"))?;
        transaction
            .execute(
                "UPDATE business_tasks
                 SET status=?2,state_revision=state_revision+1,updated_at=?3,attempt_no=?4
                 WHERE business_task_id=?1",
                params![task_id, target_status, occurred_at, next_attempt_no],
            )
            .map_err(database_error("update aggregated AI task status"))?;
    } else if current_status == "completed" {
        transaction
            .execute(
                "UPDATE business_tasks
                 SET state_revision=state_revision+1,updated_at=?2,attempt_no=?3
                 WHERE business_task_id=?1",
                params![task_id, occurred_at, next_attempt_no],
            )
            .map_err(database_error("record completed AI task attempt"))?;
    }
    Ok(target_status)
}

fn reconcile_tool_failure_incident(
    transaction: &Transaction<'_>,
    command: &AiTurnFinalizeCommand,
    task_id: Option<&str>,
) -> Result<(), PersistenceError> {
    let latest_tool_outcome = read_latest_reply_tool_outcome(transaction, &command.run_id)?;
    let conversation_completed =
        command.classification.conversation_status.as_deref() == Some("completed");
    let matching_failure = latest_tool_outcome
        .as_ref()
        .and_then(|(status, summary)| tool_failure_summary(status, summary));
    if conversation_completed || matching_failure.is_some() {
        resolve_superseded_tool_failure_incidents(
            transaction,
            command,
            if conversation_completed {
                None
            } else {
                matching_failure.as_deref()
            },
        )?;
    }

    let Some((status, summary)) = latest_tool_outcome else {
        return Ok(());
    };
    if status != "failed" {
        return Ok(());
    }
    let (Some(project_id), Some(task_id)) = (command.project_id.as_deref(), task_id) else {
        return Ok(());
    };
    let (conversation_title, phase_id) = transaction
        .query_row(
            "SELECT
                COALESCE(NULLIF(TRIM(json_extract(conversation_json,'$.title')),''),'AI 对话'),
                CASE WHEN EXISTS(
                    SELECT 1 FROM project_phases phase
                    WHERE phase.phase_id=json_extract(conversation_json,'$.context.phaseId')
                      AND phase.project_id=?2
                ) THEN json_extract(conversation_json,'$.context.phaseId') ELSE NULL END
             FROM ai_business_conversations WHERE conversation_id=?1",
            params![command.conversation_id, project_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(database_error("read failed tool conversation"))?
        .unwrap_or_else(|| ("AI 对话".to_owned(), None));
    let case_id = format!("tool-failure-incident-{}", command.run_id);
    let operation_id = format!("desktop-tool-failure-incident-{}", command.run_id);
    transaction
        .execute(
            "INSERT INTO attention_cases(
                attention_case_id,case_type,project_id,phase_id,task_id,deliverable_id,
                title,detail,status,responsibility_type,responsibility_label,reporter_label,
                priority,due_at,agent_run_id,agent_label,resolution_summary,state_revision,
                client_operation_id,request_fingerprint,created_at,updated_at,resolved_at
             ) VALUES (?1,'agent_run_incident',?2,?3,?4,NULL,?5,?6,'open','user','当前用户',
                       '词来 桌面端','high',NULL,?7,?8,NULL,0,?9,?10,?11,?11,NULL)",
            params![
                case_id,
                project_id,
                phase_id,
                task_id,
                format!("处理工具调用失败：{conversation_title}"),
                summary,
                command.run_id,
                command.agent_label.trim(),
                operation_id,
                sha256_string(&format!("{}\ntool_failure\n{summary}", command.run_id)),
                command.occurred_at,
            ],
        )
        .map_err(database_error("record failed AI tool incident"))?;
    let payload = serde_json::json!({
        "attentionCaseId": case_id,
        "conversationId": command.conversation_id,
        "runId": command.run_id,
        "taskId": task_id,
        "status": "open",
        "reason": summary,
    });
    transaction
        .execute(
            "INSERT INTO durable_events(
                event_id,workspace_id,workspace_sequence,event_type,object_type,object_id,
                payload_json,correlation_id,occurred_at,created_at
             ) VALUES (?1,NULL,NULL,'attention_case.created','attention_case',?2,?3,?1,?4,?4)",
            params![
                operation_id,
                case_id,
                payload.to_string(),
                command.occurred_at
            ],
        )
        .map_err(database_error("publish failed AI tool incident"))?;
    Ok(())
}

fn read_latest_reply_tool_outcome(
    transaction: &Transaction<'_>,
    run_id: &str,
) -> Result<Option<(String, String)>, PersistenceError> {
    let native: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ai_storage_activation WHERE storage_key='native')",
            [],
            |row| row.get(0),
        )
        .map_err(database_error("read AI storage mode"))?;
    if native {
        return transaction.query_row("SELECT json_extract(run_json,'$.latestToolOutcome.status'),json_extract(run_json,'$.latestToolOutcome.summary') FROM ai_business_runs WHERE run_id=?1 AND json_type(run_json,'$.latestToolOutcome')='object'", [run_id], |row| Ok((row.get(0)?,row.get(1)?))).optional().map_err(database_error("read latest AI tool result"));
    }
    transaction
        .query_row(
            "SELECT
                COALESCE(json_extract(event.value,'$.status'),''),
                COALESCE(
                    NULLIF(TRIM(json_extract(event.value,'$.summary')),''),
                    '工具调用失败，需要查看原对话处理。'
                )
             FROM ai_business_runs run,json_each(run.run_json,'$.events') event
             WHERE run.run_id=?1
               AND json_extract(event.value,'$.phase')='reply'
               AND json_extract(event.value,'$.kind')='mcp'
               AND json_extract(event.value,'$.eventType')='item.completed'
             ORDER BY CAST(json_extract(event.value,'$.sequence') AS INTEGER) DESC,
                      CAST(event.key AS INTEGER) DESC
             LIMIT 1",
            [run_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error("read latest AI tool outcome"))
}

fn tool_failure_summary(status: &str, summary: &str) -> Option<String> {
    if status == "failed" {
        return Some(summary.to_owned());
    }
    (status == "completed")
        .then(|| summary.strip_prefix("已使用 "))
        .flatten()
        .map(|tool| format!("{tool} 调用失败"))
}

fn resolve_superseded_tool_failure_incidents(
    transaction: &Transaction<'_>,
    command: &AiTurnFinalizeCommand,
    matching_failure: Option<&str>,
) -> Result<(), PersistenceError> {
    let cases = {
        let mut statement = transaction
            .prepare(
                "SELECT attention.attention_case_id
                 FROM attention_cases attention
                 JOIN ai_business_runs failed_run ON failed_run.run_id=attention.agent_run_id
                 WHERE attention.case_type='agent_run_incident'
                   AND attention.status='open'
                   AND attention.client_operation_id LIKE 'desktop-tool-failure-incident-%'
                   AND failed_run.conversation_id=?1
                   AND attention.agent_run_id<>?2
                   AND (?3 IS NULL OR attention.detail=?3)",
            )
            .map_err(database_error("prepare superseded tool failure incidents"))?;
        statement
            .query_map(
                params![command.conversation_id, command.run_id, matching_failure],
                |row| row.get::<_, String>(0),
            )
            .map_err(database_error("query superseded tool failure incidents"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("decode superseded tool failure incidents"))?
    };
    for case_id in cases {
        let operation_id = format!("tool-failure-resolved:{case_id}:{}", command.run_id);
        let summary = format!("后续回合已更新工具执行结果：{}。", command.run_id);
        transaction
            .execute(
                "INSERT OR IGNORE INTO attention_case_transitions(
                    transition_id,attention_case_id,from_status,to_status,actor_label,
                    resolution_summary,client_operation_id,request_fingerprint,created_at
                 ) VALUES (?1,?2,'open','resolved','系统',?3,?1,?1,?4)",
                params![operation_id, case_id, summary, command.occurred_at],
            )
            .map_err(database_error("record tool failure incident resolution"))?;
        transaction
            .execute(
                "UPDATE attention_cases
                 SET status='resolved',resolution_summary=?2,resolved_at=?3,updated_at=?3,
                     state_revision=state_revision+1
                 WHERE attention_case_id=?1 AND status='open'",
                params![case_id, summary, command.occurred_at],
            )
            .map_err(database_error("resolve superseded tool failure incident"))?;
        let payload = serde_json::json!({
            "attentionCaseId": case_id,
            "recoveredRunId": command.run_id,
            "status": "resolved",
        });
        transaction
            .execute(
                "INSERT OR IGNORE INTO durable_events(
                    event_id,workspace_id,workspace_sequence,event_type,object_type,object_id,
                    payload_json,correlation_id,occurred_at,created_at
                 ) VALUES (?1,NULL,NULL,'attention_case.resolved','attention_case',?2,?3,?1,?4,?4)",
                params![
                    operation_id,
                    case_id,
                    payload.to_string(),
                    command.occurred_at
                ],
            )
            .map_err(database_error("publish tool failure incident resolution"))?;
    }
    Ok(())
}

fn validate_ai_turn_command(command: &AiTurnFinalizeCommand) -> Result<(), PersistenceError> {
    if command
        .changes
        .payload
        .as_ref()
        .is_some_and(|payload| !payload.is_object())
        || !command.classification.raw.is_object()
        || command.conversation_id.trim().is_empty()
        || command.run_id.trim().is_empty()
        || command.agent_label.trim().is_empty()
        || command.classification.reason.trim().is_empty()
        || !command.classification.confidence.is_finite()
        || !(0.0..=1.0).contains(&command.classification.confidence)
    {
        return Err(ai_turn_conflict("ai_turn_classification_invalid"));
    }
    match (
        command.classification.action.as_str(),
        command.classification.task_action.as_str(),
    ) {
        ("unclassified", "none")
            if command.classification.task_id.is_none()
                && command.classification.conversation_status.is_none()
                && command.classification.stage.is_none()
                && command.classification.outcome_title.is_none()
                && command.classification.outcome_summary.is_none()
                && command.new_task_id.is_none() =>
        {
            Ok(())
        }
        ("apply", "create")
            if command
                .project_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && command.classification.task_id.is_none()
                && command
                    .new_task_id
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                && valid_ai_conversation_result(&command.classification) =>
        {
            Ok(())
        }
        ("apply", "link")
            if command
                .project_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && command
                    .classification
                    .task_id
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                && command.new_task_id.is_none()
                && valid_ai_conversation_result(&command.classification) =>
        {
            Ok(())
        }
        ("keep", "keep")
            if command
                .project_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && command
                    .classification
                    .task_id
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                && command.previous_task_id == command.classification.task_id
                && command.new_task_id.is_none()
                && valid_ai_conversation_result(&command.classification) =>
        {
            Ok(())
        }
        _ => Err(ai_turn_conflict("ai_turn_classification_invalid")),
    }
}

fn valid_ai_conversation_result(classification: &AiTurnClassificationRecord) -> bool {
    let has_outcome = classification
        .outcome_title
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && classification
            .outcome_summary
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    has_outcome
        && match classification.conversation_status.as_deref() {
            Some("active") => valid_ai_work_stage(classification.stage.as_deref()),
            Some("completed") => classification.stage.is_none(),
            _ => false,
        }
}

fn valid_ai_work_stage(stage: Option<&str>) -> bool {
    stage.is_some_and(|value| matches!(value, "todo" | "in_progress" | "review" | "decision"))
}

fn ai_task_contract_matches(
    candidate: (&str, &str, &str, &str),
    proposed: (&str, &str, &str, &str),
) -> bool {
    let same_title =
        normalized_ai_task_identity(candidate.0) == normalized_ai_task_identity(proposed.0);
    let same_contract_fields = [
        normalized_ai_task_identity(candidate.1) == normalized_ai_task_identity(proposed.1),
        normalized_ai_task_identity(candidate.2) == normalized_ai_task_identity(proposed.2),
        normalized_ai_task_identity(candidate.3) == normalized_ai_task_identity(proposed.3),
    ]
    .into_iter()
    .filter(|matches| *matches)
    .count();
    (same_title && same_contract_fields >= 1) || same_contract_fields == 3
}

fn normalized_ai_task_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            !character.is_whitespace()
                && !"，。！？、；：,.!?;:\"'“”‘’（）()【】[]".contains(*character)
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn required_ai_text<'a>(
    value: Option<&'a str>,
    reason: &'static str,
) -> Result<&'a str, PersistenceError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ai_turn_conflict(reason))
}

fn ai_turn_conflict(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}

fn sha256_string(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn migration_error(operation: &'static str) -> impl FnOnce(rusqlite::Error) -> PersistenceError {
    move |error| {
        PersistenceError::new(
            PersistenceErrorKind::Migration,
            format!("{operation}: {error}"),
        )
    }
}

#[cfg(test)]
mod test_historical_accounts;

#[cfg(test)]
mod test_historical_platform;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_historical_accounts::insert_historical_account;
    use crate::test_historical_platform::{
        HistoricalPlatformSnapshot, insert_historical_platform_binding,
        remove_historical_platform_binding, seed_historical_platform_snapshot,
    };
    use contexthub_application::{
        ActivatePhasePlanCommand, AttentionCaseRepository, CreateAttentionCaseCommand,
        CreateBusinessTaskCommand, CreatePhasePlanCommand, DeliverableCarrierInput,
        GlobalSearchQuery, LocalFileObservation, MoveBusinessTaskCommand, MyWorkItemRecord,
        MyWorkQuery, PhaseDraftInput, ProjectPhaseRecord, ProjectProgressRepository,
        TransitionAttentionCaseCommand, TransitionBusinessTaskCommand,
        TransitionProjectPhaseCommand, export_workspace_projection,
    };
    use contexthub_domain::{
        AccountId, ActiveConfirmationFact, AttentionCaseStatus, AttentionCaseType,
        BaselinePathRelation, BusinessTaskStatus, FilePermission, FilePermissions, FileStateInput,
        LocalNodeKind, LocalScanState, MaterializationState, MetadataValidity, PhasePlanStatus,
        PlatformConnection, PlatformDocumentState, ProjectPhaseStatus, SubmissionState,
        WorkspaceKind,
    };
    use contexthub_workspace_fs::WorkspaceProjectionWriter;

    fn apply_fixture_migration(connection: &Connection, migration: &Migration) {
        if migration.rebuilds_foreign_key_parent {
            connection
                .pragma_update(None, "foreign_keys", false)
                .unwrap();
        }
        connection.execute_batch(migration.sql).unwrap();
        if migration.rebuilds_foreign_key_parent {
            let violations: i64 = connection
                .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(violations, 0);
            connection
                .pragma_update(None, "foreign_keys", true)
                .unwrap();
        }
    }

    fn ai_turn_state(conversation_id: &str, run_id: &str) -> serde_json::Value {
        serde_json::json!({
            "agents": [],
            "conversations": [{"conversationId": conversation_id}],
            "messages": [{"messageId": format!("reply-{run_id}"), "role": "assistant", "content": "候选结果"}],
            "runs": [{"runId": run_id, "status": "completed"}]
        })
    }

    fn with_ai_state(command: &mut AiTurnFinalizeCommand, state: &serde_json::Value) {
        command.changes = AiStateChanges::full_from_state(state).unwrap();
    }

    fn ai_create_task_command(
        conversation_id: &str,
        run_id: &str,
        task_id: &str,
        expected_revision: i64,
    ) -> AiTurnFinalizeCommand {
        let raw = serde_json::json!({
            "action": "apply",
            "taskAction": "create",
            "taskId": null,
            "taskTitle": "自动归类对话",
            "objective": "归类本轮工作",
            "expectedOutput": "任务和阶段投影",
            "completionCriteria": "工作台可见",
            "conversationStatus": "active",
            "stage": "in_progress",
            "outcomeTitle": "已完成自动归类",
            "outcomeSummary": "本轮工作已关联任务并进入进行中",
            "reason": "已开始实质执行",
            "confidence": 0.96
        });
        AiTurnFinalizeCommand {
            ledger_request: None,
            expected_revision,
            changes: AiStateChanges::full_from_state(&ai_turn_state(conversation_id, run_id))
                .unwrap(),
            conversation_id: conversation_id.to_owned(),
            run_id: run_id.to_owned(),
            project_id: Some("project-ai".to_owned()),
            previous_task_id: None,
            agent_label: "ContextHub Agent".to_owned(),
            new_task_id: Some(task_id.to_owned()),
            classification: AiTurnClassificationRecord {
                action: "apply".to_owned(),
                task_action: "create".to_owned(),
                task_id: None,
                task_title: Some("自动归类对话".to_owned()),
                objective: Some("归类本轮工作".to_owned()),
                expected_output: Some("任务和阶段投影".to_owned()),
                completion_criteria: Some("工作台可见".to_owned()),
                conversation_status: Some("active".to_owned()),
                stage: Some("in_progress".to_owned()),
                outcome_title: Some("已完成自动归类".to_owned()),
                outcome_summary: Some("本轮工作已关联任务并进入进行中".to_owned()),
                reason: "已开始实质执行".to_owned(),
                confidence: 0.96,
                raw,
            },
            occurred_at: 10,
        }
    }

    /// 删除工作区（`delete_workspace`）与删除项目（`delete_project`）会逐行删除的表。
    /// 往这两条删除路径里加新表时，也要把它加进来。
    const ROW_DELETED_TABLES: &[&str] = &[
        "attention_case_transitions",
        "attention_cases",
        "business_task_moves",
        "business_task_transitions",
        "business_tasks",
        "checksum_cache",
        "conflicts",
        "deliverable_acceptances",
        "deliverable_file_bindings",
        "deliverable_version_carriers",
        "deliverable_versions",
        "deliverables",
        "document_versions",
        "documents",
        "durable_events",
        "file_confirmation_text_snapshots",
        "file_confirmations",
        "file_states",
        "local_file_observations",
        "local_files",
        "local_scan_snapshots",
        "my_work_items",
        "phase_dependencies",
        "phase_plans",
        "phase_transitions",
        "platform_snapshots",
        "project_phases",
        "project_workspaces",
        "projects",
        "sync_executions",
        "sync_plan_items",
        "sync_plans",
        "task_attempts",
        "tasks",
        "workspace_exports",
        "workspace_platform_bindings",
        "workspaces",
    ];

    #[test]
    fn 删除路径上逐行检查的外键都有可用索引() {
        // 删除工作区或项目时，上面这些表会被逐行删除，SQLite 每删一行都按外键查一遍子表。
        // 子表的引用字段没有以它开头的无条件索引时，每次查找都是整表扫描：曾经删一个
        // 一万三千文件的工作区要 54 秒，全程占着服务主锁。
        let fixture = TestDatabase::new("fk-indexes");
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let mut statement = persistence
            .connection
            .prepare(
                "SELECT m.name, fk.\"from\", fk.\"table\" FROM sqlite_master m, pragma_foreign_key_list(m.name) fk
                 WHERE m.type = 'table'
                   AND NOT EXISTS (
                     SELECT 1 FROM pragma_index_list(m.name) il, pragma_index_info(il.name) ii
                     WHERE il.partial = 0 AND ii.seqno = 0 AND ii.name = fk.\"from\"
                   )
                 ORDER BY m.name, fk.\"from\"",
            )
            .unwrap();
        let missing = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .into_iter()
            .filter(|(_, _, parent)| ROW_DELETED_TABLES.contains(&parent.as_str()))
            .map(|(table, column, parent)| format!("{table}.{column} -> {parent}"))
            .collect::<Vec<_>>();
        assert!(
            missing.is_empty(),
            "foreign keys without a usable index: {missing:?}"
        );
    }

    #[test]
    fn workbench_memory_reset_rejects_late_publication_and_import_receipts_survive_reopen() {
        let fixture = TestDatabase::new("owned-context-and-memory");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let state = db
            .ai_history("memoryGet", &serde_json::Value::Null, 1)
            .unwrap();
        db.ai_history("memoryReset", &serde_json::Value::Null, 2)
            .unwrap();
        assert!(db.ai_history("memorySave", &state, 3).is_err());
        let import = serde_json::json!({"scope":"tenant-home","threadId":"t","turns":[{"id":"old","messages":[{"id":"m","text":"原文"}]}]});
        assert_eq!(
            db.ai_history("contextPrepare", &import, 4).unwrap()["replayed"],
            false
        );
        drop(db);
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert_eq!(
            db.ai_history("contextPrepare", &import, 5).unwrap()["replayed"],
            true
        );
        db.ai_history("contextCommit", &import, 6).unwrap();
        assert_eq!(
            db.ai_history("contextGet", &import, 7).unwrap()["turns"],
            import["turns"]
        );
        let mut foreign = import.clone();
        foreign["scope"] = serde_json::json!("other-home");
        assert!(db.ai_history("contextGet", &foreign, 8).unwrap().is_null());
        let mut changed = import.clone();
        changed["turns"] = serde_json::json!([]);
        assert!(db.ai_history("contextPrepare", &changed, 9).is_err());
    }

    #[test]
    fn context_import_delete_is_scoped_idempotent_and_persists_after_reopen() {
        let fixture = TestDatabase::new("context-import-delete");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let import = serde_json::json!({"scope":"profile-a","threadId":"thread","turns":[{"id":"handoff","messages":[{"text":"应用转交正文"}]}]});
        let mut other = import.clone();
        other["scope"] = serde_json::json!("profile-b");
        for value in [&import, &other] {
            db.ai_history("contextPrepare", value, 1).unwrap();
            db.ai_history("contextCommit", value, 2).unwrap();
        }
        assert_eq!(
            db.ai_history("contextDelete", &import, 3).unwrap()["deleted"],
            true
        );
        assert_eq!(
            db.ai_history("contextDelete", &import, 4).unwrap()["deleted"],
            false
        );
        assert!(!db.ai_history("contextGet", &other, 5).unwrap().is_null());
        drop(db);
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert!(db.ai_history("contextGet", &import, 6).unwrap().is_null());
        assert!(!db.ai_history("contextGet", &other, 7).unwrap().is_null());
    }

    #[test]
    fn new_database_has_frozen_configuration_and_migration_history() {
        let fixture = TestDatabase::new("configuration");
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert_eq!(persistence.database_path(), fixture.path);
        assert_eq!(
            persistence.runtime_configuration().unwrap(),
            SqliteRuntimeConfiguration {
                journal_mode: "wal".to_owned(),
                foreign_keys: true,
                synchronous: 2,
                busy_timeout_ms: 5_000,
                temp_store: 2,
                trusted_schema: false,
            }
        );
        let mut statement = persistence
            .connection
            .prepare(
                "SELECT version, name, checksum, app_version FROM schema_migrations ORDER BY version",
            )
            .unwrap();
        let migrations = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(migrations.len(), MIGRATIONS.len());
        for (index, (version, name, checksum, app_version)) in migrations.iter().enumerate() {
            let expected = &MIGRATIONS[index];
            assert_eq!(*version, expected.version);
            assert_eq!(name, expected.name);
            assert_eq!(checksum, &migration_checksum(expected.sql));
            assert_eq!(app_version, "0.1.0");
        }
        assert!(migrations.iter().all(|migration| migration.3 == "0.1.0"));
        contexthub_platform_adapter::security::verify_restricted_path(&fixture.path, false)
            .unwrap();
    }

    #[test]
    fn capture_v63_upgrade_preserves_prepared_handoff_and_original_owner() {
        let fixture = TestDatabase::new("capture-v63-prepared-handoff");
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in MIGRATIONS.iter().filter(|m| m.version <= 63) {
            apply_fixture_migration(&connection, migration);
            connection.execute("INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES(?1,?2,?3,1,'previous-client')",params![migration.version,migration.name,migration_checksum(migration.sql)]).unwrap();
        }
        let scope = serde_json::json!({"tenant_id":"tenant","global_user_id":"user","membership_id":"member","source_device_id":"source","receiver_device_id":"receiver"}).to_string();
        let input = serde_json::json!({"prompt":"原已确认需求","agentId":"agent","context":{"workspaceRoot":""},"conversationId":null,"modelOverride":null,"permissionMode":"request"}).to_string();
        connection.execute("INSERT INTO capture_transfers VALUES('transfer','original-owner',?1,'record',1,'digest','{}','received','receipt',1,2)",[&scope]).unwrap();
        connection.execute("INSERT INTO capture_analyses(analysis_id,operation_id,transfer_id,version,source_text,source_sha256,state,source_run_id,result_json,created_at) VALUES('analysis','op','transfer',1,'原已确认需求','digest','completed','analysis-run',?1,3)",[serde_json::json!({"requirements":[],"todos":[],"questions":[]}).to_string()]).unwrap();
        connection.execute("INSERT INTO capture_handoffs VALUES('old','analysis','candidate',1,?1,NULL,NULL,'prepared',4)",[&input]).unwrap();
        drop(connection);
        for _ in 0..2 {
            let db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
            let handoff: (String, Option<String>, Option<String>, String) = db.connection.query_row("SELECT input_json,conversation_id,run_id,state FROM capture_handoffs WHERE handoff_id='old'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
            assert_eq!(handoff, (input.clone(), None, None, "prepared".into()));
            let owner: (String, String) = db.connection.query_row("SELECT scope_key,scope_json FROM capture_transfers WHERE transfer_id='transfer'", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
            assert_eq!(owner, ("original-owner".into(), scope.clone()));
            let count: i64 = db.connection.query_row("SELECT COUNT(*) FROM schema_migrations WHERE version<=63 AND app_version='previous-client'", [], |row| row.get(0)).unwrap();
            assert_eq!(count, 63);
        }
    }

    #[test]
    fn capture_v61_and_v63_upgrade_keeps_accepted_analysis_and_processing_facts() {
        for previous_version in [61, 63] {
            let fixture = TestDatabase::new(&format!("capture-v{previous_version}-execution"));
            let connection = Connection::open(&fixture.path).unwrap();
            configure_connection(&connection).unwrap();
            for migration in MIGRATIONS.iter().filter(|m| m.version <= previous_version) {
                apply_fixture_migration(&connection, migration);
                connection.execute("INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES(?1,?2,?3,123,'previous-client')",params![migration.version,migration.name,migration_checksum(migration.sql)]).unwrap();
            }
            connection.execute_batch(r#"INSERT INTO capture_transfers VALUES('transfer','owner','{}','record',1,'digest','{}','received','receipt',1,2);
                INSERT INTO capture_analyses(analysis_id,operation_id,transfer_id,version,source_text,source_sha256,state,source_run_id,created_at) VALUES('analysis','analysis-operation','transfer',1,'原有讨论','source-digest','prepared','accepted-run',3);
                INSERT INTO capture_processing VALUES('processing','transfer','speech-operation','speech-model','audio','{"frozen":true}','submitted',NULL,2,3);
                INSERT INTO capture_handoffs VALUES('handoff','analysis','candidate',1,'{"executionTarget":"agent"}','conversation','original-run','submitted',4);"#).unwrap();
            drop(connection);
            for _ in 0..2 {
                let db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
                let row:(String,String,String,Option<String>)=db.connection.query_row("SELECT source_text,source_run_id,state,execution_json FROM capture_analyses WHERE analysis_id='analysis'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
                assert_eq!(
                    row,
                    (
                        "原有讨论".into(),
                        "accepted-run".into(),
                        "prepared".into(),
                        None
                    )
                );
                let processing:(String,String)=db.connection.query_row("SELECT state,request_json FROM capture_processing WHERE processing_id='processing'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                assert_eq!(
                    processing,
                    ("submitted".into(), r#"{"frozen":true}"#.into())
                );
                let handoff: (String, String) = db
                    .connection
                    .query_row(
                        "SELECT run_id,input_json FROM capture_handoffs WHERE handoff_id='handoff'",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(
                    handoff,
                    (
                        "original-run".into(),
                        r#"{"executionTarget":"agent"}"#.into()
                    )
                );
                let count:i64=db.connection.query_row("SELECT COUNT(*) FROM schema_migrations WHERE version<=?1 AND app_version='previous-client'",[previous_version],|r|r.get(0)).unwrap();
                assert_eq!(count, previous_version);
            }
        }
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "升级及重启后的文件和项目关联在同一数据库生命周期验收"
    )]
    fn legacy_cloud_mirrors_become_independent_local_spaces_and_keep_files_and_project_links() {
        let fixture = TestDatabase::new("retire-cloud-mirror");
        let local_root = fixture.path.with_extension("files");
        std::fs::create_dir_all(&local_root).unwrap();
        std::fs::write(local_root.join("keep.txt"), b"user-modified-local-copy").unwrap();
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in &MIGRATIONS[..53] {
            apply_fixture_migration(&connection, migration);
            connection.execute("INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')", params![migration.version,migration.name,migration_checksum(migration.sql)]).unwrap();
        }
        connection.execute_batch("INSERT INTO accounts(account_id,display_name,account_status,created_at,updated_at) VALUES ('mirror-account','成员','active',1,1);
            INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at) VALUES ('mirror-project','项目','迁移','active',0,'mirror-create','mirror-create',1,1);
            INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at,workspace_kind) VALUES
                ('legacy-mirror','原团队','/tmp/mirror-placeholder','ready',2,1,2,'cloud_team'),
                ('direct','直接云端','cilai-cloud://direct-team','ready',0,1,1,'cloud_team');
            INSERT INTO workspace_exports(workspace_id,desired_revision,exported_revision,export_state,updated_at) VALUES ('legacy-mirror',2,2,'current',2),('direct',0,0,'current',1);
            INSERT INTO workspace_platform_bindings(workspace_id,account_id,platform_workspace_id,project_id,platform_root,binding_state,created_at,updated_at,team_space_id,managed_root,device_id,permission_role,access_mode) VALUES
                ('legacy-mirror','mirror-account','mirror-team','mirror-project','/','bound',1,2,'mirror-team','/tmp/mirror-placeholder','old-device','editor','remote_direct'),
                ('direct','mirror-account','direct-team','mirror-project','/','bound',1,1,'direct-team',NULL,NULL,'viewer','remote_direct');
            INSERT INTO project_workspaces(project_id,workspace_id,role,created_at) VALUES ('mirror-project','legacy-mirror','primary',1);
            INSERT INTO local_files(local_file_id,workspace_id,relative_path,normalized_relative_path,materialization_state,file_type,observed_at,scan_state,state_revision,created_at,updated_at) VALUES ('keep-file','legacy-mirror','keep.txt','keep.txt','materialized','file',1,'current',0,1,1);
            INSERT INTO cloud_workspace_operations(client_operation_id,request_fingerprint,operation_kind,project_id,team_space_id,display_name,managed_root,device_id,state,workspace_id,created_at,updated_at) VALUES ('old-operation','fingerprint','attach_existing','mirror-project','mirror-team','原团队','/tmp/mirror-placeholder','old-device','pulling_remote','legacy-mirror',1,1);").unwrap();
        connection
            .execute(
                "UPDATE workspaces SET local_root=?1 WHERE workspace_id='legacy-mirror'",
                [local_root.to_string_lossy().as_ref()],
            )
            .unwrap();
        drop(connection);
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.1").unwrap();
        let connection = &persistence.connection;
        let local: (String, String) = connection.query_row("SELECT workspace_kind,local_root FROM workspaces WHERE workspace_id='legacy-mirror'", [], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
        assert_eq!(
            local,
            (
                "local".to_owned(),
                local_root.to_string_lossy().into_owned()
            )
        );
        assert_eq!(
            std::fs::read(local_root.join("keep.txt")).unwrap(),
            b"user-modified-local-copy"
        );
        let remote: (String,String,String) = connection.query_row("SELECT w.workspace_kind,w.local_root,b.access_mode FROM workspaces w JOIN workspace_platform_bindings b ON b.workspace_id=w.workspace_id WHERE w.workspace_id='cloud-workspace-retired-legacy-mirror'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(
            remote,
            (
                "cloud_team".to_owned(),
                "cilai-cloud://mirror-team".to_owned(),
                "remote_direct".to_owned()
            )
        );
        assert_eq!(connection.query_row("SELECT binding_state FROM workspace_platform_bindings WHERE workspace_id='legacy-mirror'",[],|row|row.get::<_,String>(0)).unwrap(),"disconnected");
        assert_eq!(connection.query_row("SELECT state FROM cloud_workspace_operations WHERE client_operation_id='old-operation'",[],|row|row.get::<_,String>(0)).unwrap(),"failed");
        assert_eq!(
            connection
                .query_row(
                    "SELECT workspace_id FROM local_files WHERE local_file_id='keep-file'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "legacy-mirror"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM project_workspaces WHERE project_id='mirror-project'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT workspace_kind FROM workspaces WHERE workspace_id='direct'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "cloud_team"
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        drop(persistence);
        let reopened = SqlitePersistence::open(&fixture.path, "0.1.1").unwrap();
        assert_eq!(
            reopened
                .connection
                .query_row("SELECT count(*) FROM cloud_mirror_retirements", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
        std::fs::remove_dir_all(local_root).unwrap();
    }

    #[test]
    fn released_migration_twenty_nine_stays_frozen_and_upgrades_to_thirty() {
        const RELEASED_MIGRATION_29_CHECKSUM: &str =
            "eb6c70b1ee4fb95af8774cd20e4f1d5091fc5d08333cd01bc88a8fbd6edd039a";
        let released = &MIGRATIONS[28];
        assert_eq!(released.version, 29);
        assert_eq!(released.name, "add_cloud_workspace_baseline_transfer");
        assert_eq!(
            migration_checksum(released.sql),
            RELEASED_MIGRATION_29_CHECKSUM
        );

        let fixture = TestDatabase::new("upgrade-v29");
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in &MIGRATIONS[..29] {
            apply_fixture_migration(&connection, migration);
            connection
                .execute(
                    "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                    params![
                        migration.version,
                        migration.name,
                        migration_checksum(migration.sql)
                    ],
                )
                .unwrap();
        }
        drop(connection);

        let persistence = SqlitePersistence::open(&fixture.path, "0.1.1").unwrap();
        let migration_29_checksum: String = persistence
            .connection
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE version=29",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migration_29_checksum, RELEASED_MIGRATION_29_CHECKSUM);
        let migration_30_count: i64 = persistence
            .connection
            .query_row(
                "SELECT count(*) FROM schema_migrations WHERE version=30 AND name='add_cloud_workspace_pending_operation_index'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migration_30_count, 1);
        let index_count: i64 = persistence
            .connection
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='index' AND name='idx_cloud_workspace_one_pending_per_project'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 1);
    }

    fn prepare_local_confirmation_upgrade_fixture(connection: &Connection) {
        configure_connection(connection).unwrap();
        for migration in &MIGRATIONS[..31] {
            apply_fixture_migration(connection, migration);
            connection.execute(
                "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                params![migration.version, migration.name, migration_checksum(migration.sql)],
            ).unwrap();
        }
        connection.execute_batch(
            "INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at,workspace_kind)
             VALUES ('w-local','本地','/tmp/local','ready',0,1,1,'local'),
                    ('w-cloud','云端','/tmp/cloud','ready',0,1,1,'cloud_team');
             INSERT INTO local_files(local_file_id,workspace_id,relative_path,normalized_relative_path,materialization_state,file_type,observed_at,scan_state,state_revision,created_at,updated_at)
             VALUES ('f-local','w-local','a.md','a.md','materialized','file',1,'current',0,1,1),
                    ('f-cloud','w-cloud','b.md','b.md','materialized','file',1,'current',0,1,1),
                    ('f-converted','w-cloud','c.md','c.md','materialized','file',1,'current',0,1,1);
             INSERT INTO file_states(local_file_id,baseline_relation,local_change_state,confirmation_state,submission_state,conflict_state,display_state,capability_bits,capability_reasons_json,computed_at,state_revision)
             VALUES ('f-local','platform_absent','added','pending','none','none','confirmation_required',4,'[]',1,0),
                    ('f-cloud','platform_absent','added','pending','none','none','confirmation_required',4,'[]',1,0);
             INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('p-cloud','云端项目','验证转换历史','active',0,'project-operation','project-fingerprint',1,1);
             INSERT INTO cloud_workspace_operations(client_operation_id,request_fingerprint,operation_kind,project_id,source_workspace_id,display_name,managed_root,device_id,state,workspace_id,created_at,updated_at)
             VALUES ('conversion','conversion-fingerprint','initialize_local','p-cloud','w-cloud','云端','/tmp/cloud','device','bound','w-cloud',1,2);
             INSERT INTO file_confirmations(confirmation_id,workspace_id,local_file_id,confirmed_by_account_id,confirmed_checksum_algorithm,confirmed_checksum_value,target_scope,result,source_client,created_at,invalidated_at)
             VALUES ('old-confirmation','w-local','f-local','local-profile:default','sha256','old-content','content','confirmed','desktop',1,NULL),
                    ('converted-local-confirmation','w-cloud','f-converted','local-profile:default','sha256','old-cloud-content','content','confirmed','desktop',1,NULL),
                    ('cloud-confirmation','w-cloud','f-cloud','local-profile:default','sha256','cloud-content','content','confirmed','desktop',3,NULL);",
        ).unwrap();
    }

    #[test]
    fn local_confirmation_upgrade_retains_history_without_a_pending_action() {
        let fixture = TestDatabase::new("upgrade-local-file-status");
        let connection = Connection::open(&fixture.path).unwrap();
        prepare_local_confirmation_upgrade_fixture(&connection);
        drop(connection);

        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.1").unwrap();
        let local: (String, String, i64) = persistence.connection.query_row(
            "SELECT confirmation_state,display_state,capability_bits FROM file_states WHERE local_file_id='f-local'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(
            local,
            ("not_required".to_owned(), "local_ready".to_owned(), 0)
        );
        let cloud: (String, String) = persistence.connection.query_row(
            "SELECT confirmation_state,display_state FROM file_states WHERE local_file_id='f-cloud'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(
            cloud,
            ("pending".to_owned(), "confirmation_required".to_owned())
        );
        let history_count: i64 = persistence
            .connection
            .query_row(
                "SELECT count(*) FROM file_confirmations WHERE confirmation_id='old-confirmation'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(history_count, 1);
        let scopes: Vec<(String, String)> = {
            let mut statement = persistence.connection.prepare(
                "SELECT confirmation_id, workspace_kind_at_confirmation FROM file_confirmations ORDER BY confirmation_id"
            ).unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            scopes,
            [
                ("cloud-confirmation".to_owned(), "cloud_team".to_owned()),
                (
                    "converted-local-confirmation".to_owned(),
                    "local".to_owned()
                ),
                ("old-confirmation".to_owned(), "local".to_owned()),
            ]
        );
        let converted_active_cloud_confirmations: i64 = persistence.connection.query_row(
            "SELECT count(*) FROM file_confirmations WHERE local_file_id='f-converted' AND invalidated_at IS NULL AND workspace_kind_at_confirmation='cloud_team'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(converted_active_cloud_confirmations, 0);
        let candidate = persistence
            .transact(|repositories| {
                repositories.confirmation_candidate(&WorkspaceId::new("w-local"), "f-local")
            })
            .unwrap()
            .unwrap();
        assert!(candidate.confirmation_allowed);
    }

    #[test]
    fn legacy_unknown_cloud_outcome_is_read_only_and_preserves_actual_fields() {
        let fixture = TestDatabase::new("legacy-cloud-outcome");
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence.connection.execute_batch(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('legacy-project','历史项目','核对已有记录','active',0,'legacy-project-op','legacy-project-fingerprint',1,1);
             INSERT INTO cloud_workspace_operations(client_operation_id,request_fingerprint,operation_kind,project_id,display_name,managed_root,device_id,state,failure_reason,created_at,updated_at)
             VALUES ('legacy-unknown','legacy-fingerprint','create_empty','legacy-project','旧目录','/legacy/retained','legacy-device','creating_remote','remote_outcome_unknown',10,12);"
        ).unwrap();
        let before = persistence.connection.total_changes();
        let record = persistence
            .cloud_workspace_operation("legacy-unknown")
            .unwrap()
            .unwrap();
        assert_eq!(record.client_operation_id, "legacy-unknown");
        assert_eq!(record.request_fingerprint, "legacy-fingerprint");
        assert_eq!(record.project_id, "legacy-project");
        assert_eq!(record.operation_kind, "create_empty");
        assert_eq!(record.managed_root, "/legacy/retained");
        assert_eq!(record.device_id, "legacy-device");
        assert_eq!(record.state, "creating_remote");
        assert_eq!(
            record.failure_reason.as_deref(),
            Some("remote_outcome_unknown")
        );
        assert_eq!(record.created_at, 10);
        assert_eq!(record.updated_at, 12);
        assert!(record.workspace_id.is_none());
        assert!(record.remote_binding_id.is_none());
        assert!(
            persistence
                .cloud_workspace_operation("not-present")
                .unwrap()
                .is_none()
        );
        assert_eq!(persistence.connection.total_changes(), before);
    }

    #[test]
    fn active_cloud_bindings_cannot_reuse_a_managed_root_on_one_device() {
        let fixture = TestDatabase::new("cloud-workspace-managed-root");
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence
            .connection
            .execute_batch(
                "INSERT INTO accounts(account_id,display_name,account_status,created_at,updated_at)
                 VALUES ('account-cloud','成员','active',1,1);
                 INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at,workspace_kind)
                 VALUES ('workspace-cloud-1','团队一','/tmp/cloud-one','creating',0,1,1,'cloud_team'),
                        ('workspace-cloud-2','团队二','/tmp/cloud-two','creating',0,1,1,'cloud_team');
                 INSERT INTO workspace_platform_bindings(
                    workspace_id,account_id,platform_workspace_id,project_id,platform_root,binding_state,created_at,updated_at,
                    team_space_id,managed_root,device_id,permission_role)
                 VALUES ('workspace-cloud-1','account-cloud','team-1','project-1','/','binding',1,1,
                         'team-1','/tmp/managed-root','device-1','owner');",
            )
            .unwrap();
        assert!(
            persistence
                .connection
                .execute(
                    "INSERT INTO workspace_platform_bindings(
                        workspace_id,account_id,platform_workspace_id,project_id,platform_root,binding_state,created_at,updated_at,
                        team_space_id,managed_root,device_id,permission_role)
                     VALUES ('workspace-cloud-2','account-cloud','team-2','project-2','/','binding',1,1,
                             'team-2','/tmp/managed-root','device-1','editor')",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn archive_updates_only_metadata_and_rejects_stale_or_missing_targets() {
        let fixture = TestDatabase::new("archive-metadata");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let state = serde_json::json!({
            "conversations": [{"conversationId":"a","archived":false,"updatedAt":1}, {"conversationId":"b","archived":false,"updatedAt":1}],
            "messages":[{"messageId":"m","content":"正文".repeat(100_000)}], "runs":[]
        });
        persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
        persistence.connection.execute_batch("CREATE TRIGGER archive_payload_must_not_change BEFORE UPDATE ON ai_runtime_state BEGIN SELECT RAISE(ABORT,'payload rewritten'); END;").unwrap();
        let (revision, saved, changed) = persistence
            .set_ai_conversations_archived(1, &["a".to_owned()], true, 2)
            .unwrap();
        assert_eq!(revision, 2);
        assert_eq!(saved[0]["archived"], true);
        assert_eq!(changed, vec!["a"]);
        let (same_revision, same_saved, same_changed) = persistence
            .set_ai_conversations_archived(2, &["a".to_owned()], true, 3)
            .unwrap();
        assert_eq!(same_revision, 2);
        assert_eq!(same_saved, saved);
        assert!(same_changed.is_empty());
        let before = persistence.ai_runtime_state().unwrap();
        assert_eq!(before.1["messages"], state["messages"]);
        assert_eq!(
            persistence
                .set_ai_conversations_archived(1, &["b".to_owned()], true, 4)
                .unwrap_err()
                .reason(),
            Some("ai_state_revision_conflict")
        );
        assert!(
            persistence
                .set_ai_conversations_archived(2, &["b".to_owned(), "missing".to_owned()], true, 4)
                .is_err()
        );
        assert_eq!(persistence.ai_runtime_state().unwrap(), before);
        persistence
            .connection
            .execute_batch("DROP TRIGGER archive_payload_must_not_change;")
            .unwrap();
        assert_eq!(
            persistence
                .replace_ai_runtime_state(1, &state, 5)
                .unwrap_err()
                .reason(),
            Some("ai_state_revision_conflict")
        );
        let restored = persistence
            .set_ai_conversations_archived(2, &["a".to_owned()], false, 6)
            .unwrap();
        assert_eq!(restored.0, 3);
        drop(persistence);
        let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert_eq!(
            reopened.ai_runtime_state().unwrap().1["conversations"][0]["archived"],
            false
        );
    }

    #[test]
    fn conversation_metadata_migration_preserves_history_order_and_revision() {
        let fixture = TestDatabase::new("archive-migration");
        let connection = Connection::open(&fixture.path).unwrap();
        connection
            .execute_batch(include_str!(
                "../../../migrations/0014_add_ai_runtime_state.sql"
            ))
            .unwrap();
        let state = serde_json::json!({"conversations":[
            {"conversationId":"z","archived":true,"updatedAt":7,"context":{"projectId":"p"}},
            {"conversationId":"a","archived":false,"updatedAt":3}],
            "messages":[{"messageId":"m","content":"保留正文"}],"runs":[{"runId":"r","status":"completed"}]});
        connection
            .execute(
                "UPDATE ai_runtime_state SET state_revision=42,state_json=?1",
                [state.to_string()],
            )
            .unwrap();
        connection
            .execute_batch(include_str!(
                "../../../migrations/0034_separate_ai_conversation_metadata.sql"
            ))
            .unwrap();
        connection
            .execute_batch(include_str!(
                "../../../migrations/0038_separate_ai_messages_and_runs.sql"
            ))
            .unwrap();
        connection
            .execute_batch(include_str!(
                "../../../migrations/0055_add_ai_business_ledger.sql"
            ))
            .unwrap();
        let persistence = SqlitePersistence {
            connection,
            database_path: fixture.path.clone(),
            file_content_index: None,
        };
        assert_eq!(persistence.ai_runtime_state().unwrap(), (42, state));
        let payload: String = persistence
            .connection
            .query_row("SELECT state_json FROM ai_runtime_state", [], |row| {
                row.get(0)
            })
            .unwrap();
        let payload = serde_json::from_str::<serde_json::Value>(&payload).unwrap();
        assert!(payload.get("conversations").is_none());
        assert!(payload.get("messages").is_none());
        assert!(payload.get("runs").is_none());
    }

    fn ai_entities(
        upserts: Vec<(i64, serde_json::Value)>,
        removed: &[&str],
        total: i64,
    ) -> AiStateEntityChanges {
        AiStateEntityChanges {
            upserts,
            removed_ids: removed.iter().map(|id| (*id).to_owned()).collect(),
            total,
        }
    }

    #[test]
    fn ai_state_delta_rewrites_only_changed_entries() {
        let fixture = TestDatabase::new("ai-state-delta");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let state = serde_json::json!({
            "agents": [{"agentId": "agent-1"}],
            "conversations": [{"conversationId":"c","archived":false}],
            "messages": [
                {"messageId":"m1","conversationId":"c","role":"user","content":"正文".repeat(100_000)},
                {"messageId":"m2","conversationId":"c","role":"user","content":"第二条"}
            ],
            "runs": [{"runId":"r1","conversationId":"c","status":"completed"}]
        });
        persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
        // 大消息和其余字段都不应在增量保存里被改写
        persistence.connection.execute_batch(
            "CREATE TRIGGER payload_must_not_change BEFORE UPDATE ON ai_runtime_state BEGIN SELECT RAISE(ABORT,'payload rewritten'); END;
             CREATE TRIGGER long_message_must_not_change BEFORE UPDATE ON ai_messages WHEN old.message_id='m1' BEGIN SELECT RAISE(ABORT,'message rewritten'); END;",
        ).unwrap();
        let run = serde_json::json!({"runId":"r2","conversationId":"c","status":"running"});
        let message = serde_json::json!({"messageId":"m3","conversationId":"c","role":"user","content":"新消息"});
        let revision = persistence
            .apply_ai_runtime_changes(
                1,
                &AiStateChanges {
                    full: false,
                    payload: None,
                    conversations: ai_entities(Vec::new(), &[], 1),
                    messages: ai_entities(vec![(1, message.clone())], &["m2"], 2),
                    runs: ai_entities(vec![(1, run.clone())], &[], 2),
                },
                2,
            )
            .unwrap();
        assert_eq!(revision, 2);
        let (_, saved) = persistence.ai_runtime_state().unwrap();
        assert_eq!(saved["agents"], state["agents"]);
        assert_eq!(saved["messages"][0], state["messages"][0]);
        assert_eq!(saved["messages"][1], message);
        assert_eq!(saved["messages"].as_array().unwrap().len(), 2);
        assert_eq!(saved["runs"][1], run);
    }

    #[test]
    fn repeated_application_context_is_stored_once_and_read_back_whole() {
        let fixture = TestDatabase::new("ai-state-context-dedupe");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let context = serde_json::json!({
            "applicationId": "com.cilai.archive",
            "writeTools": [{"toolName": "save", "inputSchema": {"type": "object", "properties": {"x": {"type": "string"}}}}],
            "searchableApplications": [{"applicationId": "com.cilai.archive"}]
        });
        let state = serde_json::json!({
            "conversations": [], "runs": [],
            "messages": [
                {"messageId":"m1","role":"user","content":"一","applicationContext":context},
                {"messageId":"m2","role":"user","content":"二","applicationContext":context},
                {"messageId":"m3","role":"user","content":"三","applicationContext":null}
            ]
        });
        persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
        let contexts = |persistence: &SqlitePersistence| -> i64 {
            persistence
                .connection
                .query_row("SELECT count(*) FROM ai_application_contexts", [], |row| {
                    row.get(0)
                })
                .unwrap()
        };
        assert_eq!(contexts(&persistence), 1);
        assert_eq!(persistence.ai_runtime_state().unwrap(), (1, state.clone()));

        let without_context = serde_json::json!({
            "conversations": [], "runs": [],
            "messages": [state["messages"][2].clone()]
        });
        persistence
            .replace_ai_runtime_state(1, &without_context, 2)
            .unwrap();
        assert_eq!(contexts(&persistence), 0);

        let stored_form = serde_json::json!({
            "conversations": [], "runs": [],
            "messages": [{"messageId":"m4","role":"user","content":"四","applicationContextId":1}]
        });
        let rejected = persistence
            .replace_ai_runtime_state(2, &stored_form, 3)
            .unwrap_err();
        assert_eq!(rejected.reason(), Some("ai_state_invalid"));
    }

    #[test]
    fn context_migration_keeps_every_message_context_intact() {
        let fixture = TestDatabase::new("ai-context-migration");
        let connection = Connection::open(&fixture.path).unwrap();
        connection
            .execute_batch(include_str!(
                "../../../migrations/0014_add_ai_runtime_state.sql"
            ))
            .unwrap();
        let context = serde_json::json!({"applicationId":"a","writeTools":[{"toolName":"save"}]});
        let state = serde_json::json!({"conversations":[],"runs":[],"messages":[
            {"messageId":"m1","content":"一","applicationContext":context},
            {"messageId":"m2","content":"二","applicationContext":context},
            {"messageId":"m3","content":"三","applicationContext":null}]});
        connection
            .execute(
                "UPDATE ai_runtime_state SET state_json=?1",
                [state.to_string()],
            )
            .unwrap();
        for sql in [
            include_str!("../../../migrations/0034_separate_ai_conversation_metadata.sql"),
            include_str!("../../../migrations/0038_separate_ai_messages_and_runs.sql"),
            include_str!(
                "../../../migrations/0039_deduplicate_ai_message_application_contexts.sql"
            ),
        ] {
            connection.execute_batch(sql).unwrap();
        }
        connection
            .execute_batch(include_str!(
                "../../../migrations/0055_add_ai_business_ledger.sql"
            ))
            .unwrap();
        let persistence = SqlitePersistence {
            connection,
            database_path: fixture.path.clone(),
            file_content_index: None,
        };
        assert_eq!(persistence.ai_runtime_state().unwrap().1, state);
        let rows: i64 = persistence
            .connection
            .query_row("SELECT count(*) FROM ai_application_contexts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn ai_state_delta_rejects_a_baseline_that_differs_from_storage() {
        let fixture = TestDatabase::new("ai-state-delta-mismatch");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let state = serde_json::json!({
            "conversations": [], "runs": [],
            "messages": [{"messageId":"m1","role":"user","content":"正文"}]
        });
        persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
        let unchanged = |messages| AiStateChanges {
            full: false,
            payload: None,
            conversations: ai_entities(Vec::new(), &[], 0),
            messages,
            runs: ai_entities(Vec::new(), &[], 0),
        };
        let wrong_total = persistence
            .apply_ai_runtime_changes(1, &unchanged(ai_entities(Vec::new(), &[], 2)), 2)
            .unwrap_err();
        assert_eq!(wrong_total.reason(), Some("ai_state_delta_mismatch"));
        let missing_removal = persistence
            .apply_ai_runtime_changes(1, &unchanged(ai_entities(Vec::new(), &["absent"], 1)), 2)
            .unwrap_err();
        assert_eq!(missing_removal.reason(), Some("ai_state_delta_mismatch"));
        let mut embedded = unchanged(ai_entities(Vec::new(), &[], 1));
        embedded.payload = Some(serde_json::json!({"messages": []}));
        let embedded = persistence
            .apply_ai_runtime_changes(1, &embedded, 2)
            .unwrap_err();
        assert_eq!(embedded.reason(), Some("ai_state_invalid"));
        assert_eq!(persistence.ai_runtime_state().unwrap(), (1, state));
    }

    #[test]
    fn ai_state_delta_queues_skill_delivery_once_the_reply_is_saved() {
        let fixture = TestDatabase::new("ai-state-delta-skill");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let state = serde_json::json!({
            "conversations": [{"conversationId":"c","context":{"workspaceRoot":"/tmp/p"}}],
            "messages": [],
            "runs": [{"runId":"r","conversationId":"c","status":"completed","artifacts":[]}]
        });
        persistence.replace_ai_runtime_state(0, &state, 1).unwrap();
        assert!(persistence.skill_installation("r").unwrap().is_none());
        let reply = serde_json::json!({"messageId":"reply","conversationId":"c","runId":"r","role":"assistant","content":"技能"});
        persistence
            .apply_ai_runtime_changes(
                1,
                &AiStateChanges {
                    full: false,
                    payload: None,
                    conversations: ai_entities(Vec::new(), &[], 1),
                    messages: ai_entities(vec![(0, reply)], &[], 1),
                    runs: ai_entities(Vec::new(), &[], 1),
                },
                2,
            )
            .unwrap();
        let queued = persistence.skill_installation("r").unwrap().unwrap();
        assert_eq!(queued.status, "pending");
        let source: serde_json::Value = serde_json::from_str(&queued.source_json).unwrap();
        assert_eq!(source["message"]["messageId"], "reply");
        assert_eq!(source["workspaceRoot"], "/tmp/p");
        assert!(persistence.skill_source_run_available("r").unwrap());
    }

    #[test]
    fn ai_runtime_state_uses_revision_checked_atomic_replacement() {
        let fixture = TestDatabase::new("ai-runtime-state");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let (revision, initial) = persistence.ai_runtime_state().unwrap();
        assert_eq!(revision, 0);
        assert_eq!(initial["runs"], serde_json::json!([]));

        let updated = serde_json::json!({
            "agents": [{"agentId": "agent-1"}],
            "conversations": [],
            "messages": [],
            "runs": [],
            "permissionRequests": [],
            "inboxReadItemIds": ["run:failed-1"],
            "automationRules": []
        });
        let next_revision = persistence
            .replace_ai_runtime_state(revision, &updated, 100)
            .unwrap();
        assert_eq!(next_revision, 1);
        assert_eq!(
            persistence.ai_runtime_state().unwrap(),
            (1, updated.clone())
        );

        let conflict = persistence
            .replace_ai_runtime_state(revision, &updated, 101)
            .unwrap_err();
        assert_eq!(conflict.reason(), Some("ai_state_revision_conflict"));
        assert_eq!(persistence.ai_runtime_state().unwrap(), (1, updated));
    }

    #[test]
    fn lossy_history_replacement_rolls_back_every_record_and_revision() {
        let fixture = TestDatabase::new("text-integrity");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let clean = serde_json::json!({"conversations":[{"conversationId":"c","title":"测试技能"}],"messages":[],"runs":[{"runId":"r","conversationId":"c","artifacts":[{"path":"测试技能/蓝图.png"}]}]});
        persistence.replace_ai_runtime_state(0, &clean, 1).unwrap();
        let mut damaged = clean.clone();
        damaged["conversations"][0]["title"] = serde_json::json!("合法的另一个修改");
        damaged["runs"][0]["artifacts"][0]["path"] = serde_json::json!("测试��能/蓝图.png");
        assert_eq!(
            persistence
                .replace_ai_runtime_state(1, &damaged, 2)
                .unwrap_err()
                .reason(),
            Some("ai_text_corruption_detected")
        );
        assert_eq!(persistence.ai_runtime_state().unwrap(), (1, clean));
    }

    #[test]
    fn text_recovery_checks_source_and_atomically_preserves_business_identity() {
        let fixture = TestDatabase::new("text-recovery");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let source = persistence
            .ai_history("source", &serde_json::Value::Null, 0)
            .unwrap();
        persistence.ai_history("activate",&serde_json::json!({"sourceDigest":source["sourceDigest"],"ledger":{"commandId":"activate","records":[]},"legacyDeliveries":[]}),1).unwrap();
        persistence.ai_history("bind",&serde_json::json!({"conversationId":"c","nativeThreadId":"old-thread","title":"测试技能"}),2).unwrap();
        let record = serde_json::json!({"runId":"r","conversationId":"c","artifacts":[{"path":"测试��能/蓝图.png"}]});
        persistence.apply_ai_business_batch(&serde_json::json!({"commandId":"seed","records":[{"collection":"runs","id":"r","position":0,"expectedDigest":null,"value":record}]}),3).unwrap();
        persistence
            .connection
            .execute(
                "INSERT INTO ai_run_native_refs VALUES('r','old-thread','old-turn')",
                [],
            )
            .unwrap();
        let before = persistence.ai_business_ledger().unwrap();
        let hash = super::ai_business_ledger::digest(&before);
        let mut repaired = record.clone();
        repaired["artifacts"][0]["path"] = serde_json::json!("测试技能/蓝图.png");
        let request = serde_json::json!({"operationId":"repair","expectedLedgerDigest":hash,"ledger":{"commandId":"repair-ledger","records":[{"collection":"runs","id":"r","position":0,"expectedDigest":before["runs"][0]["digest"],"value":repaired}]},"transitions":[{"conversationId":"c","previousThreadId":"old-thread","nativeThreadId":"restored-thread","title":"测试技能","archived":false,"runs":[{"runId":"r","nativeTurnId":"r"}],"messages":[]}]});
        let mut stale = request.clone();
        stale["expectedLedgerDigest"] = serde_json::json!("stale");
        assert_eq!(
            persistence
                .ai_history("recoverText", &stale, 4)
                .unwrap_err()
                .reason(),
            Some("ai_text_recovery_source_changed")
        );
        let mut incomplete = request.clone();
        incomplete["transitions"][0]["runs"] = serde_json::json!([]);
        assert_eq!(
            persistence
                .ai_history("recoverText", &incomplete, 4)
                .unwrap_err()
                .reason(),
            Some("ai_text_recovery_incomplete")
        );
        assert_eq!(persistence.ai_business_ledger().unwrap(), before);
        persistence.ai_history("recoverText", &request, 5).unwrap();
        let after = persistence.ai_business_ledger().unwrap();
        assert_eq!(after["runs"][0]["id"], "r");
        assert_eq!(after["runs"][0]["value"], repaired);
        assert_eq!(after["runRefs"][0]["nativeThreadId"], "restored-thread");
        let mut lossy = record;
        lossy["artifacts"][0]["path"] = serde_json::json!("测试���能/蓝图.png");
        assert_eq!(persistence.apply_ai_business_batch(&serde_json::json!({"commandId":"lossy","records":[{"collection":"runs","id":"r","position":0,"expectedDigest":after["runs"][0]["digest"],"value":lossy}]}),6).unwrap_err().reason(),Some("ai_text_corruption_detected"));
        assert_eq!(persistence.ai_business_ledger().unwrap(), after);
    }

    #[test]
    fn native_cutover_keeps_business_transactions_after_removing_legacy_tables() {
        let fixture = TestDatabase::new("native-cutover");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let source = persistence
            .ai_history("source", &serde_json::Value::Null, 0)
            .unwrap();
        persistence.ai_history("activate",&serde_json::json!({"sourceDigest":source["sourceDigest"],"ledger":{"commandId":"activate","records":[]},"legacyDeliveries":[]}),1).unwrap();
        persistence.connection.execute_batch("DROP TABLE ai_conversations; DROP TABLE ai_messages; DROP TABLE ai_runs; DROP TABLE ai_runtime_state; DROP TABLE ai_runtime_revision;").unwrap();
        persistence.connection.execute("INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at) VALUES ('project-ai','AI 项目','验证归类','active',0,'op-project-ai','fp-project-ai',1,1)",[]).unwrap();
        persistence.ai_history("bind",&serde_json::json!({"conversationId":"conversation-ai","nativeThreadId":"native-thread","title":"业务核对"}),1).unwrap();
        let mut command = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
        let values = [
            (
                "conversations",
                "conversation-ai",
                serde_json::json!({"conversationId":"conversation-ai","context":{"projectId":"project-ai","workspaceRoot":"/tmp"},"updatedAt":2}),
            ),
            (
                "runs",
                "run-ai",
                serde_json::json!({"runId":"run-ai","conversationId":"conversation-ai","status":"completed","artifacts":[],"skillIds":[]}),
            ),
            (
                "messages",
                "reply-run-ai",
                serde_json::json!({"messageId":"reply-run-ai","conversationId":"conversation-ai","runId":"run-ai","role":"assistant","contentDigest":"body-digest","contentBytes":12}),
            ),
        ];
        command.ledger_request = Some(
            serde_json::json!({"commandId":"finalize-native","records":values.into_iter().map(|(collection,id,value)|serde_json::json!({"collection":collection,"id":id,"position":0,"value":value,"expectedDigest":null})).collect::<Vec<_>>(),"deliveries":[{"runId":"run-ai","messageId":"reply-run-ai","contentDigest":"body-digest","skillDelivery":{"Ok":null},"capabilityDelivery":{"Ok":null}}]}),
        );
        let result = persistence.finalize_ai_turn(&command).unwrap();
        assert_eq!(result.task_id.as_deref(), Some("task-ai"));
        assert!(persistence.finalize_ai_turn(&command).unwrap().replayed);
        let mut conflict = command.clone();
        conflict.ledger_request.as_mut().unwrap()["records"][0]["value"]["updatedAt"] =
            serde_json::json!(3);
        assert_eq!(
            persistence
                .finalize_ai_turn(&conflict)
                .unwrap_err()
                .reason(),
            Some("ai_ledger_command_conflict")
        );
        assert_eq!(
            persistence.ai_business_ledger().unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            persistence.ai_runtime_state().unwrap_err().reason(),
            Some("ai_legacy_storage_retired")
        );
        assert!(
            persistence
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM my_work_items WHERE task_id='task-ai')",
                    [],
                    |row| row.get::<_, bool>(0)
                )
                .unwrap()
        );
        let source: String = persistence
            .connection
            .query_row(
                "SELECT source_json FROM ai_skill_installations WHERE run_id='run-ai'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !serde_json::from_str::<serde_json::Value>(&source).unwrap()["message"]
                .as_object()
                .unwrap()
                .contains_key("content")
        );
        drop(persistence);
        let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert_eq!(
            reopened.ai_business_ledger().unwrap()["runs"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn ai_turn_finalization_atomically_creates_and_projects_a_phase_free_task() {
        let fixture = TestDatabase::new("ai-turn-finalization");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence
            .connection
            .execute(
                "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
                 VALUES ('project-ai','AI 项目','验证自动归类','active',0,'op-project-ai','fp-project-ai',1,1)",
                [],
            )
            .unwrap();
        let command = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
        let result = persistence.finalize_ai_turn(&command).unwrap();
        assert_eq!(result.state_revision, 1);
        assert_eq!(result.task_id.as_deref(), Some("task-ai"));
        assert_eq!(
            persistence.ai_runtime_state().unwrap(),
            (1, ai_turn_state("conversation-ai", "run-ai"))
        );
        let task_projection = persistence
            .connection
            .query_row(
                "SELECT task.phase_id,task.status,work.focus_group,work.action_type,task.assignee_type,task.assignee_label
                 FROM business_tasks task
                 JOIN my_work_items work ON work.task_id=task.business_task_id
                 WHERE task.business_task_id='task-ai'",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            task_projection,
            (
                None,
                "in_progress".to_owned(),
                "in_progress".to_owned(),
                "task_execute".to_owned(),
                "collaboration".to_owned(),
                "AI 对话协作".to_owned()
            )
        );
        assert_eq!(
            persistence
                .ai_work_task_candidates("project-ai")
                .unwrap()
                .len(),
            1
        );
        let contributions = persistence
            .transact(|repositories| repositories.business_task_contributions("task-ai"))
            .unwrap();
        assert_eq!(contributions.len(), 1);
        assert_eq!(contributions[0].run_id, "run-ai");
        assert_eq!(contributions[0].conversation_id, "conversation-ai");
        assert_eq!(contributions[0].stage.as_deref(), Some("in_progress"));
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "失败投影及后续恢复必须在同一份运行状态中完整验证"
    )]
    fn failed_tool_call_blocks_the_classified_task_until_a_later_success() {
        let fixture = TestDatabase::new("ai-tool-failure-incident");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence
            .connection
            .execute(
                "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
                 VALUES ('project-ai','AI 项目','验证工具失败归类','active',0,'op-project-ai','fp-project-ai',1,1)",
                [],
            )
            .unwrap();
        let mut failed = ai_create_task_command("conversation-ai", "run-failed", "task-ai", 0);
        let failed_state = serde_json::json!({
            "agents":[],
            "conversations":[{
                "conversationId":"conversation-ai",
                "title":"生成产品配图",
                "archived":false,
                "context":{"projectId":"project-ai","taskId":"task-ai"}
            }],
            "messages":[{
                "messageId":"reply-failed",
                "conversationId":"conversation-ai",
                "runId":"run-failed",
                "role":"assistant",
                "content":"生图失败"
            }],
            "runs":[{
                "runId":"run-failed",
                "conversationId":"conversation-ai",
                "status":"completed",
                "model":"agnes-3.0-flash",
                "events":[{
                    "sequence":13,
                    "eventType":"item.completed",
                    "phase":"reply",
                    "kind":"mcp",
                    "status":"failed",
                    "summary":"generate_image 调用失败"
                }]
            }]
        });
        with_ai_state(&mut failed, &failed_state);
        persistence.finalize_ai_turn(&failed).unwrap();

        let incident: (String, String, String) = persistence
            .connection
            .query_row(
                "SELECT status,task_id,detail FROM attention_cases WHERE agent_run_id='run-failed'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            incident,
            (
                "open".to_owned(),
                "task-ai".to_owned(),
                "generate_image 调用失败".to_owned()
            )
        );
        let blocked = my_work_items(&mut persistence)
            .into_iter()
            .find(|item| item.task_id.as_deref() == Some("task-ai"))
            .unwrap();
        assert_eq!(blocked.focus_group, "todo");
        assert_eq!(blocked.blocking_run_id.as_deref(), Some("run-failed"));
        assert_eq!(
            blocked.blocking_reason.as_deref(),
            Some("generate_image 调用失败")
        );

        let mut recovered = failed.clone();
        recovered.expected_revision = 1;
        recovered.run_id = "run-recovered".to_owned();
        recovered.new_task_id = None;
        recovered.previous_task_id = Some("task-ai".to_owned());
        recovered.occurred_at = 20;
        recovered.classification.action = "keep".to_owned();
        recovered.classification.task_action = "keep".to_owned();
        recovered.classification.task_id = Some("task-ai".to_owned());
        recovered.classification.objective = None;
        recovered.classification.expected_output = None;
        recovered.classification.completion_criteria = None;
        recovered.classification.raw = serde_json::json!({"run":"recovered"});
        let mut recovered_state = failed_state;
        recovered_state["messages"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "messageId":"reply-recovered",
                "conversationId":"conversation-ai",
                "runId":"run-recovered",
                "role":"assistant",
                "content":"生图成功"
            }));
        recovered_state["runs"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "runId":"run-recovered",
                "conversationId":"conversation-ai",
                "status":"completed",
                "model":"agnes-3.0-flash",
                "events":[{
                    "sequence":8,
                    "eventType":"item.completed",
                    "phase":"reply",
                    "kind":"mcp",
                    "status":"completed",
                    "summary":"已使用 generate_image"
                }]
            }));
        with_ai_state(&mut recovered, &recovered_state);
        persistence.finalize_ai_turn(&recovered).unwrap();

        let resolved: (String, Option<i64>) = persistence
            .connection
            .query_row(
                "SELECT status,resolved_at FROM attention_cases WHERE agent_run_id='run-failed'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(resolved, ("resolved".to_owned(), Some(20)));
        let unblocked = my_work_items(&mut persistence)
            .into_iter()
            .find(|item| item.task_id.as_deref() == Some("task-ai"))
            .unwrap();
        assert_eq!(unblocked.focus_group, "in_progress");
        assert!(unblocked.blocking_run_id.is_none());
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "归档、恢复与再次归类需在同一份数据上依次验证我的工作投影"
    )]
    fn archived_ai_conversations_leave_my_work_without_changing_task_history() {
        let fixture = TestDatabase::new("archived-ai-my-work");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence.connection.execute_batch(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-ai','AI 项目','验证归档','active',0,'op-project-ai','fp-project-ai',1,1);",
        ).unwrap();
        let mut first = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
        first.classification.stage = Some("decision".to_owned());
        first.classification.raw["stage"] = serde_json::json!("decision");
        let mut first_state = ai_turn_state("conversation-ai", "run-ai");
        first_state["conversations"][0]["archived"] = serde_json::json!(false);
        first_state["runs"][0]["conversationId"] = serde_json::json!("conversation-ai");
        with_ai_state(&mut first, &first_state);
        persistence.finalize_ai_turn(&first).unwrap();
        persistence.connection.execute_batch(
            "INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
             VALUES ('task-manual','project-ai',NULL,'人工任务','目标','','产出','标准','user','员工','medium',NULL,'todo',0,'manual-task-create','fp-manual',2,2,1);",
        ).unwrap();
        assert_eq!(my_work_items(&mut persistence).len(), 2);
        assert_eq!(
            my_work_items(&mut persistence)
                .iter()
                .find(|item| item.task_id.as_deref() == Some("task-ai"))
                .unwrap()
                .focus_group,
            "decision"
        );

        let mut archived = first_state.clone();
        archived["conversations"][0]["archived"] = serde_json::json!(true);
        persistence
            .replace_ai_runtime_state(1, &archived, 11)
            .unwrap();
        let query = MyWorkQuery {
            project_id: None,
            focus_group: None,
            limit: 200,
        };
        let archived_work = persistence
            .transact(|repositories| contexthub_application::get_my_work(repositories, &query))
            .unwrap();
        assert_eq!(archived_work.total, 1);
        assert_eq!(
            archived_work.items[0].task_id.as_deref(),
            Some("task-manual")
        );
        assert_eq!(
            persistence
                .connection
                .query_row(
                    "SELECT status FROM business_tasks WHERE business_task_id='task-ai'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "decision"
        );
        assert_eq!(
            persistence
                .transact(|repositories| repositories.business_task_contributions("task-ai"))
                .unwrap()
                .len(),
            1
        );

        let mut restored = archived.clone();
        restored["conversations"][0]["archived"] = serde_json::json!(false);
        persistence
            .replace_ai_runtime_state(2, &restored, 12)
            .unwrap();
        assert_eq!(my_work_items(&mut persistence).len(), 2);

        let mut second = first.clone();
        second.expected_revision = 3;
        second.conversation_id = "conversation-later".to_owned();
        second.run_id = "run-later".to_owned();
        second.new_task_id = None;
        second.previous_task_id = None;
        second.occurred_at = 13;
        second.classification.action = "apply".to_owned();
        second.classification.task_action = "link".to_owned();
        second.classification.task_id = Some("task-ai".to_owned());
        second.classification.objective = None;
        second.classification.expected_output = None;
        second.classification.completion_criteria = None;
        second.classification.stage = Some("todo".to_owned());
        second.classification.raw = serde_json::json!({"run": "later"});
        let mut second_state = first_state.clone();
        second_state["conversations"][0]["archived"] = serde_json::json!(true);
        second_state["conversations"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"conversationId":"conversation-later","archived":false}));
        second_state["runs"].as_array_mut().unwrap().push(serde_json::json!({"runId":"run-later","conversationId":"conversation-later","status":"completed"}));
        with_ai_state(&mut second, &second_state);
        persistence.finalize_ai_turn(&second).unwrap();
        persistence.connection.execute_batch(
            "INSERT INTO attention_cases(attention_case_id,case_type,project_id,task_id,title,detail,status,responsibility_type,responsibility_label,reporter_label,priority,agent_run_id,agent_label,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('incident-ai','agent_run_incident','project-ai','task-ai','旧运行异常','历史失败','open','user','员工','desktop','medium','run-ai','Codex',0,'op-incident-ai','fp-incident-ai',14,14);",
        ).unwrap();
        let visible = my_work_items(&mut persistence);
        assert_eq!(visible.len(), 2);
        let ai_item = visible
            .iter()
            .find(|item| item.task_id.as_deref() == Some("task-ai"))
            .unwrap();
        assert_eq!(ai_item.focus_group, "todo");
        assert!(ai_item.blocking_run_id.is_none());
        let decision_query = MyWorkQuery {
            project_id: None,
            focus_group: Some("decision".to_owned()),
            limit: 200,
        };
        let decision_work = persistence
            .transact(|repositories| {
                contexthub_application::get_my_work(repositories, &decision_query)
            })
            .unwrap();
        assert_eq!(decision_work.total, 0);
        assert!(decision_work.items.is_empty());
    }

    #[test]
    fn ai_turn_finalization_rejects_a_duplicate_project_task() {
        let fixture = TestDatabase::new("ai-turn-duplicate-task");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence
            .connection
            .execute(
                "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
                 VALUES ('project-ai','AI 项目','验证自动归类','active',0,'op-project-ai','fp-project-ai',1,1)",
                [],
            )
            .unwrap();
        let first = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
        persistence.finalize_ai_turn(&first).unwrap();
        let mut duplicate =
            ai_create_task_command("conversation-ai-2", "run-ai-2", "task-ai-duplicate", 1);
        duplicate.occurred_at = 11;
        assert_eq!(
            persistence
                .finalize_ai_turn(&duplicate)
                .unwrap_err()
                .reason(),
            Some("ai_work_task_duplicate")
        );
        assert_eq!(
            persistence
                .ai_work_task_candidates("project-ai")
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn completed_conversations_close_and_later_work_reactivates_the_same_task() {
        let fixture = TestDatabase::new("ai-turn-completion-reactivation");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence
            .connection
            .execute(
                "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
                 VALUES ('project-ai','AI 项目','验证自动归类','active',0,'op-project-ai','fp-project-ai',1,1)",
                [],
            )
            .unwrap();
        let first = ai_create_task_command("conversation-ai", "run-ai", "task-ai", 0);
        persistence.finalize_ai_turn(&first).unwrap();

        let mut completed = first.clone();
        completed.expected_revision = 1;
        completed.run_id = "run-ai-completed".to_owned();
        completed.new_task_id = None;
        completed.previous_task_id = Some("task-ai".to_owned());
        completed.occurred_at = 11;
        completed.classification = AiTurnClassificationRecord {
            action: "keep".to_owned(),
            task_action: "keep".to_owned(),
            task_id: Some("task-ai".to_owned()),
            task_title: Some("自动归类对话".to_owned()),
            objective: None,
            expected_output: None,
            completion_criteria: None,
            conversation_status: Some("completed".to_owned()),
            stage: None,
            outcome_title: Some("本轮工作完成".to_owned()),
            outcome_summary: Some("当前对话不再需要后续处理".to_owned()),
            reason: "本轮已经结束".to_owned(),
            confidence: 0.98,
            raw: serde_json::json!({"run": "completed"}),
        };
        persistence.finalize_ai_turn(&completed).unwrap();
        let closed = persistence
            .connection
            .query_row(
                "SELECT status,attempt_no,EXISTS(SELECT 1 FROM my_work_items WHERE task_id='task-ai') FROM business_tasks WHERE business_task_id='task-ai'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, bool>(2)?)),
            )
            .unwrap();
        assert_eq!(closed, ("completed".to_owned(), 1, false));
        assert_eq!(
            persistence.ai_work_task_candidates("project-ai").unwrap()[0].stage,
            "completed"
        );

        let mut reactivated = completed.clone();
        reactivated.expected_revision = 2;
        reactivated.conversation_id = "conversation-ai-later".to_owned();
        reactivated.run_id = "run-ai-later".to_owned();
        reactivated.previous_task_id = None;
        reactivated.occurred_at = 12;
        reactivated.classification.action = "apply".to_owned();
        reactivated.classification.task_action = "link".to_owned();
        reactivated.classification.conversation_status = Some("active".to_owned());
        reactivated.classification.stage = Some("todo".to_owned());
        reactivated.classification.outcome_title = Some("发现后续问题".to_owned());
        reactivated.classification.outcome_summary = Some("同一任务需要继续处理".to_owned());
        reactivated.classification.raw = serde_json::json!({"run": "reactivated"});
        persistence.finalize_ai_turn(&reactivated).unwrap();
        let reopened = persistence
            .connection
            .query_row(
                "SELECT status,attempt_no,EXISTS(SELECT 1 FROM my_work_items WHERE task_id='task-ai') FROM business_tasks WHERE business_task_id='task-ai'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, bool>(2)?)),
            )
            .unwrap();
        assert_eq!(reopened, ("todo".to_owned(), 2, true));
    }

    #[test]
    fn version_twenty_four_separates_ai_model_from_task_responsibility() {
        let fixture = TestDatabase::new("upgrade-v24-task-responsibility");
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence.connection.execute_batch(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-ai','AI 项目','验证责任迁移','active',0,'op-project-ai','fp-project-ai',1,1);
             INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
             VALUES ('task-auto','project-ai',NULL,'自动任务','目标','','产出','标准','agent','Qwen3.6','medium',NULL,'in_progress',0,'ai-task-create:run-1','fp-auto',2,2,1);
             INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
             VALUES ('task-manual','project-ai',NULL,'人工任务','目标','','产出','标准','agent','研发 Agent','medium',NULL,'in_progress',0,'manual-task-create','fp-manual',2,2,1);",
        ).unwrap();

        persistence
            .connection
            .execute_batch(MIGRATIONS[23].sql)
            .unwrap();

        let responsibility = |task_id: &str| {
            persistence
                .connection
                .query_row(
                    "SELECT task.assignee_type,task.assignee_label,work.responsibility_type,work.responsibility_label
                     FROM business_tasks task JOIN my_work_items work ON work.task_id=task.business_task_id
                     WHERE task.business_task_id=?1",
                    [task_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?)),
                )
                .unwrap()
        };
        assert_eq!(
            responsibility("task-auto"),
            (
                "collaboration".to_owned(),
                "AI 对话协作".to_owned(),
                "collaboration".to_owned(),
                "AI 对话协作".to_owned(),
            )
        );
        assert_eq!(
            responsibility("task-manual"),
            (
                "agent".to_owned(),
                "研发 Agent".to_owned(),
                "agent".to_owned(),
                "研发 Agent".to_owned(),
            )
        );
    }

    #[test]
    fn version_twenty_six_resolves_only_frontend_inferred_run_incidents() {
        let fixture = TestDatabase::new("upgrade-v26-run-incidents");
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in &MIGRATIONS[..25] {
            apply_fixture_migration(&connection, migration);
            connection
                .execute(
                    "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                    params![migration.version, migration.name, migration_checksum(migration.sql)],
                )
                .unwrap();
        }
        connection.execute_batch(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-ai','AI 项目','验证运行异常归属','active',0,'op-project-ai','fp-project-ai',1,1);
             INSERT INTO attention_cases(attention_case_id,case_type,project_id,title,detail,status,responsibility_type,responsibility_label,reporter_label,priority,agent_run_id,agent_label,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('frontend-incident','agent_run_incident','project-ai','历史失败','容量不足','open','user','user','desktop','medium','run-frontend','Codex',0,'desktop-agent-incident-run-frontend','fp-frontend',2,2);
             INSERT INTO attention_cases(attention_case_id,case_type,project_id,title,detail,status,responsibility_type,responsibility_label,reporter_label,priority,agent_run_id,agent_label,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('runtime-incident','agent_run_incident','project-ai','任务重试失败','运行失败','open','user','user','desktop','medium','run-runtime','Codex',0,'desktop-run-incident-run-runtime','fp-runtime',3,3);",
        ).unwrap();
        drop(connection);

        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let status = |case_id: &str| {
            persistence
                .connection
                .query_row(
                    "SELECT status,resolution_summary FROM attention_cases WHERE attention_case_id=?1",
                    [case_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .unwrap()
        };
        let frontend = status("frontend-incident");
        assert_eq!(frontend.0, "resolved");
        assert!(frontend.1.unwrap().contains("不再作为任务阻塞依据"));
        assert_eq!(status("runtime-incident"), ("open".to_owned(), None));
        let transition_count: i64 = persistence
            .connection
            .query_row(
                "SELECT COUNT(*) FROM attention_case_transitions WHERE attention_case_id='frontend-incident'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(transition_count, 1);
    }

    #[test]
    fn upgrading_legacy_classification_failure_preserves_history_without_work_item() {
        let fixture = TestDatabase::new("upgrade-v41-classification-incident");
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in &MIGRATIONS[..40] {
            apply_fixture_migration(&connection, migration);
            connection
                .execute(
                    "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                    params![migration.version, migration.name, migration_checksum(migration.sql)],
                )
                .unwrap();
        }
        connection.execute_batch(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-ai','图片项目','验证归类失败投影','active',0,'op-project-ai','fp-project-ai',1,1);",
        ).unwrap();
        let conversation = serde_json::json!({
            "conversationId":"conversation-ai",
            "title":"生成产品配图",
            "archived":false,
            "context":{"projectId":"project-ai","phaseId":"removed-phase","taskId":null}
        });
        let run = serde_json::json!({
            "runId":"run-classification-failed",
            "conversationId":"conversation-ai",
            "inputMessageId":"message-ai",
            "status":"completed",
            "classificationStatus":"failed",
            "classificationError":"工作归类输出未完成",
            "startedAt":10,
            "completedAt":20
        });
        connection
            .execute(
                "INSERT INTO ai_conversations(conversation_id,position,conversation_json) VALUES ('conversation-ai',0,?1)",
                [conversation.to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ai_runs(run_id,position,run_json) VALUES ('run-classification-failed',0,?1)",
                [run.to_string()],
            )
            .unwrap();
        drop(connection);

        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert!(my_work_items(&mut persistence).is_empty());
        let incident: (String, Option<String>) = persistence
            .connection
            .query_row(
                "SELECT status,task_id FROM attention_cases WHERE agent_run_id='run-classification-failed'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(incident, ("open".to_owned(), None));
    }

    #[test]
    fn version_forty_two_projects_existing_failed_tool_call_onto_its_task() {
        let fixture = TestDatabase::new("upgrade-v42-tool-failure-incident");
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in &MIGRATIONS[..41] {
            apply_fixture_migration(&connection, migration);
            connection
                .execute(
                    "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                    params![migration.version, migration.name, migration_checksum(migration.sql)],
                )
                .unwrap();
        }
        connection.execute_batch(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-ai','图片项目','验证工具失败投影','active',0,'op-project-ai','fp-project-ai',1,1);
             INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
             VALUES ('task-ai','project-ai',NULL,'创建生图技能','目标','','产出','标准','collaboration','AI 对话协作','medium',NULL,'in_progress',0,'ai-task-create:run-failed','fp-task',2,2,1);",
        ).unwrap();
        let conversation = serde_json::json!({
            "conversationId":"conversation-ai",
            "title":"创建生图技能",
            "archived":false,
            "context":{"projectId":"project-ai","taskId":"task-ai"}
        });
        let run = serde_json::json!({
            "runId":"run-failed",
            "conversationId":"conversation-ai",
            "status":"completed",
            "model":"agnes-3.0-flash",
            "completedAt":20,
            "events":[{
                "sequence":13,
                "eventType":"item.completed",
                "phase":"reply",
                "kind":"mcp",
                "status":"failed",
                "summary":"generate_image 调用失败"
            }]
        });
        connection
            .execute(
                "INSERT INTO ai_conversations(conversation_id,position,conversation_json) VALUES ('conversation-ai',0,?1)",
                [conversation.to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ai_runs(run_id,position,run_json) VALUES ('run-failed',0,?1)",
                [run.to_string()],
            )
            .unwrap();
        connection.execute(
            "INSERT INTO conversation_work_classifications(
                classification_id,run_id,conversation_id,project_id,business_task_id,
                classification_action,task_action,conversation_status,stage,outcome_title,
                outcome_summary,reason,confidence,raw_json,created_at
             ) VALUES ('classification:run-failed','run-failed','conversation-ai','project-ai','task-ai',
                       'keep','keep','active','in_progress','生图失败','需要处理','生图工具失败',0.9,'{}',20)",
            [],
        ).unwrap();
        drop(connection);

        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let incident: (String, String, String) = persistence
            .connection
            .query_row(
                "SELECT status,task_id,detail FROM attention_cases WHERE agent_run_id='run-failed'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            incident,
            (
                "open".to_owned(),
                "task-ai".to_owned(),
                "generate_image 调用失败".to_owned()
            )
        );
        let item = my_work_items(&mut persistence)
            .into_iter()
            .find(|item| item.task_id.as_deref() == Some("task-ai"))
            .unwrap();
        assert_eq!(item.focus_group, "todo");
        assert_eq!(item.blocking_run_id.as_deref(), Some("run-failed"));
        assert_eq!(
            item.blocking_reason.as_deref(),
            Some("generate_image 调用失败")
        );
    }

    fn seed_version_forty_four_shared_workspace(path: &Path) -> i64 {
        let connection = Connection::open(path).unwrap();
        for migration in &MIGRATIONS[..44] {
            apply_fixture_migration(&connection, migration);
            connection.execute(
                "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                params![migration.version, migration.name, migration_checksum(migration.sql)],
            ).unwrap();
        }
        connection.execute_batch(
            "INSERT INTO workspaces(workspace_id,display_name,local_root,local_status,state_revision,created_at,updated_at)
             VALUES ('shared','共享目录','/tmp/shared','ready',0,1,1);
             INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('a','甲','验收','active',0,'a','a',1,1),('b','乙','验收','active',0,'b','b',1,1);
             INSERT INTO project_workspaces VALUES ('a','shared','primary',1);
             INSERT INTO direct_cloud_workspace_operations VALUES ('direct','direct','a','team','shared',1);
             INSERT INTO cloud_workspace_operations(client_operation_id,request_fingerprint,operation_kind,project_id,team_space_id,display_name,managed_root,device_id,state,workspace_id,created_at,updated_at)
             VALUES ('legacy','legacy','attach_existing','a','team','空间','/tmp/shared','device','bound','shared',1,1);
             INSERT INTO cloud_workspace_transfer_entries(client_operation_id,local_relative_path,remote_relative_path,entry_kind,size_bytes,checksum_sha256,transfer_state,updated_at)
             VALUES ('legacy','keep.txt','keep.txt','file',4,'hash','completed',1);"
        ).unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO project_workspaces VALUES ('b','shared','primary',2)",
                    []
                )
                .is_err()
        );
        let triggers: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='trigger'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        triggers
    }

    #[test]
    fn version_forty_four_links_upgrade_to_shared_workspaces_and_keep_operation_history() {
        let fixture = TestDatabase::new("upgrade-v44-shared-workspaces");
        let triggers = seed_version_forty_four_shared_workspace(&fixture.path);
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let connection = &persistence.connection;
        connection
            .execute(
                "INSERT INTO project_workspaces VALUES ('b','shared','primary',2)",
                [],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO project_workspaces VALUES ('b','shared','supporting',3)",
                    []
                )
                .is_err()
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='trigger'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            triggers
        );
        assert_eq!(
            connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("PRAGMA legacy_alter_table", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(connection.query_row("SELECT count(*) FROM cloud_workspace_transfer_entries WHERE client_operation_id='legacy'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        let delete = DeleteProjectCommand {
            project_id: "a".to_owned(),
            client_operation_id: "delete-a".to_owned(),
            request_fingerprint: "delete-a".to_owned(),
            event_id: EventId::new("delete-a"),
            correlation_id: "delete-a".to_owned(),
            occurred_at: 3,
        };
        persistence
            .transact(|repositories| repositories.delete_project(&delete))
            .unwrap();
        assert_eq!(
            persistence
                .transact(
                    |repositories| repositories.projects_for_workspace(&WorkspaceId::new("shared"))
                )
                .unwrap()[0]
                .project_id,
            "b"
        );
        assert_eq!(
            persistence
                .connection
                .query_row(
                    "SELECT count(*) FROM direct_cloud_workspace_operations",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            persistence
                .cloud_workspace_operation("legacy")
                .unwrap()
                .unwrap()
                .project_id,
            "a"
        );
        drop(persistence);
        SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
    }

    #[test]
    fn version_eleven_actions_are_rebuilt_into_my_work_projection() {
        let fixture = TestDatabase::new("upgrade-v11-my-work");
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in &MIGRATIONS[..11] {
            apply_fixture_migration(&connection, migration);
            connection
                .execute(
                    "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                    params![migration.version, migration.name, migration_checksum(migration.sql)],
                )
                .unwrap();
        }
        connection.execute_batch(
            "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
             VALUES ('w-upgrade','升级工作区','/tmp/w-upgrade',NULL,'ready',0,NULL,1,1);
             INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('p-upgrade','升级项目','验证投影','active',0,'op-p-upgrade','fp-p-upgrade',1,1);
             INSERT INTO project_workspaces(project_id,workspace_id,role,created_at)
             VALUES ('p-upgrade','w-upgrade','primary',1);
             INSERT INTO deliverables(deliverable_id,project_id,title,deliverable_type,owner_label,acceptance_criteria,status,current_accepted_version_id,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('d-upgrade','p-upgrade','升级成果','document','Mika','可验收','candidate',NULL,1,'op-d-upgrade','fp-d-upgrade',2,2);
             INSERT INTO deliverable_versions(version_id,deliverable_id,version_no,status,summary,client_operation_id,request_fingerprint,created_at)
             VALUES ('dv-upgrade','d-upgrade',1,'candidate','候选','op-dv-upgrade','fp-dv-upgrade',3);
             INSERT INTO phase_plans(plan_id,project_id,version_no,status,summary,state_revision,client_operation_id,request_fingerprint,created_at,activated_at)
             VALUES ('pp-upgrade','p-upgrade',1,'active','升级计划',1,'op-pp-upgrade','fp-pp-upgrade',4,4);
             INSERT INTO project_phases(phase_id,plan_id,project_id,name,goal,acceptance_criteria,required_deliverable_id,sort_order,is_main,status,state_revision,created_at,updated_at)
             VALUES ('ph-upgrade','pp-upgrade','p-upgrade','升级阶段','形成成果','通过验收','d-upgrade',0,1,'review',2,4,6);
             INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('bt-upgrade','p-upgrade','ph-upgrade','升级任务','验证','上下文','结果','完成','user','Mika','high',100,'in_progress',2,'op-bt-upgrade','fp-bt-upgrade',5,7);",
        ).unwrap();
        drop(connection);

        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let items = my_work_items(&mut persistence);
        assert_eq!(items.len(), 3);
        assert!(
            items.iter().any(
                |item| item.source_type == "business_task" && item.focus_group == "in_progress"
            )
        );
        assert!(
            items
                .iter()
                .any(|item| item.source_type == "deliverable_version"
                    && item.focus_group == "review")
        );
        assert!(
            items
                .iter()
                .any(|item| item.source_type == "project_phase" && item.focus_group == "decision")
        );
        assert!(
            items
                .iter()
                .all(|item| item.primary_workspace_id.as_deref() == Some("w-upgrade"))
        );
    }

    #[test]
    fn existing_version_59_database_preserves_context_and_memory_on_reopen() {
        let fixture = TestDatabase::new("existing-v59");
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 58)
        {
            apply_fixture_migration(&connection, migration);
            connection
                .execute(
                    "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (?1, ?2, ?3, 1, '0.1.0')",
                    params![migration.version, migration.name, migration_checksum(migration.sql)],
                )
                .unwrap();
        }
        // 模拟已经由先前客户端写入的第 59 版资料库，保留原迁移记录与数据。
        let existing = Migration {
            version: 59,
            name: "add_workbench_context_imports",
            sql: include_str!("../../../migrations/0059_add_workbench_context_imports.sql"),
            rebuilds_foreign_key_parent: false,
        };
        assert_eq!(
            migration_checksum(existing.sql),
            "46b9783b457fb519cdcb740223f9ee30a2dc494ea83a83a376e37d19bd2c10bd"
        );
        apply_fixture_migration(&connection, &existing);
        connection
            .execute(
                "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (59, ?1, ?2, 123, 'previous-client')",
                params![existing.name, migration_checksum(existing.sql)],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ai_context_imports VALUES ('workbench', 'thread-fixture', 'digest-fixture', '{\"source\":\"fixture\"}', 'committed', 456)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ai_memory_state VALUES ('account', 7, '{\"memory\":\"fixture\"}')",
                [],
            )
            .unwrap();
        drop(connection);

        for _ in 0..2 {
            let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
            let history: (i64, String) = persistence
                .connection
                .query_row(
                    "SELECT applied_at, app_version FROM schema_migrations WHERE version = 59",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(history, (123, "previous-client".to_owned()));
            let context: (String, String, i64) = persistence
                .connection
                .query_row(
                    "SELECT source_json, status, updated_at FROM ai_context_imports WHERE scope = 'workbench' AND thread_id = 'thread-fixture'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(
                context,
                (
                    "{\"source\":\"fixture\"}".to_owned(),
                    "committed".to_owned(),
                    456
                )
            );
            let memory: (i64, String) = persistence
                .connection
                .query_row(
                    "SELECT revision, value_json FROM ai_memory_state WHERE state_key = 'account'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(memory, (7, "{\"memory\":\"fixture\"}".to_owned()));
        }
    }

    #[test]
    fn database_with_future_migration_is_rejected_without_removing_history() {
        let fixture = TestDatabase::new("future-migration");
        drop(SqlitePersistence::open(&fixture.path, "0.1.0").unwrap());
        let future_version = MIGRATIONS.last().unwrap().version + 1;
        let connection = Connection::open(&fixture.path).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (?1, 'future-migration', ?2, 456, 'future-client')",
                params![future_version, "0".repeat(64)],
            )
            .unwrap();
        drop(connection);
        let error = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap_err();
        assert_eq!(error.kind(), PersistenceErrorKind::Migration);
        assert!(error.to_string().contains("newer than this App Server"));
        let connection = Connection::open(&fixture.path).unwrap();
        let retained_version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(retained_version, future_version);
    }

    #[test]
    fn reopening_is_idempotent_and_changed_history_is_rejected() {
        let fixture = TestDatabase::new("history");
        drop(SqlitePersistence::open(&fixture.path, "0.1.0").unwrap());
        drop(SqlitePersistence::open(&fixture.path, "0.1.0").unwrap());

        let connection = Connection::open(&fixture.path).unwrap();
        connection
            .execute(
                "UPDATE schema_migrations SET checksum = ?1 WHERE version = 1",
                ["0".repeat(64)],
            )
            .unwrap();
        drop(connection);
        let error = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap_err();
        assert_eq!(error.kind(), PersistenceErrorKind::Migration);
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn version_one_database_upgrades_to_core_schema_without_reapplying_history() {
        let fixture = TestDatabase::new("upgrade-v1");
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        connection.execute_batch(MIGRATIONS[0].sql).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (1, ?1, ?2, 1, '0.1.0')",
                params![MIGRATIONS[0].name, migration_checksum(MIGRATIONS[0].sql)],
            )
            .unwrap();
        drop(connection);

        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let versions: String = persistence
            .connection
            .query_row(
                "SELECT group_concat(version, ',') FROM (SELECT version FROM schema_migrations ORDER BY version)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let expected_versions = MIGRATIONS
            .iter()
            .map(|migration| migration.version.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(versions, expected_versions);
        let account_table_exists: bool = persistence
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='accounts')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(account_table_exists);
    }

    #[test]
    fn deleting_a_project_removes_process_data_releases_workspaces_and_keeps_history() {
        let fixture = TestDatabase::new("project-delete");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_project_outcome_file(&mut persistence);

        let create = CreateProjectCommand {
            project_id: "project-delete-1".to_owned(),
            display_name: "待删除项目".to_owned(),
            goal: "验证删除范围".to_owned(),
            primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
            client_operation_id: "op-delete-project".to_owned(),
            request_fingerprint: "fp-delete-project".to_owned(),
            event_id: EventId::new("event-delete-project"),
            correlation_id: "correlation-delete-project".to_owned(),
            occurred_at: 3,
        };
        persistence
            .transact(|repositories| contexthub_application::create_project(repositories, &create))
            .unwrap();

        let events_before: i64 = persistence
            .connection
            .query_row("SELECT COUNT(*) FROM durable_events", [], |row| row.get(0))
            .unwrap();

        let delete = DeleteProjectCommand {
            project_id: "project-delete-1".to_owned(),
            client_operation_id: "op-delete-1".to_owned(),
            request_fingerprint: "fp-delete-1".to_owned(),
            event_id: EventId::new("event-delete-1"),
            correlation_id: "correlation-delete-1".to_owned(),
            occurred_at: 4,
        };
        let summary = persistence
            .transact(|repositories| contexthub_application::delete_project(repositories, &delete))
            .unwrap();
        assert_eq!(summary.project_id, "project-delete-1");
        assert_eq!(
            summary
                .released_workspace_ids
                .iter()
                .map(|workspace| workspace.as_str().to_owned())
                .collect::<Vec<String>>(),
            vec!["w-snapshot".to_owned()]
        );

        // The project and its binding are gone, so the workspace can be reused.
        let project_rows: i64 = persistence
            .connection
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(project_rows, 0);
        let binding_rows: i64 = persistence
            .connection
            .query_row("SELECT COUNT(*) FROM project_workspaces", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(binding_rows, 0);

        // The workspace itself survives: the files on disk stay browsable.
        let workspace_rows: i64 = persistence
            .connection
            .query_row(
                "SELECT COUNT(*) FROM workspaces WHERE workspace_id='w-snapshot'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(workspace_rows, 1);

        // History is an audit trail: deleting a project does not rewrite what happened.
        let events_after: i64 = persistence
            .connection
            .query_row("SELECT COUNT(*) FROM durable_events", [], |row| row.get(0))
            .unwrap();
        assert!(events_after > events_before);

        // Deleting again is refused rather than silently succeeding.
        let error = persistence
            .transact(|repositories| contexthub_application::delete_project(repositories, &delete))
            .unwrap_err();
        assert!(format!("{error:?}").contains("project_not_found"));
    }

    #[test]
    fn deleting_an_unowned_workspace_clears_its_records_but_refuses_while_still_in_a_project() {
        let fixture = TestDatabase::new("workspace-delete");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_project_outcome_file(&mut persistence);
        let create = CreateProjectCommand {
            project_id: "project-ws-delete".to_owned(),
            display_name: "占用项目".to_owned(),
            goal: "占用主文件夹".to_owned(),
            primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
            client_operation_id: "op-ws-delete".to_owned(),
            request_fingerprint: "fp-ws-delete".to_owned(),
            event_id: EventId::new("event-ws-delete"),
            correlation_id: "correlation-ws-delete".to_owned(),
            occurred_at: 3,
        };
        persistence
            .transact(|repositories| contexthub_application::create_project(repositories, &create))
            .unwrap();

        // A workspace that still anchors a project must not be deletable.
        let bound = DeleteWorkspaceCommand {
            workspace_id: WorkspaceId::new("w-snapshot"),
            client_operation_id: "op-del-bound".to_owned(),
            request_fingerprint: "fp-del-bound".to_owned(),
            occurred_at: 4,
        };
        let error = persistence
            .transact(|repositories| {
                contexthub_application::delete_workspace(
                    repositories,
                    &bound,
                    EventId::new("event-del-bound"),
                )
            })
            .unwrap_err();
        assert!(format!("{error:?}").contains("workspace_still_in_project"));

        // An unowned workspace can be removed, taking its own history with it.
        persistence
            .connection
            .execute_batch(
                "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
                 VALUES ('w-orphan','闲置文件夹','/tmp/w-orphan',NULL,'ready',0,NULL,1,1);
                 INSERT INTO durable_events(event_id,global_sequence,workspace_id,workspace_sequence,event_type,object_type,object_id,payload_json,correlation_id,occurred_at,created_at)
                 VALUES ('event-orphan-1',9001,'w-orphan',1,'workspace.connected','workspace','w-orphan','{}',NULL,1,1);",
            )
            .unwrap();
        let orphan = DeleteWorkspaceCommand {
            workspace_id: WorkspaceId::new("w-orphan"),
            client_operation_id: "op-del-orphan".to_owned(),
            request_fingerprint: "fp-del-orphan".to_owned(),
            occurred_at: 5,
        };
        let summary = persistence
            .transact(|repositories| {
                contexthub_application::delete_workspace(
                    repositories,
                    &orphan,
                    EventId::new("event-del-orphan"),
                )
            })
            .unwrap();
        assert_eq!(summary.removed_events, 1);

        let remaining: i64 = persistence
            .connection
            .query_row(
                "SELECT COUNT(*) FROM workspaces WHERE workspace_id='w-orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        let orphan_events: i64 = persistence
            .connection
            .query_row(
                "SELECT COUNT(*) FROM durable_events WHERE workspace_id='w-orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(orphan_events, 0);
        // The deletion itself stays in the audit trail, recorded without a workspace reference.
        let deletion_recorded: i64 = persistence
            .connection
            .query_row(
                "SELECT COUNT(*) FROM durable_events WHERE event_type='workspace.deleted' AND object_id='w-orphan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(deletion_recorded, 1);
    }

    #[test]
    fn project_can_be_created_without_a_workspace_and_attach_one_later() {
        let fixture = TestDatabase::new("project-without-workspace");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_project_outcome_file(&mut persistence);
        let create = CreateProjectCommand {
            project_id: "project-empty-1".to_owned(),
            display_name: "空项目".to_owned(),
            goal: "先规划后关联文件".to_owned(),
            primary_workspace_id: None,
            client_operation_id: "op-empty-project".to_owned(),
            request_fingerprint: "fp-empty-project".to_owned(),
            event_id: EventId::new("event-empty-project"),
            correlation_id: "correlation-empty-project".to_owned(),
            occurred_at: 3,
        };
        let project = persistence
            .transact(|repositories| contexthub_application::create_project(repositories, &create))
            .unwrap();
        assert_eq!(project.primary_workspace_id, None);

        let attach = AttachProjectWorkspaceCommand {
            project_id: create.project_id.clone(),
            workspace_id: WorkspaceId::new("w-snapshot"),
            client_operation_id: "op-empty-attach".to_owned(),
            request_fingerprint: "fp-empty-attach".to_owned(),
            event_id: EventId::new("event-empty-attach"),
            correlation_id: "correlation-empty-attach".to_owned(),
            occurred_at: 4,
        };
        let attached = persistence
            .transact(|repositories| {
                contexthub_application::attach_project_workspace(repositories, &attach)
            })
            .unwrap();
        assert_eq!(
            attached
                .primary_workspace_id
                .as_ref()
                .map(WorkspaceId::as_str),
            Some("w-snapshot")
        );
    }

    #[test]
    fn every_project_workspace_can_be_detached_and_the_project_may_be_empty() {
        let fixture = TestDatabase::new("project-detach-workspace");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_project_outcome_file(&mut persistence);
        persistence
            .connection
            .execute_batch(
                "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
                 VALUES ('w-second','第二文件夹','/tmp/w-second',NULL,'ready',0,NULL,1,1);",
            )
            .unwrap();
        let create = CreateProjectCommand {
            project_id: "project-detach-1".to_owned(),
            display_name: "多文件夹项目".to_owned(),
            goal: "验证移出".to_owned(),
            primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
            client_operation_id: "op-detach-project".to_owned(),
            request_fingerprint: "fp-detach-project".to_owned(),
            event_id: EventId::new("event-detach-project"),
            correlation_id: "correlation-detach-project".to_owned(),
            occurred_at: 3,
        };
        persistence
            .transact(|repositories| contexthub_application::create_project(repositories, &create))
            .unwrap();
        let attach = AttachProjectWorkspaceCommand {
            project_id: "project-detach-1".to_owned(),
            workspace_id: WorkspaceId::new("w-second"),
            client_operation_id: "op-detach-attach".to_owned(),
            request_fingerprint: "fp-detach-attach".to_owned(),
            event_id: EventId::new("event-detach-attach"),
            correlation_id: "correlation-detach-attach".to_owned(),
            occurred_at: 4,
        };
        persistence
            .transact(|repositories| {
                contexthub_application::attach_project_workspace(repositories, &attach)
            })
            .unwrap();

        let detach_primary = DetachProjectWorkspaceCommand {
            project_id: "project-detach-1".to_owned(),
            workspace_id: WorkspaceId::new("w-snapshot"),
            client_operation_id: "op-detach-primary".to_owned(),
            request_fingerprint: "fp-detach-primary".to_owned(),
            event_id: EventId::new("event-detach-primary"),
            correlation_id: "correlation-detach-primary".to_owned(),
            occurred_at: 5,
        };
        let promoted = persistence
            .transact(|repositories| {
                contexthub_application::detach_project_workspace(repositories, &detach_primary)
            })
            .unwrap();
        assert_eq!(
            promoted
                .primary_workspace_id
                .as_ref()
                .map(WorkspaceId::as_str),
            Some("w-second")
        );

        let detach = DetachProjectWorkspaceCommand {
            workspace_id: WorkspaceId::new("w-second"),
            client_operation_id: "op-detach-1".to_owned(),
            request_fingerprint: "fp-detach-1".to_owned(),
            event_id: EventId::new("event-detach-1"),
            correlation_id: "correlation-detach-1".to_owned(),
            ..detach_primary.clone()
        };
        let detached = persistence
            .transact(|repositories| {
                contexthub_application::detach_project_workspace(repositories, &detach)
            })
            .unwrap();
        assert!(!detached.replayed);

        // The last folder can also leave; the project remains as an empty business container.
        let remaining: i64 = persistence
            .connection
            .query_row(
                "SELECT COUNT(*) FROM project_workspaces WHERE project_id='project-detach-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
        assert_eq!(detached.primary_workspace_id, None);

        // Repeating the detach is a replay rather than an error.
        let replayed = persistence
            .transact(|repositories| {
                contexthub_application::detach_project_workspace(repositories, &detach)
            })
            .unwrap();
        assert!(replayed.replayed);
    }

    #[test]
    fn attaching_a_workspace_extends_a_project_without_duplicate_links() {
        let fixture = TestDatabase::new("project-attach-workspace");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_project_outcome_file(&mut persistence);
        persistence
            .connection
            .execute_batch(
                "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
                 VALUES ('w-second','第二工作区','/tmp/w-second',NULL,'ready',0,NULL,1,1);
                 INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
                 VALUES ('w-third','第三工作区','/tmp/w-third',NULL,'ready',0,NULL,1,1);",
            )
            .unwrap();

        let create = CreateProjectCommand {
            project_id: "project-attach-1".to_owned(),
            display_name: "多工作区项目".to_owned(),
            goal: "验证一个项目持有多个工作区".to_owned(),
            primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
            client_operation_id: "op-attach-project".to_owned(),
            request_fingerprint: "fp-attach-project".to_owned(),
            event_id: EventId::new("event-attach-project"),
            correlation_id: "correlation-attach-project".to_owned(),
            occurred_at: 3,
        };
        persistence
            .transact(|repositories| contexthub_application::create_project(repositories, &create))
            .unwrap();

        let attach = AttachProjectWorkspaceCommand {
            project_id: "project-attach-1".to_owned(),
            workspace_id: WorkspaceId::new("w-second"),
            client_operation_id: "op-attach-1".to_owned(),
            request_fingerprint: "fp-attach-1".to_owned(),
            event_id: EventId::new("event-attach-1"),
            correlation_id: "correlation-attach-1".to_owned(),
            occurred_at: 4,
        };
        let attached = persistence
            .transact(|repositories| {
                contexthub_application::attach_project_workspace(repositories, &attach)
            })
            .unwrap();
        assert!(!attached.replayed);

        // The project now holds both workspaces, which is the whole point of the capability.
        let workspaces = persistence
            .transact(|repositories| repositories.workspaces_for_project("project-attach-1"))
            .unwrap();
        let mut ids: Vec<String> = workspaces
            .iter()
            .map(|workspace| workspace.as_str().to_owned())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["w-second".to_owned(), "w-snapshot".to_owned()]);

        // Re-attaching the same pair is a replay, not a duplicate row.
        let replayed = persistence
            .transact(|repositories| {
                contexthub_application::attach_project_workspace(repositories, &attach)
            })
            .unwrap();
        assert!(replayed.replayed);
        let binding_count: i64 = persistence
            .connection
            .query_row(
                "SELECT COUNT(*) FROM project_workspaces WHERE project_id='project-attach-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(binding_count, 2);
    }

    #[test]
    fn attaching_a_workspace_shares_across_projects_and_refuses_unknown_workspaces() {
        let fixture = TestDatabase::new("project-attach-workspace-rejects");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_project_outcome_file(&mut persistence);
        persistence
            .connection
            .execute_batch(
                "INSERT INTO workspaces(workspace_id,display_name,local_root,local_root_identity,local_status,state_revision,last_local_scan_at,created_at,updated_at)
                 VALUES ('w-third','第三工作区','/tmp/w-third',NULL,'ready',0,NULL,1,1);",
            )
            .unwrap();
        let create = CreateProjectCommand {
            project_id: "project-attach-1".to_owned(),
            display_name: "多工作区项目".to_owned(),
            goal: "验证一个项目持有多个工作区".to_owned(),
            primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
            client_operation_id: "op-attach-project".to_owned(),
            request_fingerprint: "fp-attach-project".to_owned(),
            event_id: EventId::new("event-attach-project"),
            correlation_id: "correlation-attach-project".to_owned(),
            occurred_at: 3,
        };
        persistence
            .transact(|repositories| contexthub_application::create_project(repositories, &create))
            .unwrap();

        let second_project = CreateProjectCommand {
            project_id: "project-attach-2".to_owned(),
            display_name: "另一个项目".to_owned(),
            goal: "占用第三工作区".to_owned(),
            primary_workspace_id: Some(WorkspaceId::new("w-third")),
            client_operation_id: "op-attach-project-2".to_owned(),
            request_fingerprint: "fp-attach-project-2".to_owned(),
            event_id: EventId::new("event-attach-project-2"),
            correlation_id: "correlation-attach-project-2".to_owned(),
            occurred_at: 5,
        };
        persistence
            .transact(|repositories| {
                contexthub_application::create_project(repositories, &second_project)
            })
            .unwrap();
        let stolen = AttachProjectWorkspaceCommand {
            project_id: "project-attach-1".to_owned(),
            workspace_id: WorkspaceId::new("w-third"),
            client_operation_id: "op-attach-2".to_owned(),
            request_fingerprint: "fp-attach-2".to_owned(),
            event_id: EventId::new("event-attach-2"),
            correlation_id: "correlation-attach-2".to_owned(),
            occurred_at: 6,
        };
        persistence
            .transact(|repositories| {
                contexthub_application::attach_project_workspace(repositories, &stolen)
            })
            .unwrap();
        assert_eq!(
            persistence
                .transact(|repositories| repositories
                    .projects_for_workspace(&WorkspaceId::new("w-third")))
                .unwrap()
                .len(),
            2
        );

        // Unknown workspaces are refused too, so a typo cannot silently create a dangling binding.
        let missing = AttachProjectWorkspaceCommand {
            workspace_id: WorkspaceId::new("w-does-not-exist"),
            client_operation_id: "op-attach-3".to_owned(),
            request_fingerprint: "fp-attach-3".to_owned(),
            event_id: EventId::new("event-attach-3"),
            correlation_id: "correlation-attach-3".to_owned(),
            ..stolen.clone()
        };
        let error = persistence
            .transact(|repositories| {
                contexthub_application::attach_project_workspace(repositories, &missing)
            })
            .unwrap_err();
        assert!(format!("{error:?}").contains("project_workspace_not_found"));
    }

    #[test]
    fn project_rename_advances_the_revision_and_refuses_a_stale_one() {
        let fixture = TestDatabase::new("project-update");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_project_outcome_file(&mut persistence);
        let created = persistence
            .transact(|repositories| {
                contexthub_application::create_project(
                    repositories,
                    &CreateProjectCommand {
                        project_id: "project-rename-1".to_owned(),
                        display_name: "旧名".to_owned(),
                        goal: "旧目标".to_owned(),
                        primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
                        client_operation_id: "op-rename-1".to_owned(),
                        request_fingerprint: "fp-rename-1".to_owned(),
                        event_id: EventId::new("event-rename-1"),
                        correlation_id: "correlation-rename-1".to_owned(),
                        occurred_at: 3,
                    },
                )
            })
            .unwrap();

        let renamed = persistence
            .transact(|repositories| {
                contexthub_application::update_project(
                    repositories,
                    &UpdateProjectCommand {
                        project_id: created.project_id.clone(),
                        display_name: "新名".to_owned(),
                        goal: "新目标".to_owned(),
                        expected_revision: created.state_revision,
                        event_id: EventId::new("event-rename-2"),
                        correlation_id: "correlation-rename-2".to_owned(),
                        occurred_at: 4,
                    },
                )
            })
            .unwrap();
        assert_eq!(renamed.display_name, "新名");
        assert_eq!(renamed.goal, "新目标");
        assert_eq!(renamed.state_revision, created.state_revision + 1);

        // Re-sending the same edit now carries a revision the project has moved past; accepting it
        // would silently overwrite whatever happened in between.
        let stale = persistence.transact(|repositories| {
            contexthub_application::update_project(
                repositories,
                &UpdateProjectCommand {
                    project_id: created.project_id.clone(),
                    display_name: "更新的名".to_owned(),
                    goal: "更新的目标".to_owned(),
                    expected_revision: created.state_revision,
                    event_id: EventId::new("event-rename-3"),
                    correlation_id: "correlation-rename-3".to_owned(),
                    occurred_at: 5,
                },
            )
        });
        assert!(stale.is_err(), "stale revision must be refused");
    }

    #[test]
    fn project_rename_refuses_an_unknown_project() {
        let fixture = TestDatabase::new("project-update-missing");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_project_outcome_file(&mut persistence);
        let result = persistence.transact(|repositories| {
            contexthub_application::update_project(
                repositories,
                &UpdateProjectCommand {
                    project_id: "project-does-not-exist".to_owned(),
                    display_name: "任意".to_owned(),
                    goal: "任意".to_owned(),
                    expected_revision: 0,
                    event_id: EventId::new("event-rename-missing"),
                    correlation_id: "correlation-rename-missing".to_owned(),
                    occurred_at: 4,
                },
            )
        });
        assert!(result.is_err());
    }

    #[test]
    fn project_deliverable_file_version_and_acceptance_form_one_persisted_flow() {
        let fixture = TestDatabase::new("project-outcomes");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_project_outcome_file(&mut persistence);

        let project_command = CreateProjectCommand {
            project_id: "project-local-1".to_owned(),
            display_name: "本地项目".to_owned(),
            goal: "形成可验收成果".to_owned(),
            primary_workspace_id: Some(WorkspaceId::new("w-snapshot")),
            client_operation_id: "op-project-1".to_owned(),
            request_fingerprint: "fp-project-1".to_owned(),
            event_id: EventId::new("event-project-1"),
            correlation_id: "correlation-project-1".to_owned(),
            occurred_at: 3,
        };
        let project = persistence
            .transact(|repositories| {
                contexthub_application::create_project(repositories, &project_command)
            })
            .unwrap();
        assert_eq!(project.status, ProjectStatus::Active);
        let replayed_project = persistence
            .transact(|repositories| {
                contexthub_application::create_project(repositories, &project_command)
            })
            .unwrap();
        assert!(replayed_project.replayed);

        let deliverable_command = CreateDeliverableCommand {
            deliverable_id: "deliverable-1".to_owned(),
            project_id: project.project_id.clone(),
            title: "需求方案".to_owned(),
            deliverable_type: "document".to_owned(),
            owner_label: "Mika".to_owned(),
            acceptance_criteria: "范围、决策和边界完整".to_owned(),
            client_operation_id: "op-deliverable-1".to_owned(),
            request_fingerprint: "fp-deliverable-1".to_owned(),
            event_id: EventId::new("event-deliverable-1"),
            correlation_id: "correlation-deliverable-1".to_owned(),
            occurred_at: 4,
        };
        let deliverable = persistence
            .transact(|repositories| {
                contexthub_application::create_deliverable(repositories, &deliverable_command)
            })
            .unwrap();
        assert_eq!(deliverable.status, DeliverableStatus::Draft);

        let version_command = CreateDeliverableVersionCommand {
            version_id: "deliverable-version-1".to_owned(),
            deliverable_id: deliverable.deliverable_id.clone(),
            summary: "首轮候选".to_owned(),
            local_file_ids: vec!["project-file".to_owned()],
            carriers: vec![commit_carrier()],
            client_operation_id: "op-version-1".to_owned(),
            request_fingerprint: "fp-version-1".to_owned(),
            event_id: EventId::new("event-version-1"),
            correlation_id: "correlation-version-1".to_owned(),
            occurred_at: 5,
        };
        let version = persistence
            .transact(|repositories| {
                contexthub_application::create_deliverable_version(repositories, &version_command)
            })
            .unwrap();
        assert_eq!(version.status, DeliverableVersionStatus::Candidate);
        assert_eq!(version.bindings[0].relative_path, "交付件/方案.md");
        assert_eq!(version.carriers.len(), 1);
        assert_eq!(version.carriers[0].carrier_type, "commit");
        assert_eq!(version.carriers[0].reference, "abc1234");
        let deliverable_review = my_work_items(&mut persistence);
        assert_eq!(deliverable_review.len(), 1);
        assert_eq!(deliverable_review[0].source_type, "deliverable_version");
        assert_eq!(deliverable_review[0].focus_group, "review");

        let review_command = ReviewDeliverableVersionCommand {
            acceptance_id: "acceptance-1".to_owned(),
            deliverable_id: deliverable.deliverable_id,
            version_id: version.version_id,
            decision: AcceptanceDecision::Accepted,
            reviewer_label: "Mika".to_owned(),
            comment: "符合验收标准".to_owned(),
            expected_deliverable_revision: 1,
            client_operation_id: "op-review-1".to_owned(),
            request_fingerprint: "fp-review-1".to_owned(),
            event_id: EventId::new("event-review-1"),
            correlation_id: "correlation-review-1".to_owned(),
            occurred_at: 6,
        };
        let review = persistence
            .transact(|repositories| {
                contexthub_application::review_deliverable_version(repositories, &review_command)
            })
            .unwrap();
        assert_eq!(review.version.status, DeliverableVersionStatus::Accepted);
        assert_eq!(review.deliverable.status, DeliverableStatus::Accepted);
        assert_eq!(
            review.deliverable.current_accepted_version_id.as_deref(),
            Some("deliverable-version-1")
        );
        assert_eq!(review.deliverable.state_revision, 2);
        assert!(my_work_items(&mut persistence).is_empty());

        assert_project_outcome_events(&persistence);
    }

    #[test]
    fn phase_plan_tasks_dependencies_and_acceptance_form_one_persisted_flow() {
        let fixture = TestDatabase::new("project-progress");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);

        create_active_phase_plan(&mut persistence);

        let blocked = phase_transition("phase-2", ProjectPhaseStatus::InProgress, 0, 12, "blocked");
        let error = persistence
            .transact(|repositories| {
                contexthub_application::transition_project_phase(repositories, &blocked)
            })
            .unwrap_err();
        assert_eq!(error.reason(), Some("phase_predecessor_incomplete"));

        transition_phase(
            &mut persistence,
            "phase-1",
            ProjectPhaseStatus::InProgress,
            0,
            13,
        );
        create_and_move_business_task(&mut persistence);
        advance_business_task_through_rework(&mut persistence);
        let attempts = persistence
            .transact(|repositories| repositories.phase_plans("project-progress-1"))
            .unwrap()[0]
            .phases[0]
            .tasks[0]
            .attempt_no;
        assert_eq!(attempts, 2);

        transition_phase(
            &mut persistence,
            "phase-1",
            ProjectPhaseStatus::Review,
            1,
            24,
        );
        let phase_review = my_work_items(&mut persistence);
        assert_eq!(phase_review.len(), 1);
        assert_eq!(phase_review[0].source_type, "project_phase");
        assert_eq!(phase_review[0].focus_group, "decision");
        assert_eq!(phase_review[0].phase_name.as_deref(), Some("需求定义"));
        transition_phase(
            &mut persistence,
            "phase-1",
            ProjectPhaseStatus::Completed,
            2,
            25,
        );
        assert!(my_work_items(&mut persistence).is_empty());
        let second = transition_phase(
            &mut persistence,
            "phase-2",
            ProjectPhaseStatus::InProgress,
            0,
            26,
        );
        assert!(second.is_main);
        assert_eq!(second.status, ProjectPhaseStatus::InProgress);
    }

    #[test]
    fn phase_without_deliverable_can_be_planned_reviewed_and_completed() {
        let fixture = TestDatabase::new("project-progress-without-deliverable");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        let mut phase = phase_input("phase-without-deliverable", "探索", 0);
        phase.required_deliverable_id = None;
        let plan = persistence
            .transact(|repositories| {
                contexthub_application::create_phase_plan(
                    repositories,
                    &CreatePhasePlanCommand {
                        plan_id: "phase-plan-without-deliverable".to_owned(),
                        project_id: "project-progress-1".to_owned(),
                        summary: "先探索再决定产出".to_owned(),
                        phases: vec![phase],
                        client_operation_id: "op-phase-plan-without-deliverable".to_owned(),
                        request_fingerprint: "fp-phase-plan-without-deliverable".to_owned(),
                        event_id: EventId::new("event-phase-plan-without-deliverable"),
                        correlation_id: "correlation-phase-plan-without-deliverable".to_owned(),
                        occurred_at: 10,
                    },
                )
            })
            .unwrap();
        assert!(plan.phases[0].required_deliverable_id.is_none());
        persistence
            .transact(|repositories| {
                contexthub_application::activate_phase_plan(
                    repositories,
                    &ActivatePhasePlanCommand {
                        plan_id: plan.plan_id,
                        expected_revision: plan.state_revision,
                        client_operation_id: "op-activate-without-deliverable".to_owned(),
                        request_fingerprint: "fp-activate-without-deliverable".to_owned(),
                        event_id: EventId::new("event-activate-without-deliverable"),
                        correlation_id: "correlation-activate-without-deliverable".to_owned(),
                        occurred_at: 11,
                    },
                )
            })
            .unwrap();
        transition_phase(
            &mut persistence,
            "phase-without-deliverable",
            ProjectPhaseStatus::InProgress,
            0,
            12,
        );
        persistence.connection.execute(
            "INSERT INTO business_tasks(business_task_id,project_id,phase_id,title,objective,input_context,expected_output,completion_criteria,assignee_type,assignee_label,priority,due_at,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at,attempt_no)
             VALUES ('task-without-deliverable','project-progress-1','phase-without-deliverable','完成探索','取得结论','','结论','人工确认','user','Mika','medium',NULL,'completed',0,'op-task-without-deliverable','fp-task-without-deliverable',13,13,1)",
            [],
        ).unwrap();
        let review = transition_phase(
            &mut persistence,
            "phase-without-deliverable",
            ProjectPhaseStatus::Review,
            1,
            14,
        );
        assert_eq!(review.status, ProjectPhaseStatus::Review);
        let completed = transition_phase(
            &mut persistence,
            "phase-without-deliverable",
            ProjectPhaseStatus::Completed,
            2,
            15,
        );
        assert_eq!(completed.status, ProjectPhaseStatus::Completed);
    }

    #[test]
    fn explicit_dependency_graph_allows_parallel_successor_phases() {
        let fixture = TestDatabase::new("project-progress-parallel");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        let first = phase_input("phase-1", "共同输入", 0);
        let second = phase_input("phase-2", "并行设计", 1);
        let mut third = phase_input("phase-3", "并行验证", 2);
        third.predecessor_phase_ids = vec!["phase-1".to_owned()];
        let command = CreatePhasePlanCommand {
            plan_id: "phase-plan-parallel".to_owned(),
            project_id: "project-progress-1".to_owned(),
            summary: "一个共同前置与两个并行后继".to_owned(),
            phases: vec![first, second, third],
            client_operation_id: "op-phase-plan-parallel".to_owned(),
            request_fingerprint: "fp-phase-plan-parallel".to_owned(),
            event_id: EventId::new("event-phase-plan-parallel"),
            correlation_id: "correlation-phase-plan-parallel".to_owned(),
            occurred_at: 10,
        };
        let plan = persistence
            .transact(|repositories| {
                contexthub_application::create_phase_plan(repositories, &command)
            })
            .unwrap();
        assert_eq!(plan.phases[2].predecessor_phase_ids, vec!["phase-1"]);
        let activation = ActivatePhasePlanCommand {
            plan_id: plan.plan_id,
            expected_revision: 0,
            client_operation_id: "op-activate-parallel".to_owned(),
            request_fingerprint: "fp-activate-parallel".to_owned(),
            event_id: EventId::new("event-activate-parallel"),
            correlation_id: "correlation-activate-parallel".to_owned(),
            occurred_at: 11,
        };
        persistence
            .transact(|repositories| {
                contexthub_application::activate_phase_plan(repositories, &activation)
            })
            .unwrap();
        persistence
            .connection
            .execute(
                "UPDATE project_phases SET status='completed' WHERE phase_id='phase-1'",
                [],
            )
            .unwrap();
        let second = transition_phase(
            &mut persistence,
            "phase-2",
            ProjectPhaseStatus::InProgress,
            0,
            12,
        );
        let third = transition_phase(
            &mut persistence,
            "phase-3",
            ProjectPhaseStatus::InProgress,
            0,
            13,
        );
        assert_eq!(second.status, ProjectPhaseStatus::InProgress);
        assert_eq!(third.status, ProjectPhaseStatus::InProgress);
        let phases = persistence
            .transact(|repositories| repositories.phase_plans("project-progress-1"))
            .unwrap()[0]
            .phases
            .clone();
        assert_eq!(
            phases
                .iter()
                .filter(|phase| phase.status == ProjectPhaseStatus::InProgress)
                .count(),
            2
        );
    }

    #[test]
    fn global_search_keyset_pages_all_rows_with_equal_timestamps() {
        let fixture = TestDatabase::new("global-search-pages");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        for index in 0..27 {
            persistence.connection.execute("INSERT INTO local_files(local_file_id,workspace_id,relative_path,normalized_relative_path,materialization_state,file_type,observed_at,scan_state,state_revision,created_at,updated_at) VALUES (?1,'w-snapshot',?2,?2,'materialized','file',20,'current',0,20,20)", params![format!("page-{index:02}"), format!("分页材料-{index:02}.md")]).unwrap();
        }
        let mut after = None;
        let mut ids = Vec::new();
        loop {
            let records = persistence
                .transact(|repositories| {
                    contexthub_application::global_search(
                        repositories,
                        &GlobalSearchQuery {
                            query: "分页材料".to_owned(),
                            limit: 8,
                            after: after.clone(),
                        },
                    )
                })
                .unwrap();
            if records.is_empty() {
                break;
            }
            let last = records.last().unwrap();
            after = Some((
                last.updated_at,
                last.object_type.clone(),
                last.object_id.clone(),
            ));
            ids.extend(records.into_iter().map(|record| record.object_id));
        }
        assert_eq!(ids.len(), 27);
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            27
        );
    }

    #[test]
    fn global_search_includes_projects_without_folders() {
        let fixture = TestDatabase::new("global-search-project-no-folder");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        persistence
            .connection
            .execute(
                "DELETE FROM project_workspaces WHERE project_id='project-progress-1'",
                [],
            )
            .unwrap();
        persistence.connection.execute("UPDATE projects SET display_name='无文件夹讨论' WHERE project_id='project-progress-1'", []).unwrap();
        let results = persistence
            .transact(|repositories| {
                contexthub_application::global_search(
                    repositories,
                    &GlobalSearchQuery {
                        query: "无文件夹讨论".to_owned(),
                        limit: 8,
                        after: None,
                    },
                )
            })
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].object_type, "project");
        assert_eq!(results[0].object_id, "project-progress-1");
        assert!(results[0].workspace_id.is_empty());
    }

    #[test]
    fn global_search_returns_typed_unique_source_results() {
        let fixture = TestDatabase::new("global-search");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        create_active_phase_plan(&mut persistence);
        let results = persistence
            .transact(|repositories| {
                contexthub_application::global_search(
                    repositories,
                    &GlobalSearchQuery {
                        after: None,
                        query: "需求".to_owned(),
                        limit: 100,
                    },
                )
            })
            .unwrap();
        assert!(results.iter().any(|item| item.object_type == "phase"
            && item.title == "需求定义"
            && item.project_id.as_deref() == Some("project-progress-1")));
        assert!(results.iter().all(|item| !item.workspace_id.is_empty()));

        persistence.connection.execute_batch(
            "INSERT INTO local_files(local_file_id,workspace_id,relative_path,normalized_relative_path,materialization_state,file_type,observed_at,scan_state,state_revision,created_at,updated_at)
             VALUES ('visible-search-file','w-snapshot','文档/隐藏规则说明.md','文档/隐藏规则说明.md','materialized','file',20,'current',0,20,20),
                    ('missing-search-file','w-snapshot','文档/已删除隐藏规则说明.md','文档/已删除隐藏规则说明.md','missing','file',20,'current',0,20,20),
                    ('hidden-root-search-file','w-snapshot','.claude/隐藏规则说明.md','.claude/隐藏规则说明.md','materialized','file',20,'current',0,20,20),
                    ('hidden-nested-search-file','w-snapshot','文档/.draft/隐藏规则说明.md','文档/.draft/隐藏规则说明.md','materialized','file',20,'current',0,20,20);",
        ).unwrap();
        let file_results = persistence
            .transact(|repositories| {
                contexthub_application::global_search(
                    repositories,
                    &GlobalSearchQuery {
                        after: None,
                        query: "隐藏规则".to_owned(),
                        limit: 100,
                    },
                )
            })
            .unwrap();
        assert_eq!(
            file_results
                .iter()
                .filter(|item| item.object_type == "file")
                .map(|item| item.object_id.as_str())
                .collect::<Vec<_>>(),
            vec!["visible-search-file"]
        );
    }

    #[test]
    fn attention_cases_project_to_my_work_and_follow_type_specific_state_machines() {
        let fixture = TestDatabase::new("attention-cases");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        create_active_phase_plan(&mut persistence);
        seed_attention_cases(&mut persistence);
        assert_attention_projection(&mut persistence);
        assert_attention_transitions(&mut persistence);
        assert_attention_events(&persistence);
    }

    #[test]
    fn task_incident_attaches_to_original_task_without_duplicate_card() {
        let fixture = TestDatabase::new("task-incident");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        create_active_phase_plan(&mut persistence);
        transition_phase(
            &mut persistence,
            "phase-1",
            ProjectPhaseStatus::InProgress,
            0,
            13,
        );
        create_and_move_business_task(&mut persistence);
        transition_task(&mut persistence, BusinessTaskStatus::Todo, 2, 17);
        transition_task(&mut persistence, BusinessTaskStatus::InProgress, 3, 18);
        transition_task(&mut persistence, BusinessTaskStatus::Review, 4, 19);
        let mut command = attention_create(
            "task-incident",
            AttentionCaseType::AgentRunIncident,
            None,
            Some(("failed-task-run", "Codex")),
            30,
        );
        command.task_id = Some("business-task-1".to_owned());
        command.phase_id = None;
        persistence
            .transact(|repositories| {
                contexthub_application::create_attention_case(repositories, &command)
            })
            .unwrap();
        let items = my_work_items(&mut persistence);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source_type, "business_task");
        assert_eq!(items[0].focus_group, "todo");
        assert_eq!(items[0].blocking_run_id.as_deref(), Some("failed-task-run"));
        assert_eq!(items[0].blocking_reason.as_deref(), Some("需要及时处理"));
        persist_attention_transition(
            &mut persistence,
            &attention_transition(
                "resolve-task-incident",
                "task-incident",
                AttentionCaseStatus::Resolved,
                0,
                "依赖已修复",
                40,
            ),
        );
        let items = my_work_items(&mut persistence);
        assert_eq!(items.len(), 1);
        assert!(items[0].blocking_run_id.is_none());
        assert_eq!(items[0].focus_group, "review");
    }

    #[test]
    fn failed_work_classification_does_not_project_as_business_work() {
        let fixture = TestDatabase::new("classification-incident");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        create_active_phase_plan(&mut persistence);
        let state = serde_json::json!({
            "conversations":[{
                "conversationId":"conversation",
                "title":"生成产品配图",
                "archived":false,
                "context":{"projectId":"project-progress-1","phaseId":"phase-1","taskId":null}
            }],
            "messages":[
                {"messageId":"original","conversationId":"conversation","role":"user","content":"生成产品配图"}
            ],
            "runs":[{
                "runId":"classification-failed",
                "conversationId":"conversation",
                "inputMessageId":"original",
                "status":"completed",
                "classificationStatus":"failed",
                "classificationError":"工作归类输出未完成",
                "startedAt":10,
                "completedAt":20,
                "provider":"codex",
                "model":"test"
            }]
        });
        persistence.replace_ai_runtime_state(0, &state, 20).unwrap();
        let mut command = attention_create(
            "classification-incident",
            AttentionCaseType::AgentRunIncident,
            None,
            Some(("classification-failed", "AI 工作归类")),
            20,
        );
        command.phase_id = Some("phase-1".to_owned());
        command.title = "恢复工作归类：生成产品配图".to_owned();
        command.detail = "工作归类输出未完成".to_owned();
        command.client_operation_id =
            "desktop-classification-incident-classification-failed".to_owned();
        persistence
            .transact(|repositories| {
                contexthub_application::create_attention_case(repositories, &command)
            })
            .unwrap();

        assert!(my_work_items(&mut persistence).is_empty());
        assert_eq!(persistence.ai_runtime_state().unwrap().1, state);
    }

    #[test]
    fn repeated_failed_classifications_preserve_history_without_business_cards() {
        let fixture = TestDatabase::new("classification-incident-latest-only");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_progress_project(&mut persistence);
        create_active_phase_plan(&mut persistence);
        let state = serde_json::json!({
            "conversations":[{
                "conversationId":"conversation",
                "title":"生成产品配图",
                "archived":false,
                "context":{"projectId":"project-progress-1","phaseId":"phase-1","taskId":null}
            }],
            "messages":[
                {"messageId":"original","conversationId":"conversation","role":"user","content":"生成产品配图"},
                {"messageId":"retry","conversationId":"conversation","role":"user","content":"重试"}
            ],
            "runs":[
                {"runId":"classification-old","conversationId":"conversation","inputMessageId":"original","status":"completed","classificationStatus":"failed","classificationError":"第一次归类失败","startedAt":10,"completedAt":20,"provider":"codex","model":"test"},
                {"runId":"classification-latest","conversationId":"conversation","inputMessageId":"retry","status":"completed","classificationStatus":"failed","classificationError":"第二次归类失败","startedAt":30,"completedAt":40,"provider":"codex","model":"test"}
            ]
        });
        persistence.replace_ai_runtime_state(0, &state, 40).unwrap();
        for (case_id, run_id, detail, occurred_at) in [
            (
                "classification-old-case",
                "classification-old",
                "第一次归类失败",
                20,
            ),
            (
                "classification-latest-case",
                "classification-latest",
                "第二次归类失败",
                40,
            ),
        ] {
            let mut command = attention_create(
                case_id,
                AttentionCaseType::AgentRunIncident,
                None,
                Some((run_id, "AI 工作归类")),
                occurred_at,
            );
            command.phase_id = Some("phase-1".to_owned());
            command.title = "恢复工作归类：生成产品配图".to_owned();
            command.detail = detail.to_owned();
            command.client_operation_id = format!("desktop-classification-incident-{run_id}");
            persistence
                .transact(|repositories| {
                    contexthub_application::create_attention_case(repositories, &command)
                })
                .unwrap();
        }

        assert!(my_work_items(&mut persistence).is_empty());

        persistence.replace_ai_runtime_state(1, &state, 50).unwrap();
        let statuses: (String, String) = persistence.connection.query_row(
            "SELECT older.status,latest.status FROM attention_cases older JOIN attention_cases latest ON latest.attention_case_id='classification-latest-case' WHERE older.attention_case_id='classification-old-case'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert_eq!(statuses, ("resolved".to_owned(), "open".to_owned()));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn classification_recovery_requires_committed_results_and_survives_upgrade() {
        for upgrade in [false, true] {
            for (scenario, status, committed, superseded, expected) in [
                ("continue", "applied", true, false, "resolved"),
                ("not-committed", "applied", false, false, "open"),
                ("still-failed", "failed", false, false, "open"),
                ("replaced", "applied", true, true, "open"),
            ] {
                let fixture = TestDatabase::new(scenario);
                let mut persistence = if upgrade {
                    let connection = Connection::open(&fixture.path).unwrap();
                    configure_connection(&connection).unwrap();
                    for migration in &MIGRATIONS[..48] {
                        apply_fixture_migration(&connection, migration);
                        connection.execute(
                            "INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES (?1,?2,?3,1,'0.1.0')",
                            params![migration.version, migration.name, migration_checksum(migration.sql)],
                        ).unwrap();
                    }
                    connection.execute_batch("CREATE TEMP TABLE ai_storage_activation(storage_key TEXT); CREATE TEMP VIEW ai_business_conversations AS SELECT * FROM ai_conversations; CREATE TEMP VIEW ai_business_messages AS SELECT * FROM ai_messages; CREATE TEMP VIEW ai_business_runs AS SELECT * FROM ai_runs;").unwrap();
                    SqlitePersistence {
                        connection,
                        database_path: fixture.path.clone(),
                        file_content_index: None,
                    }
                } else {
                    SqlitePersistence::open(&fixture.path, "0.1.0").unwrap()
                };
                seed_progress_project(&mut persistence);
                let mut state = serde_json::json!({
                    "conversations":[{"conversationId":"conversation","context":{"projectId":"project-progress-1","taskId":null}}],
                    "messages":[
                        {"messageId":"original","conversationId":"conversation","role":"user","content":"生成图片"},
                        {"messageId":"next","conversationId":"conversation","role":"user","content":"继续完成"}
                    ],
                    "runs":[
                        {"runId":"failed","conversationId":"conversation","inputMessageId":"original","status":"completed","classificationStatus":"failed","startedAt":10,"completedAt":20},
                        {"runId":"success","conversationId":"conversation","inputMessageId":"next","status":"completed","classificationStatus":status,"superseded":superseded,"startedAt":30,"completedAt":40}
                    ]
                });
                persistence.replace_ai_runtime_state(0, &state, 40).unwrap();
                let mut command = attention_create(
                    "classification-case",
                    AttentionCaseType::AgentRunIncident,
                    None,
                    Some(("failed", "AI 工作归类")),
                    20,
                );
                command.phase_id = None;
                command.client_operation_id = "desktop-classification-incident-failed".to_owned();
                persistence
                    .transact(|repositories| {
                        contexthub_application::create_attention_case(repositories, &command)
                    })
                    .unwrap();
                if committed {
                    persistence.connection.execute(
                        "INSERT INTO conversation_work_classifications(classification_id,run_id,conversation_id,project_id,business_task_id,classification_action,task_action,conversation_status,stage,outcome_title,outcome_summary,reason,confidence,raw_json,created_at) VALUES ('classified','success','conversation','project-progress-1',NULL,'unclassified','none',NULL,NULL,NULL,NULL,'已校验',1,'{}',40)", [],
                    ).unwrap();
                }
                if upgrade {
                    let projection = MIGRATIONS[40]
                        .sql
                        .split("INSERT OR IGNORE INTO attention_cases(")
                        .next()
                        .unwrap();
                    persistence.connection.execute_batch(projection).unwrap();
                    persistence.connection.execute("UPDATE attention_cases SET updated_at=updated_at WHERE attention_case_id='classification-case'", []).unwrap();
                    assert_eq!(persistence.connection.query_row("SELECT COUNT(*) FROM my_work_items WHERE attention_case_id='classification-case'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
                    drop(persistence);
                    persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
                } else {
                    state["conversations"][0]["updatedAt"] = serde_json::json!(50);
                    persistence.replace_ai_runtime_state(1, &state, 50).unwrap();
                }
                let actual: String = persistence.connection.query_row("SELECT status FROM attention_cases WHERE attention_case_id='classification-case'", [], |row| row.get(0)).unwrap();
                assert_eq!(actual, expected, "{scenario}, upgrade={upgrade}");
                assert!(my_work_items(&mut persistence).is_empty());
                assert_eq!(persistence.ai_runtime_state().unwrap().1, state);
                drop(persistence);
                let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
                let transitions: i64 = reopened.connection.query_row("SELECT COUNT(*) FROM attention_case_transitions WHERE attention_case_id='classification-case'", [], |row| row.get(0)).unwrap();
                assert_eq!(transitions, i64::from(expected == "resolved"));
            }
        }
    }

    #[test]
    fn incident_recovery_requires_matching_request_and_preserves_history() {
        for (scenario, prompt, retry, task, expected) in [
            ("same-request", "原始请求", false, None, "resolved"),
            ("unrelated", "无关的新请求", false, None, "open"),
            ("explicit-retry", "重试", true, None, "resolved"),
            (
                "changed-task",
                "原始请求",
                false,
                Some("other-task"),
                "open",
            ),
        ] {
            let fixture = TestDatabase::new(scenario);
            let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
            seed_progress_project(&mut persistence);
            create_active_phase_plan(&mut persistence);
            let command = attention_create(
                "incident",
                AttentionCaseType::AgentRunIncident,
                None,
                Some(("failed", "Codex")),
                20,
            );
            persistence
                .transact(|repositories| {
                    contexthub_application::create_attention_case(repositories, &command)
                })
                .unwrap();
            let state = serde_json::json!({
                "conversations":[{"conversationId":"conversation","context":{"projectId":"project-progress-1","taskId":task}}],
                "messages":[
                    {"messageId":"original","conversationId":"conversation","role":"user","content":"原始请求"},
                    {"messageId":"next","conversationId":"conversation","role":"user","content":prompt}
                ],
                "runs":[
                    {"runId":"failed","conversationId":"conversation","inputMessageId":"original","status":"failed","startedAt":10,"completedAt":20,"provider":"codex","model":"test"},
                    {"runId":"success","conversationId":"conversation","inputMessageId":"next","retryOfRunId":if retry {Some("failed")} else {None},"status":"completed","startedAt":30,"completedAt":40,"provider":"other","model":"different"}
                ]
            });
            persistence.replace_ai_runtime_state(0, &state, 40).unwrap();
            assert!(my_work_items(&mut persistence).is_empty());
            let status: String = persistence
                .connection
                .query_row(
                    "SELECT status FROM attention_cases WHERE attention_case_id='incident'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(status, expected, "{scenario}");
            assert_eq!(persistence.ai_runtime_state().unwrap().1, state);
            drop(persistence);
            let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
            let transitions: i64 = reopened
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM attention_case_transitions",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(transitions, i64::from(expected == "resolved"));
        }
    }

    #[test]
    fn version_two_workspace_data_is_split_without_changing_local_identity_or_children() {
        let fixture = TestDatabase::new("upgrade-v2-workspace");
        let connection = Connection::open(&fixture.path).unwrap();
        configure_connection(&connection).unwrap();
        for migration in &MIGRATIONS[..2] {
            apply_fixture_migration(&connection, migration);
            connection
                .execute(
                    "INSERT INTO schema_migrations(version, name, checksum, applied_at, app_version) VALUES (?1, ?2, ?3, 1, '0.1.0')",
                    params![migration.version, migration.name, migration_checksum(migration.sql)],
                )
                .unwrap();
        }
        connection
            .execute_batch(
                "INSERT INTO accounts(account_id, display_name, account_status, created_at, updated_at) VALUES ('a-legacy', 'Legacy', 'active', 1, 1);
                 INSERT INTO workspaces(workspace_id, account_id, project_id, display_name, local_root, local_root_identity, platform_root, status, state_revision, platform_snapshot_revision, last_platform_check_at, last_local_scan_at, created_at, updated_at) VALUES ('w-legacy', 'a-legacy', 'p-legacy', 'Legacy workspace', '/tmp/legacy', 'dev:1:ino:2', '/docs', 'conflict', 7, 'rev-9', 8, 9, 1, 10);
                 INSERT INTO workspace_exports(workspace_id, desired_revision, exported_revision, export_state, updated_at) VALUES ('w-legacy', 2, 1, 'pending', 10);
                 INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, materialization_state, file_type, observed_at, scan_state, state_revision, created_at, updated_at) VALUES ('f-legacy', 'w-legacy', 'a.md', 'a.md', 'materialized', 'file', 9, 'current', 0, 1, 9);
                 INSERT INTO file_states(local_file_id, baseline_relation, local_change_state, confirmation_state, submission_state, conflict_state, display_state, capability_bits, capability_reasons_json, computed_at, state_revision) VALUES ('f-legacy', 'unknown', 'clean', 'not_required', 'none', 'none', 'current', 0, '[]', 9, 0);",
            )
            .unwrap();
        drop(connection);

        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let workspace: (String, String, String, String, i64, Option<i64>) = persistence
            .connection
            .query_row(
                "SELECT workspace_id, local_root, local_root_identity, local_status, state_revision, last_local_scan_at FROM workspaces WHERE workspace_id='w-legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .unwrap();
        assert_eq!(
            workspace,
            (
                "w-legacy".to_owned(),
                "/tmp/legacy".to_owned(),
                "dev:1:ino:2".to_owned(),
                "attention_required".to_owned(),
                7,
                Some(9),
            )
        );
        let binding: (String, Option<String>, Option<String>, String, String, Option<String>) = persistence
            .connection
            .query_row(
                "SELECT account_id, organization_id, platform_workspace_id, platform_root, binding_state, platform_snapshot_revision FROM workspace_platform_bindings WHERE workspace_id='w-legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .unwrap();
        assert_eq!(
            binding,
            (
                "a-legacy".to_owned(),
                None,
                None,
                "/docs".to_owned(),
                "bound".to_owned(),
                Some("rev-9".to_owned()),
            )
        );
        let child_workspace: String = persistence
            .connection
            .query_row(
                "SELECT workspace_id FROM local_files WHERE local_file_id='f-legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(child_workspace, "w-legacy");
        let retained_file_state: String = persistence
            .connection
            .query_row(
                "SELECT local_file_id FROM file_states WHERE local_file_id='f-legacy'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained_file_state, "f-legacy");
        let violations: i64 = persistence
            .connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0);
    }

    #[test]
    fn m0_core_schema_has_required_tables_indexes_and_integrity() {
        let fixture = TestDatabase::new("core-schema");
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let expected_tables = [
            "accounts",
            "app_metadata",
            "checksum_cache",
            "client_event_cursors",
            "confirmation_invalidations",
            "conflict_resolutions",
            "conflicts",
            "document_versions",
            "documents",
            "durable_events",
            "file_confirmation_text_snapshots",
            "file_confirmations",
            "file_states",
            "local_file_observations",
            "local_files",
            "local_scan_snapshots",
            "platform_snapshots",
            "schema_migrations",
            "sync_execution_items",
            "sync_executions",
            "sync_plan_items",
            "sync_plans",
            "task_attempts",
            "tasks",
            "workspace_exports",
            "workspace_platform_bindings",
            "workspaces",
        ];
        for table in expected_tables {
            let exists: bool = persistence
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing table {table}");
        }
        let expected_indexes = [
            "idx_confirmation_invalidations_history",
            "idx_file_confirmations_active",
            "idx_file_confirmations_client_operation",
            "idx_file_confirmations_history",
            "idx_file_confirmation_text_snapshots_file",
            "idx_sync_plans_client_operation",
            "idx_sync_executions_client_operation",
            "idx_conflict_resolutions_client_operation",
            "idx_local_files_workspace_order",
        ];
        for index in expected_indexes {
            let exists: bool = persistence
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='index' AND name=?1)",
                    [index],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(exists, "missing index {index}");
        }
        let query_plan = persistence
            .connection
            .prepare(
                "EXPLAIN QUERY PLAN SELECT local_file_id FROM local_files WHERE workspace_id=?1 ORDER BY normalized_relative_path, local_file_id LIMIT ?2",
            )
            .unwrap()
            .query_map(params!["workspace", 501], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            query_plan
                .iter()
                .any(|detail| detail.contains("idx_local_files_workspace_order"))
        );
        assert!(
            query_plan
                .iter()
                .all(|detail| !detail.contains("TEMP B-TREE"))
        );
        let integrity: String = persistence
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        let foreign_key_violations: i64 = persistence
            .connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(foreign_key_violations, 0);
    }

    #[test]
    fn workspace_file_pages_exclude_missing_history_and_keep_one_materialized_row_per_path() {
        let fixture = TestDatabase::new("workspace-file-path-projection");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence.connection.execute_batch(
            "INSERT INTO workspaces(workspace_id, display_name, local_root, local_status, state_revision, created_at, updated_at) VALUES ('w1', 'Workspace', '/tmp/w1', 'ready', 0, 1, 1);
             INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, materialization_state, file_type, observed_at, scan_state, state_revision, created_at, updated_at) VALUES
               ('a-missing', 'w1', 'a.md', 'a.md', 'missing', 'file', 1, 'current', 1, 1, 2),
               ('a-present', 'w1', 'a.md', 'a.md', 'materialized', 'file', 3, 'current', 0, 3, 3),
               ('b-missing-old', 'w1', 'b.md', 'b.md', 'missing', 'file', 1, 'current', 1, 1, 2),
               ('b-missing-new', 'w1', 'b.md', 'b.md', 'missing', 'file', 2, 'current', 2, 2, 4),
               ('c-present', 'w1', 'c.md', 'c.md', 'materialized', 'file', 5, 'current', 0, 5, 5);",
        ).unwrap();

        let first = persistence
            .transact(|repositories| {
                repositories.workspace_files_page(&WorkspaceId::new("w1"), None, 1)
            })
            .unwrap();
        assert_eq!(
            first
                .files
                .iter()
                .map(|file| file.local_file_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-present"]
        );
        assert!(first.has_more);

        let second = persistence
            .transact(|repositories| {
                repositories.workspace_files_page(
                    &WorkspaceId::new("w1"),
                    first.next_cursor.as_deref(),
                    1,
                )
            })
            .unwrap();
        assert_eq!(
            second
                .files
                .iter()
                .map(|file| file.local_file_id.as_str())
                .collect::<Vec<_>>(),
            vec!["c-present"]
        );
        assert!(!second.has_more);
    }

    #[test]
    fn m0_core_constraints_reject_invalid_or_ambiguous_state() {
        let fixture = TestDatabase::new("core-constraints");
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let connection = &persistence.connection;
        connection
            .execute(
                "INSERT INTO workspaces(workspace_id, display_name, local_root, local_status, state_revision, created_at, updated_at) VALUES ('w-local', 'Local', '/tmp/w-local', 'ready', 0, 1, 1)",
                [],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO workspace_platform_bindings(workspace_id, account_id, organization_id, platform_workspace_id, project_id, platform_root, binding_state, created_at, updated_at) VALUES ('w-local', 'missing', 'org', 'pw', 'p', '/', 'bound', 1, 1)",
                    [],
                )
                .is_err()
        );
        connection
            .execute_batch(
                "INSERT INTO accounts(account_id, display_name, account_status, created_at, updated_at) VALUES ('a1', 'Account', 'active', 1, 1);
                 INSERT INTO workspaces(workspace_id, display_name, local_root, local_status, state_revision, created_at, updated_at) VALUES ('w1', 'Workspace', '/tmp/w1', 'ready', 0, 1, 1);
                 INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, materialization_state, file_type, observed_at, scan_state, state_revision, created_at, updated_at) VALUES ('f1', 'w1', 'a.md', 'a.md', 'materialized', 'file', 1, 'current', 0, 1, 1);",
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO local_files(local_file_id, workspace_id, relative_path, normalized_relative_path, materialization_state, file_type, observed_at, scan_state, state_revision, created_at, updated_at) VALUES ('f2', 'w1', './a.md', 'a.md', 'materialized', 'file', 1, 'current', 0, 1, 1)",
                    [],
                )
                .is_err()
        );
        assert!(
            connection
                .execute(
                    "INSERT INTO tasks(task_id, task_type, workspace_id, state, progress_completed, cancellable, payload_json, correlation_id, created_by_client, created_at) VALUES ('t1', 'scan', 'w1', 'not-a-state', 0, 1, '{}', 'c1', 'desktop', 1)",
                    [],
                )
                .is_err()
        );
        connection
            .execute(
                "INSERT INTO file_confirmations(confirmation_id, workspace_id, local_file_id, confirmed_by_account_id, confirmed_checksum_algorithm, confirmed_checksum_value, target_scope, result, source_client, created_at) VALUES ('c1', 'w1', 'f1', 'a1', 'sha256', 'one', 'content', 'confirmed', 'desktop', 1)",
                [],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO file_confirmations(confirmation_id, workspace_id, local_file_id, confirmed_by_account_id, confirmed_checksum_algorithm, confirmed_checksum_value, target_scope, result, source_client, created_at) VALUES ('c2', 'w1', 'f1', 'a1', 'sha256', 'two', 'content', 'confirmed', 'cli', 2)",
                    [],
                )
                .is_err()
        );
        connection
            .execute(
                "INSERT INTO conflicts(conflict_id, workspace_id, local_file_id, conflict_type, state, detected_at, state_revision) VALUES ('x1', 'w1', 'f1', 'content_diverged', 'detected', 1, 0)",
                [],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO conflicts(conflict_id, workspace_id, local_file_id, conflict_type, state, detected_at, state_revision) VALUES ('x2', 'w1', 'f1', 'delete_modify', 'in_progress', 2, 0)",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn local_workspace_creation_needs_no_account_and_binding_lifecycle_preserves_history() {
        let fixture = TestDatabase::new("local-workspace-lifecycle");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let command = contexthub_application::CreateLocalWorkspaceCommand {
            workspace_id: WorkspaceId::new("w-local"),
            display_name: "Local workspace".to_owned(),
            canonical_local_root: "/tmp/w-local".to_owned(),
            local_root_identity: Some("dev:1:ino:20".to_owned()),
            scan_task_id: TaskId::new("task-scan-local"),
            event_id: EventId::new("event-workspace-local"),
            correlation_id: "corr-local".to_owned(),
            created_by_client: "desktop".to_owned(),
            occurred_at: 10,
        };
        let workspace = persistence
            .transact(|repositories| {
                contexthub_application::create_local_workspace(repositories, &command)
            })
            .unwrap();
        assert_eq!(workspace.local_status, LocalWorkspaceStatus::Scanning);
        assert_default_local_identity(&persistence.connection);
        let scan_task: (String, String, String) = persistence
            .connection
            .query_row(
                "SELECT task_type, state, workspace_id FROM tasks WHERE task_id='task-scan-local'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            scan_task,
            ("scan".to_owned(), "queued".to_owned(), "w-local".to_owned())
        );
        persistence
            .transact(|repositories| {
                insert_historical_account(&repositories.transaction, "a1", "Account", 11)?;
                insert_historical_platform_binding(
                    &repositories.transaction,
                    &WorkspacePlatformBindingRecord {
                        workspace_id: WorkspaceId::new("w-local"),
                        account_id: AccountId::new("a1"),
                        organization_id: Some("org1".to_owned()),
                        platform_workspace_id: Some("platform-w1".to_owned()),
                        project_id: "project1".to_owned(),
                        platform_root: "/docs".to_owned(),
                        binding_state: WorkspacePlatformBindingState::Bound,
                        platform_snapshot_revision: Some("rev1".to_owned()),
                        last_platform_check_at: Some(11),
                        team_space_id: None,
                        managed_root: None,
                        device_id: None,
                        baseline_revision: None,
                        change_cursor: None,
                        permission_role: None,
                        permission_revision: None,
                        created_at: 11,
                        updated_at: 11,
                    },
                )
            })
            .unwrap();
        let bound_workspace = persistence
            .transact(|repositories| {
                assert!(
                    repositories
                        .platform_binding(&WorkspaceId::new("w-local"))?
                        .is_some()
                );
                repositories
                    .workspace(&WorkspaceId::new("w-local"))?
                    .ok_or_else(|| {
                        PersistenceError::new(
                            PersistenceErrorKind::Database,
                            "workspace missing during binding test",
                        )
                    })
            })
            .unwrap();
        assert_eq!(bound_workspace.workspace_id.as_str(), "w-local");
        persistence
            .transact(|repositories| {
                remove_historical_platform_binding(
                    &repositories.transaction,
                    &WorkspaceId::new("w-local"),
                    31,
                )
            })
            .unwrap();
        let retained: (i64, i64, i64) = persistence
            .connection
            .query_row(
                "SELECT (SELECT count(*) FROM workspaces WHERE workspace_id='w-local'), (SELECT count(*) FROM tasks WHERE workspace_id='w-local'), (SELECT count(*) FROM durable_events WHERE workspace_id='w-local')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(retained, (1, 1, 1));
    }

    fn assert_default_local_identity(connection: &Connection) {
        let count = connection
            .query_row("SELECT count(*) FROM accounts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        let identity = connection
            .query_row(
                "SELECT display_name, identity_kind FROM accounts WHERE account_id='local-profile:default'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(identity, ("本机操作人".to_owned(), "local".to_owned()));
    }

    #[test]
    fn repository_unit_of_work_commits_state_and_event_atomically() {
        let fixture = TestDatabase::new("unit-of-work");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let workspace = WorkspaceRecord {
            workspace_id: WorkspaceId::new("w1"),
            workspace_kind: WorkspaceKind::Local,
            display_name: "Workspace".to_owned(),
            local_root: "/tmp/w1".to_owned(),
            local_root_identity: Some("dev:1:ino:1".to_owned()),
            local_status: LocalWorkspaceStatus::Creating,
            state_revision: 0,
            last_local_scan_at: None,
            created_at: 1,
            updated_at: 1,
        };
        let export = WorkspaceExportRecord {
            workspace_id: workspace.workspace_id.clone(),
            desired_revision: 0,
            exported_revision: 0,
            updated_at: 1,
        };
        persistence
            .transact(|repositories| {
                insert_historical_account(&repositories.transaction, "a1", "Account", 1)?;
                repositories.insert_workspace(&workspace)?;
                repositories.initialize_workspace_export(&export)?;
                let stored = repositories.append_event(&workspace_event("e1", 1))?;
                assert_eq!(stored.global_sequence, 1);
                assert_eq!(stored.workspace_sequence, Some(1));
                Ok(())
            })
            .unwrap();

        let rollback_error = persistence
            .transact(|repositories| {
                repositories.update_workspace_status(
                    &WorkspaceId::new("w1"),
                    0,
                    LocalWorkspaceStatus::Ready,
                    2,
                )?;
                repositories.append_event(&workspace_event("e1", 2))?;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(rollback_error.kind(), PersistenceErrorKind::Database);
        let state_after_rollback: (String, i64) = persistence
            .connection
            .query_row(
                "SELECT local_status, state_revision FROM workspaces WHERE workspace_id='w1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state_after_rollback, ("creating".to_owned(), 0));
        let event_count: i64 = persistence
            .connection
            .query_row("SELECT count(*) FROM durable_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(event_count, 1);

        let updated = persistence
            .transact(|repositories| {
                let updated = repositories.update_workspace_status(
                    &WorkspaceId::new("w1"),
                    0,
                    LocalWorkspaceStatus::Ready,
                    3,
                )?;
                let event = repositories.append_event(&workspace_event("e2", 3))?;
                assert_eq!(event.workspace_sequence, Some(2));
                Ok(updated)
            })
            .unwrap();
        assert_eq!(updated.local_status, LocalWorkspaceStatus::Ready);
        assert_eq!(updated.state_revision, 1);

        let stale_error = persistence
            .transact(|repositories| {
                repositories.update_workspace_status(
                    &WorkspaceId::new("w1"),
                    0,
                    LocalWorkspaceStatus::AttentionRequired,
                    4,
                )?;
                repositories.append_event(&workspace_event("e3", 4))?;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(stale_error.kind(), PersistenceErrorKind::Conflict);
        let final_event_count: i64 = persistence
            .connection
            .query_row("SELECT count(*) FROM durable_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(final_event_count, 2);
    }

    #[test]
    fn 观测快照有上限不会无限增长() {
        // 每次扫描都写一份完整观测。手动扫描年代这只是慢性膨胀；接上文件监听之后
        // 变成急性：实测两万多文件的工作区每份 22,347 行，库一小时内涨到 3.3 GB。
        let fixture = TestDatabase::new("observation-retention");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, false);

        for generation in 1..=14 {
            persistence
                .transact(|repositories| {
                    repositories.commit_local_snapshot(&local_snapshot(
                        &format!("ls-{generation}"),
                        generation,
                        generation * 1_000,
                        vec![local_observation(
                            "file-1",
                            "notes/a.md",
                            "dev:1:ino:10",
                            "one",
                        )],
                    ))
                })
                .unwrap();
        }

        let snapshots_with_observations: i64 = persistence
            .connection
            .query_row(
                "SELECT COUNT(DISTINCT snapshot_id) FROM local_file_observations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(snapshots_with_observations, 10);
    }

    #[test]
    fn 单条目快照只补录记录不把工作区标为索引完成() {
        let fixture = TestDatabase::new("local-snapshot-partial");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, false);
        let mut partial = local_snapshot(
            "ls-partial",
            1,
            10,
            vec![local_observation(
                "file-1",
                "notes/a.md",
                "dev:1:ino:10",
                "one",
            )],
        );
        partial.snapshot_kind = SnapshotKind::Incremental;
        partial.covers_whole_tree = false;
        let committed = persistence
            .transact(|repositories| repositories.commit_local_snapshot(&partial))
            .unwrap();
        assert_eq!(committed.created_local_file_ids, vec!["file-1"]);
        let workspace = persistence
            .transact(|repositories| repositories.workspace(&WorkspaceId::new("w-snapshot")))
            .unwrap()
            .unwrap();
        assert_eq!(workspace.local_status, LocalWorkspaceStatus::Scanning);
        assert_eq!(workspace.last_local_scan_at, None);

        let complete = local_snapshot(
            "ls-complete",
            2,
            20,
            vec![local_observation(
                "file-2",
                "notes/a.md",
                "dev:1:ino:10",
                "one",
            )],
        );
        persistence
            .transact(|repositories| repositories.commit_local_snapshot(&complete))
            .unwrap();
        let workspace = persistence
            .transact(|repositories| repositories.workspace(&WorkspaceId::new("w-snapshot")))
            .unwrap()
            .unwrap();
        assert_eq!(workspace.local_status, LocalWorkspaceStatus::Ready);
        assert_eq!(workspace.last_local_scan_at, Some(20));
    }

    #[test]
    fn 按快照只读出本次新增的文件记录() {
        let fixture = TestDatabase::new("first-seen-scope");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, false);
        let first = local_snapshot(
            "ls-a",
            1,
            10,
            vec![local_observation("file-a", "a.md", "dev:1:ino:1", "one")],
        );
        persistence
            .transact(|repositories| repositories.commit_local_snapshot(&first))
            .unwrap();
        let mut second = local_snapshot(
            "ls-b",
            2,
            20,
            vec![
                local_observation("file-a2", "a.md", "dev:1:ino:1", "one"),
                local_observation("file-b", "b.md", "dev:1:ino:2", "two"),
            ],
        );
        second.snapshot_kind = SnapshotKind::Incremental;
        persistence
            .transact(|repositories| repositories.commit_local_snapshot(&second))
            .unwrap();
        let created = persistence
            .transact(|repositories| {
                repositories.workspace_files_first_seen_in(&WorkspaceId::new("w-snapshot"), "ls-b")
            })
            .unwrap();
        assert_eq!(
            created
                .iter()
                .map(|file| file.local_file_id.as_str())
                .collect::<Vec<_>>(),
            vec!["file-b"]
        );
    }

    #[test]
    fn 有读取问题的快照不能推断删除也不能标为全量() {
        let fixture = TestDatabase::new("local-snapshot-partial-invalid");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, false);
        let mut full_partial = local_snapshot("ls-bad-1", 1, 10, vec![]);
        full_partial.covers_whole_tree = false;
        assert!(
            persistence
                .transact(|repositories| repositories.commit_local_snapshot(&full_partial))
                .is_err()
        );
        let mut removing_partial = local_snapshot("ls-bad-2", 1, 10, vec![]);
        removing_partial.snapshot_kind = SnapshotKind::Incremental;
        removing_partial.covers_whole_tree = false;
        removing_partial.issue_count = 1;
        removing_partial.removed_normalized_paths = vec!["notes/a.md".to_owned()];
        assert!(
            persistence
                .transact(|repositories| repositories.commit_local_snapshot(&removing_partial))
                .is_err()
        );
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the acceptance test keeps the complete move, replacement and deletion history visible"
    )]
    #[test]
    fn local_snapshots_distinguish_move_replacement_and_deletion() {
        let fixture = TestDatabase::new("local-snapshot-identity");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, false);

        let first = persistence
            .transact(|repositories| {
                repositories.commit_local_snapshot(&local_snapshot(
                    "ls-1",
                    1,
                    10,
                    vec![local_observation(
                        "file-1",
                        "notes/a.md",
                        "dev:1:ino:10",
                        "one",
                    )],
                ))
            })
            .unwrap();
        assert_eq!(first.created_local_file_ids, vec!["file-1"]);

        let moved = persistence
            .transact(|repositories| {
                repositories.commit_local_snapshot(&local_snapshot(
                    "ls-2",
                    2,
                    20,
                    vec![local_observation(
                        "file-unused",
                        "archive/a.md",
                        "dev:1:ino:10",
                        "one",
                    )],
                ))
            })
            .unwrap();
        assert_eq!(moved.retained_local_file_ids, vec!["file-1"]);
        assert!(moved.created_local_file_ids.is_empty());

        let replaced = persistence
            .transact(|repositories| {
                repositories.commit_local_snapshot(&local_snapshot(
                    "ls-3",
                    3,
                    30,
                    vec![local_observation(
                        "file-2",
                        "archive/a.md",
                        "dev:1:ino:20",
                        "two",
                    )],
                ))
            })
            .unwrap();
        assert_eq!(replaced.created_local_file_ids, vec!["file-2"]);
        assert_eq!(replaced.missing_local_file_ids, vec!["file-1"]);

        let deleted = persistence
            .transact(|repositories| {
                repositories.commit_local_snapshot(&local_snapshot("ls-4", 4, 40, vec![]))
            })
            .unwrap();
        assert_eq!(deleted.missing_local_file_ids, vec!["file-2"]);
        let rows = persistence
            .connection
            .prepare(
                "SELECT local_file_id, normalized_relative_path, materialization_state, first_seen_snapshot_id, last_seen_snapshot_id, missing_since_snapshot_id FROM local_files ORDER BY local_file_id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    "file-1".to_owned(),
                    "archive/a.md".to_owned(),
                    "missing".to_owned(),
                    Some("ls-1".to_owned()),
                    Some("ls-2".to_owned()),
                    Some("ls-3".to_owned()),
                ),
                (
                    "file-2".to_owned(),
                    "archive/a.md".to_owned(),
                    "missing".to_owned(),
                    Some("ls-3".to_owned()),
                    Some("ls-3".to_owned()),
                    Some("ls-4".to_owned()),
                ),
            ]
        );
        let observation_count: i64 = persistence
            .connection
            .query_row("SELECT count(*) FROM local_file_observations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(observation_count, 3);
    }

    #[test]
    fn incremental_snapshot_swaps_paths_without_losing_stable_identities() {
        let fixture = TestDatabase::new("local-snapshot-swap");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, false);
        persistence
            .transact(|repositories| {
                repositories.commit_local_snapshot(&local_snapshot(
                    "ls-swap-1",
                    1,
                    10,
                    vec![
                        local_observation("file-a", "a.md", "dev:1:ino:10", "a"),
                        local_observation("file-b", "b.md", "dev:1:ino:20", "b"),
                    ],
                ))
            })
            .unwrap();
        let mut incremental = local_snapshot(
            "ls-swap-2",
            2,
            20,
            vec![
                local_observation("unused-a", "b.md", "dev:1:ino:10", "a"),
                local_observation("unused-b", "a.md", "dev:1:ino:20", "b"),
            ],
        );
        incremental.snapshot_kind = SnapshotKind::Incremental;
        let result = persistence
            .transact(|repositories| repositories.commit_local_snapshot(&incremental))
            .unwrap();
        assert_eq!(result.retained_local_file_ids, vec!["file-a", "file-b"]);
        assert!(result.created_local_file_ids.is_empty());
        assert!(result.missing_local_file_ids.is_empty());
        let paths: String = persistence
            .connection
            .query_row(
                "SELECT group_concat(local_file_id || ':' || normalized_relative_path, ',') FROM (SELECT local_file_id, normalized_relative_path FROM local_files WHERE materialization_state='materialized' ORDER BY local_file_id)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(paths, "file-a:b.md,file-b:a.md");
    }

    #[test]
    fn platform_snapshots_keep_document_identity_and_immutable_versions() {
        let fixture = TestDatabase::new("platform-snapshot-identity");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, true);
        persistence
            .transact(|repositories| {
                seed_historical_platform_snapshot(
                    &repositories.transaction,
                    &platform_snapshot("ps-1", "revision-1", "docs/a.md", "version-1", 10),
                )
            })
            .unwrap();
        persistence
            .transact(|repositories| {
                seed_historical_platform_snapshot(
                    &repositories.transaction,
                    &platform_snapshot("ps-2", "revision-2", "archive/a.md", "version-2", 20),
                )
            })
            .unwrap();
        let document: (String, String, String) = persistence
            .connection
            .query_row(
                "SELECT document_id, normalized_platform_path, latest_published_version_id FROM documents",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            document,
            (
                "document-1".to_owned(),
                "archive/a.md".to_owned(),
                "version-2".to_owned(),
            )
        );
        let versions: String = persistence
            .connection
            .query_row(
                "SELECT group_concat(version_id, ',') FROM (SELECT version_id FROM document_versions ORDER BY version_id)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(versions, "version-1,version-2");

        let duplicate_version = persistence
            .transact(|repositories| {
                seed_historical_platform_snapshot(
                    &repositories.transaction,
                    &platform_snapshot("ps-3", "revision-3", "archive/a.md", "version-2", 30),
                )
            })
            .unwrap_err();
        assert_eq!(duplicate_version.kind(), PersistenceErrorKind::Database);
        let rolled_back_snapshot: i64 = persistence
            .connection
            .query_row(
                "SELECT count(*) FROM platform_snapshots WHERE snapshot_id='ps-3'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rolled_back_snapshot, 0);
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "the acceptance test preserves the complete state, confirmation, conflict and rollback history"
    )]
    fn computed_file_state_persists_confirmation_invalidation_conflict_and_event_atomically() {
        let fixture = TestDatabase::new("computed-file-state");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, true);
        persistence
            .transact(|repositories| {
                seed_historical_platform_snapshot(
                    &repositories.transaction,
                    &platform_snapshot(
                        "state-platform-1",
                        "state-revision-1",
                        "docs/a.md",
                        "version-1",
                        10,
                    ),
                )?;
                repositories.commit_local_snapshot(&local_snapshot(
                    "state-local-1",
                    1,
                    11,
                    vec![local_observation(
                        "state-file-1",
                        "docs/a.md",
                        "dev:1:ino:30",
                        "checksum-version-1",
                    )],
                ))?;
                Ok(())
            })
            .unwrap();
        persistence
            .connection
            .execute(
                "UPDATE local_files SET document_id='document-1' WHERE local_file_id='state-file-1'",
                [],
            )
            .unwrap();

        let current = state_input("checksum-version-1", "version-1", "checksum-version-1");
        let persisted = persistence
            .transact(|repositories| {
                contexthub_application::recompute_file_state(
                    repositories,
                    &state_command("state-current", None, current, 12),
                )
            })
            .unwrap();
        assert_eq!(persisted.state_revision, 0);
        assert_eq!(persisted.output.display_state.as_str(), "synced");

        persistence
            .connection
            .execute(
                "INSERT INTO file_confirmations(confirmation_id, workspace_id, local_file_id, document_id, confirmed_by_account_id, confirmed_checksum_algorithm, confirmed_checksum_value, baseline_version_id, target_scope, result, source_client, created_at) VALUES ('confirmation-1', 'w-snapshot', 'state-file-1', 'document-1', 'account-snapshot', 'sha256', 'checksum-version-1', 'version-1', 'project', 'confirmed', 'desktop', 13)",
                [],
            )
            .unwrap();
        let mut modified = state_input("local-change", "version-1", "checksum-version-1");
        modified.active_confirmation = Some(ActiveConfirmationFact {
            checksum_algorithm: "sha256".to_owned(),
            checksum_value: "checksum-version-1".to_owned(),
            baseline_version_id: Some("version-1".to_owned()),
        });
        let invalidated = persistence
            .transact(|repositories| {
                contexthub_application::recompute_file_state(
                    repositories,
                    &state_command("state-modified", Some(0), modified, 14),
                )
            })
            .unwrap();
        assert_eq!(invalidated.state_revision, 1);
        assert_eq!(
            invalidated.output.confirmation_state,
            ConfirmationState::InvalidatedContentChanged
        );
        let invalidation: (String, String) = persistence
            .connection
            .query_row(
                "SELECT reason, current_checksum FROM confirmation_invalidations WHERE confirmation_id='confirmation-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            invalidation,
            (
                "local_content_changed".to_owned(),
                "local-change".to_owned()
            )
        );
        let invalidation_event: (String, String) = persistence
            .connection
            .query_row(
                "SELECT event_type, object_id FROM durable_events WHERE event_id='event-invalidation-state-modified'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            invalidation_event,
            (
                "file.confirmation_invalidated".to_owned(),
                "confirmation-1".to_owned()
            )
        );

        persistence
            .transact(|repositories| {
                seed_historical_platform_snapshot(
                    &repositories.transaction,
                    &platform_snapshot(
                        "state-platform-2",
                        "state-revision-2",
                        "docs/a.md",
                        "version-2",
                        15,
                    ),
                )
            })
            .unwrap();
        let diverged = state_input("local-change", "version-2", "checksum-version-2");
        let conflict = persistence
            .transact(|repositories| {
                contexthub_application::recompute_file_state(
                    repositories,
                    &state_command("state-diverged", Some(1), diverged, 16),
                )
            })
            .unwrap();
        assert_eq!(
            conflict.output.conflict_state,
            ConflictState::ContentDiverged
        );
        let conflict_type: String = persistence
            .connection
            .query_row(
                "SELECT conflict_type FROM conflicts WHERE local_file_id='state-file-1' AND state='detected'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(conflict_type, "content_diverged");

        let omitted_conflict = persistence
            .transact(|repositories| {
                contexthub_application::recompute_file_state(
                    repositories,
                    &state_command(
                        "state-omitted-conflict",
                        Some(2),
                        state_input("local-change", "version-2", "checksum-version-2"),
                        17,
                    ),
                )
            })
            .unwrap_err();
        assert_eq!(omitted_conflict.kind(), PersistenceErrorKind::Conflict);

        let stale = persistence
            .transact(|repositories| {
                contexthub_application::recompute_file_state(
                    repositories,
                    &state_command(
                        "state-stale",
                        Some(1),
                        state_input("local-change", "version-2", "checksum-version-2"),
                        18,
                    ),
                )
            })
            .unwrap_err();
        assert_eq!(stale.kind(), PersistenceErrorKind::Conflict);
        let stale_event_count: i64 = persistence
            .connection
            .query_row(
                "SELECT count(*) FROM durable_events WHERE event_id='event-state-stale'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stale_event_count, 0);
        let omitted_event_count: i64 = persistence
            .connection
            .query_row(
                "SELECT count(*) FROM durable_events WHERE event_id='event-state-omitted-conflict'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(omitted_event_count, 0);
        drop(persistence);
        let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let retained: (String, String, String) = reopened
            .connection
            .query_row(
                "SELECT c.state, fs.conflict_state, fs.confirmation_state FROM conflicts c JOIN file_states fs ON fs.local_file_id=c.local_file_id WHERE c.conflict_id='conflict-state-diverged'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            retained,
            (
                "detected".to_owned(),
                "content_diverged".to_owned(),
                "blocked_by_conflict".to_owned()
            )
        );
        let invalidated_at: Option<i64> = reopened
            .connection
            .query_row(
                "SELECT invalidated_at FROM file_confirmations WHERE confirmation_id='confirmation-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(invalidated_at, Some(14));
    }

    fn legacy_sync_rows(connection: &Connection) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        [
            "sync_plans",
            "sync_plan_items",
            "sync_executions",
            "sync_execution_items",
            "tasks",
        ]
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let count = statement.column_count();
            statement
                .query_map([], |row| (0..count).map(|index| row.get(index)).collect())
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .collect()
    }

    #[test]
    fn legacy_sync_history_remains_unchanged_across_reopen() {
        let fixture = TestDatabase::new("retained-sync-history");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, false);
        persistence.connection.execute_batch(
            "INSERT INTO sync_plans(plan_id, workspace_id, direction, state, local_state_revision, created_by_account_id, created_by_client, created_at, expires_at, item_count, risk_summary_json, client_operation_id, request_fingerprint)
             VALUES ('old-plan','w-snapshot','submit','executing',7,'local-profile:default','desktop',10,20,1,'{\"blocked\":0}','old-plan-operation','old-plan-fingerprint');
             INSERT INTO sync_plan_items(plan_item_id, plan_id, relative_path, action, allowed, requires_confirmation, risk_level, ordinal, expected_local_checksum_value, frozen_confirmation_state)
             VALUES ('old-item','old-plan','historical.md','update',1,1,'medium',0,'original-checksum','confirmed');
             INSERT INTO tasks(task_id, task_type, workspace_id, state, progress_completed, progress_total, cancellable, payload_json, correlation_id, created_by_client, created_at)
             VALUES ('old-task','sync','w-snapshot','outcome_unknown',0,1,0,'{\"original\":true}','old-correlation','desktop',11);
             INSERT INTO sync_executions(execution_id, plan_id, task_id, state, success_count, failed_count, skipped_count, conflict_count, correlation_id, client_operation_id, request_fingerprint)
             VALUES ('old-execution','old-plan','old-task','outcome_unknown',0,0,0,0,'old-correlation','old-execution-operation','old-execution-fingerprint');
             INSERT INTO sync_execution_items(execution_item_id, execution_id, plan_item_id, state, attempt_count, platform_request_id, error_reason)
             VALUES ('old-execution-item','old-execution','old-item','outcome_unknown',2,'original-request','receipt-unconfirmed');"
        ).unwrap();
        let before = legacy_sync_rows(&persistence.connection);
        assert!(before.iter().all(|rows| rows.len() == 1));
        drop(persistence);
        let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert_eq!(legacy_sync_rows(&reopened.connection), before);
        let violations: i64 = reopened
            .connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(violations, 0);
    }

    #[test]
    fn disabled_identity_invalidates_current_confirmations_with_durable_events() {
        let fixture = TestDatabase::new("identity-invalidation");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_snapshot_workspace(&mut persistence, false);
        persistence
            .transact(|repositories| {
                repositories.commit_local_snapshot(&local_snapshot(
                    "identity-local-1",
                    1,
                    10,
                    vec![local_observation(
                        "identity-file-1",
                        "note.md",
                        "dev:1:ino:88",
                        "identity-checksum",
                    )],
                ))
            })
            .unwrap();
        persistence
            .connection
            .execute(
                "INSERT INTO file_confirmations(confirmation_id, workspace_id, local_file_id, confirmed_by_account_id, confirmed_checksum_algorithm, confirmed_checksum_value, target_scope, result, source_client, created_at) VALUES ('identity-confirmation', 'w-snapshot', 'identity-file-1', 'local-profile:default', 'sha256', 'identity-checksum', 'content', 'confirmed', 'desktop', 11)",
                [],
            )
            .unwrap();
        persistence
            .connection
            .execute(
                "UPDATE accounts SET account_status='disabled', updated_at=12 WHERE account_id='local-profile:default'",
                [],
            )
            .unwrap();
        let invalidations = persistence
            .transact(|repositories| {
                contexthub_application::invalidate_identity_confirmations(
                    repositories,
                    &InvalidateIdentityConfirmationsCommand {
                        identity_id: AccountId::new("local-profile:default"),
                        invalidation_id_prefix: "identity-invalidation:".to_owned(),
                        event_id_prefix: "identity-event:".to_owned(),
                        correlation_id: "identity-correlation".to_owned(),
                        occurred_at: 12,
                    },
                )
            })
            .unwrap();
        assert_eq!(invalidations.len(), 1);
        assert_eq!(invalidations[0].reason, "identity_changed");
        let audit: (String, String) = persistence
            .connection
            .query_row(
                "SELECT reason, event_type FROM confirmation_invalidations JOIN durable_events ON durable_events.object_id=confirmation_invalidations.confirmation_id WHERE confirmation_invalidations.confirmation_id='identity-confirmation'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            audit,
            (
                "identity_changed".to_owned(),
                "file.confirmation_invalidated".to_owned()
            )
        );
    }

    #[test]
    fn durable_event_store_replays_global_and_workspace_pages_without_gaps() {
        let fixture = TestDatabase::new("event-replay");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_event_store(&mut persistence);

        let first = persistence.replay_global_after(0, 2).unwrap();
        assert_eq!(event_sequences(&first), vec![1, 2]);
        assert_eq!(first.next_sequence, 2);
        assert!(first.has_more);
        let second = persistence
            .replay_global_after(first.next_sequence, 2)
            .unwrap();
        assert_eq!(event_sequences(&second), vec![3, 4]);
        assert!(second.has_more);
        let third = persistence
            .replay_global_after(second.next_sequence, 2)
            .unwrap();
        assert_eq!(event_sequences(&third), vec![5]);
        assert!(!third.has_more);

        let workspace_first = persistence
            .replay_workspace_after(&WorkspaceId::new("w1"), 0, 2)
            .unwrap();
        assert_eq!(
            workspace_first
                .events
                .iter()
                .map(|event| event.workspace_sequence.unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(workspace_first.next_sequence, 2);
        assert!(workspace_first.has_more);
        let workspace_second = persistence
            .replay_workspace_after(&WorkspaceId::new("w1"), workspace_first.next_sequence, 2)
            .unwrap();
        assert_eq!(workspace_second.events[0].event_id.as_str(), "e5");
        assert_eq!(workspace_second.next_sequence, 3);
    }

    #[test]
    fn client_event_cursor_survives_reopen_and_rejects_unavailable_replay() {
        let fixture = TestDatabase::new("event-cursor");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        seed_event_store(&mut persistence);
        persistence
            .transact(|repositories| {
                assert_eq!(
                    repositories.acknowledge_event_cursor(
                        "desktop-main",
                        &AccountId::new("a1"),
                        4,
                        10,
                    )?,
                    4
                );
                assert_eq!(
                    repositories.acknowledge_event_cursor(
                        "desktop-main",
                        &AccountId::new("a1"),
                        2,
                        11,
                    )?,
                    4
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(
            persistence
                .client_cursor("desktop-main", &AccountId::new("a1"))
                .unwrap(),
            Some(4)
        );
        let future_cursor = persistence
            .transact(|repositories| {
                repositories.acknowledge_event_cursor("desktop-main", &AccountId::new("a1"), 6, 12)
            })
            .unwrap_err();
        assert_eq!(future_cursor.kind(), PersistenceErrorKind::Conflict);

        drop(persistence);
        let reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        assert_eq!(
            reopened
                .client_cursor("desktop-main", &AccountId::new("a1"))
                .unwrap(),
            Some(4)
        );
        let missed = reopened.replay_global_after(4, 10).unwrap();
        assert_eq!(event_sequences(&missed), vec![5]);
        reopened
            .connection
            .execute("DELETE FROM durable_events WHERE global_sequence=1", [])
            .unwrap();
        let expired = reopened.replay_global_after(0, 10).unwrap_err();
        assert_eq!(expired.kind(), PersistenceErrorKind::ReplayUnavailable);
        let available = reopened.replay_global_available(10).unwrap();
        assert_eq!(event_sequences(&available), vec![2, 3, 4, 5]);
        let ahead = reopened.replay_global_after(6, 10).unwrap_err();
        assert_eq!(ahead.kind(), PersistenceErrorKind::ReplayUnavailable);
        let invalid_page = reopened.replay_global_after(4, 0).unwrap_err();
        assert_eq!(invalid_page.kind(), PersistenceErrorKind::Configuration);
    }

    #[test]
    fn task_manager_persists_lifecycle_progress_result_and_attempt() {
        let fixture = TestDatabase::new("task-lifecycle");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let task = test_task("t1", true);
        persistence
            .transact(|repositories| repositories.insert_task(&task))
            .unwrap();
        let (started, attempt) = persistence
            .transact(|repositories| {
                repositories.start_task(
                    &TaskId::new("t1"),
                    &TaskAttemptId::new("ta1"),
                    "server-1",
                    2,
                )
            })
            .unwrap();
        assert_eq!(started.state, TaskState::Running);
        assert_eq!(started.started_at, Some(2));
        assert_eq!(attempt.attempt_no, 1);
        assert_eq!(attempt.state, TaskAttemptState::Running);

        let progressed = persistence
            .transact(|repositories| {
                repositories.update_task_progress(&TaskProgress {
                    task_id: TaskId::new("t1"),
                    progress_completed: 2,
                    progress_total: Some(5),
                    current_stage: Some("scan".to_owned()),
                    heartbeat_at: 3,
                })
            })
            .unwrap();
        assert_eq!(progressed.progress_completed, 2);
        assert_eq!(progressed.progress_total, Some(5));
        assert_invalid_progress(&mut persistence, 1, Some(5));
        assert_invalid_progress(&mut persistence, 3, Some(6));

        for (expected_state, target_state, occurred_at) in [
            (TaskState::Running, TaskState::WaitingForUser, 5),
            (TaskState::WaitingForUser, TaskState::Running, 6),
        ] {
            persistence
                .transact(|repositories| {
                    transition_without_result(
                        repositories,
                        "t1",
                        expected_state,
                        target_state,
                        occurred_at,
                    )
                })
                .unwrap();
        }
        persistence
            .transact(|repositories| {
                let completed = repositories.transition_task(&TaskTransition {
                    task_id: TaskId::new("t1"),
                    expected_state: TaskState::Running,
                    target_state: TaskState::Succeeded,
                    occurred_at: 7,
                    result_ref_type: Some("sync_plan".to_owned()),
                    result_ref_id: Some("sp1".to_owned()),
                    error_reason: None,
                    error_detail_json: None,
                })?;
                repositories.append_event(&task_event("task-e1", "t1", 7))?;
                Ok(completed)
            })
            .unwrap();
        drop(persistence);

        let mut reopened = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let completed = reopened
            .transact(|repositories| repositories.task(&TaskId::new("t1")))
            .unwrap()
            .unwrap();
        assert_eq!(completed.state, TaskState::Succeeded);
        assert_eq!(completed.progress_completed, 2);
        assert_eq!(completed.result_ref_id.as_deref(), Some("sp1"));
        assert_eq!(completed.completed_at, Some(7));
        let attempt_state: String = reopened
            .connection
            .query_row(
                "SELECT state FROM task_attempts WHERE attempt_id='ta1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attempt_state, "succeeded");
    }

    #[test]
    fn task_manager_rolls_back_terminal_event_and_restarts_interrupted_attempt() {
        let fixture = TestDatabase::new("task-restart");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence
            .transact(|repositories| {
                repositories.insert_task(&test_task("t2", true))?;
                repositories.start_task(
                    &TaskId::new("t2"),
                    &TaskAttemptId::new("ta21"),
                    "server-1",
                    2,
                )?;
                repositories.append_event(&task_event("duplicate", "t2", 2))?;
                Ok(())
            })
            .unwrap();
        let rollback = persistence
            .transact(|repositories| {
                repositories.transition_task(&failed_transition("t2", 3))?;
                repositories.append_event(&task_event("duplicate", "t2", 3))?;
                Ok(())
            })
            .unwrap_err();
        assert_eq!(rollback.kind(), PersistenceErrorKind::Database);
        let after_rollback = persistence
            .transact(|repositories| repositories.task(&TaskId::new("t2")))
            .unwrap()
            .unwrap();
        assert_eq!(after_rollback.state, TaskState::Running);

        persistence
            .transact(|repositories| {
                repositories.transition_task(&TaskTransition {
                    task_id: TaskId::new("t2"),
                    expected_state: TaskState::Running,
                    target_state: TaskState::Interrupted,
                    occurred_at: 4,
                    result_ref_type: None,
                    result_ref_id: None,
                    error_reason: Some("server_stopped".to_owned()),
                    error_detail_json: None,
                })?;
                let (_, second_attempt) = repositories.start_task(
                    &TaskId::new("t2"),
                    &TaskAttemptId::new("ta22"),
                    "server-2",
                    5,
                )?;
                assert_eq!(second_attempt.attempt_no, 2);
                repositories.transition_task(&TaskTransition {
                    task_id: TaskId::new("t2"),
                    expected_state: TaskState::Running,
                    target_state: TaskState::Cancelled,
                    occurred_at: 6,
                    result_ref_type: None,
                    result_ref_id: None,
                    error_reason: Some("user_cancelled".to_owned()),
                    error_detail_json: None,
                })?;
                Ok(())
            })
            .unwrap();
        let attempts = persistence
            .connection
            .prepare("SELECT attempt_no, state FROM task_attempts WHERE task_id='t2' ORDER BY attempt_no")
            .unwrap()
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            attempts,
            vec![(1, "interrupted".to_owned()), (2, "interrupted".to_owned())]
        );
    }

    #[test]
    fn task_manager_reconciles_unknown_outcome_and_rejects_forbidden_cancellation() {
        let fixture = TestDatabase::new("task-reconcile");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        let reconciled = persistence
            .transact(|repositories| {
                repositories.insert_task(&test_task("t4", false))?;
                repositories.start_task(
                    &TaskId::new("t4"),
                    &TaskAttemptId::new("ta41"),
                    "server-2",
                    7,
                )?;
                repositories.transition_task(&TaskTransition {
                    task_id: TaskId::new("t4"),
                    expected_state: TaskState::Running,
                    target_state: TaskState::OutcomeUnknown,
                    occurred_at: 8,
                    result_ref_type: None,
                    result_ref_id: None,
                    error_reason: Some("platform_result_unconfirmed".to_owned()),
                    error_detail_json: None,
                })?;
                repositories.transition_task(&TaskTransition {
                    task_id: TaskId::new("t4"),
                    expected_state: TaskState::OutcomeUnknown,
                    target_state: TaskState::Succeeded,
                    occurred_at: 9,
                    result_ref_type: Some("submission".to_owned()),
                    result_ref_id: Some("submission-1".to_owned()),
                    error_reason: None,
                    error_detail_json: None,
                })
            })
            .unwrap();
        assert_eq!(reconciled.state, TaskState::Succeeded);
        assert_eq!(reconciled.completed_at, Some(9));

        persistence
            .transact(|repositories| repositories.insert_task(&test_task("t3", false)))
            .unwrap();
        let cancellation = persistence
            .transact(|repositories| {
                repositories.transition_task(&TaskTransition {
                    task_id: TaskId::new("t3"),
                    expected_state: TaskState::Queued,
                    target_state: TaskState::Cancelled,
                    occurred_at: 7,
                    result_ref_type: None,
                    result_ref_id: None,
                    error_reason: Some("user_cancelled".to_owned()),
                    error_detail_json: None,
                })
            })
            .unwrap_err();
        assert_eq!(cancellation.kind(), PersistenceErrorKind::Conflict);
    }

    fn seed_event_store(persistence: &mut SqlitePersistence) {
        persistence
            .transact(|repositories| {
                insert_historical_account(&repositories.transaction, "a1", "Account", 1)?;
                for (workspace_id, local_root) in [("w1", "/tmp/w1"), ("w2", "/tmp/w2")] {
                    repositories.insert_workspace(&WorkspaceRecord {
                        workspace_id: WorkspaceId::new(workspace_id),
                        workspace_kind: WorkspaceKind::Local,
                        display_name: workspace_id.to_owned(),
                        local_root: local_root.to_owned(),
                        local_root_identity: None,
                        local_status: LocalWorkspaceStatus::Ready,
                        state_revision: 0,
                        last_local_scan_at: None,
                        created_at: 1,
                        updated_at: 1,
                    })?;
                }
                for event in [
                    replay_test_event("e1", Some("w1"), 1),
                    replay_test_event("e2", None, 2),
                    replay_test_event("e3", Some("w2"), 3),
                    replay_test_event("e4", Some("w1"), 4),
                    replay_test_event("e5", Some("w1"), 5),
                ] {
                    repositories.append_event(&event)?;
                }
                Ok(())
            })
            .unwrap();
    }

    fn event_sequences(page: &EventReplayPage) -> Vec<i64> {
        page.events
            .iter()
            .map(|event| event.global_sequence)
            .collect()
    }

    fn replay_test_event(
        event_id: &str,
        workspace_id: Option<&str>,
        timestamp: i64,
    ) -> NewDurableEvent {
        NewDurableEvent {
            event_id: EventId::new(event_id),
            workspace_id: workspace_id.map(WorkspaceId::new),
            event_type: "test.event".to_owned(),
            object_type: None,
            object_id: None,
            payload_json: format!("{{\"at\":{timestamp}}}"),
            correlation_id: None,
            occurred_at: timestamp,
            created_at: timestamp,
        }
    }

    fn workspace_event(event_id: &str, timestamp: i64) -> NewDurableEvent {
        NewDurableEvent {
            event_id: EventId::new(event_id),
            workspace_id: Some(WorkspaceId::new("w1")),
            event_type: "workspace.changed".to_owned(),
            object_type: Some("workspace".to_owned()),
            object_id: Some("w1".to_owned()),
            payload_json: "{}".to_owned(),
            correlation_id: Some(format!("c{timestamp}")),
            occurred_at: timestamp,
            created_at: timestamp,
        }
    }

    fn seed_snapshot_workspace(persistence: &mut SqlitePersistence, bind_platform: bool) {
        persistence
            .transact(|repositories| {
                repositories.insert_workspace(&WorkspaceRecord {
                    workspace_id: WorkspaceId::new("w-snapshot"),
                    workspace_kind: WorkspaceKind::Local,
                    display_name: "Snapshot workspace".to_owned(),
                    local_root: "/tmp/w-snapshot".to_owned(),
                    local_root_identity: Some("dev:1:ino:1".to_owned()),
                    local_status: LocalWorkspaceStatus::Scanning,
                    state_revision: 0,
                    last_local_scan_at: None,
                    created_at: 1,
                    updated_at: 1,
                })?;
                repositories.initialize_workspace_export(&WorkspaceExportRecord {
                    workspace_id: WorkspaceId::new("w-snapshot"),
                    desired_revision: 1,
                    exported_revision: 0,
                    updated_at: 1,
                })?;
                if bind_platform {
                    insert_historical_account(
                        &repositories.transaction,
                        "account-snapshot",
                        "Snapshot account",
                        1,
                    )?;
                    insert_historical_platform_binding(
                        &repositories.transaction,
                        &WorkspacePlatformBindingRecord {
                            workspace_id: WorkspaceId::new("w-snapshot"),
                            account_id: AccountId::new("account-snapshot"),
                            organization_id: Some("organization-1".to_owned()),
                            platform_workspace_id: Some("platform-workspace-1".to_owned()),
                            project_id: "project-1".to_owned(),
                            platform_root: "/docs".to_owned(),
                            binding_state: WorkspacePlatformBindingState::Bound,
                            platform_snapshot_revision: None,
                            last_platform_check_at: None,
                            team_space_id: None,
                            managed_root: None,
                            device_id: None,
                            baseline_revision: None,
                            change_cursor: None,
                            permission_role: None,
                            permission_revision: None,
                            created_at: 1,
                            updated_at: 1,
                        },
                    )?;
                }
                Ok(())
            })
            .unwrap();
    }

    fn seed_project_outcome_file(persistence: &mut SqlitePersistence) {
        seed_snapshot_workspace(persistence, false);
        persistence
            .transact(|repositories| {
                repositories.commit_local_snapshot(&local_snapshot(
                    "project-snapshot",
                    1,
                    2,
                    vec![local_observation(
                        "project-file",
                        "交付件/方案.md",
                        "dev:1:ino:2",
                        "project-checksum",
                    )],
                ))
            })
            .unwrap();
    }

    fn seed_progress_project(persistence: &mut SqlitePersistence) {
        seed_snapshot_workspace(persistence, false);
        persistence.connection.execute_batch(
            "INSERT INTO projects(project_id,display_name,goal,status,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('project-progress-1','推进项目','形成可验收成果','active',0,'seed-project-progress','seed-project-progress',1,1);
             INSERT INTO project_workspaces(project_id,workspace_id,role,created_at)
             VALUES ('project-progress-1','w-snapshot','primary',1);
             INSERT INTO deliverables(deliverable_id,project_id,title,deliverable_type,owner_label,acceptance_criteria,status,current_accepted_version_id,state_revision,client_operation_id,request_fingerprint,created_at,updated_at)
             VALUES ('deliverable-progress-1','project-progress-1','阶段成果','document','Mika','内容完整','draft',NULL,0,'seed-deliverable-progress','seed-deliverable-progress',2,2);
             INSERT INTO deliverable_versions(version_id,deliverable_id,version_no,status,summary,client_operation_id,request_fingerprint,created_at,accepted_at)
             VALUES ('deliverable-progress-version-1','deliverable-progress-1',1,'accepted','已验收版本','seed-version-progress','seed-version-progress',3,4);
             UPDATE deliverables SET status='accepted',current_accepted_version_id='deliverable-progress-version-1',state_revision=1,updated_at=4
             WHERE deliverable_id='deliverable-progress-1';",
        ).unwrap();
    }

    fn phase_input(phase_id: &str, name: &str, sort_order: i64) -> PhaseDraftInput {
        PhaseDraftInput {
            phase_id: phase_id.to_owned(),
            name: name.to_owned(),
            goal: format!("完成{name}"),
            acceptance_criteria: format!("{name}已验收"),
            required_deliverable_id: Some("deliverable-progress-1".to_owned()),
            sort_order,
            predecessor_phase_ids: if sort_order > 0 {
                vec![format!("phase-{sort_order}")]
            } else {
                Vec::new()
            },
        }
    }

    fn create_active_phase_plan(persistence: &mut SqlitePersistence) {
        let plan_command = CreatePhasePlanCommand {
            plan_id: "phase-plan-1".to_owned(),
            project_id: "project-progress-1".to_owned(),
            summary: "从需求推进到验收".to_owned(),
            phases: vec![
                phase_input("phase-1", "需求定义", 0),
                phase_input("phase-2", "开发验收", 1),
            ],
            client_operation_id: "op-phase-plan-1".to_owned(),
            request_fingerprint: "fp-phase-plan-1".to_owned(),
            event_id: EventId::new("event-phase-plan-1"),
            correlation_id: "correlation-phase-plan-1".to_owned(),
            occurred_at: 10,
        };
        let draft = persistence
            .transact(|repositories| {
                contexthub_application::create_phase_plan(repositories, &plan_command)
            })
            .unwrap();
        assert_eq!(draft.status, PhasePlanStatus::Draft);
        let activation = ActivatePhasePlanCommand {
            plan_id: draft.plan_id,
            expected_revision: draft.state_revision,
            client_operation_id: "op-phase-activate-1".to_owned(),
            request_fingerprint: "fp-phase-activate-1".to_owned(),
            event_id: EventId::new("event-phase-activate-1"),
            correlation_id: "correlation-phase-activate-1".to_owned(),
            occurred_at: 11,
        };
        let active = persistence
            .transact(|repositories| {
                contexthub_application::activate_phase_plan(repositories, &activation)
            })
            .unwrap();
        assert_eq!(active.status, PhasePlanStatus::Active);
    }

    fn phase_transition(
        phase_id: &str,
        target_status: ProjectPhaseStatus,
        expected_revision: i64,
        occurred_at: i64,
        suffix: &str,
    ) -> TransitionProjectPhaseCommand {
        TransitionProjectPhaseCommand {
            transition_id: format!("phase-transition-{suffix}"),
            phase_id: phase_id.to_owned(),
            target_status,
            expected_revision,
            actor_label: "Mika".to_owned(),
            comment: "确认推进".to_owned(),
            client_operation_id: format!("op-phase-transition-{suffix}"),
            request_fingerprint: format!("fp-phase-transition-{suffix}"),
            event_id: EventId::new(format!("event-phase-transition-{suffix}")),
            correlation_id: format!("correlation-phase-transition-{suffix}"),
            occurred_at,
        }
    }

    fn transition_phase(
        persistence: &mut SqlitePersistence,
        phase_id: &str,
        target_status: ProjectPhaseStatus,
        expected_revision: i64,
        occurred_at: i64,
    ) -> ProjectPhaseRecord {
        let suffix = format!("{phase_id}-{occurred_at}");
        let command = phase_transition(
            phase_id,
            target_status,
            expected_revision,
            occurred_at,
            &suffix,
        );
        persistence
            .transact(|repositories| {
                contexthub_application::transition_project_phase(repositories, &command)
            })
            .unwrap()
    }

    fn commit_carrier() -> DeliverableCarrierInput {
        DeliverableCarrierInput {
            carrier_id: "carrier-commit-1".to_owned(),
            carrier_type: "commit".to_owned(),
            carrier_role: "supporting".to_owned(),
            label: "实现提交".to_owned(),
            reference: "abc1234".to_owned(),
            version_label: "提交 abc1234".to_owned(),
            source_version_id: None,
        }
    }

    fn create_and_move_business_task(persistence: &mut SqlitePersistence) {
        let command = CreateBusinessTaskCommand {
            business_task_id: "business-task-1".to_owned(),
            project_id: "project-progress-1".to_owned(),
            phase_id: Some("phase-1".to_owned()),
            title: "完成需求说明".to_owned(),
            objective: "明确开发边界".to_owned(),
            input_context: "用户访谈".to_owned(),
            expected_output: "需求说明".to_owned(),
            completion_criteria: "范围、例外和验收口径完整".to_owned(),
            assignee_type: "user".to_owned(),
            assignee_label: "Mika".to_owned(),
            priority: "high".to_owned(),
            due_at: Some(100),
            client_operation_id: "op-business-task-1".to_owned(),
            request_fingerprint: "fp-business-task-1".to_owned(),
            event_id: EventId::new("event-business-task-1"),
            correlation_id: "correlation-business-task-1".to_owned(),
            occurred_at: 14,
        };
        let task = persistence
            .transact(|repositories| {
                contexthub_application::create_business_task(repositories, &command)
            })
            .unwrap();
        assert_eq!(task.status, BusinessTaskStatus::Candidate);
        assert_eq!(my_work_items(persistence)[0].focus_group, "todo");
        for (revision, target_phase, occurred_at) in [(0, "phase-2", 15), (1, "phase-1", 16)] {
            move_business_task(persistence, revision, target_phase, occurred_at);
        }
        let move_count: i64 = persistence
            .connection
            .query_row("SELECT COUNT(*) FROM business_task_moves", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(move_count, 2);
    }

    fn move_business_task(
        persistence: &mut SqlitePersistence,
        expected_revision: i64,
        target_phase: &str,
        occurred_at: i64,
    ) {
        let command = MoveBusinessTaskCommand {
            move_id: format!("business-task-move-{target_phase}"),
            business_task_id: "business-task-1".to_owned(),
            target_phase_id: target_phase.to_owned(),
            expected_revision,
            actor_label: "Mika".to_owned(),
            dependency_impact: "依赖随目标阶段重新确认".to_owned(),
            deliverable_impact: "输出归属随阶段调整".to_owned(),
            acceptance_impact: "原阶段与目标阶段验收口径已确认".to_owned(),
            client_operation_id: format!("op-business-task-move-{target_phase}"),
            request_fingerprint: format!("fp-business-task-move-{target_phase}"),
            event_id: EventId::new(format!("event-business-task-move-{target_phase}")),
            correlation_id: format!("correlation-business-task-move-{target_phase}"),
            occurred_at,
        };
        let moved = persistence
            .transact(|repositories| {
                contexthub_application::move_business_task(repositories, &command)
            })
            .unwrap();
        assert_eq!(moved.phase_id.as_deref(), Some(target_phase));
    }

    fn advance_business_task_through_rework(persistence: &mut SqlitePersistence) {
        for (revision, target, now, expected_group) in [
            (2, BusinessTaskStatus::Todo, 17, Some("todo")),
            (3, BusinessTaskStatus::InProgress, 18, Some("in_progress")),
            (4, BusinessTaskStatus::Review, 19, Some("review")),
            (5, BusinessTaskStatus::Rework, 20, Some("todo")),
            (6, BusinessTaskStatus::InProgress, 21, Some("in_progress")),
            (7, BusinessTaskStatus::Review, 22, Some("review")),
            (8, BusinessTaskStatus::Completed, 23, None),
        ] {
            transition_task(persistence, target, revision, now);
            assert_eq!(
                my_work_items(persistence)
                    .first()
                    .map(|item| item.focus_group.as_str()),
                expected_group
            );
        }
    }

    fn transition_task(
        persistence: &mut SqlitePersistence,
        target_status: BusinessTaskStatus,
        expected_revision: i64,
        occurred_at: i64,
    ) {
        let suffix = format!("{}-{occurred_at}", target_status.as_str());
        let command = TransitionBusinessTaskCommand {
            transition_id: format!("business-task-transition-{suffix}"),
            business_task_id: "business-task-1".to_owned(),
            target_status,
            expected_revision,
            actor_label: "Mika".to_owned(),
            comment: "确认任务状态".to_owned(),
            client_operation_id: format!("op-business-task-transition-{suffix}"),
            request_fingerprint: format!("fp-business-task-transition-{suffix}"),
            event_id: EventId::new(format!("event-business-task-transition-{suffix}")),
            correlation_id: format!("correlation-business-task-transition-{suffix}"),
            occurred_at,
        };
        persistence
            .transact(|repositories| {
                contexthub_application::transition_business_task(repositories, &command)
            })
            .unwrap();
    }

    fn attention_create(
        id: &str,
        case_type: AttentionCaseType,
        deliverable_id: Option<&str>,
        agent: Option<(&str, &str)>,
        occurred_at: i64,
    ) -> CreateAttentionCaseCommand {
        CreateAttentionCaseCommand {
            attention_case_id: id.to_owned(),
            case_type,
            project_id: "project-progress-1".to_owned(),
            phase_id: Some("phase-1".to_owned()),
            task_id: None,
            deliverable_id: deliverable_id.map(str::to_owned),
            title: format!("关注事项 {id}"),
            detail: "需要及时处理".to_owned(),
            responsibility_type: "user".to_owned(),
            responsibility_label: "Mika".to_owned(),
            reporter_label: "Mika".to_owned(),
            priority: "high".to_owned(),
            due_at: None,
            agent_run_id: agent.map(|value| value.0.to_owned()),
            agent_label: agent.map(|value| value.1.to_owned()),
            decision_options: (case_type == AttentionCaseType::DecisionRequest)
                .then(|| "方案 A；方案 B".to_owned()),
            decision_impact: (case_type == AttentionCaseType::DecisionRequest)
                .then(|| "影响项目路线".to_owned()),
            decision_evidence: (case_type == AttentionCaseType::DecisionRequest)
                .then(|| "已验收成果与风险记录".to_owned()),
            decision_recommendation: (case_type == AttentionCaseType::DecisionRequest)
                .then(|| "推荐方案 A".to_owned()),
            decision_consequence: (case_type == AttentionCaseType::DecisionRequest)
                .then(|| "延迟下一阶段".to_owned()),
            client_operation_id: format!("op-{id}"),
            request_fingerprint: format!("fp-{id}"),
            event_id: EventId::new(format!("event-{id}")),
            correlation_id: format!("correlation-{id}"),
            occurred_at,
        }
    }

    fn seed_attention_cases(persistence: &mut SqlitePersistence) {
        let blocker = attention_create(
            "blocker-1",
            AttentionCaseType::ProjectBlocker,
            None,
            None,
            20,
        );
        let commands = [
            blocker.clone(),
            attention_create(
                "decision-1",
                AttentionCaseType::DecisionRequest,
                Some("deliverable-progress-1"),
                None,
                21,
            ),
            attention_create(
                "agent-1",
                AttentionCaseType::AgentRunIncident,
                None,
                Some(("run-1", "Research Agent")),
                22,
            ),
        ];
        for command in &commands {
            persistence
                .transact(|repositories| {
                    contexthub_application::create_attention_case(repositories, command)
                })
                .unwrap();
        }
        let replayed = persistence
            .transact(|repositories| {
                contexthub_application::create_attention_case(repositories, &blocker)
            })
            .unwrap();
        assert!(replayed.replayed);
    }

    fn assert_attention_projection(persistence: &mut SqlitePersistence) {
        let items = my_work_items(persistence);
        assert_eq!(items.len(), 2);
        for (source, group, action) in [
            ("project_blocker", "todo", "blocker_resolve"),
            ("decision_request", "decision", "decision_respond"),
        ] {
            assert!(items.iter().any(|item| item.source_type == source
                && item.focus_group == group
                && item.action_type == action));
        }
    }

    fn assert_attention_transitions(persistence: &mut SqlitePersistence) {
        let invalid = attention_transition(
            "invalid-blocker-ack",
            "blocker-1",
            AttentionCaseStatus::Acknowledged,
            0,
            "",
            23,
        );
        let error = persistence
            .transact(|repositories| {
                contexthub_application::transition_attention_case(repositories, &invalid)
            })
            .unwrap_err();
        assert_eq!(error.reason(), Some("invalid_attention_transition"));

        persist_attention_transition(
            persistence,
            &attention_transition(
                "agent-ack",
                "agent-1",
                AttentionCaseStatus::Acknowledged,
                0,
                "",
                24,
            ),
        );
        assert!(
            my_work_items(persistence)
                .iter()
                .all(|item| item.source_id != "agent-1")
        );
        let mut delegated = attention_transition(
            "decision-delegate",
            "decision-1",
            AttentionCaseStatus::Open,
            0,
            "由交付负责人做最终判断",
            25,
        );
        delegated.decision_action = Some("delegated".to_owned());
        delegated.delegated_to = Some("交付负责人".to_owned());
        persist_attention_transition(persistence, &delegated);
        let delegated_item = my_work_items(persistence)
            .into_iter()
            .find(|item| item.source_id == "decision-1")
            .unwrap();
        assert_eq!(delegated_item.responsibility_label, "交付负责人");
        for transition in [
            attention_transition(
                "blocker-resolve",
                "blocker-1",
                AttentionCaseStatus::Resolved,
                0,
                "依赖已补齐",
                26,
            ),
            attention_transition(
                "decision-resolve",
                "decision-1",
                AttentionCaseStatus::Resolved,
                1,
                "采用方案 A",
                27,
            ),
            attention_transition(
                "agent-resolve",
                "agent-1",
                AttentionCaseStatus::Resolved,
                1,
                "外部执行器已恢复",
                28,
            ),
        ] {
            persist_attention_transition(persistence, &transition);
        }
        let decision = persistence
            .transact(|repositories| repositories.attention_cases("project-progress-1"))
            .unwrap()
            .into_iter()
            .find(|item| item.attention_case_id == "decision-1")
            .unwrap();
        assert_eq!(decision.decision_action.as_deref(), Some("approved"));
        assert_eq!(
            decision.decision_action_summary.as_deref(),
            Some("采用方案 A")
        );
        assert_eq!(decision.decision_actor_label.as_deref(), Some("Mika"));
        assert!(decision.decision_delegated_to.is_none());
        assert!(my_work_items(persistence).is_empty());
    }

    fn persist_attention_transition(
        persistence: &mut SqlitePersistence,
        transition: &TransitionAttentionCaseCommand,
    ) {
        persistence
            .transact(|repositories| {
                contexthub_application::transition_attention_case(repositories, transition)
            })
            .unwrap();
    }

    fn assert_attention_events(persistence: &SqlitePersistence) {
        let event_types = persistence.connection.prepare("SELECT event_type FROM durable_events WHERE object_type='attention_case' ORDER BY global_sequence").unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(
            event_types,
            vec![
                "project_blocker.opened",
                "decision_request.opened",
                "agent_run_incident.reported",
                "attention_case.acknowledged",
                "decision_request.delegated",
                "attention_case.resolved",
                "attention_case.resolved",
                "attention_case.resolved"
            ]
        );
    }

    fn attention_transition(
        suffix: &str,
        attention_case_id: &str,
        target_status: AttentionCaseStatus,
        expected_revision: i64,
        resolution_summary: &str,
        occurred_at: i64,
    ) -> TransitionAttentionCaseCommand {
        TransitionAttentionCaseCommand {
            transition_id: format!("attention-transition-{suffix}"),
            attention_case_id: attention_case_id.to_owned(),
            target_status,
            expected_revision,
            actor_label: "Mika".to_owned(),
            resolution_summary: resolution_summary.to_owned(),
            decision_action: (attention_case_id == "decision-1").then(|| "approved".to_owned()),
            delegated_to: None,
            client_operation_id: format!("op-attention-transition-{suffix}"),
            request_fingerprint: format!("fp-attention-transition-{suffix}"),
            event_id: EventId::new(format!("event-attention-transition-{suffix}")),
            correlation_id: format!("correlation-attention-transition-{suffix}"),
            occurred_at,
        }
    }

    fn my_work_items(persistence: &mut SqlitePersistence) -> Vec<MyWorkItemRecord> {
        persistence
            .transact(|repositories| {
                contexthub_application::get_my_work(
                    repositories,
                    &MyWorkQuery {
                        project_id: None,
                        focus_group: None,
                        limit: 200,
                    },
                )
            })
            .unwrap()
            .items
    }

    fn assert_project_outcome_events(persistence: &SqlitePersistence) {
        let event_types = persistence
            .connection
            .prepare("SELECT event_type FROM durable_events ORDER BY global_sequence")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(event_types.contains(&"project.created".to_owned()));
        assert!(event_types.contains(&"deliverable.version.accepted".to_owned()));
    }

    fn local_observation(
        proposed_id: &str,
        normalized_path: &str,
        identity: &str,
        checksum: &str,
    ) -> LocalFileObservation {
        LocalFileObservation {
            proposed_local_file_id: proposed_id.to_owned(),
            relative_path: normalized_path.to_owned(),
            normalized_relative_path: normalized_path.to_owned(),
            file_identity: Some(identity.to_owned()),
            file_type: "file".to_owned(),
            size_bytes: Some(10),
            modified_at_fs: Some(5),
            checksum_algorithm: Some("sha256".to_owned()),
            checksum_value: Some(checksum.to_owned()),
        }
    }

    fn local_snapshot(
        snapshot_id: &str,
        generation: i64,
        timestamp: i64,
        observations: Vec<LocalFileObservation>,
    ) -> LocalScanSnapshot {
        LocalScanSnapshot {
            snapshot_id: snapshot_id.to_owned(),
            workspace_id: WorkspaceId::new("w-snapshot"),
            generation,
            snapshot_kind: SnapshotKind::Full,
            observed_at: timestamp,
            completed_at: timestamp,
            issue_count: 0,
            event_id: EventId::new(format!("event-{snapshot_id}")),
            correlation_id: format!("correlation-{snapshot_id}"),
            observations,
            removed_normalized_paths: Vec::new(),
            covers_whole_tree: true,
        }
    }

    fn platform_snapshot(
        snapshot_id: &str,
        revision: &str,
        path: &str,
        version_id: &str,
        timestamp: i64,
    ) -> HistoricalPlatformSnapshot {
        HistoricalPlatformSnapshot {
            snapshot_id: snapshot_id.into(),
            revision: revision.into(),
            path: path.into(),
            version_id: version_id.into(),
            timestamp,
        }
    }

    fn state_input(
        local_checksum: &str,
        latest_version: &str,
        latest_checksum: &str,
    ) -> FileStateInput {
        FileStateInput {
            workspace_kind: contexthub_domain::WorkspaceKind::CloudTeam,
            platform_connection: PlatformConnection::Bound,
            platform_state: PlatformDocumentState::Published,
            materialization_state: MaterializationState::Materialized,
            scan_state: LocalScanState::Current,
            node_kind: LocalNodeKind::File,
            baseline_path_relation: BaselinePathRelation::Unchanged,
            baseline_version_id: Some("version-1".to_owned()),
            baseline_checksum_algorithm: Some("sha256".to_owned()),
            baseline_checksum_value: Some("checksum-version-1".to_owned()),
            latest_published_version_id: Some(latest_version.to_owned()),
            latest_published_checksum_algorithm: Some("sha256".to_owned()),
            latest_published_checksum_value: Some(latest_checksum.to_owned()),
            local_checksum_algorithm: Some("sha256".to_owned()),
            local_checksum_value: Some(local_checksum.to_owned()),
            active_confirmation: None,
            submission_state: SubmissionState::None,
            active_conflict: None,
            permissions: FilePermissions::from_allowed([
                FilePermission::Access,
                FilePermission::Confirm,
                FilePermission::Submit,
            ]),
            metadata_validity: MetadataValidity::Valid,
        }
    }

    fn state_command(
        label: &str,
        expected_state_revision: Option<i64>,
        input: FileStateInput,
        computed_at: i64,
    ) -> RecomputeFileStateCommand {
        RecomputeFileStateCommand {
            workspace_id: WorkspaceId::new("w-snapshot"),
            local_file_id: "state-file-1".to_owned(),
            expected_state_revision,
            input,
            confirmation_invalidation_id: format!("invalidation-{label}"),
            confirmation_invalidation_event_id: EventId::new(format!("event-invalidation-{label}")),
            derived_conflict_id: format!("conflict-{label}"),
            event_id: EventId::new(format!("event-{label}")),
            correlation_id: format!("correlation-{label}"),
            computed_at,
        }
    }

    fn test_task(task_id: &str, cancellable: bool) -> TaskRecord {
        TaskRecord {
            task_id: TaskId::new(task_id),
            task_type: "scan".to_owned(),
            workspace_id: None,
            state: TaskState::Queued,
            progress_completed: 0,
            progress_total: None,
            current_stage: None,
            cancellable,
            payload_json: "{}".to_owned(),
            result_ref_type: None,
            result_ref_id: None,
            correlation_id: format!("correlation-{task_id}"),
            created_by_client: "desktop".to_owned(),
            created_at: 1,
            started_at: None,
            completed_at: None,
            heartbeat_at: None,
            error_reason: None,
            error_detail_json: None,
        }
    }

    fn task_event(event_id: &str, task_id: &str, timestamp: i64) -> NewDurableEvent {
        NewDurableEvent {
            event_id: EventId::new(event_id),
            workspace_id: None,
            event_type: "task.changed".to_owned(),
            object_type: Some("task".to_owned()),
            object_id: Some(task_id.to_owned()),
            payload_json: "{}".to_owned(),
            correlation_id: Some(format!("correlation-{task_id}")),
            occurred_at: timestamp,
            created_at: timestamp,
        }
    }

    fn failed_transition(task_id: &str, timestamp: i64) -> TaskTransition {
        TaskTransition {
            task_id: TaskId::new(task_id),
            expected_state: TaskState::Running,
            target_state: TaskState::Failed,
            occurred_at: timestamp,
            result_ref_type: None,
            result_ref_id: None,
            error_reason: Some("execution_failed".to_owned()),
            error_detail_json: Some("{}".to_owned()),
        }
    }

    fn transition_without_result(
        repositories: &mut SqliteRepositoryTransaction<'_>,
        task_id: &str,
        expected_state: TaskState,
        target_state: TaskState,
        occurred_at: i64,
    ) -> Result<TaskRecord, PersistenceError> {
        repositories.transition_task(&TaskTransition {
            task_id: TaskId::new(task_id),
            expected_state,
            target_state,
            occurred_at,
            result_ref_type: None,
            result_ref_id: None,
            error_reason: None,
            error_detail_json: None,
        })
    }

    fn assert_invalid_progress(
        persistence: &mut SqlitePersistence,
        progress_completed: i64,
        progress_total: Option<i64>,
    ) {
        let error = persistence
            .transact(|repositories| {
                repositories.update_task_progress(&TaskProgress {
                    task_id: TaskId::new("t1"),
                    progress_completed,
                    progress_total,
                    current_stage: Some("scan".to_owned()),
                    heartbeat_at: 4,
                })
            })
            .unwrap_err();
        assert_eq!(error.kind(), PersistenceErrorKind::Conflict);
    }

    #[test]
    fn unknown_schema_without_history_is_not_adopted() {
        let fixture = TestDatabase::new("unknown-schema");
        let connection = Connection::open(&fixture.path).unwrap();
        connection
            .execute_batch("CREATE TABLE unexpected(id INTEGER PRIMARY KEY);")
            .unwrap();
        drop(connection);
        let error = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap_err();
        assert_eq!(error.kind(), PersistenceErrorKind::Migration);
        assert!(error.to_string().contains("without a migration history"));
    }

    #[test]
    fn active_writer_is_detected_without_waiting() {
        let fixture = TestDatabase::new("writer");
        let persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        validate_no_active_writer(&fixture.path).unwrap();
        persistence
            .connection
            .execute_batch("BEGIN IMMEDIATE")
            .unwrap();
        let started = std::time::Instant::now();
        let error = validate_no_active_writer(&fixture.path).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(error.kind(), PersistenceErrorKind::Database);
        persistence.connection.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "single acceptance history proves business commit, projection conflict, restoration, and retry"
    )]
    fn projection_export_tracks_pending_failure_conflict_and_success_without_rolling_back_business()
    {
        let fixture = TestDatabase::new("projection-export");
        let workspace_root = TestWorkspace::new("projection-export");
        let mut persistence = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        persistence
            .transact(|repositories| {
                repositories.insert_workspace(&WorkspaceRecord {
                    workspace_id: WorkspaceId::new("w-projection"),
                    workspace_kind: WorkspaceKind::Local,
                    display_name: "Projection".to_owned(),
                    local_root: workspace_root.path.to_string_lossy().into_owned(),
                    local_root_identity: None,
                    local_status: LocalWorkspaceStatus::Ready,
                    state_revision: 0,
                    last_local_scan_at: None,
                    created_at: 1,
                    updated_at: 1,
                })?;
                repositories.initialize_workspace_export(&WorkspaceExportRecord {
                    workspace_id: WorkspaceId::new("w-projection"),
                    desired_revision: 1,
                    exported_revision: 0,
                    updated_at: 1,
                })
            })
            .unwrap();
        let mut writer = WorkspaceProjectionWriter;
        assert_eq!(
            export_workspace_projection(
                &mut persistence,
                &mut writer,
                &WorkspaceId::new("w-projection"),
                2,
            )
            .unwrap(),
            contexthub_application::ProjectionExportOutcome::Exported
        );
        let projection_directory = workspace_root.path.join(".cilai");
        let original_workspace = fs::read(projection_directory.join("workspace.json")).unwrap();
        let first_state: contexthub_manifest::ExportState = serde_json::from_slice(
            &fs::read(projection_directory.join("export-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(first_state.database_export_revision, 1);

        // 模拟升级前的目录；数据库版本和清单摘要不变也必须完成目录迁移。
        let legacy_directory = workspace_root.path.join(".contexthub");
        fs::rename(&projection_directory, &legacy_directory).unwrap();
        assert_eq!(
            export_workspace_projection(
                &mut persistence,
                &mut writer,
                &WorkspaceId::new("w-projection"),
                2,
            )
            .unwrap(),
            contexthub_application::ProjectionExportOutcome::AlreadyCurrent
        );
        assert!(!legacy_directory.exists());
        assert_eq!(
            fs::read(projection_directory.join("workspace.json")).unwrap(),
            original_workspace
        );

        persistence
            .transact(|repositories| {
                insert_historical_account(&repositories.transaction, "a-projection", "Account", 3)?;
                insert_historical_platform_binding(
                    &repositories.transaction,
                    &WorkspacePlatformBindingRecord {
                        workspace_id: WorkspaceId::new("w-projection"),
                        account_id: AccountId::new("a-projection"),
                        organization_id: Some("org".to_owned()),
                        platform_workspace_id: Some("platform-workspace".to_owned()),
                        project_id: "project".to_owned(),
                        platform_root: "/docs".to_owned(),
                        binding_state: WorkspacePlatformBindingState::Bound,
                        platform_snapshot_revision: None,
                        last_platform_check_at: None,
                        team_space_id: None,
                        managed_root: None,
                        device_id: None,
                        baseline_revision: None,
                        change_cursor: None,
                        permission_role: None,
                        permission_revision: None,
                        created_at: 3,
                        updated_at: 3,
                    },
                )
            })
            .unwrap();
        fs::write(projection_directory.join("workspace.json"), b"{}\n").unwrap();
        assert!(matches!(
            export_workspace_projection(
                &mut persistence,
                &mut writer,
                &WorkspaceId::new("w-projection"),
                4,
            ),
            Err(contexthub_application::ExportWorkspaceProjectionError::Writer(_))
        ));
        let after_failure: (i64, i64, String, i64) = persistence
            .connection
            .query_row(
                "SELECT e.desired_revision, e.exported_revision, e.export_state, (SELECT count(*) FROM workspace_platform_bindings WHERE workspace_id=e.workspace_id) FROM workspace_exports e WHERE e.workspace_id='w-projection'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(after_failure, (2, 1, "conflict".to_owned(), 1));

        fs::write(
            projection_directory.join("workspace.json"),
            original_workspace,
        )
        .unwrap();
        export_workspace_projection(
            &mut persistence,
            &mut writer,
            &WorkspaceId::new("w-projection"),
            5,
        )
        .unwrap();
        let final_state: (i64, i64, String, Option<String>) = persistence
            .connection
            .query_row(
                "SELECT desired_revision, exported_revision, export_state, last_error_reason FROM workspace_exports WHERE workspace_id='w-projection'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(final_state, (2, 2, "current".to_owned(), None));
        let workspace: contexthub_manifest::WorkspaceManifest =
            serde_json::from_slice(&fs::read(projection_directory.join("workspace.json")).unwrap())
                .unwrap();
        assert_eq!(workspace.project_id.as_deref(), Some("project"));
        assert_eq!(workspace.database_export_revision, 2);
    }

    #[test]
    fn user_skill_mutations_are_revisioned_and_idempotent() {
        let database = TestDatabase::new("user-skills");
        let mut persistence = SqlitePersistence::open(&database.path, "test").unwrap();
        let save = UserSkillSaveCommand {
            client_operation_id: "save-skill-1".to_owned(),
            request_fingerprint: "a".repeat(64),
            skill_id: "skill_0123456789abcdef0123456789abcdef".to_owned(),
            name: "周报整理".to_owned(),
            normalized_name: "周报整理".to_owned(),
            description: "整理周报".to_owned(),
            managed_relative_path: "user/skill_0123456789abcdef0123456789abcdef/SKILL.md"
                .to_owned(),
            enabled: true,
            automatic: false,
            expected_revision: None,
            content_digest: "b".repeat(64),
            occurred_at: 10,
        };
        let created = persistence.save_user_skill(&save).unwrap();
        assert_eq!(created.state_revision, 1);
        assert!(created.enabled);
        assert!(!created.automatic);

        let repeated = persistence.save_user_skill(&save).unwrap();
        assert_eq!(repeated, created);

        let updated = persistence
            .set_user_skill_state(
                &created.skill_id,
                true,
                true,
                created.state_revision,
                "set-state-1",
                &"c".repeat(64),
                11,
            )
            .unwrap();
        assert_eq!(updated.state_revision, 2);
        assert!(updated.automatic);

        let conflict = persistence
            .set_user_skill_state(
                &created.skill_id,
                false,
                false,
                created.state_revision,
                "set-state-stale",
                &"d".repeat(64),
                12,
            )
            .unwrap_err();
        assert_eq!(conflict.reason(), Some("skill_revision_conflict"));

        assert!(
            persistence
                .delete_user_skill(
                    &created.skill_id,
                    updated.state_revision,
                    "delete-skill-1",
                    &"e".repeat(64),
                    13,
                )
                .unwrap()
        );
        assert!(persistence.user_skill(&created.skill_id).unwrap().is_none());
        assert!(
            persistence
                .delete_user_skill(
                    &created.skill_id,
                    updated.state_revision,
                    "delete-skill-1",
                    &"e".repeat(64),
                    13,
                )
                .unwrap()
        );
        assert_eq!(
            persistence
                .delete_user_skill(
                    "skill_ffffffffffffffffffffffffffffffff",
                    updated.state_revision,
                    "delete-skill-1",
                    &"e".repeat(64),
                    13,
                )
                .unwrap_err()
                .reason(),
            Some("operation_id_reused")
        );
    }

    #[derive(Debug)]
    struct TestDatabase {
        root: PathBuf,
        path: PathBuf,
    }

    struct TestWorkspace {
        path: PathBuf,
    }

    impl TestWorkspace {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "ch-workspace-{label}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }

    impl TestDatabase {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("ch-sqlite-{label}-{}-{nanos}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self {
                path: root.join("contexthub.db"),
                root,
            }
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            for suffix in ["-shm", "-wal", ""] {
                let path = PathBuf::from(format!("{}{suffix}", self.path.display()));
                if path.exists() {
                    fs::remove_file(path).unwrap();
                }
            }
            let backups = self.root.join("schema-backups");
            if backups.exists() {
                fs::remove_dir_all(backups).unwrap();
            }
            fs::remove_dir(&self.root).unwrap();
        }
    }
}
