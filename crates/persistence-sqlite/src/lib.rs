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
mod file_search_index;
pub use file_search_index::{FileSearchContent, FileSearchScan};
mod global_search;
pub use global_search::GlobalSearchReader;
mod workspace_repository;
use workspace_repository::mark_projection_pending;
mod project_outcomes;
mod task_repository;

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
    Migration {
        version: 77,
        name: "add_file_text_search",
        sql: include_str!("../../../migrations/0077_add_file_text_search.sql"),
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

fn configuration_error(message: &'static str) -> PersistenceError {
    PersistenceError::new(PersistenceErrorKind::Configuration, message)
}

fn conflict_error(message: &'static str) -> PersistenceError {
    PersistenceError::new(PersistenceErrorKind::Conflict, message)
}

fn confirmation_conflict(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
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
mod tests;
