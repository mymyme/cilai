#![recursion_limit = "512"]
use contexthub_protocol::{
    AiRuntimeQuery, AiRuntimeQueryScope, ConversationSessionChange, ConversationSessionPatch,
    ConversationSessionSnapshot,
};
use contexthub_protocol::{
    PersonalApplicationDataAction, PersonalApplicationDataConfirmParams,
    PersonalApplicationDataExecuteParams, PersonalApplicationDataOperation,
    PersonalApplicationDataParams, PersonalApplicationDataPrepareParams,
    PersonalApplicationDataState,
};
use contexthub_protocol::{
    PersonalMemoryAuthorization, PersonalMemoryEnableConfirmation, PersonalMemoryParams,
    PersonalMemoryPreferences, PersonalMemoryResetConfirmParams, PersonalMemoryResetFinishParams,
    PersonalMemoryResetOperation, PersonalMemoryResetOutcome, PersonalMemoryResetParams,
    PersonalMemoryResetPermit, PersonalMemoryResetPrepareParams, PersonalMemoryResetScope,
    PersonalMemoryResetScopeCompleteParams, PersonalMemoryResetScopeResult,
    PersonalMemoryResetStartParams, PersonalMemoryResetStartResult, PersonalMemoryResetState,
    PersonalMemoryResetWriterParams,
};
use contexthub_protocol::{
    PersonalNamespaceActionAckParams, PersonalNamespaceActionOutcome,
    PersonalNamespaceActionParams, PersonalNamespaceActionPermit, PersonalNamespaceActionReceipt,
    PersonalNamespaceActionState, PersonalNamespaceBrowserSnapshot, PersonalNamespaceLogFile,
    PersonalNamespaceLogSnapshot, PersonalNamespaceStorageConfirmParams,
    PersonalNamespaceStorageKind, PersonalNamespaceStorageOperation,
    PersonalNamespaceStorageParams, PersonalNamespaceStoragePrepareParams,
    PersonalNamespaceStorageStartParams, PersonalNamespaceStorageStartResult,
};
use contexthub_protocol::{
    PersonalPageAiManifest, PersonalPageAuditEvent, PersonalPageAuditFinishParams,
    PersonalPageAuditListParams, PersonalPageAuditListResult, PersonalPageAuditMethod,
    PersonalPageAuditOutcome, PersonalPageAuditPrepareParams, PersonalPageAuditRecord,
    PersonalPageBindingParams, PersonalPageBindingSnapshot, PersonalPageConfirmationPolicy,
    PersonalPageContextType, PersonalPageRiskLevel, PersonalPageRuntimeBinding, PersonalPageTool,
    PersonalPageUiAction,
};
use contexthub_protocol::{
    PersonalRestoreWorkspaceDestination, PersonalRestoreWorkspacePlanParams,
    PersonalRestoredWorkspace, PersonalWorkspaceContentEntry, PersonalWorkspaceContentSnapshot,
};

use contexthub_protocol::{
    PersonalAutomationAdmitParams, PersonalAutomationAdmitResult, PersonalAutomationBindParams,
    PersonalAutomationBindResult, PersonalAutomationClaim, PersonalAutomationFinishOutcome,
    PersonalAutomationFinishParams, PersonalAutomationFinishResult,
    PersonalAutomationListTriggersParams, PersonalAutomationPrepareExecutionParams,
    PersonalAutomationResolveInterruptedParams, PersonalAutomationRuleRecord,
    PersonalAutomationTriggerItem, PersonalAutomationTriggerOutcome, PersonalAutomationTriggerPage,
    PersonalAutomationTriggerRecord,
};

use contexthub_protocol::{
    ModelUsageBeginParams, ModelUsageCost, ModelUsageFinishParams, ModelUsageListParams,
    ModelUsageListResult, ModelUsageParams, ModelUsageRecord, ModelUsageState, ModelUsageTokens,
};

use contexthub_protocol::{
    AiApplicationAction, AiApplicationParams, AiHistoryParams, AiHistoryResult,
    AiLedgerApplyParams, AiLedgerApplyResult, AiLedgerGetParams, AiLedgerGetResult,
    AiLedgerRecordMutation, AiLedgerRecordReceipt,
};
use contexthub_protocol::{
    LocalModelRuntime, ModelAuthMode, ModelCapabilityEvidence, ModelCapabilityState,
    ModelConnection, ModelConnectionChange, ModelConnectionChangeState,
    ModelConnectionCommitParams, ModelConnectionConfig, ModelConnectionKind,
    ModelConnectionMutation, ModelConnectionMutationParams, ModelConnectionParams,
    ModelConnectionPreflight, ModelConnectionPreflightParams, ModelConnectionReference,
    ModelConnectionSavePrepareParams, ModelConnectionSetEnabledParams, ModelCredentialAction,
    ModelDescriptor, ModelDescriptorObservation, ModelDescriptorObserveParams,
    ModelDescriptorSaveParams, ModelEvidenceSource, ModelExecutionLease, ModelExecutionLeaseParams,
    ModelExecutionListParams, ModelExecutionPrepareParams, ModelExecutionSnapshot,
    ModelHistoryConsent, ModelNetworkAuthentication, ModelNetworkConfig, ModelNetworkMode,
    ModelOperationParams, ModelPurpose, ModelPurposeBinding, ModelPurposeSaveParams,
    ModelSecretRef, ModelSelection, ModelSelectionPolicy, ModelThreadRuntimeScope,
    ModelValidationLayer, ModelValidationOutcome, ModelValidationPrepareParams,
    ModelValidationRecord, ModelValidationReportParams, PersonalProfileManifest,
};
use contexthub_protocol::{
    McpAuthMode, McpChangeState, McpConnection, McpConnectionChange, McpConnectionConfig,
    McpConnectionParams, McpCredentialAction, McpCredentialCleanupConfirmParams,
    McpCredentialCleanupParams, McpCredentialCleanupPlan, McpCredentialCleanupState,
    McpExecutionFinishParams, McpExecutionIntent, McpExecutionLease, McpExecutionParams,
    McpExecutionPrepareParams, McpExecutionRegisterParams, McpExecutionState, McpGrantParams,
    McpLocalPermissions, McpMutationParams, McpNetworkConfig, McpOperationParams, McpPurpose,
    McpSaveCommitParams, McpSavePrepareParams, McpSecretRef, McpSnapshotParams, McpSource,
    McpToolDefinition, McpToolSnapshot, McpValidateParams, McpValidationRecord,
    McpValidationReportParams, McpValidationState, PersonalBackupArtifact,
    PersonalBackupCaptureParams, PersonalBackupFile, PersonalBackupManifest,
    PersonalBackupNativeHome, PersonalBackupPrepareParams, PersonalBackupSelection,
    PersonalBackupWorkspace, PersonalDataCancelParams, PersonalDataCategory, PersonalDataIssue,
    PersonalDataListParams, PersonalDataOperation, PersonalDataOperationKind,
    PersonalDataOperationState, PersonalDataParams, PersonalDataQuiescence, PersonalDataResult,
    PersonalMaintenanceAcquireParams, PersonalMaintenanceDrainStatus, PersonalMaintenanceFence,
    PersonalMaintenanceListParams, PersonalMaintenanceParams, PersonalMaintenanceState,
    PersonalRestoreAbortBeforeHelperParams, PersonalRestoreApplyPrepareParams,
    PersonalRestorePrepareParams, PersonalRestorePreview, PersonalWriterAbortParams,
    PersonalWriterLease, PersonalWriterLeaseParams, PersonalWriterListParams, PersonalWriterMode,
    PersonalWriterPrepareParams, PersonalWriterRegisterParams, PersonalWriterState,
};
use contexthub_protocol::{
    ModelAuthAbortUnlaunchedParams, ModelCredentialCleanupAckParams, ModelCredentialCleanupState,
};
use contexthub_protocol::{
    ModelAuthChangeState, ModelAuthIntent, ModelAuthLogoutPrepareParams, ModelAuthProcess,
    ModelAuthRecoverParams, ModelAuthRegisterProcessParams, ModelAuthScope, ModelAuthScopeState,
    ModelAuthStartPrepareParams, ModelAuthStep, ModelAuthStepParams, ModelAuthStepState,
    ModelConnectionChangeIntent, ModelRuntimeKind, RootAuthChange,
};
use contexthub_protocol::{
    ModelCredentialCleanupCandidate, ModelCredentialCleanupConfirmParams,
    ModelCredentialCleanupPlan, ModelCredentialCleanupPlanState,
    ModelCredentialCleanupPrepareParams,
};
use contexthub_protocol::{ModelExecutionAbortUnlaunchedParams, ModelValidationExecutionParams};
use contexthub_protocol::{
    ModelExecutionLeaseDiagnostic, ModelExecutionOwner, ModelExecutionProcess,
    ModelExecutionRecoverParams, ModelExecutionRecoverResult, ModelExecutionRecoveryState,
    ModelExecutionRegisterProcessParams,
};
use contexthub_protocol::{
    PersonalPackageImportCancelParams, PersonalPackageImportCommitParams,
    PersonalPackageImportCommitResult, PersonalPackageImportListParams,
    PersonalPackageImportParams, PersonalPackageImportPrepareParams, PersonalPackageImportPreview,
    PersonalPackageImportSource, PersonalPackageImportState, PersonalPackageImportTarget,
};

use contexthub_protocol::{
    AgentIncidentReportParams, AgentIncidentReportResult, AiApplicationSessionKind,
    AiConversationSetArchivedParams, AiConversationSetArchivedResult, AiStateApplyParams,
    AiStateApplyResult, AiStateDelta, AiStateEntityDelta, AiStateEntityUpsert, AiStateGetParams,
    AiStateGetResult, AiStateReplaceParams, AiStateReplaceResult, AiThreadAcknowledgeParams,
    AiThreadApplicationScope, AiThreadCleanupParams, AiThreadGetParams, AiThreadMutationResult,
    AiThreadOperationEvent, AiThreadOperationKind, AiThreadPrepareParams, AiThreadReleaseParams,
    AiThreadState, AiTurnClassification, AiTurnFinalizeParams, AiTurnFinalizeResult,
    AiWorkTaskCandidate, AiWorkTaskCandidatesParams, AiWorkTaskCandidatesResult,
    ApplicationSkillBinding, AttentionCaseItem, AttentionCaseListParams, AttentionCaseListResult,
    AttentionCaseTransitionParams, AttentionCaseTransitionResult, BusinessTaskContributionItem,
    BusinessTaskCreateParams, BusinessTaskCreateResult, BusinessTaskGetParams,
    BusinessTaskGetResult, BusinessTaskItem, BusinessTaskMoveParams, BusinessTaskMoveResult,
    BusinessTaskTransitionParams, BusinessTaskTransitionResult,
    CapabilityCreationAcceptanceExample, CapabilityCreationCreateParams,
    CapabilityCreationCreateResult, CapabilityCreationGetParams, CapabilityCreationGetResult,
    CapabilityCreationInstallParams, CapabilityCreationInstallResult, CapabilityCreationListParams,
    CapabilityCreationListResult, CapabilityCreationPreviewParams, CapabilityCreationPreviewResult,
    CapabilityCreationProject, CapabilityCreationSourceFile, ClientCapabilities, ClientInfo,
    ComputeTextDiffParams, ComputeTextDiffResult, ConfirmFileParams, ConfirmFileResult,
    ConfirmationHistoryItem, ConfirmationHistoryParams, ConfirmationHistoryResult, ConflictItem,
    DecisionRequestCreateParams, DecisionRequestCreateResult, DeliverableCarrierInputItem,
    DeliverableCarrierItem, DeliverableCreateParams, DeliverableCreateResult,
    DeliverableFileBindingItem, DeliverableItem, DeliverableListParams, DeliverableListResult,
    DeliverableVersionCreateParams, DeliverableVersionCreateResult, DeliverableVersionItem,
    DeliverableVersionReviewParams, DeliverableVersionReviewResult, DiffSourceRef,
    DurableEventItem, ErrorResponse, EventReplayParams, EventReplayResult, GlobalSearchItem,
    GlobalSearchParams, GlobalSearchResult, InitializeParams, InitializeResult, MyWorkGetParams,
    MyWorkGetResult, MyWorkItem, OpenPreviewParams, OpenPreviewResult,
    PersonalApplicationOpenParams, PersonalApplicationOpenResult, PhaseDraftItem,
    PhasePlanActivateParams, PhasePlanActivateResult, PhasePlanCreateParams, PhasePlanCreateResult,
    PhasePlanItem, PreviewAvailability, PreviewFormat, PreviewSource, ProjectAttachWorkspaceParams,
    ProjectAttachWorkspaceResult, ProjectBlockerCreateParams, ProjectBlockerCreateResult,
    ProjectCreateParams, ProjectCreateResult, ProjectDeleteParams, ProjectDeleteResult,
    ProjectDetachWorkspaceParams, ProjectDetachWorkspaceResult, ProjectGetParams, ProjectGetResult,
    ProjectListParams, ProjectListResult, ProjectPhaseItem, ProjectPhaseTransitionParams,
    ProjectPhaseTransitionResult, ProjectPortfolioItem, ProjectPortfolioListParams,
    ProjectPortfolioListResult, ProjectPortfolioPhaseItem, ProjectProgressGetParams,
    ProjectProgressGetResult, ProjectSummary, ProjectUpdateParams, ProjectUpdateResult,
    ProtocolLimits, ProtocolRange, ReadTextDiffParams, ReadTextDiffResult, ReadTextResourceParams,
    ReadTextResourceResult, RequestId, ResumeCursor, ResumeResult, ServerCapabilities, ServerInfo,
    SessionInfo, SkillConversationCatalogItem, SkillConversationCatalogParams,
    SkillConversationCatalogResult, SkillDeleteParams, SkillDeleteResult, SkillDetail,
    SkillGetParams, SkillGetResult, SkillImportDirectoryParams, SkillImportParams,
    SkillImportResult, SkillListParams, SkillListResult, SkillPackageFile, SkillPackageFileInput,
    SkillRunInstallation, SkillRunListParams, SkillRunListResult, SkillRunRetryParams,
    SkillRuntimeCatalogItem, SkillRuntimeCatalogParams, SkillRuntimeCatalogResult, SkillSaveParams,
    SkillSaveResult, SkillSetStateParams, SkillSetStateResult, SkillSummary, SuccessResponse,
    TaskAssigneeType, TextDiffRow, TextDiffRowKind, TextDiffSummary, UnifiedSearchCursor,
    UnifiedSearchDeepLink, UnifiedSearchGroup, UnifiedSearchItem, UnifiedSearchLocalTarget,
    UnifiedSearchParams, UnifiedSearchPartialFailure, UnifiedSearchResult, WorkPriority,
    WorkspaceCreateParams, WorkspaceCreateResult, WorkspaceDeleteParams, WorkspaceDeleteResult,
    WorkspaceDirectoryCreateParams, WorkspaceDirectoryCreateResult, WorkspaceDirectoryEntry,
    WorkspaceFileItem, WorkspaceFilesParams, WorkspaceFilesResult, WorkspaceIndexPathParams,
    WorkspaceIndexPathResult, WorkspaceIndexRequestParams, WorkspaceIndexRequestResult,
    WorkspaceIndexStatusParams, WorkspaceIndexStatusResult, WorkspaceListDirectoryParams,
    WorkspaceListDirectoryResult, WorkspaceListParams, WorkspaceListResult, WorkspaceScanParams,
    WorkspaceScanResult, WorkspaceSummary, WorkspaceTargetValidateParams,
    WorkspaceTargetValidateResult,
};
use contexthub_protocol::{
    FileHydrationParams, FileHydrationResult, ReadBinaryPreviewParams, ReadBinaryPreviewResult,
};
use contexthub_protocol::{
    PersonalMaintenanceOperationStatus, PersonalStorageCategory, PersonalStorageConfirmParams,
    PersonalStorageCoverage, PersonalStorageCoverageState, PersonalStorageExecuteParams,
    PersonalStorageInventory, PersonalStorageInventoryParams, PersonalStorageItem,
    PersonalStorageItemResult, PersonalStorageOperation, PersonalStorageOperationState,
    PersonalStorageOutcome, PersonalStorageParams, PersonalStoragePrepareParams,
    PersonalStorageReference, PersonalStorageResult,
};
use schemars::{JsonSchema, schema_for};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use ts_rs::TS;

struct Artifact {
    path: PathBuf,
    content: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "--check".to_owned());
    if mode != "--check" && mode != "--write" {
        return Err("usage: generate-contracts [--check|--write]".to_owned());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("cannot resolve repository root")?;
    let artifacts = artifacts(root)?;
    validate_artifacts(&artifacts)?;
    let mut drift = Vec::new();
    for artifact in artifacts {
        if mode == "--write" {
            if let Some(parent) = artifact.path.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(&artifact.path, artifact.content).map_err(|error| error.to_string())?;
        } else if fs::read_to_string(&artifact.path).ok().as_deref() != Some(&artifact.content) {
            drift.push(artifact.path);
        }
    }
    if drift.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "protocol generated artifacts drifted; run with --write:\n{}",
            drift
                .iter()
                .map(|path| format!("- {}", path.display()))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

fn validate_artifacts(artifacts: &[Artifact]) -> Result<(), String> {
    let mut paths = std::collections::BTreeSet::new();
    for artifact in artifacts {
        if !paths.insert(&artifact.path) {
            return Err(format!(
                "duplicate generated artifact: {}",
                artifact.path.display()
            ));
        }
        if artifact
            .path
            .extension()
            .is_some_and(|extension| extension == "ts")
        {
            let mut names = std::collections::BTreeSet::new();
            for line in artifact
                .content
                .lines()
                .filter(|line| line.starts_with("export type "))
            {
                let name = line
                    .split_whitespace()
                    .nth(2)
                    .ok_or("invalid generated declaration")?;
                if !names.insert(name) {
                    return Err(format!("duplicate generated declaration: {name}"));
                }
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "all generated protocol artifacts remain visibly enumerated in one frozen source"
)]
fn artifacts(root: &Path) -> Result<Vec<Artifact>, String> {
    let schema_dir = root.join("schemas/jsonrpc");
    Ok(vec![
        json_artifact(
            schema_dir.join("model-auth-abort-unlaunched-params.schema.json"),
            &schema_for!(ModelAuthAbortUnlaunchedParams),
        )?,
        json_artifact(
            schema_dir.join("model-validation-execution-params.schema.json"),
            &schema_for!(ModelValidationExecutionParams),
        )?,
        json_artifact(
            schema_dir.join("model-execution-abort-unlaunched-params.schema.json"),
            &schema_for!(ModelExecutionAbortUnlaunchedParams),
        )?,
        json_artifact(
            schema_dir.join("root-auth-change-list.schema.json"),
            &schema_for!(Vec<RootAuthChange>),
        )?,
        json_artifact(
            schema_dir.join("model-runtime-kind.schema.json"),
            &schema_for!(ModelRuntimeKind),
        )?,
        json_artifact(
            schema_dir.join("model-connection-change-intent.schema.json"),
            &schema_for!(ModelConnectionChangeIntent),
        )?,
        json_artifact(
            schema_dir.join("model-auth-scope-state.schema.json"),
            &schema_for!(ModelAuthScopeState),
        )?,
        json_artifact(
            schema_dir.join("model-auth-scope.schema.json"),
            &schema_for!(ModelAuthScope),
        )?,
        json_artifact(
            schema_dir.join("model-auth-intent.schema.json"),
            &schema_for!(ModelAuthIntent),
        )?,
        json_artifact(
            schema_dir.join("model-auth-change-state.schema.json"),
            &schema_for!(ModelAuthChangeState),
        )?,
        json_artifact(
            schema_dir.join("model-auth-step-state.schema.json"),
            &schema_for!(ModelAuthStepState),
        )?,
        json_artifact(
            schema_dir.join("model-auth-step.schema.json"),
            &schema_for!(ModelAuthStep),
        )?,
        json_artifact(
            schema_dir.join("root-auth-change.schema.json"),
            &schema_for!(RootAuthChange),
        )?,
        json_artifact(
            schema_dir.join("model-auth-start-prepare-params.schema.json"),
            &schema_for!(ModelAuthStartPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("model-auth-logout-prepare-params.schema.json"),
            &schema_for!(ModelAuthLogoutPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("model-auth-process.schema.json"),
            &schema_for!(ModelAuthProcess),
        )?,
        json_artifact(
            schema_dir.join("model-auth-register-process-params.schema.json"),
            &schema_for!(ModelAuthRegisterProcessParams),
        )?,
        json_artifact(
            schema_dir.join("model-auth-recover-params.schema.json"),
            &schema_for!(ModelAuthRecoverParams),
        )?,
        json_artifact(
            schema_dir.join("model-auth-step-params.schema.json"),
            &schema_for!(ModelAuthStepParams),
        )?,
        json_artifact(
            schema_dir.join("model-validation-record-list.schema.json"),
            &schema_for!(Vec<ModelValidationRecord>),
        )?,
        json_artifact(
            schema_dir.join("model-purpose-binding-list.schema.json"),
            &schema_for!(Vec<ModelPurposeBinding>),
        )?,
        json_artifact(
            schema_dir.join("model-descriptor-list.schema.json"),
            &schema_for!(Vec<ModelDescriptor>),
        )?,
        json_artifact(
            schema_dir.join("model-connection-change-list.schema.json"),
            &schema_for!(Vec<ModelConnectionChange>),
        )?,
        json_artifact(
            schema_dir.join("model-connection-list.schema.json"),
            &schema_for!(Vec<ModelConnection>),
        )?,
        json_artifact(
            schema_dir.join("model-execution-list-params.schema.json"),
            &schema_for!(ModelExecutionListParams),
        )?,
        json_artifact(
            schema_dir.join("model-execution-lease-list.schema.json"),
            &schema_for!(Vec<ModelExecutionLease>),
        )?,
        json_artifact(
            schema_dir.join("model-execution-owner.schema.json"),
            &schema_for!(ModelExecutionOwner),
        )?,
        json_artifact(
            schema_dir.join("model-execution-process.schema.json"),
            &schema_for!(ModelExecutionProcess),
        )?,
        json_artifact(
            schema_dir.join("model-execution-register-process-params.schema.json"),
            &schema_for!(ModelExecutionRegisterProcessParams),
        )?,
        json_artifact(
            schema_dir.join("model-execution-recovery-state.schema.json"),
            &schema_for!(ModelExecutionRecoveryState),
        )?,
        json_artifact(
            schema_dir.join("model-execution-lease-diagnostic.schema.json"),
            &schema_for!(ModelExecutionLeaseDiagnostic),
        )?,
        json_artifact(
            schema_dir.join("model-execution-recover-params.schema.json"),
            &schema_for!(ModelExecutionRecoverParams),
        )?,
        json_artifact(
            schema_dir.join("model-execution-recover-result.schema.json"),
            &schema_for!(ModelExecutionRecoverResult),
        )?,
        json_artifact(
            schema_dir.join("model-thread-runtime-scope.schema.json"),
            &schema_for!(ModelThreadRuntimeScope),
        )?,
        json_artifact(
            schema_dir.join("model-connection-kind.schema.json"),
            &schema_for!(ModelConnectionKind),
        )?,
        json_artifact(
            schema_dir.join("model-auth-mode.schema.json"),
            &schema_for!(ModelAuthMode),
        )?,
        json_artifact(
            schema_dir.join("local-model-runtime.schema.json"),
            &schema_for!(LocalModelRuntime),
        )?,
        json_artifact(
            schema_dir.join("model-secret-ref.schema.json"),
            &schema_for!(ModelSecretRef),
        )?,
        json_artifact(
            schema_dir.join("model-network-mode.schema.json"),
            &schema_for!(ModelNetworkMode),
        )?,
        json_artifact(
            schema_dir.join("model-network-config.schema.json"),
            &schema_for!(ModelNetworkConfig),
        )?,
        json_artifact(
            schema_dir.join("model-connection-config.schema.json"),
            &schema_for!(ModelConnectionConfig),
        )?,
        json_artifact(
            schema_dir.join("model-connection.schema.json"),
            &schema_for!(ModelConnection),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-trigger-outcome.schema.json"),
            &schema_for!(PersonalAutomationTriggerOutcome),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-finish-outcome.schema.json"),
            &schema_for!(PersonalAutomationFinishOutcome),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-rule-record.schema.json"),
            &schema_for!(PersonalAutomationRuleRecord),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-claim.schema.json"),
            &schema_for!(PersonalAutomationClaim),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-trigger-record.schema.json"),
            &schema_for!(PersonalAutomationTriggerRecord),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-admit-params.schema.json"),
            &schema_for!(PersonalAutomationAdmitParams),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-admit-result.schema.json"),
            &schema_for!(PersonalAutomationAdmitResult),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-prepare-execution-params.schema.json"),
            &schema_for!(PersonalAutomationPrepareExecutionParams),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-bind-params.schema.json"),
            &schema_for!(PersonalAutomationBindParams),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-bind-result.schema.json"),
            &schema_for!(PersonalAutomationBindResult),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-finish-params.schema.json"),
            &schema_for!(PersonalAutomationFinishParams),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-finish-result.schema.json"),
            &schema_for!(PersonalAutomationFinishResult),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-list-triggers-params.schema.json"),
            &schema_for!(PersonalAutomationListTriggersParams),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-trigger-item.schema.json"),
            &schema_for!(PersonalAutomationTriggerItem),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-trigger-page.schema.json"),
            &schema_for!(PersonalAutomationTriggerPage),
        )?,
        json_artifact(
            schema_dir.join("personal-automation-resolve-interrupted-params.schema.json"),
            &schema_for!(PersonalAutomationResolveInterruptedParams),
        )?,
        json_artifact(
            schema_dir.join("model-network-authentication.schema.json"),
            &schema_for!(ModelNetworkAuthentication),
        )?,
        json_artifact(
            schema_dir.join("model-usage-state.schema.json"),
            &schema_for!(ModelUsageState),
        )?,
        json_artifact(
            schema_dir.join("model-usage-tokens.schema.json"),
            &schema_for!(ModelUsageTokens),
        )?,
        json_artifact(
            schema_dir.join("model-usage-cost.schema.json"),
            &schema_for!(ModelUsageCost),
        )?,
        json_artifact(
            schema_dir.join("model-usage-begin-params.schema.json"),
            &schema_for!(ModelUsageBeginParams),
        )?,
        json_artifact(
            schema_dir.join("model-usage-finish-params.schema.json"),
            &schema_for!(ModelUsageFinishParams),
        )?,
        json_artifact(
            schema_dir.join("model-usage-params.schema.json"),
            &schema_for!(ModelUsageParams),
        )?,
        json_artifact(
            schema_dir.join("model-usage-list-params.schema.json"),
            &schema_for!(ModelUsageListParams),
        )?,
        json_artifact(
            schema_dir.join("model-usage-record.schema.json"),
            &schema_for!(ModelUsageRecord),
        )?,
        json_artifact(
            schema_dir.join("model-usage-list-result.schema.json"),
            &schema_for!(ModelUsageListResult),
        )?,
        json_artifact(
            schema_dir.join("model-credential-action.schema.json"),
            &schema_for!(ModelCredentialAction),
        )?,
        json_artifact(
            schema_dir.join("model-connection-save-prepare-params.schema.json"),
            &schema_for!(ModelConnectionSavePrepareParams),
        )?,
        json_artifact(
            schema_dir.join("model-connection-change-state.schema.json"),
            &schema_for!(ModelConnectionChangeState),
        )?,
        json_artifact(
            schema_dir.join("model-connection-change.schema.json"),
            &schema_for!(ModelConnectionChange),
        )?,
        json_artifact(
            schema_dir.join("model-credential-cleanup-state.schema.json"),
            &schema_for!(ModelCredentialCleanupState),
        )?,
        json_artifact(
            schema_dir.join("model-credential-cleanup-ack-params.schema.json"),
            &schema_for!(ModelCredentialCleanupAckParams),
        )?,
        json_artifact(
            schema_dir.join("model-credential-cleanup-candidate-list.schema.json"),
            &schema_for!(Vec<ModelCredentialCleanupCandidate>),
        )?,
        json_artifact(
            schema_dir.join("model-credential-cleanup-plan.schema.json"),
            &schema_for!(ModelCredentialCleanupPlan),
        )?,
        json_artifact(
            schema_dir.join("personal-application-data-action.schema.json"),
            &schema_for!(PersonalApplicationDataAction),
        )?,
        json_artifact(
            schema_dir.join("personal-application-data-state.schema.json"),
            &schema_for!(PersonalApplicationDataState),
        )?,
        json_artifact(
            schema_dir.join("personal-application-data-prepare-params.schema.json"),
            &schema_for!(PersonalApplicationDataPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-application-data-params.schema.json"),
            &schema_for!(PersonalApplicationDataParams),
        )?,
        json_artifact(
            schema_dir.join("personal-application-data-confirm-params.schema.json"),
            &schema_for!(PersonalApplicationDataConfirmParams),
        )?,
        json_artifact(
            schema_dir.join("personal-application-data-execute-params.schema.json"),
            &schema_for!(PersonalApplicationDataExecuteParams),
        )?,
        json_artifact(
            schema_dir.join("personal-application-data-operation.schema.json"),
            &schema_for!(PersonalApplicationDataOperation),
        )?,
        json_artifact(
            schema_dir.join("personal-application-data-operation-list.schema.json"),
            &schema_for!(Vec<PersonalApplicationDataOperation>),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-preferences.schema.json"),
            &schema_for!(PersonalMemoryPreferences),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-enable-confirmation.schema.json"),
            &schema_for!(PersonalMemoryEnableConfirmation),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-authorization.schema.json"),
            &schema_for!(PersonalMemoryAuthorization),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-params.schema.json"),
            &schema_for!(PersonalMemoryParams),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-state.schema.json"),
            &schema_for!(PersonalMemoryResetState),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-scope.schema.json"),
            &schema_for!(PersonalMemoryResetScope),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-outcome.schema.json"),
            &schema_for!(PersonalMemoryResetOutcome),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-scope-result.schema.json"),
            &schema_for!(PersonalMemoryResetScopeResult),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-operation.schema.json"),
            &schema_for!(PersonalMemoryResetOperation),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-prepare-params.schema.json"),
            &schema_for!(PersonalMemoryResetPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-params.schema.json"),
            &schema_for!(PersonalMemoryResetParams),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-confirm-params.schema.json"),
            &schema_for!(PersonalMemoryResetConfirmParams),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-start-params.schema.json"),
            &schema_for!(PersonalMemoryResetStartParams),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-permit.schema.json"),
            &schema_for!(PersonalMemoryResetPermit),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-start-result.schema.json"),
            &schema_for!(PersonalMemoryResetStartResult),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-writer-params.schema.json"),
            &schema_for!(PersonalMemoryResetWriterParams),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-scope-complete-params.schema.json"),
            &schema_for!(PersonalMemoryResetScopeCompleteParams),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-finish-params.schema.json"),
            &schema_for!(PersonalMemoryResetFinishParams),
        )?,
        json_artifact(
            schema_dir.join("personal-memory-reset-operation-list.schema.json"),
            &schema_for!(Vec<PersonalMemoryResetOperation>),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-storage-kind.schema.json"),
            &schema_for!(PersonalNamespaceStorageKind),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-log-file.schema.json"),
            &schema_for!(PersonalNamespaceLogFile),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-log-snapshot.schema.json"),
            &schema_for!(PersonalNamespaceLogSnapshot),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-browser-snapshot.schema.json"),
            &schema_for!(PersonalNamespaceBrowserSnapshot),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-storage-prepare-params.schema.json"),
            &schema_for!(PersonalNamespaceStoragePrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-action-state.schema.json"),
            &schema_for!(PersonalNamespaceActionState),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-action-outcome.schema.json"),
            &schema_for!(PersonalNamespaceActionOutcome),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-action-receipt.schema.json"),
            &schema_for!(PersonalNamespaceActionReceipt),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-storage-operation.schema.json"),
            &schema_for!(PersonalNamespaceStorageOperation),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-storage-params.schema.json"),
            &schema_for!(PersonalNamespaceStorageParams),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-storage-confirm-params.schema.json"),
            &schema_for!(PersonalNamespaceStorageConfirmParams),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-storage-start-params.schema.json"),
            &schema_for!(PersonalNamespaceStorageStartParams),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-action-permit.schema.json"),
            &schema_for!(PersonalNamespaceActionPermit),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-storage-start-result.schema.json"),
            &schema_for!(PersonalNamespaceStorageStartResult),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-action-params.schema.json"),
            &schema_for!(PersonalNamespaceActionParams),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-action-ack-params.schema.json"),
            &schema_for!(PersonalNamespaceActionAckParams),
        )?,
        json_artifact(
            schema_dir.join("personal-namespace-storage-operation-list.schema.json"),
            &schema_for!(Vec<PersonalNamespaceStorageOperation>),
        )?,
        json_artifact(
            schema_dir.join("personal-page-risk-level.schema.json"),
            &schema_for!(PersonalPageRiskLevel),
        )?,
        json_artifact(
            schema_dir.join("personal-page-confirmation-policy.schema.json"),
            &schema_for!(PersonalPageConfirmationPolicy),
        )?,
        json_artifact(
            schema_dir.join("personal-page-context-type.schema.json"),
            &schema_for!(PersonalPageContextType),
        )?,
        json_artifact(
            schema_dir.join("personal-page-ui-action.schema.json"),
            &schema_for!(PersonalPageUiAction),
        )?,
        json_artifact(
            schema_dir.join("personal-page-tool.schema.json"),
            &schema_for!(PersonalPageTool),
        )?,
        json_artifact(
            schema_dir.join("personal-page-ai-manifest.schema.json"),
            &schema_for!(PersonalPageAiManifest),
        )?,
        json_artifact(
            schema_dir.join("personal-page-binding-params.schema.json"),
            &schema_for!(PersonalPageBindingParams),
        )?,
        json_artifact(
            schema_dir.join("personal-page-runtime-binding.schema.json"),
            &schema_for!(PersonalPageRuntimeBinding),
        )?,
        json_artifact(
            schema_dir.join("personal-page-binding-snapshot.schema.json"),
            &schema_for!(PersonalPageBindingSnapshot),
        )?,
        json_artifact(
            schema_dir.join("personal-page-audit-method.schema.json"),
            &schema_for!(PersonalPageAuditMethod),
        )?,
        json_artifact(
            schema_dir.join("personal-page-audit-outcome.schema.json"),
            &schema_for!(PersonalPageAuditOutcome),
        )?,
        json_artifact(
            schema_dir.join("personal-page-audit-prepare-params.schema.json"),
            &schema_for!(PersonalPageAuditPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-page-audit-finish-params.schema.json"),
            &schema_for!(PersonalPageAuditFinishParams),
        )?,
        json_artifact(
            schema_dir.join("personal-page-audit-record.schema.json"),
            &schema_for!(PersonalPageAuditRecord),
        )?,
        json_artifact(
            schema_dir.join("personal-page-audit-list-params.schema.json"),
            &schema_for!(PersonalPageAuditListParams),
        )?,
        json_artifact(
            schema_dir.join("personal-page-audit-event.schema.json"),
            &schema_for!(PersonalPageAuditEvent),
        )?,
        json_artifact(
            schema_dir.join("personal-page-audit-list-result.schema.json"),
            &schema_for!(PersonalPageAuditListResult),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-category.schema.json"),
            &schema_for!(PersonalStorageCategory),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-coverage-state.schema.json"),
            &schema_for!(PersonalStorageCoverageState),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-coverage.schema.json"),
            &schema_for!(PersonalStorageCoverage),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-reference.schema.json"),
            &schema_for!(PersonalStorageReference),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-item.schema.json"),
            &schema_for!(PersonalStorageItem),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-inventory.schema.json"),
            &schema_for!(PersonalStorageInventory),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-operation-state.schema.json"),
            &schema_for!(PersonalStorageOperationState),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-outcome.schema.json"),
            &schema_for!(PersonalStorageOutcome),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-item-result.schema.json"),
            &schema_for!(PersonalStorageItemResult),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-result.schema.json"),
            &schema_for!(PersonalStorageResult),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-operation.schema.json"),
            &schema_for!(PersonalStorageOperation),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-inventory-params.schema.json"),
            &schema_for!(PersonalStorageInventoryParams),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-prepare-params.schema.json"),
            &schema_for!(PersonalStoragePrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-execute-params.schema.json"),
            &schema_for!(PersonalStorageExecuteParams),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-params.schema.json"),
            &schema_for!(PersonalStorageParams),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-confirm-params.schema.json"),
            &schema_for!(PersonalStorageConfirmParams),
        )?,
        json_artifact(
            schema_dir.join("personal-maintenance-operation-status.schema.json"),
            &schema_for!(PersonalMaintenanceOperationStatus),
        )?,
        json_artifact(
            schema_dir.join("personal-storage-operation-list.schema.json"),
            &schema_for!(Vec<PersonalStorageOperation>),
        )?,
        json_artifact(
            schema_dir.join("personal-data-category.schema.json"),
            &schema_for!(PersonalDataCategory),
        )?,
        json_artifact(
            schema_dir.join("personal-data-operation-kind.schema.json"),
            &schema_for!(PersonalDataOperationKind),
        )?,
        json_artifact(
            schema_dir.join("personal-data-operation-state.schema.json"),
            &schema_for!(PersonalDataOperationState),
        )?,
        json_artifact(
            schema_dir.join("personal-data-issue.schema.json"),
            &schema_for!(PersonalDataIssue),
        )?,
        json_artifact(
            schema_dir.join("personal-backup-selection.schema.json"),
            &schema_for!(PersonalBackupSelection),
        )?,
        json_artifact(
            schema_dir.join("personal-workspace-content-entry.schema.json"),
            &schema_for!(PersonalWorkspaceContentEntry),
        )?,
        json_artifact(
            schema_dir.join("personal-workspace-content-snapshot.schema.json"),
            &schema_for!(PersonalWorkspaceContentSnapshot),
        )?,
        json_artifact(
            schema_dir.join("personal-restore-workspace-destination.schema.json"),
            &schema_for!(PersonalRestoreWorkspaceDestination),
        )?,
        json_artifact(
            schema_dir.join("personal-restored-workspace.schema.json"),
            &schema_for!(PersonalRestoredWorkspace),
        )?,
        json_artifact(
            schema_dir.join("personal-restore-workspace-plan-params.schema.json"),
            &schema_for!(PersonalRestoreWorkspacePlanParams),
        )?,
        json_artifact(
            schema_dir.join("personal-backup-workspace.schema.json"),
            &schema_for!(PersonalBackupWorkspace),
        )?,
        json_artifact(
            schema_dir.join("personal-backup-native-home.schema.json"),
            &schema_for!(PersonalBackupNativeHome),
        )?,
        json_artifact(
            schema_dir.join("personal-backup-file.schema.json"),
            &schema_for!(PersonalBackupFile),
        )?,
        json_artifact(
            schema_dir.join("personal-backup-manifest.schema.json"),
            &schema_for!(PersonalBackupManifest),
        )?,
        json_artifact(
            schema_dir.join("personal-restore-preview.schema.json"),
            &schema_for!(PersonalRestorePreview),
        )?,
        json_artifact(
            schema_dir.join("personal-data-result.schema.json"),
            &schema_for!(PersonalDataResult),
        )?,
        json_artifact(
            schema_dir.join("personal-data-operation.schema.json"),
            &schema_for!(PersonalDataOperation),
        )?,
        json_artifact(
            schema_dir.join("personal-backup-prepare-params.schema.json"),
            &schema_for!(PersonalBackupPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-data-quiescence.schema.json"),
            &schema_for!(PersonalDataQuiescence),
        )?,
        json_artifact(
            schema_dir.join("personal-backup-capture-params.schema.json"),
            &schema_for!(PersonalBackupCaptureParams),
        )?,
        json_artifact(
            schema_dir.join("personal-restore-prepare-params.schema.json"),
            &schema_for!(PersonalRestorePrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-restore-apply-prepare-params.schema.json"),
            &schema_for!(PersonalRestoreApplyPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-restore-abort-before-helper-params.schema.json"),
            &schema_for!(PersonalRestoreAbortBeforeHelperParams),
        )?,
        json_artifact(
            schema_dir.join("personal-data-params.schema.json"),
            &schema_for!(PersonalDataParams),
        )?,
        json_artifact(
            schema_dir.join("personal-data-list-params.schema.json"),
            &schema_for!(PersonalDataListParams),
        )?,
        json_artifact(
            schema_dir.join("personal-data-cancel-params.schema.json"),
            &schema_for!(PersonalDataCancelParams),
        )?,
        json_artifact(
            schema_dir.join("personal-backup-artifact.schema.json"),
            &schema_for!(PersonalBackupArtifact),
        )?,
        json_artifact(
            schema_dir.join("personal-writer-mode.schema.json"),
            &schema_for!(PersonalWriterMode),
        )?,
        json_artifact(
            schema_dir.join("personal-writer-state.schema.json"),
            &schema_for!(PersonalWriterState),
        )?,
        json_artifact(
            schema_dir.join("personal-writer-prepare-params.schema.json"),
            &schema_for!(PersonalWriterPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-writer-lease.schema.json"),
            &schema_for!(PersonalWriterLease),
        )?,
        json_artifact(
            schema_dir.join("personal-writer-register-params.schema.json"),
            &schema_for!(PersonalWriterRegisterParams),
        )?,
        json_artifact(
            schema_dir.join("personal-writer-lease-params.schema.json"),
            &schema_for!(PersonalWriterLeaseParams),
        )?,
        json_artifact(
            schema_dir.join("personal-writer-abort-params.schema.json"),
            &schema_for!(PersonalWriterAbortParams),
        )?,
        json_artifact(
            schema_dir.join("personal-writer-list-params.schema.json"),
            &schema_for!(PersonalWriterListParams),
        )?,
        json_artifact(
            schema_dir.join("personal-maintenance-state.schema.json"),
            &schema_for!(PersonalMaintenanceState),
        )?,
        json_artifact(
            schema_dir.join("personal-maintenance-list-params.schema.json"),
            &schema_for!(PersonalMaintenanceListParams),
        )?,
        json_artifact(
            schema_dir.join("personal-maintenance-fence-list.schema.json"),
            &schema_for!(Vec<PersonalMaintenanceFence>),
        )?,
        json_artifact(
            schema_dir.join("personal-maintenance-drain-status.schema.json"),
            &schema_for!(PersonalMaintenanceDrainStatus),
        )?,
        json_artifact(
            schema_dir.join("personal-maintenance-fence.schema.json"),
            &schema_for!(PersonalMaintenanceFence),
        )?,
        json_artifact(
            schema_dir.join("personal-maintenance-acquire-params.schema.json"),
            &schema_for!(PersonalMaintenanceAcquireParams),
        )?,
        json_artifact(
            schema_dir.join("personal-maintenance-params.schema.json"),
            &schema_for!(PersonalMaintenanceParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-purpose.schema.json"),
            &schema_for!(McpPurpose),
        )?,
        json_artifact(
            schema_dir.join("mcp-source.schema.json"),
            &schema_for!(McpSource),
        )?,
        json_artifact(
            schema_dir.join("mcp-local-permissions.schema.json"),
            &schema_for!(McpLocalPermissions),
        )?,
        json_artifact(
            schema_dir.join("mcp-auth-mode.schema.json"),
            &schema_for!(McpAuthMode),
        )?,
        json_artifact(
            schema_dir.join("mcp-network-config.schema.json"),
            &schema_for!(McpNetworkConfig),
        )?,
        json_artifact(
            schema_dir.join("mcp-connection-config.schema.json"),
            &schema_for!(McpConnectionConfig),
        )?,
        json_artifact(
            schema_dir.join("mcp-secret-ref.schema.json"),
            &schema_for!(McpSecretRef),
        )?,
        json_artifact(
            schema_dir.join("mcp-tool-definition.schema.json"),
            &schema_for!(McpToolDefinition),
        )?,
        json_artifact(
            schema_dir.join("mcp-validation-state.schema.json"),
            &schema_for!(McpValidationState),
        )?,
        json_artifact(
            schema_dir.join("mcp-validation-record.schema.json"),
            &schema_for!(McpValidationRecord),
        )?,
        json_artifact(
            schema_dir.join("mcp-connection.schema.json"),
            &schema_for!(McpConnection),
        )?,
        json_artifact(
            schema_dir.join("mcp-credential-action.schema.json"),
            &schema_for!(McpCredentialAction),
        )?,
        json_artifact(
            schema_dir.join("mcp-save-prepare-params.schema.json"),
            &schema_for!(McpSavePrepareParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-change-state.schema.json"),
            &schema_for!(McpChangeState),
        )?,
        json_artifact(
            schema_dir.join("mcp-connection-change.schema.json"),
            &schema_for!(McpConnectionChange),
        )?,
        json_artifact(
            schema_dir.join("mcp-operation-params.schema.json"),
            &schema_for!(McpOperationParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-connection-params.schema.json"),
            &schema_for!(McpConnectionParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-save-commit-params.schema.json"),
            &schema_for!(McpSaveCommitParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-mutation-params.schema.json"),
            &schema_for!(McpMutationParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-grant-params.schema.json"),
            &schema_for!(McpGrantParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-execution-intent.schema.json"),
            &schema_for!(McpExecutionIntent),
        )?,
        json_artifact(
            schema_dir.join("mcp-execution-prepare-params.schema.json"),
            &schema_for!(McpExecutionPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-execution-state.schema.json"),
            &schema_for!(McpExecutionState),
        )?,
        json_artifact(
            schema_dir.join("mcp-execution-lease.schema.json"),
            &schema_for!(McpExecutionLease),
        )?,
        json_artifact(
            schema_dir.join("mcp-execution-params.schema.json"),
            &schema_for!(McpExecutionParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-execution-register-params.schema.json"),
            &schema_for!(McpExecutionRegisterParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-execution-finish-params.schema.json"),
            &schema_for!(McpExecutionFinishParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-validation-report-params.schema.json"),
            &schema_for!(McpValidationReportParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-validate-params.schema.json"),
            &schema_for!(McpValidateParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-snapshot-params.schema.json"),
            &schema_for!(McpSnapshotParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-tool-snapshot.schema.json"),
            &schema_for!(McpToolSnapshot),
        )?,
        json_artifact(
            schema_dir.join("mcp-credential-cleanup-params.schema.json"),
            &schema_for!(McpCredentialCleanupParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-credential-cleanup-state.schema.json"),
            &schema_for!(McpCredentialCleanupState),
        )?,
        json_artifact(
            schema_dir.join("mcp-credential-cleanup-plan.schema.json"),
            &schema_for!(McpCredentialCleanupPlan),
        )?,
        json_artifact(
            schema_dir.join("mcp-credential-cleanup-confirm-params.schema.json"),
            &schema_for!(McpCredentialCleanupConfirmParams),
        )?,
        json_artifact(
            schema_dir.join("mcp-connection-change-list.schema.json"),
            &schema_for!(Vec<McpConnectionChange>),
        )?,
        json_artifact(
            schema_dir.join("mcp-credential-cleanup-plan-list.schema.json"),
            &schema_for!(Vec<McpCredentialCleanupPlan>),
        )?,
        json_artifact(
            schema_dir.join("personal-data-operation-list.schema.json"),
            &schema_for!(Vec<PersonalDataOperation>),
        )?,
        json_artifact(
            schema_dir.join("personal-writer-lease-list.schema.json"),
            &schema_for!(Vec<PersonalWriterLease>),
        )?,
        json_artifact(
            schema_dir.join("mcp-connection-list.schema.json"),
            &schema_for!(Vec<McpConnection>),
        )?,
        json_artifact(
            schema_dir.join("mcp-change-list.schema.json"),
            &schema_for!(Vec<McpConnectionChange>),
        )?,
        json_artifact(
            schema_dir.join("mcp-execution-lease-list.schema.json"),
            &schema_for!(Vec<McpExecutionLease>),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-target.schema.json"),
            &schema_for!(PersonalPackageImportTarget),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-state.schema.json"),
            &schema_for!(PersonalPackageImportState),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-source.schema.json"),
            &schema_for!(PersonalPackageImportSource),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-preview.schema.json"),
            &schema_for!(PersonalPackageImportPreview),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-prepare-params.schema.json"),
            &schema_for!(PersonalPackageImportPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-commit-params.schema.json"),
            &schema_for!(PersonalPackageImportCommitParams),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-params.schema.json"),
            &schema_for!(PersonalPackageImportParams),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-cancel-params.schema.json"),
            &schema_for!(PersonalPackageImportCancelParams),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-list-params.schema.json"),
            &schema_for!(PersonalPackageImportListParams),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-commit-result.schema.json"),
            &schema_for!(PersonalPackageImportCommitResult),
        )?,
        json_artifact(
            schema_dir.join("personal-package-import-list.schema.json"),
            &schema_for!(Vec<PersonalPackageImportPreview>),
        )?,
        json_artifact(
            schema_dir.join("model-credential-cleanup-plan-state.schema.json"),
            &schema_for!(ModelCredentialCleanupPlanState),
        )?,
        json_artifact(
            schema_dir.join("model-credential-cleanup-prepare-params.schema.json"),
            &schema_for!(ModelCredentialCleanupPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("model-credential-cleanup-confirm-params.schema.json"),
            &schema_for!(ModelCredentialCleanupConfirmParams),
        )?,
        json_artifact(
            schema_dir.join("model-connection-commit-params.schema.json"),
            &schema_for!(ModelConnectionCommitParams),
        )?,
        json_artifact(
            schema_dir.join("model-operation-params.schema.json"),
            &schema_for!(ModelOperationParams),
        )?,
        json_artifact(
            schema_dir.join("model-connection-params.schema.json"),
            &schema_for!(ModelConnectionParams),
        )?,
        json_artifact(
            schema_dir.join("model-evidence-source.schema.json"),
            &schema_for!(ModelEvidenceSource),
        )?,
        json_artifact(
            schema_dir.join("model-capability-state.schema.json"),
            &schema_for!(ModelCapabilityState),
        )?,
        json_artifact(
            schema_dir.join("model-capability-evidence.schema.json"),
            &schema_for!(ModelCapabilityEvidence),
        )?,
        json_artifact(
            schema_dir.join("model-descriptor.schema.json"),
            &schema_for!(ModelDescriptor),
        )?,
        json_artifact(
            schema_dir.join("model-descriptor-save-params.schema.json"),
            &schema_for!(ModelDescriptorSaveParams),
        )?,
        json_artifact(
            schema_dir.join("model-descriptor-observation.schema.json"),
            &schema_for!(ModelDescriptorObservation),
        )?,
        json_artifact(
            schema_dir.join("model-descriptor-observe-params.schema.json"),
            &schema_for!(ModelDescriptorObserveParams),
        )?,
        json_artifact(
            schema_dir.join("model-purpose.schema.json"),
            &schema_for!(ModelPurpose),
        )?,
        json_artifact(
            schema_dir.join("model-selection-policy.schema.json"),
            &schema_for!(ModelSelectionPolicy),
        )?,
        json_artifact(
            schema_dir.join("model-selection.schema.json"),
            &schema_for!(ModelSelection),
        )?,
        json_artifact(
            schema_dir.join("model-history-consent.schema.json"),
            &schema_for!(ModelHistoryConsent),
        )?,
        json_artifact(
            schema_dir.join("model-purpose-binding.schema.json"),
            &schema_for!(ModelPurposeBinding),
        )?,
        json_artifact(
            schema_dir.join("model-purpose-save-params.schema.json"),
            &schema_for!(ModelPurposeSaveParams),
        )?,
        json_artifact(
            schema_dir.join("model-validation-layer.schema.json"),
            &schema_for!(ModelValidationLayer),
        )?,
        json_artifact(
            schema_dir.join("model-validation-outcome.schema.json"),
            &schema_for!(ModelValidationOutcome),
        )?,
        json_artifact(
            schema_dir.join("model-validation-prepare-params.schema.json"),
            &schema_for!(ModelValidationPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("model-validation-record.schema.json"),
            &schema_for!(ModelValidationRecord),
        )?,
        json_artifact(
            schema_dir.join("model-validation-report-params.schema.json"),
            &schema_for!(ModelValidationReportParams),
        )?,
        json_artifact(
            schema_dir.join("model-execution-prepare-params.schema.json"),
            &schema_for!(ModelExecutionPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("model-execution-snapshot.schema.json"),
            &schema_for!(ModelExecutionSnapshot),
        )?,
        json_artifact(
            schema_dir.join("model-execution-lease.schema.json"),
            &schema_for!(ModelExecutionLease),
        )?,
        json_artifact(
            schema_dir.join("model-execution-lease-params.schema.json"),
            &schema_for!(ModelExecutionLeaseParams),
        )?,
        json_artifact(
            schema_dir.join("model-connection-mutation.schema.json"),
            &schema_for!(ModelConnectionMutation),
        )?,
        json_artifact(
            schema_dir.join("model-connection-preflight-params.schema.json"),
            &schema_for!(ModelConnectionPreflightParams),
        )?,
        json_artifact(
            schema_dir.join("model-connection-reference.schema.json"),
            &schema_for!(ModelConnectionReference),
        )?,
        json_artifact(
            schema_dir.join("model-connection-preflight.schema.json"),
            &schema_for!(ModelConnectionPreflight),
        )?,
        json_artifact(
            schema_dir.join("model-connection-mutation-params.schema.json"),
            &schema_for!(ModelConnectionMutationParams),
        )?,
        json_artifact(
            schema_dir.join("model-connection-set-enabled-params.schema.json"),
            &schema_for!(ModelConnectionSetEnabledParams),
        )?,
        json_artifact(
            schema_dir.join("personal-profile-manifest.schema.json"),
            &schema_for!(PersonalProfileManifest),
        )?,
        json_artifact(
            schema_dir.join("initialize-params.schema.json"),
            &schema_for!(InitializeParams),
        )?,
        json_artifact(
            schema_dir.join("initialize-result.schema.json"),
            &schema_for!(InitializeResult),
        )?,
        json_artifact(
            schema_dir.join("success-response.schema.json"),
            &schema_for!(SuccessResponse),
        )?,
        json_artifact(
            schema_dir.join("error-response.schema.json"),
            &schema_for!(ErrorResponse),
        )?,
        json_artifact(
            schema_dir.join("preview-open-params.schema.json"),
            &schema_for!(OpenPreviewParams),
        )?,
        json_artifact(
            schema_dir.join("preview-open-result.schema.json"),
            &schema_for!(OpenPreviewResult),
        )?,
        json_artifact(
            schema_dir.join("text-resource-read-params.schema.json"),
            &schema_for!(ReadTextResourceParams),
        )?,
        json_artifact(
            schema_dir.join("text-resource-read-result.schema.json"),
            &schema_for!(ReadTextResourceResult),
        )?,
        json_artifact(
            schema_dir.join("text-diff-compute-params.schema.json"),
            &schema_for!(ComputeTextDiffParams),
        )?,
        json_artifact(
            schema_dir.join("text-diff-compute-result.schema.json"),
            &schema_for!(ComputeTextDiffResult),
        )?,
        json_artifact(
            schema_dir.join("text-diff-read-params.schema.json"),
            &schema_for!(ReadTextDiffParams),
        )?,
        json_artifact(
            schema_dir.join("text-diff-read-result.schema.json"),
            &schema_for!(ReadTextDiffResult),
        )?,
        json_artifact(
            schema_dir.join("file-confirm-params.schema.json"),
            &schema_for!(ConfirmFileParams),
        )?,
        json_artifact(
            schema_dir.join("file-confirm-result.schema.json"),
            &schema_for!(ConfirmFileResult),
        )?,
        json_artifact(
            schema_dir.join("confirmation-history-params.schema.json"),
            &schema_for!(ConfirmationHistoryParams),
        )?,
        json_artifact(
            schema_dir.join("confirmation-history-result.schema.json"),
            &schema_for!(ConfirmationHistoryResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-list-params.schema.json"),
            &schema_for!(WorkspaceListParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-list-result.schema.json"),
            &schema_for!(WorkspaceListResult),
        )?,
        json_artifact(
            schema_dir.join("skill-run-list-params.schema.json"),
            &schema_for!(SkillRunListParams),
        )?,
        json_artifact(
            schema_dir.join("skill-run-list-result.schema.json"),
            &schema_for!(SkillRunListResult),
        )?,
        json_artifact(
            schema_dir.join("skill-run-retry-params.schema.json"),
            &schema_for!(SkillRunRetryParams),
        )?,
        json_artifact(
            schema_dir.join("skill-run-installation.schema.json"),
            &schema_for!(SkillRunInstallation),
        )?,
        json_artifact(
            schema_dir.join("skill-list-params.schema.json"),
            &schema_for!(SkillListParams),
        )?,
        json_artifact(
            schema_dir.join("skill-list-result.schema.json"),
            &schema_for!(SkillListResult),
        )?,
        json_artifact(
            schema_dir.join("skill-get-params.schema.json"),
            &schema_for!(SkillGetParams),
        )?,
        json_artifact(
            schema_dir.join("skill-get-result.schema.json"),
            &schema_for!(SkillGetResult),
        )?,
        json_artifact(
            schema_dir.join("skill-save-params.schema.json"),
            &schema_for!(SkillSaveParams),
        )?,
        json_artifact(
            schema_dir.join("skill-save-result.schema.json"),
            &schema_for!(SkillSaveResult),
        )?,
        json_artifact(
            schema_dir.join("skill-import-params.schema.json"),
            &schema_for!(SkillImportParams),
        )?,
        json_artifact(
            schema_dir.join("skill-import-directory-params.schema.json"),
            &schema_for!(SkillImportDirectoryParams),
        )?,
        json_artifact(
            schema_dir.join("skill-import-result.schema.json"),
            &schema_for!(SkillImportResult),
        )?,
        json_artifact(
            schema_dir.join("skill-set-state-params.schema.json"),
            &schema_for!(SkillSetStateParams),
        )?,
        json_artifact(
            schema_dir.join("skill-set-state-result.schema.json"),
            &schema_for!(SkillSetStateResult),
        )?,
        json_artifact(
            schema_dir.join("skill-delete-params.schema.json"),
            &schema_for!(SkillDeleteParams),
        )?,
        json_artifact(
            schema_dir.join("skill-delete-result.schema.json"),
            &schema_for!(SkillDeleteResult),
        )?,
        json_artifact(
            schema_dir.join("skill-conversation-catalog-params.schema.json"),
            &schema_for!(SkillConversationCatalogParams),
        )?,
        json_artifact(
            schema_dir.join("skill-conversation-catalog-result.schema.json"),
            &schema_for!(SkillConversationCatalogResult),
        )?,
        json_artifact(
            schema_dir.join("skill-runtime-catalog-params.schema.json"),
            &schema_for!(SkillRuntimeCatalogParams),
        )?,
        json_artifact(
            schema_dir.join("skill-runtime-catalog-result.schema.json"),
            &schema_for!(SkillRuntimeCatalogResult),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-create-params.schema.json"),
            &schema_for!(CapabilityCreationCreateParams),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-create-result.schema.json"),
            &schema_for!(CapabilityCreationCreateResult),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-list-params.schema.json"),
            &schema_for!(CapabilityCreationListParams),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-list-result.schema.json"),
            &schema_for!(CapabilityCreationListResult),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-get-params.schema.json"),
            &schema_for!(CapabilityCreationGetParams),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-get-result.schema.json"),
            &schema_for!(CapabilityCreationGetResult),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-preview-params.schema.json"),
            &schema_for!(CapabilityCreationPreviewParams),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-preview-result.schema.json"),
            &schema_for!(CapabilityCreationPreviewResult),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-install-params.schema.json"),
            &schema_for!(CapabilityCreationInstallParams),
        )?,
        json_artifact(
            schema_dir.join("capability-creation-install-result.schema.json"),
            &schema_for!(CapabilityCreationInstallResult),
        )?,
        json_artifact(
            schema_dir.join("personal-application-open-params.schema.json"),
            &schema_for!(PersonalApplicationOpenParams),
        )?,
        json_artifact(
            schema_dir.join("personal-application-open-result.schema.json"),
            &schema_for!(PersonalApplicationOpenResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-directory-create-params.schema.json"),
            &schema_for!(WorkspaceDirectoryCreateParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-directory-create-result.schema.json"),
            &schema_for!(WorkspaceDirectoryCreateResult),
        )?,
        json_artifact(
            schema_dir.join("file-hydration-params.schema.json"),
            &schema_for!(FileHydrationParams),
        )?,
        json_artifact(
            schema_dir.join("file-hydration-result.schema.json"),
            &schema_for!(FileHydrationResult),
        )?,
        json_artifact(
            schema_dir.join("read-binary-preview-params.schema.json"),
            &schema_for!(ReadBinaryPreviewParams),
        )?,
        json_artifact(
            schema_dir.join("read-binary-preview-result.schema.json"),
            &schema_for!(ReadBinaryPreviewResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-create-params.schema.json"),
            &schema_for!(WorkspaceCreateParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-create-result.schema.json"),
            &schema_for!(WorkspaceCreateResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-target-validate-params.schema.json"),
            &schema_for!(WorkspaceTargetValidateParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-target-validate-result.schema.json"),
            &schema_for!(WorkspaceTargetValidateResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-scan-params.schema.json"),
            &schema_for!(WorkspaceScanParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-scan-result.schema.json"),
            &schema_for!(WorkspaceScanResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-files-params.schema.json"),
            &schema_for!(WorkspaceFilesParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-files-result.schema.json"),
            &schema_for!(WorkspaceFilesResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-list-directory-params.schema.json"),
            &schema_for!(WorkspaceListDirectoryParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-list-directory-result.schema.json"),
            &schema_for!(WorkspaceListDirectoryResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-index-request-params.schema.json"),
            &schema_for!(WorkspaceIndexRequestParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-index-request-result.schema.json"),
            &schema_for!(WorkspaceIndexRequestResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-index-status-params.schema.json"),
            &schema_for!(WorkspaceIndexStatusParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-index-status-result.schema.json"),
            &schema_for!(WorkspaceIndexStatusResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-index-path-params.schema.json"),
            &schema_for!(WorkspaceIndexPathParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-index-path-result.schema.json"),
            &schema_for!(WorkspaceIndexPathResult),
        )?,
        json_artifact(
            schema_dir.join("ai-conversation-set-archived-params.schema.json"),
            &schema_for!(AiConversationSetArchivedParams),
        )?,
        json_artifact(
            schema_dir.join("ai-conversation-set-archived-result.schema.json"),
            &schema_for!(AiConversationSetArchivedResult),
        )?,
        json_artifact(
            schema_dir.join("ai-application-params.schema.json"),
            &schema_for!(AiApplicationParams),
        )?,
        json_artifact(
            schema_dir.join("ai-thread-get-params.schema.json"),
            &schema_for!(AiThreadGetParams),
        )?,
        json_artifact(
            schema_dir.join("ai-thread-state.schema.json"),
            &schema_for!(AiThreadState),
        )?,
        json_artifact(
            schema_dir.join("ai-thread-prepare-params.schema.json"),
            &schema_for!(AiThreadPrepareParams),
        )?,
        json_artifact(
            schema_dir.join("ai-thread-acknowledge-params.schema.json"),
            &schema_for!(AiThreadAcknowledgeParams),
        )?,
        json_artifact(
            schema_dir.join("ai-thread-cleanup-params.schema.json"),
            &schema_for!(AiThreadCleanupParams),
        )?,
        json_artifact(
            schema_dir.join("ai-thread-release-params.schema.json"),
            &schema_for!(AiThreadReleaseParams),
        )?,
        json_artifact(
            schema_dir.join("ai-thread-mutation-result.schema.json"),
            &schema_for!(AiThreadMutationResult),
        )?,
        json_artifact(
            schema_dir.join("ai-history-params.schema.json"),
            &schema_for!(AiHistoryParams),
        )?,
        json_artifact(
            schema_dir.join("ai-history-result.schema.json"),
            &schema_for!(AiHistoryResult),
        )?,
        json_artifact(
            schema_dir.join("ai-ledger-get-params.schema.json"),
            &schema_for!(AiLedgerGetParams),
        )?,
        json_artifact(
            schema_dir.join("ai-ledger-get-result.schema.json"),
            &schema_for!(AiLedgerGetResult),
        )?,
        json_artifact(
            schema_dir.join("ai-ledger-apply-params.schema.json"),
            &schema_for!(AiLedgerApplyParams),
        )?,
        json_artifact(
            schema_dir.join("ai-ledger-apply-result.schema.json"),
            &schema_for!(AiLedgerApplyResult),
        )?,
        json_artifact(
            schema_dir.join("ai-state-get-params.schema.json"),
            &schema_for!(AiStateGetParams),
        )?,
        json_artifact(
            schema_dir.join("ai-state-get-result.schema.json"),
            &schema_for!(AiStateGetResult),
        )?,
        json_artifact(
            schema_dir.join("ai-state-replace-params.schema.json"),
            &schema_for!(AiStateReplaceParams),
        )?,
        json_artifact(
            schema_dir.join("ai-state-replace-result.schema.json"),
            &schema_for!(AiStateReplaceResult),
        )?,
        json_artifact(
            schema_dir.join("ai-state-apply-params.schema.json"),
            &schema_for!(AiStateApplyParams),
        )?,
        json_artifact(
            schema_dir.join("ai-state-apply-result.schema.json"),
            &schema_for!(AiStateApplyResult),
        )?,
        json_artifact(
            schema_dir.join("ai-work-task-candidates-params.schema.json"),
            &schema_for!(AiWorkTaskCandidatesParams),
        )?,
        json_artifact(
            schema_dir.join("ai-work-task-candidates-result.schema.json"),
            &schema_for!(AiWorkTaskCandidatesResult),
        )?,
        json_artifact(
            schema_dir.join("ai-turn-finalize-params.schema.json"),
            &schema_for!(AiTurnFinalizeParams),
        )?,
        json_artifact(
            schema_dir.join("ai-turn-finalize-result.schema.json"),
            &schema_for!(AiTurnFinalizeResult),
        )?,
        json_artifact(
            schema_dir.join("project-list-params.schema.json"),
            &schema_for!(ProjectListParams),
        )?,
        json_artifact(
            schema_dir.join("project-list-result.schema.json"),
            &schema_for!(ProjectListResult),
        )?,
        json_artifact(
            schema_dir.join("project-create-params.schema.json"),
            &schema_for!(ProjectCreateParams),
        )?,
        json_artifact(
            schema_dir.join("project-create-result.schema.json"),
            &schema_for!(ProjectCreateResult),
        )?,
        json_artifact(
            schema_dir.join("project-delete-params.schema.json"),
            &schema_for!(ProjectDeleteParams),
        )?,
        json_artifact(
            schema_dir.join("project-delete-result.schema.json"),
            &schema_for!(ProjectDeleteResult),
        )?,
        json_artifact(
            schema_dir.join("workspace-delete-params.schema.json"),
            &schema_for!(WorkspaceDeleteParams),
        )?,
        json_artifact(
            schema_dir.join("workspace-delete-result.schema.json"),
            &schema_for!(WorkspaceDeleteResult),
        )?,
        json_artifact(
            schema_dir.join("project-detach-workspace-params.schema.json"),
            &schema_for!(ProjectDetachWorkspaceParams),
        )?,
        json_artifact(
            schema_dir.join("project-detach-workspace-result.schema.json"),
            &schema_for!(ProjectDetachWorkspaceResult),
        )?,
        json_artifact(
            schema_dir.join("project-attach-workspace-params.schema.json"),
            &schema_for!(ProjectAttachWorkspaceParams),
        )?,
        json_artifact(
            schema_dir.join("project-attach-workspace-result.schema.json"),
            &schema_for!(ProjectAttachWorkspaceResult),
        )?,
        json_artifact(
            schema_dir.join("project-get-params.schema.json"),
            &schema_for!(ProjectGetParams),
        )?,
        json_artifact(
            schema_dir.join("project-get-result.schema.json"),
            &schema_for!(ProjectGetResult),
        )?,
        json_artifact(
            schema_dir.join("project-portfolio-list-params.schema.json"),
            &schema_for!(ProjectPortfolioListParams),
        )?,
        json_artifact(
            schema_dir.join("project-portfolio-list-result.schema.json"),
            &schema_for!(ProjectPortfolioListResult),
        )?,
        json_artifact(
            schema_dir.join("deliverable-list-params.schema.json"),
            &schema_for!(DeliverableListParams),
        )?,
        json_artifact(
            schema_dir.join("deliverable-list-result.schema.json"),
            &schema_for!(DeliverableListResult),
        )?,
        json_artifact(
            schema_dir.join("deliverable-create-params.schema.json"),
            &schema_for!(DeliverableCreateParams),
        )?,
        json_artifact(
            schema_dir.join("deliverable-create-result.schema.json"),
            &schema_for!(DeliverableCreateResult),
        )?,
        json_artifact(
            schema_dir.join("deliverable-version-create-params.schema.json"),
            &schema_for!(DeliverableVersionCreateParams),
        )?,
        json_artifact(
            schema_dir.join("deliverable-version-create-result.schema.json"),
            &schema_for!(DeliverableVersionCreateResult),
        )?,
        json_artifact(
            schema_dir.join("deliverable-version-review-params.schema.json"),
            &schema_for!(DeliverableVersionReviewParams),
        )?,
        json_artifact(
            schema_dir.join("deliverable-version-review-result.schema.json"),
            &schema_for!(DeliverableVersionReviewResult),
        )?,
        json_artifact(
            schema_dir.join("my-work-get-params.schema.json"),
            &schema_for!(MyWorkGetParams),
        )?,
        json_artifact(
            schema_dir.join("my-work-get-result.schema.json"),
            &schema_for!(MyWorkGetResult),
        )?,
        json_artifact(
            schema_dir.join("global-search-params.schema.json"),
            &schema_for!(GlobalSearchParams),
        )?,
        json_artifact(
            schema_dir.join("global-search-result.schema.json"),
            &schema_for!(GlobalSearchResult),
        )?,
        json_artifact(
            schema_dir.join("unified-search-params.schema.json"),
            &schema_for!(UnifiedSearchParams),
        )?,
        json_artifact(
            schema_dir.join("unified-search-result.schema.json"),
            &schema_for!(UnifiedSearchResult),
        )?,
        json_artifact(
            schema_dir.join("attention-case-list-params.schema.json"),
            &schema_for!(AttentionCaseListParams),
        )?,
        json_artifact(
            schema_dir.join("attention-case-list-result.schema.json"),
            &schema_for!(AttentionCaseListResult),
        )?,
        json_artifact(
            schema_dir.join("project-blocker-create-params.schema.json"),
            &schema_for!(ProjectBlockerCreateParams),
        )?,
        json_artifact(
            schema_dir.join("project-blocker-create-result.schema.json"),
            &schema_for!(ProjectBlockerCreateResult),
        )?,
        json_artifact(
            schema_dir.join("decision-request-create-params.schema.json"),
            &schema_for!(DecisionRequestCreateParams),
        )?,
        json_artifact(
            schema_dir.join("decision-request-create-result.schema.json"),
            &schema_for!(DecisionRequestCreateResult),
        )?,
        json_artifact(
            schema_dir.join("agent-incident-report-params.schema.json"),
            &schema_for!(AgentIncidentReportParams),
        )?,
        json_artifact(
            schema_dir.join("agent-incident-report-result.schema.json"),
            &schema_for!(AgentIncidentReportResult),
        )?,
        json_artifact(
            schema_dir.join("attention-case-transition-params.schema.json"),
            &schema_for!(AttentionCaseTransitionParams),
        )?,
        json_artifact(
            schema_dir.join("attention-case-transition-result.schema.json"),
            &schema_for!(AttentionCaseTransitionResult),
        )?,
        json_artifact(
            schema_dir.join("project-progress-get-params.schema.json"),
            &schema_for!(ProjectProgressGetParams),
        )?,
        json_artifact(
            schema_dir.join("project-progress-get-result.schema.json"),
            &schema_for!(ProjectProgressGetResult),
        )?,
        json_artifact(
            schema_dir.join("phase-plan-create-params.schema.json"),
            &schema_for!(PhasePlanCreateParams),
        )?,
        json_artifact(
            schema_dir.join("phase-plan-create-result.schema.json"),
            &schema_for!(PhasePlanCreateResult),
        )?,
        json_artifact(
            schema_dir.join("phase-plan-activate-params.schema.json"),
            &schema_for!(PhasePlanActivateParams),
        )?,
        json_artifact(
            schema_dir.join("phase-plan-activate-result.schema.json"),
            &schema_for!(PhasePlanActivateResult),
        )?,
        json_artifact(
            schema_dir.join("project-phase-transition-params.schema.json"),
            &schema_for!(ProjectPhaseTransitionParams),
        )?,
        json_artifact(
            schema_dir.join("project-phase-transition-result.schema.json"),
            &schema_for!(ProjectPhaseTransitionResult),
        )?,
        json_artifact(
            schema_dir.join("business-task-create-params.schema.json"),
            &schema_for!(BusinessTaskCreateParams),
        )?,
        json_artifact(
            schema_dir.join("business-task-create-result.schema.json"),
            &schema_for!(BusinessTaskCreateResult),
        )?,
        json_artifact(
            schema_dir.join("business-task-get-params.schema.json"),
            &schema_for!(BusinessTaskGetParams),
        )?,
        json_artifact(
            schema_dir.join("business-task-get-result.schema.json"),
            &schema_for!(BusinessTaskGetResult),
        )?,
        json_artifact(
            schema_dir.join("business-task-transition-params.schema.json"),
            &schema_for!(BusinessTaskTransitionParams),
        )?,
        json_artifact(
            schema_dir.join("business-task-transition-result.schema.json"),
            &schema_for!(BusinessTaskTransitionResult),
        )?,
        json_artifact(
            schema_dir.join("business-task-move-params.schema.json"),
            &schema_for!(BusinessTaskMoveParams),
        )?,
        json_artifact(
            schema_dir.join("business-task-move-result.schema.json"),
            &schema_for!(BusinessTaskMoveResult),
        )?,
        json_artifact(
            schema_dir.join("event-replay-params.schema.json"),
            &schema_for!(EventReplayParams),
        )?,
        json_artifact(
            schema_dir.join("event-replay-result.schema.json"),
            &schema_for!(EventReplayResult),
        )?,
        json_artifact(
            schema_dir.join("methods.json"),
            &serde_json::json!({
                "protocolVersion": contexthub_protocol::PROTOCOL_VERSION,
                "methods": [
                    {"name":"modelAuth/readPrepare","kind":"request","audience":"host_internal","paramsSchema":"model-auth-start-prepare-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/catalogPrepare","kind":"request","audience":"host_internal","paramsSchema":"model-auth-start-prepare-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/abortUnlaunched","kind":"request","audience":"host_internal","paramsSchema":"model-auth-abort-unlaunched-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/startPrepare","kind":"request","audience":"host_internal","paramsSchema":"model-auth-start-prepare-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/logoutPrepare","kind":"request","audience":"host_internal","paramsSchema":"model-auth-logout-prepare-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/registerProcess","kind":"request","audience":"host_internal","paramsSchema":"model-auth-register-process-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/recover","kind":"request","audience":"host_internal","paramsSchema":"model-auth-recover-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/step","kind":"request","audience":"host_internal","paramsSchema":"model-auth-step-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/finish","kind":"request","audience":"host_internal","paramsSchema":"model-operation-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/cancel","kind":"request","audience":"host_internal","paramsSchema":"model-operation-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/get","kind":"request","audience":"host_internal","paramsSchema":"model-operation-params.schema.json","resultSchema":"root-auth-change.schema.json"},
                    {"name":"modelAuth/list","kind":"request","audience":"host_internal","paramsSchema":"model-connection-params.schema.json","resultSchema":"root-auth-change-list.schema.json"},
                    {"name":"modelValidation/prepareExecution","kind":"request","audience":"host_internal","paramsSchema":"model-validation-execution-params.schema.json","resultSchema":"model-execution-lease.schema.json"},
                    {"name":"modelExecution/abortUnlaunched","kind":"request","audience":"host_internal","paramsSchema":"model-execution-abort-unlaunched-params.schema.json","resultSchema":"model-execution-lease.schema.json"},
                    {"name":"modelExecution/list","kind":"request","audience":"host_internal","paramsSchema":"model-execution-list-params.schema.json","resultSchema":"model-execution-lease-list.schema.json"},
                    {"name": "modelConnection/list", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection-list.schema.json"},
                    {"name": "modelConnection/get", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection.schema.json", "paramsSchema": "model-connection-params.schema.json"},
                    {"name": "modelConnection/changeList", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection-change-list.schema.json"},
                    {"name": "modelConnection/changeStatus", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection-change.schema.json", "paramsSchema": "model-operation-params.schema.json"},
                    {"name": "modelConnection/savePrepare", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection-change.schema.json", "paramsSchema": "model-connection-save-prepare-params.schema.json"},
                    {"name": "modelConnection/saveCommit", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection.schema.json", "paramsSchema": "model-connection-commit-params.schema.json"},
                    {"name": "modelConnection/saveAbort", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection-change.schema.json", "paramsSchema": "model-operation-params.schema.json"},
                    {"name": "modelConnection/credentialCleanupAck", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection-change.schema.json", "paramsSchema": "model-credential-cleanup-ack-params.schema.json"},
                    {"name":"mcpConnection/list","kind":"request","audience":"host_internal","resultSchema":"mcp-connection-list.schema.json"},
                    {"name":"mcpConnection/get","kind":"request","audience":"host_internal","resultSchema":"mcp-connection.schema.json","paramsSchema":"mcp-connection-params.schema.json"},
                    {"name":"mcpConnection/changeList","kind":"request","audience":"host_internal","resultSchema":"mcp-connection-change-list.schema.json"},
                    {"name":"mcpConnection/changeStatus","kind":"request","audience":"host_internal","resultSchema":"mcp-connection-change.schema.json","paramsSchema":"mcp-operation-params.schema.json"},
                    {"name":"mcpConnection/savePrepare","kind":"request","audience":"host_internal","resultSchema":"mcp-connection-change.schema.json","paramsSchema":"mcp-save-prepare-params.schema.json"},
                    {"name":"mcpConnection/saveCommit","kind":"request","audience":"host_internal","resultSchema":"mcp-connection.schema.json","paramsSchema":"mcp-save-commit-params.schema.json"},
                    {"name":"mcpConnection/saveAbort","kind":"request","audience":"host_internal","resultSchema":"mcp-connection-change.schema.json","paramsSchema":"mcp-operation-params.schema.json"},
                    {"name":"mcpConnection/grant","kind":"request","audience":"host_internal","resultSchema":"mcp-connection.schema.json","paramsSchema":"mcp-grant-params.schema.json"},
                    {"name":"mcpConnection/disable","kind":"request","audience":"host_internal","resultSchema":"mcp-connection.schema.json","paramsSchema":"mcp-mutation-params.schema.json"},
                    {"name":"mcpConnection/delete","kind":"request","audience":"host_internal","resultSchema":"mcp-connection.schema.json","paramsSchema":"mcp-mutation-params.schema.json"},
                    {"name":"mcpTool/snapshot","kind":"request","audience":"host_internal","resultSchema":"mcp-tool-snapshot.schema.json","paramsSchema":"mcp-snapshot-params.schema.json"},
                    {"name":"mcpExecution/list","kind":"request","audience":"host_internal","resultSchema":"mcp-execution-lease-list.schema.json"},
                    {"name":"mcpExecution/prepare","kind":"request","audience":"host_internal","resultSchema":"mcp-execution-lease.schema.json","paramsSchema":"mcp-execution-prepare-params.schema.json"},
                    {"name":"mcpExecution/registerProcess","kind":"request","audience":"host_internal","resultSchema":"mcp-execution-lease.schema.json","paramsSchema":"mcp-execution-register-params.schema.json"},
                    {"name":"mcpExecution/validate","kind":"request","audience":"host_internal","resultSchema":"mcp-execution-lease.schema.json","paramsSchema":"mcp-execution-params.schema.json"},
                    {"name":"mcpExecution/finish","kind":"request","audience":"host_internal","resultSchema":"mcp-execution-lease.schema.json","paramsSchema":"mcp-execution-finish-params.schema.json"},
                    {"name":"mcpExecution/abortUnlaunched","kind":"request","audience":"host_internal","resultSchema":"mcp-execution-lease.schema.json","paramsSchema":"mcp-execution-params.schema.json"},
                    {"name":"mcpExecution/reportValidation","kind":"request","audience":"host_internal","resultSchema":"mcp-validation-record.schema.json","paramsSchema":"mcp-validation-report-params.schema.json"},
                    {"name":"mcpCredentialCleanup/list","kind":"request","audience":"host_internal","resultSchema":"mcp-credential-cleanup-plan-list.schema.json"},
                    {"name":"mcpCredentialCleanup/prepare","kind":"request","audience":"host_internal","resultSchema":"mcp-credential-cleanup-plan.schema.json","paramsSchema":"mcp-credential-cleanup-params.schema.json"},
                    {"name":"mcpCredentialCleanup/confirm","kind":"request","audience":"host_internal","resultSchema":"mcp-credential-cleanup-plan.schema.json","paramsSchema":"mcp-credential-cleanup-confirm-params.schema.json"},
                    {"name":"personalMaintenance/recover","kind":"request","audience":"host_internal","paramsSchema":"personal-maintenance-params.schema.json","resultSchema":"personal-maintenance-fence.schema.json"},
                    {"name":"personalApplicationData/prepare","kind":"request","audience":"host_internal","resultSchema":"personal-application-data-operation.schema.json","paramsSchema":"personal-application-data-prepare-params.schema.json"},
                    {"name":"personalApplicationData/get","kind":"request","audience":"host_internal","resultSchema":"personal-application-data-operation.schema.json","paramsSchema":"personal-application-data-params.schema.json"},
                    {"name":"personalApplicationData/list","kind":"request","audience":"host_internal","resultSchema":"personal-application-data-operation-list.schema.json"},
                    {"name":"personalApplicationData/cancel","kind":"request","audience":"host_internal","resultSchema":"personal-application-data-operation.schema.json","paramsSchema":"personal-application-data-confirm-params.schema.json"},
                    {"name":"personalApplicationData/rollback","kind":"request","audience":"host_internal","resultSchema":"personal-application-data-operation.schema.json","paramsSchema":"personal-application-data-execute-params.schema.json"},
                    {"name":"personalApplicationData/resolveInterrupted","kind":"request","audience":"host_internal","paramsSchema":"personal-application-data-confirm-params.schema.json","resultSchema":"personal-application-data-operation.schema.json"},
                    {"name":"personalMemory/authorization","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-params.schema.json","resultSchema":"personal-memory-authorization.schema.json"},
                    {"name":"personalMemoryReset/prepare","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-reset-prepare-params.schema.json","resultSchema":"personal-memory-reset-operation.schema.json"},
                    {"name":"personalMemoryReset/start","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-reset-start-params.schema.json","resultSchema":"personal-memory-reset-start-result.schema.json"},
                    {"name":"personalMemoryReset/prepareWriter","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-reset-writer-params.schema.json","resultSchema":"personal-writer-lease.schema.json"},
                    {"name":"personalMemoryReset/scopeComplete","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-reset-scope-complete-params.schema.json","resultSchema":"personal-memory-reset-operation.schema.json"},
                    {"name":"personalMemoryReset/finish","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-reset-finish-params.schema.json","resultSchema":"personal-memory-reset-operation.schema.json"},
                    {"name":"personalMemoryReset/get","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-reset-params.schema.json","resultSchema":"personal-memory-reset-operation.schema.json"},
                    {"name":"personalMemoryReset/list","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-params.schema.json","resultSchema":"personal-memory-reset-operation-list.schema.json"},
                    {"name":"personalMemoryReset/reconcile","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-reset-params.schema.json","resultSchema":"personal-memory-reset-operation.schema.json"},
                    {"name":"personalMemoryReset/cancel","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-reset-confirm-params.schema.json","resultSchema":"personal-memory-reset-operation.schema.json"},
                    {"name":"personalMemoryReset/resolveInterrupted","kind":"request","audience":"host_internal","paramsSchema":"personal-memory-reset-confirm-params.schema.json","resultSchema":"personal-memory-reset-operation.schema.json"},
                    {"name":"personalMaintenance/operationStatus","kind":"request","audience":"host_internal","paramsSchema":"personal-data-params.schema.json","resultSchema":"personal-maintenance-operation-status.schema.json"},
                    {"name":"personalNamespaceStorage/prepare","kind":"request","audience":"host_internal","paramsSchema":"personal-namespace-storage-prepare-params.schema.json","resultSchema":"personal-namespace-storage-operation.schema.json"},
                    {"name":"personalNamespaceStorage/start","kind":"request","audience":"host_internal","paramsSchema":"personal-namespace-storage-start-params.schema.json","resultSchema":"personal-namespace-storage-start-result.schema.json"},
                    {"name":"personalNamespaceStorage/beginAction","kind":"request","audience":"host_internal","paramsSchema":"personal-namespace-action-params.schema.json","resultSchema":"personal-namespace-action-receipt.schema.json"},
                    {"name":"personalNamespaceStorage/ackAction","kind":"request","audience":"host_internal","paramsSchema":"personal-namespace-action-ack-params.schema.json","resultSchema":"personal-namespace-action-receipt.schema.json"},
                    {"name":"personalNamespaceStorage/get","kind":"request","audience":"host_internal","paramsSchema":"personal-namespace-storage-params.schema.json","resultSchema":"personal-namespace-storage-operation.schema.json"},
                    {"name":"personalNamespaceStorage/reconcile","kind":"request","audience":"host_internal","paramsSchema":"personal-namespace-storage-params.schema.json","resultSchema":"personal-namespace-storage-operation.schema.json"},
                    {"name":"personalNamespaceStorage/finish","kind":"request","audience":"host_internal","paramsSchema":"personal-namespace-storage-confirm-params.schema.json","resultSchema":"personal-namespace-storage-operation.schema.json"},
                    {"name":"personalNamespaceStorage/cancel","kind":"request","audience":"host_internal","paramsSchema":"personal-namespace-storage-confirm-params.schema.json","resultSchema":"personal-namespace-storage-operation.schema.json"},
                    {"name":"personalNamespaceStorage/resolveInterrupted","kind":"request","audience":"host_internal","paramsSchema":"personal-namespace-storage-confirm-params.schema.json","resultSchema":"personal-namespace-storage-operation.schema.json"},
                    {"name":"personalNamespaceStorage/list","kind":"request","audience":"host_internal","paramsSchema":"personal-storage-inventory-params.schema.json","resultSchema":"personal-namespace-storage-operation-list.schema.json"},
                    {"name":"personalPage/binding","kind":"request","audience":"host_internal","paramsSchema":"personal-page-binding-params.schema.json","resultSchema":"personal-page-binding-snapshot.schema.json"},
                    {"name":"personalPage/auditPrepare","kind":"request","audience":"host_internal","paramsSchema":"personal-page-audit-prepare-params.schema.json","resultSchema":"personal-page-audit-record.schema.json"},
                    {"name":"personalPage/auditFinish","kind":"request","audience":"host_internal","paramsSchema":"personal-page-audit-finish-params.schema.json","resultSchema":"personal-page-audit-record.schema.json"},
                    {"name":"personalPage/auditList","kind":"request","audience":"host_internal","paramsSchema":"personal-page-audit-list-params.schema.json","resultSchema":"personal-page-audit-list-result.schema.json"},
                    {"name":"personalStorage/inventory","kind":"request","audience":"host_internal","paramsSchema":"personal-storage-inventory-params.schema.json","resultSchema":"personal-storage-inventory.schema.json"},
                    {"name":"personalStorage/prepare","kind":"request","audience":"host_internal","paramsSchema":"personal-storage-prepare-params.schema.json","resultSchema":"personal-storage-operation.schema.json"},
                    {"name":"personalStorage/execute","kind":"request","audience":"host_internal","paramsSchema":"personal-storage-execute-params.schema.json","resultSchema":"personal-storage-operation.schema.json"},
                    {"name":"personalStorage/get","kind":"request","audience":"host_internal","paramsSchema":"personal-storage-params.schema.json","resultSchema":"personal-storage-operation.schema.json"},
                    {"name":"personalStorage/list","kind":"request","audience":"host_internal","paramsSchema":"personal-storage-inventory-params.schema.json","resultSchema":"personal-storage-operation-list.schema.json"},
                    {"name":"personalStorage/reconcile","kind":"request","audience":"host_internal","paramsSchema":"personal-storage-params.schema.json","resultSchema":"personal-storage-operation.schema.json"},
                    {"name":"personalStorage/cancel","kind":"request","audience":"host_internal","paramsSchema":"personal-storage-confirm-params.schema.json","resultSchema":"personal-storage-operation.schema.json"},
                    {"name":"personalStorage/resolveInterrupted","kind":"request","audience":"host_internal","paramsSchema":"personal-storage-confirm-params.schema.json","resultSchema":"personal-storage-operation.schema.json"},
                    {"name":"personalData/backupPrepare","kind":"request","audience":"host_internal","paramsSchema":"personal-backup-prepare-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalData/backupCapture","kind":"request","audience":"host_internal","paramsSchema":"personal-backup-capture-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalData/backupArtifact","kind":"request","audience":"host_internal","paramsSchema":"personal-data-params.schema.json","resultSchema":"personal-backup-artifact.schema.json"},
                    {"name":"personalData/restorePrepare","kind":"request","audience":"host_internal","paramsSchema":"personal-restore-prepare-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalData/restoreWorkspacePlan","kind":"request","audience":"host_internal","paramsSchema":"personal-restore-workspace-plan-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalData/restoreApplyPrepare","kind":"request","audience":"host_internal","paramsSchema":"personal-restore-apply-prepare-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalData/restoreAbortBeforeHelper","kind":"request","audience":"host_internal","paramsSchema":"personal-restore-abort-before-helper-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalData/get","kind":"request","audience":"host_internal","paramsSchema":"personal-data-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalData/list","kind":"request","audience":"host_internal","paramsSchema":"personal-data-list-params.schema.json","resultSchema":"personal-data-operation-list.schema.json"},
                    {"name":"personalData/cancel","kind":"request","audience":"host_internal","paramsSchema":"personal-data-cancel-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalData/resolveInterrupted","kind":"request","audience":"host_internal","paramsSchema":"personal-data-cancel-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalData/reconcile","kind":"request","audience":"host_internal","paramsSchema":"personal-data-params.schema.json","resultSchema":"personal-data-operation.schema.json"},
                    {"name":"personalMaintenance/acquire","kind":"request","audience":"host_internal","paramsSchema":"personal-maintenance-acquire-params.schema.json","resultSchema":"personal-maintenance-fence.schema.json"},
                    {"name":"personalMaintenance/release","kind":"request","audience":"host_internal","paramsSchema":"personal-maintenance-params.schema.json","resultSchema":"personal-maintenance-fence.schema.json"},
                    {"name":"personalMaintenance/list","kind":"request","audience":"host_internal","paramsSchema":"personal-maintenance-list-params.schema.json","resultSchema":"personal-maintenance-fence-list.schema.json"},
                    {"name":"personalMaintenance/drainStatus","kind":"request","audience":"host_internal","paramsSchema":"personal-maintenance-params.schema.json","resultSchema":"personal-maintenance-drain-status.schema.json"},
                    {"name":"personalMaintenance/read","kind":"request","audience":"host_internal","paramsSchema":"personal-maintenance-params.schema.json","resultSchema":"personal-maintenance-fence.schema.json"},
                    {"name":"profileWriter/prepare","kind":"request","audience":"host_internal","paramsSchema":"personal-writer-prepare-params.schema.json","resultSchema":"personal-writer-lease.schema.json"},
                    {"name":"profileWriter/registerProcess","kind":"request","audience":"host_internal","paramsSchema":"personal-writer-register-params.schema.json","resultSchema":"personal-writer-lease.schema.json"},
                    {"name":"profileWriter/abortUnlaunched","kind":"request","audience":"host_internal","paramsSchema":"personal-writer-abort-params.schema.json","resultSchema":"personal-writer-lease.schema.json"},
                    {"name":"profileWriter/release","kind":"request","audience":"host_internal","paramsSchema":"personal-writer-lease-params.schema.json","resultSchema":"personal-writer-lease.schema.json"},
                    {"name":"profileWriter/read","kind":"request","audience":"host_internal","paramsSchema":"personal-writer-lease-params.schema.json","resultSchema":"personal-writer-lease.schema.json"},
                    {"name":"profileWriter/list","kind":"request","audience":"host_internal","paramsSchema":"personal-writer-list-params.schema.json","resultSchema":"personal-writer-lease-list.schema.json"},
                    {"name":"profileWriter/recover","kind":"request","audience":"host_internal","paramsSchema":"personal-writer-lease-params.schema.json","resultSchema":"personal-writer-lease.schema.json"},
                    {"name":"personalPackage/importPrepare","kind":"request","audience":"host_internal","paramsSchema":"personal-package-import-prepare-params.schema.json","resultSchema":"personal-package-import-preview.schema.json"},
                    {"name":"personalPackage/importCommit","kind":"request","audience":"host_internal","paramsSchema":"personal-package-import-commit-params.schema.json","resultSchema":"personal-package-import-commit-result.schema.json"},
                    {"name":"personalPackage/importGet","kind":"request","audience":"host_internal","paramsSchema":"personal-package-import-params.schema.json","resultSchema":"personal-package-import-preview.schema.json"},
                    {"name":"personalPackage/importList","kind":"request","audience":"host_internal","paramsSchema":"personal-package-import-list-params.schema.json","resultSchema":"personal-package-import-list.schema.json"},
                    {"name":"personalPackage/importCancel","kind":"request","audience":"host_internal","paramsSchema":"personal-package-import-cancel-params.schema.json","resultSchema":"personal-package-import-preview.schema.json"},
                    {"name":"personalAutomation/admit","kind":"request","audience":"host_internal","paramsSchema":"personal-automation-admit-params.schema.json","resultSchema":"personal-automation-admit-result.schema.json"},
                    {"name":"personalAutomation/prepareExecution","kind":"request","audience":"host_internal","paramsSchema":"personal-automation-prepare-execution-params.schema.json","resultSchema":"model-execution-lease.schema.json"},
                    {"name":"personalAutomation/bind","kind":"request","audience":"host_internal","paramsSchema":"personal-automation-bind-params.schema.json","resultSchema":"personal-automation-bind-result.schema.json"},
                    {"name":"personalAutomation/finish","kind":"request","audience":"host_internal","paramsSchema":"personal-automation-finish-params.schema.json","resultSchema":"personal-automation-finish-result.schema.json"},
                    {"name":"personalAutomation/listTriggers","kind":"request","audience":"host_internal","paramsSchema":"personal-automation-list-triggers-params.schema.json","resultSchema":"personal-automation-trigger-page.schema.json"},
                    {"name":"personalAutomation/resolveInterrupted","kind":"request","audience":"host_internal","paramsSchema":"personal-automation-resolve-interrupted-params.schema.json","resultSchema":"personal-automation-finish-result.schema.json"},
                    {"name":"modelUsage/begin","kind":"request","audience":"host_internal","paramsSchema":"model-usage-begin-params.schema.json","resultSchema":"model-usage-record.schema.json"},
                    {"name":"modelUsage/finish","kind":"request","audience":"host_internal","paramsSchema":"model-usage-finish-params.schema.json","resultSchema":"model-usage-record.schema.json"},
                    {"name":"modelUsage/get","kind":"request","audience":"host_internal","paramsSchema":"model-usage-params.schema.json","resultSchema":"model-usage-record.schema.json"},
                    {"name":"modelUsage/list","kind":"request","audience":"host_internal","paramsSchema":"model-usage-list-params.schema.json","resultSchema":"model-usage-list-result.schema.json"},
                    {"name":"modelCredentialCleanup/list","kind":"request","audience":"host_internal","resultSchema":"model-credential-cleanup-candidate-list.schema.json"},
                    {"name":"modelCredentialCleanup/prepare","kind":"request","audience":"host_internal","paramsSchema":"model-credential-cleanup-prepare-params.schema.json","resultSchema":"model-credential-cleanup-plan.schema.json"},
                    {"name":"modelCredentialCleanup/ack","kind":"request","audience":"host_internal","paramsSchema":"model-credential-cleanup-confirm-params.schema.json","resultSchema":"model-credential-cleanup-plan.schema.json"},
                    {"name": "modelConnection/preflight", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection-preflight.schema.json", "paramsSchema": "model-connection-preflight-params.schema.json"},
                    {"name": "modelConnection/setEnabled", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection.schema.json", "paramsSchema": "model-connection-set-enabled-params.schema.json"},
                    {"name": "modelConnection/deleteCommit", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection.schema.json", "paramsSchema": "model-connection-mutation-params.schema.json"},
                    {"name": "modelConnection/disconnectCommit", "kind": "request", "audience": "host_internal", "resultSchema": "model-connection.schema.json", "paramsSchema": "model-connection-mutation-params.schema.json"},
                    {"name": "modelCatalog/read", "kind": "request", "audience": "host_internal", "resultSchema": "model-descriptor-list.schema.json", "paramsSchema": "model-connection-params.schema.json"},
                    {"name": "modelDescriptor/save", "kind": "request", "audience": "host_internal", "resultSchema": "model-descriptor.schema.json", "paramsSchema": "model-descriptor-save-params.schema.json"},
                    {"name": "modelDescriptor/observe", "kind": "request", "audience": "host_internal", "resultSchema": "model-descriptor-list.schema.json", "paramsSchema": "model-descriptor-observe-params.schema.json"},
                    {"name": "modelPurpose/list", "kind": "request", "audience": "host_internal", "resultSchema": "model-purpose-binding-list.schema.json"},
                    {"name": "modelPurpose/save", "kind": "request", "audience": "host_internal", "resultSchema": "model-purpose-binding.schema.json", "paramsSchema": "model-purpose-save-params.schema.json"},
                    {"name": "modelValidation/list", "kind": "request", "audience": "host_internal", "resultSchema": "model-validation-record-list.schema.json", "paramsSchema": "model-connection-params.schema.json"},
                    {"name": "modelValidation/prepare", "kind": "request", "audience": "host_internal", "resultSchema": "model-validation-record.schema.json", "paramsSchema": "model-validation-prepare-params.schema.json"},
                    {"name": "modelValidation/report", "kind": "request", "audience": "host_internal", "resultSchema": "model-validation-record.schema.json", "paramsSchema": "model-validation-report-params.schema.json"},
                    {"name": "modelExecution/resolve", "kind": "request", "audience": "host_internal", "resultSchema": "model-execution-snapshot.schema.json", "paramsSchema": "model-execution-prepare-params.schema.json"},
                    {"name": "modelExecution/prepare", "kind": "request", "audience": "host_internal", "resultSchema": "model-execution-lease.schema.json", "paramsSchema": "model-execution-prepare-params.schema.json"},
                    {"name": "modelExecution/validate", "kind": "request", "audience": "host_internal", "resultSchema": "model-execution-lease.schema.json", "paramsSchema": "model-execution-lease-params.schema.json"},
                    {"name": "modelExecution/validateForRequest", "kind": "request", "audience": "host_internal", "resultSchema": "model-execution-lease.schema.json", "paramsSchema": "model-execution-lease-params.schema.json"},
                    {"name": "modelExecution/registerProcess", "kind": "request", "audience": "host_internal", "resultSchema": "model-execution-lease.schema.json", "paramsSchema": "model-execution-register-process-params.schema.json"},
                    {"name": "modelExecution/release", "kind": "request", "audience": "host_internal", "resultSchema": "model-execution-lease.schema.json", "paramsSchema": "model-execution-lease-params.schema.json"},
                    {"name": "modelExecution/recover", "kind": "request", "audience": "host_internal", "resultSchema": "model-execution-recover-result.schema.json", "paramsSchema": "model-execution-recover-params.schema.json"},
                    { "name": "initialize", "kind": "request", "paramsSchema": "initialize-params.schema.json", "resultSchema": "initialize-result.schema.json" },
                    { "name": "initialized", "kind": "notification" },
                    { "name": "ping", "kind": "request" },
                    { "name": "preview/open", "kind": "request", "paramsSchema": "preview-open-params.schema.json", "resultSchema": "preview-open-result.schema.json" },
                    { "name": "textResource/read", "kind": "request", "paramsSchema": "text-resource-read-params.schema.json", "resultSchema": "text-resource-read-result.schema.json" },
                    { "name": "textDiff/compute", "kind": "request", "paramsSchema": "text-diff-compute-params.schema.json", "resultSchema": "text-diff-compute-result.schema.json" },
                    { "name": "textDiff/read", "kind": "request", "paramsSchema": "text-diff-read-params.schema.json", "resultSchema": "text-diff-read-result.schema.json" },
                    { "name": "file/confirm", "kind": "request", "paramsSchema": "file-confirm-params.schema.json", "resultSchema": "file-confirm-result.schema.json" },
                    { "name": "file/confirmationHistory", "kind": "request", "paramsSchema": "confirmation-history-params.schema.json", "resultSchema": "confirmation-history-result.schema.json" },
                    { "name": "workspace/list", "kind": "request", "paramsSchema": "workspace-list-params.schema.json", "resultSchema": "workspace-list-result.schema.json" },
                    { "name": "skill/runList", "kind": "request", "paramsSchema": "skill-run-list-params.schema.json", "resultSchema": "skill-run-list-result.schema.json" },
                    { "name": "skill/runRetry", "kind": "request", "paramsSchema": "skill-run-retry-params.schema.json", "resultSchema": "skill-run-installation.schema.json" },
                    { "name": "skill/list", "kind": "request", "paramsSchema": "skill-list-params.schema.json", "resultSchema": "skill-list-result.schema.json" },
                    { "name": "skill/get", "kind": "request", "paramsSchema": "skill-get-params.schema.json", "resultSchema": "skill-get-result.schema.json" },
                    { "name": "skill/save", "kind": "request", "paramsSchema": "skill-save-params.schema.json", "resultSchema": "skill-save-result.schema.json" },
                    { "name": "skill/import", "kind": "request", "paramsSchema": "skill-import-params.schema.json", "resultSchema": "skill-import-result.schema.json" },
                    { "name": "skill/importDirectory", "kind": "request", "paramsSchema": "skill-import-directory-params.schema.json", "resultSchema": "skill-import-result.schema.json" },
                    { "name": "skill/setState", "kind": "request", "paramsSchema": "skill-set-state-params.schema.json", "resultSchema": "skill-set-state-result.schema.json" },
                    { "name": "skill/delete", "kind": "request", "paramsSchema": "skill-delete-params.schema.json", "resultSchema": "skill-delete-result.schema.json" },
                    { "name": "skill/conversationCatalog", "kind": "request", "paramsSchema": "skill-conversation-catalog-params.schema.json", "resultSchema": "skill-conversation-catalog-result.schema.json" },
                    { "name": "skill/runtimeCatalog", "kind": "request", "paramsSchema": "skill-runtime-catalog-params.schema.json", "resultSchema": "skill-runtime-catalog-result.schema.json" },
                    { "name": "capabilityCreation/create", "kind": "request", "paramsSchema": "capability-creation-create-params.schema.json", "resultSchema": "capability-creation-create-result.schema.json" },
                    { "name": "capabilityCreation/list", "kind": "request", "paramsSchema": "capability-creation-list-params.schema.json", "resultSchema": "capability-creation-list-result.schema.json" },
                    { "name": "capabilityCreation/get", "kind": "request", "paramsSchema": "capability-creation-get-params.schema.json", "resultSchema": "capability-creation-get-result.schema.json" },
                    { "name": "capabilityCreation/preview", "kind": "request", "paramsSchema": "capability-creation-preview-params.schema.json", "resultSchema": "capability-creation-preview-result.schema.json" },
                    { "name": "personalApplication/open", "kind": "request", "paramsSchema": "personal-application-open-params.schema.json", "resultSchema": "personal-application-open-result.schema.json" },
                    { "name": "workspace/createDirectory", "kind": "request", "paramsSchema": "workspace-directory-create-params.schema.json", "resultSchema": "workspace-directory-create-result.schema.json" },
                    { "name": "file/hydrate", "kind": "request", "paramsSchema": "file-hydration-params.schema.json", "resultSchema": "file-hydration-result.schema.json" },
                    { "name": "preview/readBinary", "kind": "request", "paramsSchema": "read-binary-preview-params.schema.json", "resultSchema": "read-binary-preview-result.schema.json" },
                    { "name": "workspace/create", "kind": "request", "paramsSchema": "workspace-create-params.schema.json", "resultSchema": "workspace-create-result.schema.json" },
                    { "name": "workspace/targetValidate", "kind": "request", "paramsSchema": "workspace-target-validate-params.schema.json", "resultSchema": "workspace-target-validate-result.schema.json" },
                    { "name": "workspace/scan", "kind": "request", "paramsSchema": "workspace-scan-params.schema.json", "resultSchema": "workspace-scan-result.schema.json" },
                    { "name": "workspace/delete", "kind": "request", "paramsSchema": "workspace-delete-params.schema.json", "resultSchema": "workspace-delete-result.schema.json" },
                    { "name": "workspace/files", "kind": "request", "paramsSchema": "workspace-files-params.schema.json", "resultSchema": "workspace-files-result.schema.json" },
                    { "name": "workspace/listDirectory", "kind": "request", "paramsSchema": "workspace-list-directory-params.schema.json", "resultSchema": "workspace-list-directory-result.schema.json" },
                    { "name": "workspace/indexRequest", "kind": "request", "paramsSchema": "workspace-index-request-params.schema.json", "resultSchema": "workspace-index-request-result.schema.json" },
                    { "name": "workspace/indexStatus", "kind": "request", "paramsSchema": "workspace-index-status-params.schema.json", "resultSchema": "workspace-index-status-result.schema.json" },
                    { "name": "workspace/indexPath", "kind": "request", "paramsSchema": "workspace-index-path-params.schema.json", "resultSchema": "workspace-index-path-result.schema.json" },
                    { "name": "aiConversation/setArchived", "kind": "request", "paramsSchema": "ai-conversation-set-archived-params.schema.json", "resultSchema": "ai-conversation-set-archived-result.schema.json" },
                    { "name": "aiThread/get", "kind": "request", "paramsSchema": "ai-thread-get-params.schema.json", "resultSchema": "ai-thread-state.schema.json" },
                    { "name": "aiThread/prepare", "kind": "request", "paramsSchema": "ai-thread-prepare-params.schema.json", "resultSchema": "ai-thread-mutation-result.schema.json" },
                    { "name": "aiThread/acknowledge", "kind": "request", "paramsSchema": "ai-thread-acknowledge-params.schema.json", "resultSchema": "ai-thread-mutation-result.schema.json" },
                    { "name": "aiThread/reconcile", "kind": "request", "paramsSchema": "ai-thread-acknowledge-params.schema.json", "resultSchema": "ai-thread-mutation-result.schema.json" },
                    { "name": "aiThread/cleanup", "kind": "request", "paramsSchema": "ai-thread-cleanup-params.schema.json", "resultSchema": "ai-thread-mutation-result.schema.json" },
                    { "name": "aiThread/release", "kind": "request", "paramsSchema": "ai-thread-release-params.schema.json", "resultSchema": "ai-thread-mutation-result.schema.json" },
                    { "name": "aiHistory/execute", "kind": "request", "paramsSchema": "ai-history-params.schema.json", "resultSchema": "ai-history-result.schema.json" },
                    { "name": "aiLedger/get", "kind": "request", "paramsSchema": "ai-ledger-get-params.schema.json", "resultSchema": "ai-ledger-get-result.schema.json" },
                    { "name": "aiLedger/apply", "kind": "request", "paramsSchema": "ai-ledger-apply-params.schema.json", "resultSchema": "ai-ledger-apply-result.schema.json" },
                    { "name": "aiState/get", "kind": "request", "paramsSchema": "ai-state-get-params.schema.json", "resultSchema": "ai-state-get-result.schema.json" },
                    { "name": "aiState/replace", "kind": "request", "paramsSchema": "ai-state-replace-params.schema.json", "resultSchema": "ai-state-replace-result.schema.json" },
                    { "name": "aiState/apply", "kind": "request", "paramsSchema": "ai-state-apply-params.schema.json", "resultSchema": "ai-state-apply-result.schema.json" },
                    { "name": "aiWork/taskCandidates", "kind": "request", "paramsSchema": "ai-work-task-candidates-params.schema.json", "resultSchema": "ai-work-task-candidates-result.schema.json" },
                    { "name": "aiTurn/finalize", "kind": "request", "paramsSchema": "ai-turn-finalize-params.schema.json", "resultSchema": "ai-turn-finalize-result.schema.json" },
                    { "name": "project/list", "kind": "request", "paramsSchema": "project-list-params.schema.json", "resultSchema": "project-list-result.schema.json" },
                    { "name": "project/create", "kind": "request", "paramsSchema": "project-create-params.schema.json", "resultSchema": "project-create-result.schema.json" },
                    { "name": "project/get", "kind": "request", "paramsSchema": "project-get-params.schema.json", "resultSchema": "project-get-result.schema.json" },
                    { "name": "project/attachWorkspace", "kind": "request", "paramsSchema": "project-attach-workspace-params.schema.json", "resultSchema": "project-attach-workspace-result.schema.json" },
                    { "name": "project/detachWorkspace", "kind": "request", "paramsSchema": "project-detach-workspace-params.schema.json", "resultSchema": "project-detach-workspace-result.schema.json" },
                    { "name": "project/delete", "kind": "request", "paramsSchema": "project-delete-params.schema.json", "resultSchema": "project-delete-result.schema.json" },
                    { "name": "projectPortfolio/list", "kind": "request", "paramsSchema": "project-portfolio-list-params.schema.json", "resultSchema": "project-portfolio-list-result.schema.json" },
                    { "name": "deliverable/list", "kind": "request", "paramsSchema": "deliverable-list-params.schema.json", "resultSchema": "deliverable-list-result.schema.json" },
                    { "name": "deliverable/create", "kind": "request", "paramsSchema": "deliverable-create-params.schema.json", "resultSchema": "deliverable-create-result.schema.json" },
                    { "name": "deliverable/version/create", "kind": "request", "paramsSchema": "deliverable-version-create-params.schema.json", "resultSchema": "deliverable-version-create-result.schema.json" },
                    { "name": "deliverable/version/review", "kind": "request", "paramsSchema": "deliverable-version-review-params.schema.json", "resultSchema": "deliverable-version-review-result.schema.json" },
                    { "name": "myWork/get", "kind": "request", "paramsSchema": "my-work-get-params.schema.json", "resultSchema": "my-work-get-result.schema.json" },
                    { "name": "globalSearch/query", "kind": "request", "paramsSchema": "global-search-params.schema.json", "resultSchema": "global-search-result.schema.json" },
                    { "name": "unifiedSearch/query", "kind": "request", "paramsSchema": "unified-search-params.schema.json", "resultSchema": "unified-search-result.schema.json" },
                    { "name": "attentionCase/list", "kind": "request", "paramsSchema": "attention-case-list-params.schema.json", "resultSchema": "attention-case-list-result.schema.json" },
                    { "name": "projectBlocker/create", "kind": "request", "paramsSchema": "project-blocker-create-params.schema.json", "resultSchema": "project-blocker-create-result.schema.json" },
                    { "name": "decisionRequest/create", "kind": "request", "paramsSchema": "decision-request-create-params.schema.json", "resultSchema": "decision-request-create-result.schema.json" },
                    { "name": "agentIncident/report", "kind": "request", "paramsSchema": "agent-incident-report-params.schema.json", "resultSchema": "agent-incident-report-result.schema.json" },
                    { "name": "attentionCase/transition", "kind": "request", "paramsSchema": "attention-case-transition-params.schema.json", "resultSchema": "attention-case-transition-result.schema.json" },
                    { "name": "projectProgress/get", "kind": "request", "paramsSchema": "project-progress-get-params.schema.json", "resultSchema": "project-progress-get-result.schema.json" },
                    { "name": "phasePlan/create", "kind": "request", "paramsSchema": "phase-plan-create-params.schema.json", "resultSchema": "phase-plan-create-result.schema.json" },
                    { "name": "phasePlan/activate", "kind": "request", "paramsSchema": "phase-plan-activate-params.schema.json", "resultSchema": "phase-plan-activate-result.schema.json" },
                    { "name": "projectPhase/transition", "kind": "request", "paramsSchema": "project-phase-transition-params.schema.json", "resultSchema": "project-phase-transition-result.schema.json" },
                    { "name": "businessTask/create", "kind": "request", "paramsSchema": "business-task-create-params.schema.json", "resultSchema": "business-task-create-result.schema.json" },
                    { "name": "businessTask/get", "kind": "request", "paramsSchema": "business-task-get-params.schema.json", "resultSchema": "business-task-get-result.schema.json" },
                    { "name": "businessTask/transition", "kind": "request", "paramsSchema": "business-task-transition-params.schema.json", "resultSchema": "business-task-transition-result.schema.json" },
                    { "name": "businessTask/move", "kind": "request", "paramsSchema": "business-task-move-params.schema.json", "resultSchema": "business-task-move-result.schema.json" },
                    { "name": "event/replay", "kind": "request", "paramsSchema": "event-replay-params.schema.json", "resultSchema": "event-replay-result.schema.json" },
                    { "name": "shutdown/prepare", "kind": "request" },
                    { "name": "shutdown/execute", "kind": "request" }
                ],
                "standardErrorSchema": "error-response.schema.json"
            }),
        )?,
        Artifact {
            path: root.join("apps/desktop-ui/src/generated/protocol.ts"),
            content: typescript_contract(),
        },
    ])
}

fn json_artifact<T: Serialize>(path: PathBuf, value: &T) -> Result<Artifact, String> {
    let mut content = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    content.push('\n');
    Ok(Artifact { path, content })
}

#[expect(
    clippy::too_many_lines,
    reason = "all generated TypeScript declarations remain visibly enumerated in one frozen source"
)]
fn typescript_contract() -> String {
    let config = ts_rs::Config::default();
    let declarations = [
        AiRuntimeQueryScope::decl(&config),
        AiRuntimeQuery::decl(&config),
        ConversationSessionSnapshot::decl(&config),
        ConversationSessionPatch::decl(&config),
        ConversationSessionChange::decl(&config),
        ModelAuthAbortUnlaunchedParams::decl(&config),
        ModelValidationExecutionParams::decl(&config),
        ModelExecutionAbortUnlaunchedParams::decl(&config),
        ModelRuntimeKind::decl(&config),
        ModelConnectionChangeIntent::decl(&config),
        ModelAuthScopeState::decl(&config),
        ModelAuthScope::decl(&config),
        ModelAuthIntent::decl(&config),
        ModelAuthChangeState::decl(&config),
        ModelAuthStepState::decl(&config),
        ModelAuthStep::decl(&config),
        RootAuthChange::decl(&config),
        ModelAuthStartPrepareParams::decl(&config),
        ModelAuthLogoutPrepareParams::decl(&config),
        ModelAuthProcess::decl(&config),
        ModelAuthRegisterProcessParams::decl(&config),
        ModelAuthRecoverParams::decl(&config),
        ModelAuthStepParams::decl(&config),
        ModelExecutionOwner::decl(&config),
        ModelExecutionProcess::decl(&config),
        ModelExecutionRegisterProcessParams::decl(&config),
        ModelExecutionRecoveryState::decl(&config),
        ModelExecutionLeaseDiagnostic::decl(&config),
        ModelExecutionRecoverParams::decl(&config),
        ModelExecutionRecoverResult::decl(&config),
        ModelThreadRuntimeScope::decl(&config),
        ModelConnectionKind::decl(&config),
        ModelAuthMode::decl(&config),
        LocalModelRuntime::decl(&config),
        ModelSecretRef::decl(&config),
        ModelNetworkMode::decl(&config),
        PersonalAutomationTriggerOutcome::decl(&config),
        PersonalAutomationFinishOutcome::decl(&config),
        PersonalAutomationRuleRecord::decl(&config),
        PersonalAutomationClaim::decl(&config),
        PersonalAutomationTriggerRecord::decl(&config),
        PersonalAutomationAdmitParams::decl(&config),
        PersonalAutomationAdmitResult::decl(&config),
        PersonalAutomationPrepareExecutionParams::decl(&config),
        PersonalAutomationBindParams::decl(&config),
        PersonalAutomationBindResult::decl(&config),
        PersonalAutomationFinishParams::decl(&config),
        PersonalAutomationFinishResult::decl(&config),
        PersonalAutomationListTriggersParams::decl(&config),
        PersonalAutomationTriggerItem::decl(&config),
        PersonalAutomationTriggerPage::decl(&config),
        PersonalAutomationResolveInterruptedParams::decl(&config),
        ModelNetworkAuthentication::decl(&config),
        ModelUsageState::decl(&config),
        ModelUsageTokens::decl(&config),
        ModelUsageCost::decl(&config),
        ModelUsageBeginParams::decl(&config),
        ModelUsageFinishParams::decl(&config),
        ModelUsageParams::decl(&config),
        ModelUsageListParams::decl(&config),
        ModelUsageRecord::decl(&config),
        ModelUsageListResult::decl(&config),
        ModelNetworkConfig::decl(&config),
        ModelConnectionConfig::decl(&config),
        ModelConnection::decl(&config),
        ModelCredentialAction::decl(&config),
        ModelConnectionSavePrepareParams::decl(&config),
        ModelConnectionChangeState::decl(&config),
        ModelConnectionChange::decl(&config),
        ModelCredentialCleanupState::decl(&config),
        ModelCredentialCleanupAckParams::decl(&config),
        PersonalApplicationDataAction::decl(&config),
        PersonalApplicationDataState::decl(&config),
        PersonalApplicationDataPrepareParams::decl(&config),
        PersonalApplicationDataParams::decl(&config),
        PersonalApplicationDataConfirmParams::decl(&config),
        PersonalApplicationDataExecuteParams::decl(&config),
        PersonalApplicationDataOperation::decl(&config),
        PersonalMemoryPreferences::decl(&config),
        PersonalMemoryEnableConfirmation::decl(&config),
        PersonalMemoryAuthorization::decl(&config),
        PersonalMemoryParams::decl(&config),
        PersonalMemoryResetState::decl(&config),
        PersonalMemoryResetScope::decl(&config),
        PersonalMemoryResetOutcome::decl(&config),
        PersonalMemoryResetScopeResult::decl(&config),
        PersonalMemoryResetOperation::decl(&config),
        PersonalMemoryResetPrepareParams::decl(&config),
        PersonalMemoryResetParams::decl(&config),
        PersonalMemoryResetConfirmParams::decl(&config),
        PersonalMemoryResetStartParams::decl(&config),
        PersonalMemoryResetPermit::decl(&config),
        PersonalMemoryResetStartResult::decl(&config),
        PersonalMemoryResetWriterParams::decl(&config),
        PersonalMemoryResetScopeCompleteParams::decl(&config),
        PersonalMemoryResetFinishParams::decl(&config),
        PersonalNamespaceStorageKind::decl(&config),
        PersonalNamespaceLogFile::decl(&config),
        PersonalNamespaceLogSnapshot::decl(&config),
        PersonalNamespaceBrowserSnapshot::decl(&config),
        PersonalNamespaceStoragePrepareParams::decl(&config),
        PersonalNamespaceActionState::decl(&config),
        PersonalNamespaceActionOutcome::decl(&config),
        PersonalNamespaceActionReceipt::decl(&config),
        PersonalNamespaceStorageOperation::decl(&config),
        PersonalNamespaceStorageParams::decl(&config),
        PersonalNamespaceStorageConfirmParams::decl(&config),
        PersonalNamespaceStorageStartParams::decl(&config),
        PersonalNamespaceActionPermit::decl(&config),
        PersonalNamespaceStorageStartResult::decl(&config),
        PersonalNamespaceActionParams::decl(&config),
        PersonalNamespaceActionAckParams::decl(&config),
        PersonalPageRiskLevel::decl(&config),
        PersonalPageConfirmationPolicy::decl(&config),
        PersonalPageContextType::decl(&config),
        PersonalPageUiAction::decl(&config),
        PersonalPageTool::decl(&config),
        PersonalPageAiManifest::decl(&config),
        PersonalPageBindingParams::decl(&config),
        PersonalPageRuntimeBinding::decl(&config),
        PersonalPageBindingSnapshot::decl(&config),
        PersonalPageAuditMethod::decl(&config),
        PersonalPageAuditOutcome::decl(&config),
        PersonalPageAuditPrepareParams::decl(&config),
        PersonalPageAuditFinishParams::decl(&config),
        PersonalPageAuditRecord::decl(&config),
        PersonalPageAuditListParams::decl(&config),
        PersonalPageAuditEvent::decl(&config),
        PersonalPageAuditListResult::decl(&config),
        PersonalStorageCategory::decl(&config),
        PersonalStorageCoverageState::decl(&config),
        PersonalStorageCoverage::decl(&config),
        PersonalStorageReference::decl(&config),
        PersonalStorageItem::decl(&config),
        PersonalStorageInventory::decl(&config),
        PersonalStorageOperationState::decl(&config),
        PersonalStorageOutcome::decl(&config),
        PersonalStorageItemResult::decl(&config),
        PersonalStorageResult::decl(&config),
        PersonalStorageOperation::decl(&config),
        PersonalStorageInventoryParams::decl(&config),
        PersonalStoragePrepareParams::decl(&config),
        PersonalStorageExecuteParams::decl(&config),
        PersonalStorageParams::decl(&config),
        PersonalStorageConfirmParams::decl(&config),
        PersonalMaintenanceOperationStatus::decl(&config),
        PersonalDataCategory::decl(&config),
        PersonalDataOperationKind::decl(&config),
        PersonalDataOperationState::decl(&config),
        PersonalDataIssue::decl(&config),
        PersonalBackupSelection::decl(&config),
        PersonalWorkspaceContentEntry::decl(&config),
        PersonalWorkspaceContentSnapshot::decl(&config),
        PersonalRestoreWorkspaceDestination::decl(&config),
        PersonalRestoredWorkspace::decl(&config),
        PersonalRestoreWorkspacePlanParams::decl(&config),
        PersonalBackupWorkspace::decl(&config),
        PersonalBackupNativeHome::decl(&config),
        PersonalBackupFile::decl(&config),
        PersonalBackupManifest::decl(&config),
        PersonalRestorePreview::decl(&config),
        PersonalDataResult::decl(&config),
        PersonalDataOperation::decl(&config),
        PersonalBackupPrepareParams::decl(&config),
        PersonalDataQuiescence::decl(&config),
        PersonalBackupCaptureParams::decl(&config),
        PersonalRestorePrepareParams::decl(&config),
        PersonalRestoreApplyPrepareParams::decl(&config),
        PersonalRestoreAbortBeforeHelperParams::decl(&config),
        PersonalDataParams::decl(&config),
        PersonalDataListParams::decl(&config),
        PersonalDataCancelParams::decl(&config),
        PersonalBackupArtifact::decl(&config),
        PersonalWriterMode::decl(&config),
        PersonalWriterState::decl(&config),
        PersonalWriterPrepareParams::decl(&config),
        PersonalWriterLease::decl(&config),
        PersonalWriterRegisterParams::decl(&config),
        PersonalWriterLeaseParams::decl(&config),
        PersonalWriterAbortParams::decl(&config),
        PersonalWriterListParams::decl(&config),
        PersonalMaintenanceState::decl(&config),
        PersonalMaintenanceFence::decl(&config),
        PersonalMaintenanceDrainStatus::decl(&config),
        PersonalMaintenanceListParams::decl(&config),
        PersonalMaintenanceAcquireParams::decl(&config),
        PersonalMaintenanceParams::decl(&config),
        McpPurpose::decl(&config),
        McpSource::decl(&config),
        McpLocalPermissions::decl(&config),
        McpAuthMode::decl(&config),
        McpNetworkConfig::decl(&config),
        McpConnectionConfig::decl(&config),
        McpSecretRef::decl(&config),
        McpToolDefinition::decl(&config),
        McpValidationState::decl(&config),
        McpValidationRecord::decl(&config),
        McpConnection::decl(&config),
        McpCredentialAction::decl(&config),
        McpSavePrepareParams::decl(&config),
        McpChangeState::decl(&config),
        McpConnectionChange::decl(&config),
        McpOperationParams::decl(&config),
        McpConnectionParams::decl(&config),
        McpSaveCommitParams::decl(&config),
        McpMutationParams::decl(&config),
        McpGrantParams::decl(&config),
        McpExecutionIntent::decl(&config),
        McpExecutionPrepareParams::decl(&config),
        McpExecutionState::decl(&config),
        McpExecutionLease::decl(&config),
        McpExecutionParams::decl(&config),
        McpExecutionRegisterParams::decl(&config),
        McpExecutionFinishParams::decl(&config),
        McpValidationReportParams::decl(&config),
        McpValidateParams::decl(&config),
        McpSnapshotParams::decl(&config),
        McpToolSnapshot::decl(&config),
        McpCredentialCleanupParams::decl(&config),
        McpCredentialCleanupState::decl(&config),
        McpCredentialCleanupPlan::decl(&config),
        McpCredentialCleanupConfirmParams::decl(&config),
        PersonalPackageImportTarget::decl(&config),
        PersonalPackageImportState::decl(&config),
        PersonalPackageImportSource::decl(&config),
        PersonalPackageImportPreview::decl(&config),
        PersonalPackageImportPrepareParams::decl(&config),
        PersonalPackageImportCommitParams::decl(&config),
        PersonalPackageImportParams::decl(&config),
        PersonalPackageImportCancelParams::decl(&config),
        PersonalPackageImportListParams::decl(&config),
        PersonalPackageImportCommitResult::decl(&config),
        ModelCredentialCleanupCandidate::decl(&config),
        ModelCredentialCleanupPlanState::decl(&config),
        ModelCredentialCleanupPlan::decl(&config),
        ModelCredentialCleanupPrepareParams::decl(&config),
        ModelCredentialCleanupConfirmParams::decl(&config),
        ModelConnectionCommitParams::decl(&config),
        ModelOperationParams::decl(&config),
        ModelConnectionParams::decl(&config),
        ModelEvidenceSource::decl(&config),
        ModelCapabilityState::decl(&config),
        ModelCapabilityEvidence::decl(&config),
        ModelDescriptor::decl(&config),
        ModelDescriptorSaveParams::decl(&config),
        ModelDescriptorObservation::decl(&config),
        ModelDescriptorObserveParams::decl(&config),
        ModelPurpose::decl(&config),
        ModelSelectionPolicy::decl(&config),
        ModelSelection::decl(&config),
        ModelHistoryConsent::decl(&config),
        ModelPurposeBinding::decl(&config),
        ModelPurposeSaveParams::decl(&config),
        ModelValidationLayer::decl(&config),
        ModelValidationOutcome::decl(&config),
        ModelValidationPrepareParams::decl(&config),
        ModelValidationRecord::decl(&config),
        ModelValidationReportParams::decl(&config),
        ModelExecutionPrepareParams::decl(&config),
        ModelExecutionSnapshot::decl(&config),
        ModelExecutionLease::decl(&config),
        ModelExecutionListParams::decl(&config),
        ModelExecutionLeaseParams::decl(&config),
        ModelConnectionMutation::decl(&config),
        ModelConnectionPreflightParams::decl(&config),
        ModelConnectionReference::decl(&config),
        ModelConnectionPreflight::decl(&config),
        ModelConnectionMutationParams::decl(&config),
        ModelConnectionSetEnabledParams::decl(&config),
        PersonalProfileManifest::decl(&config),
        RequestId::decl(&config),
        ClientInfo::decl(&config),
        ProtocolRange::decl(&config),
        ClientCapabilities::decl(&config),
        ResumeCursor::decl(&config),
        InitializeParams::decl(&config),
        ServerInfo::decl(&config),
        ServerCapabilities::decl(&config),
        SessionInfo::decl(&config),
        ProtocolLimits::decl(&config),
        ResumeResult::decl(&config),
        InitializeResult::decl(&config),
        PreviewSource::decl(&config),
        PreviewFormat::decl(&config),
        PreviewAvailability::decl(&config),
        OpenPreviewParams::decl(&config),
        OpenPreviewResult::decl(&config),
        FileHydrationParams::decl(&config),
        FileHydrationResult::decl(&config),
        ReadBinaryPreviewParams::decl(&config),
        ReadBinaryPreviewResult::decl(&config),
        ReadTextResourceParams::decl(&config),
        ReadTextResourceResult::decl(&config),
        DiffSourceRef::decl(&config),
        ComputeTextDiffParams::decl(&config),
        TextDiffSummary::decl(&config),
        ComputeTextDiffResult::decl(&config),
        ReadTextDiffParams::decl(&config),
        TextDiffRowKind::decl(&config),
        TextDiffRow::decl(&config),
        ReadTextDiffResult::decl(&config),
        ConfirmFileParams::decl(&config),
        ConfirmFileResult::decl(&config),
        ConfirmationHistoryParams::decl(&config),
        ConfirmationHistoryItem::decl(&config),
        ConfirmationHistoryResult::decl(&config),
        WorkspaceListParams::decl(&config),
        WorkspaceSummary::decl(&config),
        WorkspaceListResult::decl(&config),
        SkillRunListParams::decl(&config),
        SkillRunListResult::decl(&config),
        SkillRunRetryParams::decl(&config),
        SkillRunInstallation::decl(&config),
        SkillListParams::decl(&config),
        SkillSummary::decl(&config),
        SkillListResult::decl(&config),
        SkillGetParams::decl(&config),
        SkillPackageFile::decl(&config),
        SkillPackageFileInput::decl(&config),
        SkillDetail::decl(&config),
        SkillGetResult::decl(&config),
        SkillSaveParams::decl(&config),
        SkillSaveResult::decl(&config),
        SkillImportParams::decl(&config),
        SkillImportDirectoryParams::decl(&config),
        SkillImportResult::decl(&config),
        SkillSetStateParams::decl(&config),
        SkillSetStateResult::decl(&config),
        SkillDeleteParams::decl(&config),
        SkillDeleteResult::decl(&config),
        SkillConversationCatalogParams::decl(&config),
        SkillConversationCatalogItem::decl(&config),
        SkillConversationCatalogResult::decl(&config),
        SkillRuntimeCatalogParams::decl(&config),
        SkillRuntimeCatalogItem::decl(&config),
        SkillRuntimeCatalogResult::decl(&config),
        CapabilityCreationAcceptanceExample::decl(&config),
        CapabilityCreationSourceFile::decl(&config),
        CapabilityCreationProject::decl(&config),
        CapabilityCreationCreateParams::decl(&config),
        CapabilityCreationCreateResult::decl(&config),
        CapabilityCreationListParams::decl(&config),
        CapabilityCreationListResult::decl(&config),
        CapabilityCreationGetParams::decl(&config),
        CapabilityCreationGetResult::decl(&config),
        CapabilityCreationPreviewParams::decl(&config),
        CapabilityCreationPreviewResult::decl(&config),
        CapabilityCreationInstallParams::decl(&config),
        CapabilityCreationInstallResult::decl(&config),
        PersonalApplicationOpenParams::decl(&config),
        PersonalApplicationOpenResult::decl(&config),
        ApplicationSkillBinding::decl(&config),
        WorkspaceDirectoryCreateParams::decl(&config),
        WorkspaceDirectoryCreateResult::decl(&config),
        WorkspaceCreateParams::decl(&config),
        WorkspaceCreateResult::decl(&config),
        WorkspaceTargetValidateParams::decl(&config),
        WorkspaceTargetValidateResult::decl(&config),
        WorkspaceScanParams::decl(&config),
        WorkspaceScanResult::decl(&config),
        WorkspaceFilesParams::decl(&config),
        WorkspaceFileItem::decl(&config),
        WorkspaceFilesResult::decl(&config),
        WorkspaceListDirectoryParams::decl(&config),
        WorkspaceDirectoryEntry::decl(&config),
        WorkspaceListDirectoryResult::decl(&config),
        WorkspaceIndexRequestParams::decl(&config),
        WorkspaceIndexRequestResult::decl(&config),
        WorkspaceIndexStatusParams::decl(&config),
        WorkspaceIndexStatusResult::decl(&config),
        WorkspaceIndexPathParams::decl(&config),
        WorkspaceIndexPathResult::decl(&config),
        AiConversationSetArchivedParams::decl(&config),
        AiConversationSetArchivedResult::decl(&config),
        AiApplicationAction::decl(&config),
        AiApplicationParams::decl(&config),
        AiThreadGetParams::decl(&config),
        AiThreadState::decl(&config),
        AiThreadOperationKind::decl(&config),
        AiThreadOperationEvent::decl(&config),
        AiThreadApplicationScope::decl(&config),
        AiApplicationSessionKind::decl(&config),
        AiThreadPrepareParams::decl(&config),
        AiThreadAcknowledgeParams::decl(&config),
        AiThreadReleaseParams::decl(&config),
        AiThreadCleanupParams::decl(&config),
        AiThreadMutationResult::decl(&config),
        AiHistoryParams::decl(&config),
        AiHistoryResult::decl(&config),
        AiLedgerGetParams::decl(&config),
        AiLedgerGetResult::decl(&config),
        AiLedgerRecordMutation::decl(&config),
        AiLedgerApplyParams::decl(&config),
        AiLedgerRecordReceipt::decl(&config),
        AiLedgerApplyResult::decl(&config),
        AiStateGetParams::decl(&config),
        AiStateGetResult::decl(&config),
        AiStateReplaceParams::decl(&config),
        AiStateReplaceResult::decl(&config),
        AiStateEntityUpsert::decl(&config),
        AiStateEntityDelta::decl(&config),
        AiStateDelta::decl(&config),
        AiStateApplyParams::decl(&config),
        AiStateApplyResult::decl(&config),
        AiWorkTaskCandidatesParams::decl(&config),
        AiWorkTaskCandidate::decl(&config),
        AiWorkTaskCandidatesResult::decl(&config),
        AiTurnClassification::decl(&config),
        AiTurnFinalizeParams::decl(&config),
        AiTurnFinalizeResult::decl(&config),
        ProjectListParams::decl(&config),
        ProjectSummary::decl(&config),
        ProjectListResult::decl(&config),
        TaskAssigneeType::decl(&config),
        WorkPriority::decl(&config),
        ProjectDeleteParams::decl(&config),
        ProjectDeleteResult::decl(&config),
        WorkspaceDeleteParams::decl(&config),
        WorkspaceDeleteResult::decl(&config),
        ProjectDetachWorkspaceParams::decl(&config),
        ProjectDetachWorkspaceResult::decl(&config),
        ProjectAttachWorkspaceParams::decl(&config),
        ProjectAttachWorkspaceResult::decl(&config),
        ProjectCreateParams::decl(&config),
        ProjectCreateResult::decl(&config),
        ProjectUpdateParams::decl(&config),
        ProjectUpdateResult::decl(&config),
        ProjectGetParams::decl(&config),
        ProjectGetResult::decl(&config),
        ProjectPortfolioListParams::decl(&config),
        ProjectPortfolioPhaseItem::decl(&config),
        ProjectPortfolioItem::decl(&config),
        ProjectPortfolioListResult::decl(&config),
        DeliverableListParams::decl(&config),
        DeliverableFileBindingItem::decl(&config),
        DeliverableCarrierItem::decl(&config),
        DeliverableCarrierInputItem::decl(&config),
        DeliverableVersionItem::decl(&config),
        DeliverableItem::decl(&config),
        DeliverableListResult::decl(&config),
        DeliverableCreateParams::decl(&config),
        DeliverableCreateResult::decl(&config),
        DeliverableVersionCreateParams::decl(&config),
        DeliverableVersionCreateResult::decl(&config),
        DeliverableVersionReviewParams::decl(&config),
        DeliverableVersionReviewResult::decl(&config),
        MyWorkGetParams::decl(&config),
        MyWorkItem::decl(&config),
        MyWorkGetResult::decl(&config),
        GlobalSearchParams::decl(&config),
        GlobalSearchItem::decl(&config),
        GlobalSearchResult::decl(&config),
        UnifiedSearchCursor::decl(&config),
        UnifiedSearchParams::decl(&config),
        UnifiedSearchLocalTarget::decl(&config),
        UnifiedSearchDeepLink::decl(&config),
        UnifiedSearchItem::decl(&config),
        UnifiedSearchGroup::decl(&config),
        UnifiedSearchPartialFailure::decl(&config),
        UnifiedSearchResult::decl(&config),
        AttentionCaseItem::decl(&config),
        AttentionCaseListParams::decl(&config),
        AttentionCaseListResult::decl(&config),
        ProjectBlockerCreateParams::decl(&config),
        ProjectBlockerCreateResult::decl(&config),
        DecisionRequestCreateParams::decl(&config),
        DecisionRequestCreateResult::decl(&config),
        AgentIncidentReportParams::decl(&config),
        AgentIncidentReportResult::decl(&config),
        AttentionCaseTransitionParams::decl(&config),
        AttentionCaseTransitionResult::decl(&config),
        ProjectProgressGetParams::decl(&config),
        BusinessTaskItem::decl(&config),
        ProjectPhaseItem::decl(&config),
        PhasePlanItem::decl(&config),
        ProjectProgressGetResult::decl(&config),
        PhaseDraftItem::decl(&config),
        PhasePlanCreateParams::decl(&config),
        PhasePlanCreateResult::decl(&config),
        PhasePlanActivateParams::decl(&config),
        PhasePlanActivateResult::decl(&config),
        ProjectPhaseTransitionParams::decl(&config),
        ProjectPhaseTransitionResult::decl(&config),
        BusinessTaskCreateParams::decl(&config),
        BusinessTaskCreateResult::decl(&config),
        BusinessTaskGetParams::decl(&config),
        BusinessTaskContributionItem::decl(&config),
        BusinessTaskGetResult::decl(&config),
        BusinessTaskTransitionParams::decl(&config),
        BusinessTaskTransitionResult::decl(&config),
        BusinessTaskMoveParams::decl(&config),
        BusinessTaskMoveResult::decl(&config),
        EventReplayParams::decl(&config),
        DurableEventItem::decl(&config),
        EventReplayResult::decl(&config),
        ConflictItem::decl(&config),
    ]
    .map(|declaration| {
        format!(
            "export {}",
            declaration
                .lines()
                .map(str::trim_end)
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    format!(
        "// Generated by contexthub-protocol. Do not edit.\n\nexport type JsonValue = null | boolean | number | string | JsonValue[] | {{ [key: string]: JsonValue }};\n{}\nexport const conversationSessionKeys = {} as const;\nexport type JsonRpcRequest<TParams> = {{ jsonrpc: \"2.0\"; id: RequestId; method: string; params?: TParams }};\nexport type JsonRpcNotification<TParams> = {{ jsonrpc: \"2.0\"; method: string; params?: TParams }};\nexport type JsonRpcSuccess<TResult> = {{ jsonrpc: \"2.0\"; id: RequestId; result: TResult }};\nexport type JsonRpcError = {{ jsonrpc: \"2.0\"; id: RequestId | null; error: {{ code: number; message: string; data?: unknown }} }};\n",
        declarations.join("\n"),
        serde_json::to_string(contexthub_protocol::CONVERSATION_SESSION_KEYS)
            .expect("会话字段可序列化")
    )
}

#[allow(dead_code)]
fn assert_schema_source<T: JsonSchema>() {}

#[cfg(test)]
mod tests {
    use super::artifacts;
    use std::collections::BTreeSet;
    use std::path::Path;

    #[test]
    fn personal_method_catalog_keeps_local_work_without_advertising_removed_services() {
        let generated = artifacts(Path::new("repository")).unwrap();
        let methods = generated
            .iter()
            .find(|artifact| artifact.path.ends_with("methods.json"))
            .unwrap();
        let catalog: serde_json::Value = serde_json::from_str(&methods.content).unwrap();
        let names: BTreeSet<_> = catalog["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| method["name"].as_str().unwrap())
            .collect();
        for prefix in [
            "cloudWorkspace/",
            "syncPlan/",
            "sync/",
            "syncExecution/",
            "conflict/",
            "applicationCatalog/",
            "aiBrowserPolicy/",
        ] {
            assert!(!names.iter().any(|name| name.starts_with(prefix)));
        }
        for method in [
            "workspace/create",
            "file/confirm",
            "preview/open",
            "personalApplicationData/prepare",
            "personalData/restoreAbortBeforeHelper",
            "mcpConnection/list",
        ] {
            assert!(names.contains(method), "{method}");
        }
        let declarations = generated
            .iter()
            .find(|artifact| artifact.path.extension().is_some_and(|value| value == "ts"))
            .unwrap();
        for shared_type in [
            "ConflictItem",
            "ApplicationSkillBinding",
            "PersonalApplicationDataOperation",
            "PersonalNamespaceStorageOperation",
            "PersonalPageBindingSnapshot",
            "PersonalRestorePreview",
            "ModelNetworkConfig",
        ] {
            assert!(
                declarations
                    .content
                    .contains(&format!("export type {shared_type} ="))
            );
        }
    }

    #[test]
    fn removed_enterprise_directory_contracts_are_not_generated() {
        let generated = artifacts(Path::new("repository")).unwrap();
        let declarations = generated
            .iter()
            .find(|item| item.path.extension().is_some_and(|value| value == "ts"))
            .unwrap();
        for name in [
            "ApplicationDelivery",
            "ApplicationTool",
            "ApplicationCatalogItem",
            "ApplicationCatalogListResult",
            "ApplicationLaunchResult",
            "AiBrowserPolicyGetParams",
            "AiBrowserPolicyGetResult",
        ] {
            assert!(
                !declarations
                    .content
                    .contains(&format!("export type {name} ="))
            );
        }
        for name in [
            "ai-browser-policy-get-params.schema.json",
            "ai-browser-policy-get-result.schema.json",
        ] {
            assert!(
                !generated
                    .iter()
                    .any(|artifact| artifact.path.ends_with(name))
            );
        }
    }

    #[test]
    fn checked_in_schema_directory_contains_exactly_the_current_contract_artifacts() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap();
        let schema_dir = root.join("schemas/jsonrpc");
        let expected: BTreeSet<_> = artifacts(root)
            .unwrap()
            .into_iter()
            .map(|artifact| artifact.path)
            .filter(|path| path.parent() == Some(schema_dir.as_path()))
            .collect();
        let actual: BTreeSet<_> = std::fs::read_dir(&schema_dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(actual, expected);
    }
}
