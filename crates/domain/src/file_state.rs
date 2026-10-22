use crate::WorkspaceKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformDocumentState {
    NotApplicable,
    Unknown,
    Published,
    Draft,
    InReview,
    Rejected,
    Archived,
    Deleted,
    Inaccessible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializationState {
    NotMaterialized,
    Materializing,
    Materialized,
    Missing,
    Unsupported,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalScanState {
    Pending,
    Scanning,
    Current,
    Unreadable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaselineRelation {
    Unknown,
    Current,
    Behind,
    PlatformAbsent,
    PlatformArchived,
    Inaccessible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalChangeState {
    Clean,
    Added,
    Modified,
    DeletedCandidate,
    RenamedCandidate,
    Unreadable,
    Scanning,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfirmationState {
    NotRequired,
    Pending,
    Confirmed,
    InvalidatedContentChanged,
    InvalidatedBaselineChanged,
    BlockedByConflict,
    BlockedByPermission,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmissionState {
    None,
    Planning,
    Ready,
    Submitting,
    Submitted,
    PendingReview,
    Published,
    Rejected,
    ChangesRequested,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictState {
    None,
    UpdateAvailable,
    ContentDiverged,
    DeleteModify,
    RenameConflict,
    PathCollision,
    CaseCollision,
    PermissionConflict,
    MetadataConflict,
    Unresolved,
}

impl ConflictState {
    #[must_use]
    pub const fn blocks_mutation(self) -> bool {
        !matches!(self, Self::None | Self::UpdateAvailable)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileDisplayState {
    LocalReady,
    MetadataError,
    PermissionRevoked,
    SyncConflict,
    OutcomeUnknown,
    ReviewReturned,
    PendingReview,
    ConfirmationRequired,
    ConfirmedReady,
    PlatformUpdate,
    LocalMissing,
    LocalModified,
    LocalAdded,
    Synced,
    NotMaterialized,
    Scanning,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileOperation {
    Preview,
    Pull,
    Confirm,
    DiscardLocalChange,
    PrepareSubmission,
    Submit,
    Review,
    ResolveConflict,
    OpenExternalTool,
}

impl FileOperation {
    #[must_use]
    pub const fn bit(self) -> u16 {
        1 << self as u16
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Pull => "pull",
            Self::Confirm => "confirm",
            Self::DiscardLocalChange => "discard_local_change",
            Self::PrepareSubmission => "prepare_submission",
            Self::Submit => "submit",
            Self::Review => "review",
            Self::ResolveConflict => "resolve_conflict",
            Self::OpenExternalTool => "open_external_tool",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityReason {
    FileNotMaterialized,
    FileUnreadable,
    ScanInProgress,
    PlatformUnavailable,
    PlatformHasNoUpdate,
    ConfirmationNotRequired,
    ConfirmationRequired,
    ConflictUnresolved,
    PermissionRevoked,
    SubmissionPlanRequired,
    ReviewNotPending,
    NoLocalChange,
    NoConflict,
}

impl CapabilityReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FileNotMaterialized => "file_not_materialized",
            Self::FileUnreadable => "file_unreadable",
            Self::ScanInProgress => "scan_in_progress",
            Self::PlatformUnavailable => "platform_unavailable",
            Self::PlatformHasNoUpdate => "platform_has_no_update",
            Self::ConfirmationNotRequired => "confirmation_not_required",
            Self::ConfirmationRequired => "confirmation_required",
            Self::ConflictUnresolved => "conflict_unresolved",
            Self::PermissionRevoked => "permission_revoked",
            Self::SubmissionPlanRequired => "submission_plan_required",
            Self::ReviewNotPending => "review_not_pending",
            Self::NoLocalChange => "no_local_change",
            Self::NoConflict => "no_conflict",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityBlock {
    pub operation: FileOperation,
    pub reason: CapabilityReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationCapabilities {
    pub bits: u16,
    pub blocks: Vec<CapabilityBlock>,
}

impl OperationCapabilities {
    #[must_use]
    pub const fn allows(&self, operation: FileOperation) -> bool {
        self.bits & operation.bit() != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveConfirmationFact {
    pub checksum_algorithm: String,
    pub checksum_value: String,
    pub baseline_version_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformConnection {
    LocalOnly,
    Bound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalNodeKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BaselinePathRelation {
    Unchanged,
    Changed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataValidity {
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilePermission {
    Access,
    Confirm,
    Submit,
    Review,
}

impl FilePermission {
    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilePermissions(u8);

impl FilePermissions {
    #[must_use]
    pub fn from_allowed(permissions: impl IntoIterator<Item = FilePermission>) -> Self {
        permissions
            .into_iter()
            .fold(Self(0), |set, permission| Self(set.0 | permission.bit()))
    }

    #[must_use]
    pub const fn allows(self, permission: FilePermission) -> bool {
        self.0 & permission.bit() != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileStateInput {
    pub workspace_kind: WorkspaceKind,
    pub platform_connection: PlatformConnection,
    pub platform_state: PlatformDocumentState,
    pub materialization_state: MaterializationState,
    pub scan_state: LocalScanState,
    pub node_kind: LocalNodeKind,
    pub baseline_path_relation: BaselinePathRelation,
    pub baseline_version_id: Option<String>,
    pub baseline_checksum_algorithm: Option<String>,
    pub baseline_checksum_value: Option<String>,
    pub latest_published_version_id: Option<String>,
    pub latest_published_checksum_algorithm: Option<String>,
    pub latest_published_checksum_value: Option<String>,
    pub local_checksum_algorithm: Option<String>,
    pub local_checksum_value: Option<String>,
    pub active_confirmation: Option<ActiveConfirmationFact>,
    pub submission_state: SubmissionState,
    pub active_conflict: Option<ConflictState>,
    pub permissions: FilePermissions,
    pub metadata_validity: MetadataValidity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileStateOutput {
    pub baseline_relation: BaselineRelation,
    pub local_change_state: LocalChangeState,
    pub confirmation_state: ConfirmationState,
    pub submission_state: SubmissionState,
    pub conflict_state: ConflictState,
    pub display_state: FileDisplayState,
    pub capabilities: OperationCapabilities,
}

#[must_use]
pub fn compute_file_state(input: &FileStateInput) -> FileStateOutput {
    let baseline_relation = compute_baseline_relation(input);
    let local_change_state = compute_local_change(input);
    let conflict_state = compute_conflict(input, baseline_relation, local_change_state);
    let confirmation_state = compute_confirmation(input, local_change_state, conflict_state);
    let display_state = compute_display(
        input,
        baseline_relation,
        local_change_state,
        confirmation_state,
        conflict_state,
    );
    let capabilities = compute_capabilities(
        input,
        local_change_state,
        confirmation_state,
        conflict_state,
    );
    FileStateOutput {
        baseline_relation,
        local_change_state,
        confirmation_state,
        submission_state: input.submission_state,
        conflict_state,
        display_state,
        capabilities,
    }
}

fn compute_baseline_relation(input: &FileStateInput) -> BaselineRelation {
    if input.workspace_kind == WorkspaceKind::Local
        || input.platform_connection == PlatformConnection::LocalOnly
        || input.platform_state == PlatformDocumentState::NotApplicable
    {
        return BaselineRelation::PlatformAbsent;
    }
    match input.platform_state {
        PlatformDocumentState::Inaccessible => return BaselineRelation::Inaccessible,
        PlatformDocumentState::Archived => return BaselineRelation::PlatformArchived,
        _ => {}
    }
    let Some(latest) = input.latest_published_version_id.as_deref() else {
        return BaselineRelation::PlatformAbsent;
    };
    match input.baseline_version_id.as_deref() {
        Some(baseline) if baseline == latest => BaselineRelation::Current,
        Some(_) => BaselineRelation::Behind,
        None => BaselineRelation::Unknown,
    }
}

fn compute_local_change(input: &FileStateInput) -> LocalChangeState {
    if matches!(
        input.scan_state,
        LocalScanState::Pending | LocalScanState::Scanning
    ) || input.materialization_state == MaterializationState::Materializing
    {
        return LocalChangeState::Scanning;
    }
    if input.scan_state == LocalScanState::Unreadable
        || matches!(
            input.materialization_state,
            MaterializationState::Unsupported | MaterializationState::Invalid
        )
    {
        return LocalChangeState::Unreadable;
    }
    if input.materialization_state == MaterializationState::Missing {
        return if input.baseline_version_id.is_some() {
            LocalChangeState::DeletedCandidate
        } else {
            LocalChangeState::Added
        };
    }
    if input.materialization_state == MaterializationState::NotMaterialized {
        return LocalChangeState::Clean;
    }
    if input.baseline_path_relation == BaselinePathRelation::Changed
        && input.baseline_version_id.is_some()
    {
        return LocalChangeState::RenamedCandidate;
    }
    if input.baseline_version_id.is_none() {
        return LocalChangeState::Added;
    }
    if input.node_kind == LocalNodeKind::Directory {
        return LocalChangeState::Clean;
    }
    match (
        input.baseline_checksum_algorithm.as_deref(),
        input.baseline_checksum_value.as_deref(),
        input.local_checksum_algorithm.as_deref(),
        input.local_checksum_value.as_deref(),
    ) {
        (Some(baseline_algorithm), Some(baseline), Some(local_algorithm), Some(local))
            if baseline_algorithm == local_algorithm && baseline == local =>
        {
            LocalChangeState::Clean
        }
        (Some(_), Some(_), Some(_), Some(_)) => LocalChangeState::Modified,
        _ => LocalChangeState::Unreadable,
    }
}

fn compute_conflict(
    input: &FileStateInput,
    baseline_relation: BaselineRelation,
    local_change_state: LocalChangeState,
) -> ConflictState {
    if input.metadata_validity == MetadataValidity::Invalid || metadata_is_inconsistent(input) {
        return ConflictState::MetadataConflict;
    }
    if input.workspace_kind == WorkspaceKind::Local {
        return ConflictState::None;
    }
    if let Some(active) = input.active_conflict
        && active.blocks_mutation()
    {
        return active;
    }
    let locally_changed = matches!(
        local_change_state,
        LocalChangeState::Added
            | LocalChangeState::Modified
            | LocalChangeState::DeletedCandidate
            | LocalChangeState::RenamedCandidate
    );
    if input.platform_connection == PlatformConnection::Bound
        && locally_changed
        && (!input.permissions.allows(FilePermission::Access)
            || !input.permissions.allows(FilePermission::Submit))
    {
        return ConflictState::PermissionConflict;
    }
    if matches!(
        input.platform_state,
        PlatformDocumentState::Deleted | PlatformDocumentState::Archived
    ) {
        return match local_change_state {
            LocalChangeState::Added
            | LocalChangeState::Modified
            | LocalChangeState::RenamedCandidate => ConflictState::DeleteModify,
            LocalChangeState::Clean => ConflictState::UpdateAvailable,
            LocalChangeState::DeletedCandidate
            | LocalChangeState::Unreadable
            | LocalChangeState::Scanning => ConflictState::None,
        };
    }
    if baseline_relation == BaselineRelation::Behind {
        return match local_change_state {
            LocalChangeState::Clean => ConflictState::UpdateAvailable,
            LocalChangeState::DeletedCandidate => ConflictState::DeleteModify,
            LocalChangeState::RenamedCandidate => ConflictState::RenameConflict,
            LocalChangeState::Modified | LocalChangeState::Added => ConflictState::ContentDiverged,
            _ => ConflictState::None,
        };
    }
    ConflictState::None
}

fn metadata_is_inconsistent(input: &FileStateInput) -> bool {
    let incomplete_pair =
        |algorithm: &Option<String>, value: &Option<String>| algorithm.is_some() != value.is_some();
    if incomplete_pair(
        &input.baseline_checksum_algorithm,
        &input.baseline_checksum_value,
    ) || incomplete_pair(
        &input.latest_published_checksum_algorithm,
        &input.latest_published_checksum_value,
    ) || incomplete_pair(&input.local_checksum_algorithm, &input.local_checksum_value)
    {
        return true;
    }
    input.baseline_version_id.is_some()
        && input.baseline_version_id == input.latest_published_version_id
        && (input.baseline_checksum_algorithm != input.latest_published_checksum_algorithm
            || input.baseline_checksum_value != input.latest_published_checksum_value)
}

fn compute_confirmation(
    input: &FileStateInput,
    local_change_state: LocalChangeState,
    conflict_state: ConflictState,
) -> ConfirmationState {
    if input.workspace_kind == WorkspaceKind::Local {
        return ConfirmationState::NotRequired;
    }
    if conflict_state.blocks_mutation() {
        return ConfirmationState::BlockedByConflict;
    }
    if !input.permissions.allows(FilePermission::Confirm) {
        return ConfirmationState::BlockedByPermission;
    }
    if local_change_state == LocalChangeState::Clean {
        return ConfirmationState::NotRequired;
    }
    if matches!(
        local_change_state,
        LocalChangeState::Unreadable | LocalChangeState::Scanning
    ) {
        return ConfirmationState::Pending;
    }
    let Some(confirmation) = input.active_confirmation.as_ref() else {
        return ConfirmationState::Pending;
    };
    if confirmation.checksum_algorithm != input.local_checksum_algorithm.as_deref().unwrap_or("")
        || confirmation.checksum_value != input.local_checksum_value.as_deref().unwrap_or("")
    {
        return ConfirmationState::InvalidatedContentChanged;
    }
    if confirmation.baseline_version_id != input.baseline_version_id {
        return ConfirmationState::InvalidatedBaselineChanged;
    }
    ConfirmationState::Confirmed
}

fn compute_display(
    input: &FileStateInput,
    baseline_relation: BaselineRelation,
    local_change_state: LocalChangeState,
    confirmation_state: ConfirmationState,
    conflict_state: ConflictState,
) -> FileDisplayState {
    if input.workspace_kind == WorkspaceKind::Local {
        if conflict_state == ConflictState::MetadataConflict {
            return FileDisplayState::MetadataError;
        }
        if input.materialization_state == MaterializationState::Missing {
            return FileDisplayState::LocalMissing;
        }
        if input.materialization_state == MaterializationState::NotMaterialized {
            return FileDisplayState::NotMaterialized;
        }
        return match local_change_state {
            LocalChangeState::Scanning => FileDisplayState::Scanning,
            LocalChangeState::Unreadable => FileDisplayState::Unavailable,
            _ => FileDisplayState::LocalReady,
        };
    }
    if conflict_state == ConflictState::MetadataConflict {
        return FileDisplayState::MetadataError;
    }
    if conflict_state == ConflictState::PermissionConflict
        || baseline_relation == BaselineRelation::Inaccessible
    {
        return FileDisplayState::PermissionRevoked;
    }
    if conflict_state.blocks_mutation() {
        return FileDisplayState::SyncConflict;
    }
    match input.submission_state {
        SubmissionState::OutcomeUnknown => return FileDisplayState::OutcomeUnknown,
        SubmissionState::Rejected | SubmissionState::ChangesRequested => {
            return FileDisplayState::ReviewReturned;
        }
        SubmissionState::PendingReview => return FileDisplayState::PendingReview,
        _ => {}
    }
    if matches!(
        confirmation_state,
        ConfirmationState::Pending
            | ConfirmationState::InvalidatedContentChanged
            | ConfirmationState::InvalidatedBaselineChanged
    ) {
        return FileDisplayState::ConfirmationRequired;
    }
    if confirmation_state == ConfirmationState::Confirmed {
        return FileDisplayState::ConfirmedReady;
    }
    if conflict_state == ConflictState::UpdateAvailable {
        return FileDisplayState::PlatformUpdate;
    }
    match local_change_state {
        LocalChangeState::DeletedCandidate => FileDisplayState::LocalMissing,
        LocalChangeState::Modified | LocalChangeState::RenamedCandidate => {
            FileDisplayState::LocalModified
        }
        LocalChangeState::Added => FileDisplayState::LocalAdded,
        LocalChangeState::Scanning => FileDisplayState::Scanning,
        LocalChangeState::Unreadable => FileDisplayState::Unavailable,
        LocalChangeState::Clean
            if input.materialization_state == MaterializationState::NotMaterialized =>
        {
            FileDisplayState::NotMaterialized
        }
        LocalChangeState::Clean => FileDisplayState::Synced,
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "all nine operation decisions remain adjacent so their bit and disabled-reason outputs cannot drift"
)]
fn compute_capabilities(
    input: &FileStateInput,
    local_change_state: LocalChangeState,
    confirmation_state: ConfirmationState,
    conflict_state: ConflictState,
) -> OperationCapabilities {
    let mut capabilities = OperationCapabilities {
        bits: 0,
        blocks: Vec::new(),
    };
    let readable = input.materialization_state == MaterializationState::Materialized
        && !matches!(
            local_change_state,
            LocalChangeState::Unreadable | LocalChangeState::Scanning
        );
    set_capability(
        &mut capabilities,
        FileOperation::Preview,
        readable,
        local_file_reason(input, local_change_state),
    );
    set_capability(
        &mut capabilities,
        FileOperation::OpenExternalTool,
        readable,
        local_file_reason(input, local_change_state),
    );
    set_capability(
        &mut capabilities,
        FileOperation::Pull,
        input.workspace_kind == WorkspaceKind::CloudTeam
            && conflict_state == ConflictState::UpdateAvailable
            && input.permissions.allows(FilePermission::Access),
        if !input.permissions.allows(FilePermission::Access) {
            CapabilityReason::PermissionRevoked
        } else if input.platform_connection == PlatformConnection::LocalOnly {
            CapabilityReason::PlatformUnavailable
        } else {
            CapabilityReason::PlatformHasNoUpdate
        },
    );
    let confirmable_state = matches!(
        confirmation_state,
        ConfirmationState::Pending
            | ConfirmationState::InvalidatedContentChanged
            | ConfirmationState::InvalidatedBaselineChanged
    );
    set_capability(
        &mut capabilities,
        FileOperation::Confirm,
        confirmable_state
            && input.permissions.allows(FilePermission::Confirm)
            && !conflict_state.blocks_mutation()
            && readable,
        if conflict_state.blocks_mutation() {
            CapabilityReason::ConflictUnresolved
        } else if !input.permissions.allows(FilePermission::Confirm) {
            CapabilityReason::PermissionRevoked
        } else if !readable {
            local_file_reason(input, local_change_state)
        } else {
            CapabilityReason::ConfirmationNotRequired
        },
    );
    let has_local_change = matches!(
        local_change_state,
        LocalChangeState::Added | LocalChangeState::Modified | LocalChangeState::RenamedCandidate
    );
    set_capability(
        &mut capabilities,
        FileOperation::DiscardLocalChange,
        has_local_change && readable && !conflict_state.blocks_mutation(),
        if conflict_state.blocks_mutation() {
            CapabilityReason::ConflictUnresolved
        } else if !readable {
            local_file_reason(input, local_change_state)
        } else {
            CapabilityReason::NoLocalChange
        },
    );
    let may_prepare = input.workspace_kind == WorkspaceKind::CloudTeam
        && confirmation_state == ConfirmationState::Confirmed
        && input.platform_connection == PlatformConnection::Bound
        && input.permissions.allows(FilePermission::Submit)
        && !conflict_state.blocks_mutation();
    set_capability(
        &mut capabilities,
        FileOperation::PrepareSubmission,
        may_prepare,
        mutation_reason(input, confirmation_state, conflict_state),
    );
    set_capability(
        &mut capabilities,
        FileOperation::Submit,
        may_prepare && input.submission_state == SubmissionState::Ready,
        if may_prepare {
            CapabilityReason::SubmissionPlanRequired
        } else {
            mutation_reason(input, confirmation_state, conflict_state)
        },
    );
    set_capability(
        &mut capabilities,
        FileOperation::Review,
        input.workspace_kind == WorkspaceKind::CloudTeam
            && input.platform_state == PlatformDocumentState::InReview
            && input.permissions.allows(FilePermission::Review),
        if input.permissions.allows(FilePermission::Review) {
            CapabilityReason::ReviewNotPending
        } else {
            CapabilityReason::PermissionRevoked
        },
    );
    set_capability(
        &mut capabilities,
        FileOperation::ResolveConflict,
        input.workspace_kind == WorkspaceKind::CloudTeam
            && conflict_state.blocks_mutation()
            && input.permissions.allows(FilePermission::Access),
        if input.permissions.allows(FilePermission::Access) {
            CapabilityReason::NoConflict
        } else {
            CapabilityReason::PermissionRevoked
        },
    );
    capabilities
}

fn set_capability(
    capabilities: &mut OperationCapabilities,
    operation: FileOperation,
    allowed: bool,
    reason: CapabilityReason,
) {
    if allowed {
        capabilities.bits |= operation.bit();
    } else {
        capabilities
            .blocks
            .push(CapabilityBlock { operation, reason });
    }
}

fn local_file_reason(
    input: &FileStateInput,
    local_change_state: LocalChangeState,
) -> CapabilityReason {
    if local_change_state == LocalChangeState::Scanning {
        CapabilityReason::ScanInProgress
    } else if local_change_state == LocalChangeState::Unreadable {
        CapabilityReason::FileUnreadable
    } else if input.materialization_state != MaterializationState::Materialized {
        CapabilityReason::FileNotMaterialized
    } else {
        CapabilityReason::NoLocalChange
    }
}

fn mutation_reason(
    input: &FileStateInput,
    confirmation_state: ConfirmationState,
    conflict_state: ConflictState,
) -> CapabilityReason {
    if conflict_state.blocks_mutation() {
        CapabilityReason::ConflictUnresolved
    } else if input.workspace_kind == WorkspaceKind::Local
        || input.platform_connection == PlatformConnection::LocalOnly
    {
        CapabilityReason::PlatformUnavailable
    } else if !input.permissions.allows(FilePermission::Submit) {
        CapabilityReason::PermissionRevoked
    } else if confirmation_state != ConfirmationState::Confirmed {
        CapabilityReason::ConfirmationRequired
    } else {
        CapabilityReason::SubmissionPlanRequired
    }
}

macro_rules! string_value {
    ($type:ty, {$($variant:ident => $value:literal),+ $(,)?}) => {
        impl $type {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }
    };
}

string_value!(BaselineRelation, {
    Unknown => "unknown", Current => "current", Behind => "behind",
    PlatformAbsent => "platform_absent", PlatformArchived => "platform_archived",
    Inaccessible => "inaccessible"
});
string_value!(LocalChangeState, {
    Clean => "clean", Added => "added", Modified => "modified",
    DeletedCandidate => "deleted_candidate", RenamedCandidate => "renamed_candidate",
    Unreadable => "unreadable", Scanning => "scanning"
});
string_value!(ConfirmationState, {
    NotRequired => "not_required", Pending => "pending", Confirmed => "confirmed",
    InvalidatedContentChanged => "invalidated_content_changed",
    InvalidatedBaselineChanged => "invalidated_baseline_changed",
    BlockedByConflict => "blocked_by_conflict", BlockedByPermission => "blocked_by_permission"
});
string_value!(SubmissionState, {
    None => "none", Planning => "planning", Ready => "ready", Submitting => "submitting",
    Submitted => "submitted", PendingReview => "pending_review", Published => "published",
    Rejected => "rejected", ChangesRequested => "changes_requested", Failed => "failed",
    OutcomeUnknown => "outcome_unknown"
});
string_value!(ConflictState, {
    None => "none", UpdateAvailable => "update_available", ContentDiverged => "content_diverged",
    DeleteModify => "delete_modify", RenameConflict => "rename_conflict",
    PathCollision => "path_collision", CaseCollision => "case_collision",
    PermissionConflict => "permission_conflict", MetadataConflict => "metadata_conflict",
    Unresolved => "unresolved"
});
string_value!(FileDisplayState, {
    LocalReady => "local_ready",
    MetadataError => "metadata_error", PermissionRevoked => "permission_revoked",
    SyncConflict => "sync_conflict", OutcomeUnknown => "outcome_unknown",
    ReviewReturned => "review_returned", PendingReview => "pending_review",
    ConfirmationRequired => "confirmation_required", ConfirmedReady => "confirmed_ready",
    PlatformUpdate => "platform_update", LocalMissing => "local_missing",
    LocalModified => "local_modified", LocalAdded => "local_added", Synced => "synced",
    NotMaterialized => "not_materialized", Scanning => "scanning", Unavailable => "unavailable"
});

#[cfg(test)]
mod tests {
    use super::*;

    fn published_input() -> FileStateInput {
        FileStateInput {
            workspace_kind: WorkspaceKind::CloudTeam,
            platform_connection: PlatformConnection::Bound,
            platform_state: PlatformDocumentState::Published,
            materialization_state: MaterializationState::Materialized,
            scan_state: LocalScanState::Current,
            node_kind: LocalNodeKind::File,
            baseline_path_relation: BaselinePathRelation::Unchanged,
            baseline_version_id: Some("v1".to_owned()),
            baseline_checksum_algorithm: Some("sha256".to_owned()),
            baseline_checksum_value: Some("one".to_owned()),
            latest_published_version_id: Some("v1".to_owned()),
            latest_published_checksum_algorithm: Some("sha256".to_owned()),
            latest_published_checksum_value: Some("one".to_owned()),
            local_checksum_algorithm: Some("sha256".to_owned()),
            local_checksum_value: Some("one".to_owned()),
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

    #[test]
    fn clean_current_file_is_synced_without_confirmation() {
        let state = compute_file_state(&published_input());
        assert_eq!(state.baseline_relation, BaselineRelation::Current);
        assert_eq!(state.local_change_state, LocalChangeState::Clean);
        assert_eq!(state.confirmation_state, ConfirmationState::NotRequired);
        assert_eq!(state.conflict_state, ConflictState::None);
        assert_eq!(state.display_state, FileDisplayState::Synced);
        assert!(state.capabilities.allows(FileOperation::Preview));
        assert!(!state.capabilities.allows(FileOperation::Confirm));
    }

    #[test]
    fn local_change_requires_confirmation_and_matching_evidence_enables_submission() {
        let mut input = published_input();
        input.local_checksum_value = Some("two".to_owned());
        let pending = compute_file_state(&input);
        assert_eq!(pending.local_change_state, LocalChangeState::Modified);
        assert_eq!(pending.confirmation_state, ConfirmationState::Pending);
        assert!(pending.capabilities.allows(FileOperation::Confirm));
        input.active_confirmation = Some(ActiveConfirmationFact {
            checksum_algorithm: "sha256".to_owned(),
            checksum_value: "two".to_owned(),
            baseline_version_id: Some("v1".to_owned()),
        });
        let confirmed = compute_file_state(&input);
        assert_eq!(confirmed.confirmation_state, ConfirmationState::Confirmed);
        assert!(
            confirmed
                .capabilities
                .allows(FileOperation::PrepareSubmission)
        );
    }

    #[test]
    fn platform_update_distinguishes_safe_pull_from_divergence() {
        let mut input = published_input();
        input.latest_published_version_id = Some("v2".to_owned());
        input.latest_published_checksum_value = Some("platform-two".to_owned());
        let safe = compute_file_state(&input);
        assert_eq!(safe.baseline_relation, BaselineRelation::Behind);
        assert_eq!(safe.conflict_state, ConflictState::UpdateAvailable);
        assert!(safe.capabilities.allows(FileOperation::Pull));
        input.local_checksum_value = Some("local-two".to_owned());
        let diverged = compute_file_state(&input);
        assert_eq!(diverged.conflict_state, ConflictState::ContentDiverged);
        assert_eq!(
            diverged.confirmation_state,
            ConfirmationState::BlockedByConflict
        );
        assert!(!diverged.capabilities.allows(FileOperation::Pull));
        assert!(diverged.capabilities.allows(FileOperation::ResolveConflict));
    }

    #[test]
    fn confirmation_invalidates_for_content_then_baseline_changes() {
        let mut input = published_input();
        input.local_checksum_value = Some("two".to_owned());
        input.active_confirmation = Some(ActiveConfirmationFact {
            checksum_algorithm: "sha256".to_owned(),
            checksum_value: "one".to_owned(),
            baseline_version_id: Some("v1".to_owned()),
        });
        assert_eq!(
            compute_file_state(&input).confirmation_state,
            ConfirmationState::InvalidatedContentChanged
        );
        input.active_confirmation.as_mut().unwrap().checksum_value = "two".to_owned();
        input
            .active_confirmation
            .as_mut()
            .unwrap()
            .baseline_version_id = Some("v0".to_owned());
        assert_eq!(
            compute_file_state(&input).confirmation_state,
            ConfirmationState::InvalidatedBaselineChanged
        );
    }

    #[test]
    fn metadata_and_permission_conflicts_take_display_priority() {
        let mut input = published_input();
        input.local_checksum_value = Some("two".to_owned());
        input.metadata_validity = MetadataValidity::Invalid;
        assert_eq!(
            compute_file_state(&input).display_state,
            FileDisplayState::MetadataError
        );
        input.metadata_validity = MetadataValidity::Valid;
        input.permissions =
            FilePermissions::from_allowed([FilePermission::Access, FilePermission::Confirm]);
        let permission = compute_file_state(&input);
        assert_eq!(permission.conflict_state, ConflictState::PermissionConflict);
        assert_eq!(
            permission.display_state,
            FileDisplayState::PermissionRevoked
        );
    }

    #[test]
    fn same_platform_version_with_different_checksum_is_metadata_conflict() {
        let mut input = published_input();
        input.latest_published_checksum_value = Some("impossible-other-checksum".to_owned());
        let state = compute_file_state(&input);
        assert_eq!(state.conflict_state, ConflictState::MetadataConflict);
        assert_eq!(state.display_state, FileDisplayState::MetadataError);
        assert!(!state.capabilities.allows(FileOperation::Confirm));
    }

    #[test]
    fn local_only_added_and_platform_backed_missing_are_distinct() {
        let mut local = published_input();
        local.workspace_kind = WorkspaceKind::Local;
        local.platform_connection = PlatformConnection::LocalOnly;
        local.platform_state = PlatformDocumentState::NotApplicable;
        local.baseline_version_id = None;
        local.baseline_checksum_algorithm = None;
        local.baseline_checksum_value = None;
        local.latest_published_version_id = None;
        local.latest_published_checksum_algorithm = None;
        local.latest_published_checksum_value = None;
        local.permissions =
            FilePermissions::from_allowed([FilePermission::Access, FilePermission::Confirm]);
        let added = compute_file_state(&local);
        assert_eq!(added.baseline_relation, BaselineRelation::PlatformAbsent);
        assert_eq!(added.local_change_state, LocalChangeState::Added);
        assert_eq!(added.confirmation_state, ConfirmationState::NotRequired);
        assert_eq!(added.display_state, FileDisplayState::LocalReady);
        assert!(!added.capabilities.allows(FileOperation::Confirm));
        assert_eq!(added.conflict_state, ConflictState::None);
        assert!(!added.capabilities.allows(FileOperation::PrepareSubmission));

        let mut missing = published_input();
        missing.materialization_state = MaterializationState::Missing;
        let deleted = compute_file_state(&missing);
        assert_eq!(
            deleted.local_change_state,
            LocalChangeState::DeletedCandidate
        );
        assert_eq!(
            deleted.display_state,
            FileDisplayState::ConfirmationRequired
        );
    }

    #[test]
    fn platform_deletion_conflicts_only_with_unsaved_local_content() {
        let mut input = published_input();
        input.platform_state = PlatformDocumentState::Deleted;
        let safe = compute_file_state(&input);
        assert_eq!(safe.conflict_state, ConflictState::UpdateAvailable);
        assert!(safe.capabilities.allows(FileOperation::Pull));

        input.local_checksum_value = Some("local-change".to_owned());
        let conflict = compute_file_state(&input);
        assert_eq!(conflict.conflict_state, ConflictState::DeleteModify);

        input.materialization_state = MaterializationState::Missing;
        let both_deleted = compute_file_state(&input);
        assert_eq!(
            both_deleted.local_change_state,
            LocalChangeState::DeletedCandidate
        );
        assert_eq!(both_deleted.conflict_state, ConflictState::None);
    }

    #[test]
    fn submission_and_review_display_priority_is_stable() {
        let mut input = published_input();
        input.local_checksum_value = Some("local-change".to_owned());
        input.submission_state = SubmissionState::OutcomeUnknown;
        assert_eq!(
            compute_file_state(&input).display_state,
            FileDisplayState::OutcomeUnknown
        );
        input.submission_state = SubmissionState::ChangesRequested;
        assert_eq!(
            compute_file_state(&input).display_state,
            FileDisplayState::ReviewReturned
        );
        input.submission_state = SubmissionState::PendingReview;
        assert_eq!(
            compute_file_state(&input).display_state,
            FileDisplayState::PendingReview
        );
    }
}
