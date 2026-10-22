CREATE TABLE app_metadata (
    key TEXT PRIMARY KEY CHECK (length(key) > 0),
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

CREATE TABLE accounts (
    account_id TEXT PRIMARY KEY CHECK (length(account_id) > 0),
    display_name TEXT NOT NULL,
    account_status TEXT NOT NULL CHECK (account_status IN ('active', 'disabled', 'unknown')),
    credential_ref TEXT,
    active_org_id TEXT,
    permission_revision TEXT,
    last_session_check_at INTEGER CHECK (last_session_check_at >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

CREATE TABLE workspaces (
    workspace_id TEXT PRIMARY KEY CHECK (length(workspace_id) > 0),
    account_id TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE RESTRICT,
    project_id TEXT NOT NULL CHECK (length(project_id) > 0),
    display_name TEXT NOT NULL,
    local_root TEXT NOT NULL CHECK (length(local_root) > 0),
    local_root_identity TEXT,
    platform_root TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('unbound', 'binding', 'ready', 'syncing', 'attention_required', 'conflict', 'unauthorized', 'invalid')),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    platform_snapshot_revision TEXT,
    last_platform_check_at INTEGER CHECK (last_platform_check_at >= 0),
    last_local_scan_at INTEGER CHECK (last_local_scan_at >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE(local_root)
) STRICT;

CREATE TABLE workspace_exports (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    desired_revision INTEGER NOT NULL CHECK (desired_revision >= 0),
    exported_revision INTEGER NOT NULL CHECK (exported_revision >= 0),
    manifest_checksum TEXT,
    export_state TEXT NOT NULL CHECK (export_state IN ('current', 'pending', 'failed', 'conflict')),
    last_error_reason TEXT,
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

CREATE TABLE documents (
    document_id TEXT PRIMARY KEY CHECK (length(document_id) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    parent_document_id TEXT REFERENCES documents(document_id) ON DELETE RESTRICT,
    platform_path TEXT NOT NULL,
    normalized_platform_path TEXT NOT NULL,
    node_type TEXT NOT NULL CHECK (node_type IN ('file', 'directory')),
    scope TEXT NOT NULL,
    owner_id TEXT,
    platform_state TEXT NOT NULL CHECK (platform_state IN ('unknown', 'published', 'draft', 'in_review', 'rejected', 'archived', 'deleted', 'inaccessible')),
    latest_published_version_id TEXT,
    latest_published_checksum_algorithm TEXT,
    latest_published_checksum_value TEXT,
    mime_type TEXT,
    size_bytes INTEGER CHECK (size_bytes >= 0),
    metadata_revision TEXT,
    platform_updated_at INTEGER CHECK (platform_updated_at >= 0),
    platform_checked_at INTEGER NOT NULL CHECK (platform_checked_at >= 0),
    capabilities_json TEXT NOT NULL
) STRICT;

CREATE TABLE document_versions (
    version_id TEXT PRIMARY KEY CHECK (length(version_id) > 0),
    document_id TEXT NOT NULL REFERENCES documents(document_id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('draft', 'review', 'published', 'rejected', 'archived')),
    checksum_algorithm TEXT NOT NULL,
    checksum_value TEXT NOT NULL,
    git_commit TEXT,
    size_bytes INTEGER CHECK (size_bytes >= 0),
    created_by TEXT,
    created_at INTEGER CHECK (created_at >= 0),
    published_at INTEGER CHECK (published_at >= 0),
    cached_at INTEGER NOT NULL CHECK (cached_at >= 0)
) STRICT;

CREATE TABLE local_files (
    local_file_id TEXT PRIMARY KEY CHECK (length(local_file_id) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    document_id TEXT REFERENCES documents(document_id) ON DELETE RESTRICT,
    parent_local_file_id TEXT REFERENCES local_files(local_file_id) ON DELETE RESTRICT,
    relative_path TEXT NOT NULL,
    normalized_relative_path TEXT NOT NULL,
    file_identity TEXT,
    materialization_state TEXT NOT NULL CHECK (materialization_state IN ('not_materialized', 'materializing', 'materialized', 'missing', 'unsupported', 'invalid')),
    file_type TEXT NOT NULL CHECK (file_type IN ('file', 'directory', 'symlink')),
    size_bytes INTEGER CHECK (size_bytes >= 0),
    modified_at_fs INTEGER CHECK (modified_at_fs >= 0),
    observed_at INTEGER NOT NULL CHECK (observed_at >= 0),
    local_checksum_algorithm TEXT,
    local_checksum_value TEXT,
    scan_state TEXT NOT NULL CHECK (scan_state IN ('pending', 'scanning', 'current', 'unreadable')),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE(workspace_id, normalized_relative_path)
) STRICT;

CREATE TABLE file_states (
    local_file_id TEXT PRIMARY KEY REFERENCES local_files(local_file_id) ON DELETE RESTRICT,
    baseline_version_id TEXT REFERENCES document_versions(version_id) ON DELETE RESTRICT,
    baseline_checksum_algorithm TEXT,
    baseline_checksum_value TEXT,
    baseline_relation TEXT NOT NULL CHECK (baseline_relation IN ('unknown', 'current', 'behind', 'platform_absent', 'platform_archived', 'inaccessible')),
    local_change_state TEXT NOT NULL CHECK (local_change_state IN ('clean', 'added', 'modified', 'deleted_candidate', 'renamed_candidate', 'unreadable', 'scanning')),
    confirmation_state TEXT NOT NULL CHECK (confirmation_state IN ('not_required', 'pending', 'confirmed', 'invalidated_content_changed', 'invalidated_baseline_changed', 'blocked_by_conflict', 'blocked_by_permission')),
    submission_state TEXT NOT NULL CHECK (submission_state IN ('none', 'planning', 'ready', 'submitting', 'submitted', 'pending_review', 'published', 'rejected', 'changes_requested', 'failed', 'outcome_unknown')),
    conflict_state TEXT NOT NULL CHECK (conflict_state IN ('none', 'update_available', 'content_diverged', 'delete_modify', 'rename_conflict', 'path_collision', 'case_collision', 'permission_conflict', 'metadata_conflict', 'unresolved')),
    display_state TEXT NOT NULL,
    capability_bits INTEGER NOT NULL CHECK (capability_bits >= 0),
    capability_reasons_json TEXT NOT NULL,
    computed_at INTEGER NOT NULL CHECK (computed_at >= 0),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0)
) STRICT;

CREATE TABLE checksum_cache (
    cache_key TEXT PRIMARY KEY CHECK (length(cache_key) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    local_file_id TEXT NOT NULL REFERENCES local_files(local_file_id) ON DELETE RESTRICT,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    modified_at_fs INTEGER NOT NULL CHECK (modified_at_fs >= 0),
    file_identity TEXT,
    checksum_algorithm TEXT NOT NULL,
    checksum_value TEXT NOT NULL,
    computed_at INTEGER NOT NULL CHECK (computed_at >= 0)
) STRICT;

CREATE TABLE file_confirmations (
    confirmation_id TEXT PRIMARY KEY CHECK (length(confirmation_id) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    local_file_id TEXT NOT NULL REFERENCES local_files(local_file_id) ON DELETE RESTRICT,
    document_id TEXT REFERENCES documents(document_id) ON DELETE RESTRICT,
    confirmed_by_account_id TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE RESTRICT,
    confirmed_checksum_algorithm TEXT NOT NULL,
    confirmed_checksum_value TEXT NOT NULL,
    baseline_version_id TEXT REFERENCES document_versions(version_id) ON DELETE RESTRICT,
    target_scope TEXT NOT NULL,
    result TEXT NOT NULL CHECK (result IN ('confirmed', 'request_changes', 'discarded')),
    source_client TEXT NOT NULL CHECK (source_client IN ('desktop', 'cli')),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    invalidated_at INTEGER CHECK (invalidated_at >= 0),
    invalidation_reason TEXT
) STRICT;

CREATE TABLE confirmation_invalidations (
    invalidation_id TEXT PRIMARY KEY CHECK (length(invalidation_id) > 0),
    confirmation_id TEXT NOT NULL REFERENCES file_confirmations(confirmation_id) ON DELETE RESTRICT,
    reason TEXT NOT NULL CHECK (reason IN ('local_content_changed', 'baseline_changed', 'permission_changed', 'conflict_detected')),
    previous_checksum TEXT,
    current_checksum TEXT,
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    correlation_id TEXT
) STRICT;

CREATE TABLE sync_plans (
    plan_id TEXT PRIMARY KEY CHECK (length(plan_id) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    direction TEXT NOT NULL CHECK (direction IN ('pull', 'submit')),
    state TEXT NOT NULL CHECK (state IN ('building', 'ready', 'expired', 'executing', 'completed', 'cancelled', 'invalidated')),
    platform_snapshot_revision TEXT,
    local_state_revision INTEGER NOT NULL CHECK (local_state_revision >= 0),
    created_by_account_id TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE RESTRICT,
    created_by_client TEXT NOT NULL CHECK (created_by_client IN ('desktop', 'cli')),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    expires_at INTEGER NOT NULL CHECK (expires_at >= 0),
    invalidated_reason TEXT,
    item_count INTEGER NOT NULL CHECK (item_count >= 0),
    risk_summary_json TEXT NOT NULL
) STRICT;

CREATE TABLE sync_plan_items (
    plan_item_id TEXT PRIMARY KEY CHECK (length(plan_item_id) > 0),
    plan_id TEXT NOT NULL REFERENCES sync_plans(plan_id) ON DELETE RESTRICT,
    local_file_id TEXT REFERENCES local_files(local_file_id) ON DELETE RESTRICT,
    document_id TEXT REFERENCES documents(document_id) ON DELETE RESTRICT,
    relative_path TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('download', 'create', 'update', 'delete', 'move', 'skip', 'conflict')),
    baseline_version_id TEXT REFERENCES document_versions(version_id) ON DELETE RESTRICT,
    target_version_id TEXT REFERENCES document_versions(version_id) ON DELETE RESTRICT,
    expected_local_checksum_algorithm TEXT,
    expected_local_checksum_value TEXT,
    target_checksum_algorithm TEXT,
    target_checksum_value TEXT,
    allowed INTEGER NOT NULL CHECK (allowed IN (0, 1)),
    reason TEXT,
    requires_confirmation INTEGER NOT NULL CHECK (requires_confirmation IN (0, 1)),
    risk_level TEXT NOT NULL CHECK (risk_level IN ('low', 'medium', 'high', 'critical')),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    UNIQUE(plan_id, ordinal)
) STRICT;

CREATE TABLE tasks (
    task_id TEXT PRIMARY KEY CHECK (length(task_id) > 0),
    task_type TEXT NOT NULL CHECK (length(task_type) > 0),
    workspace_id TEXT REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'waiting_for_user', 'succeeded', 'failed', 'cancelled', 'interrupted', 'outcome_unknown')),
    progress_completed INTEGER NOT NULL CHECK (progress_completed >= 0),
    progress_total INTEGER CHECK (progress_total >= 0),
    current_stage TEXT,
    cancellable INTEGER NOT NULL CHECK (cancellable IN (0, 1)),
    payload_json TEXT NOT NULL,
    result_ref_type TEXT,
    result_ref_id TEXT,
    correlation_id TEXT NOT NULL,
    created_by_client TEXT NOT NULL CHECK (created_by_client IN ('desktop', 'cli')),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    started_at INTEGER CHECK (started_at >= 0),
    completed_at INTEGER CHECK (completed_at >= 0),
    heartbeat_at INTEGER CHECK (heartbeat_at >= 0),
    error_reason TEXT,
    error_detail_json TEXT,
    CHECK (progress_total IS NULL OR progress_completed <= progress_total)
) STRICT;

CREATE TABLE task_attempts (
    attempt_id TEXT PRIMARY KEY CHECK (length(attempt_id) > 0),
    task_id TEXT NOT NULL REFERENCES tasks(task_id) ON DELETE RESTRICT,
    attempt_no INTEGER NOT NULL CHECK (attempt_no > 0),
    server_instance_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('running', 'succeeded', 'failed', 'interrupted')),
    started_at INTEGER NOT NULL CHECK (started_at >= 0),
    ended_at INTEGER CHECK (ended_at >= 0),
    error_reason TEXT,
    UNIQUE(task_id, attempt_no)
) STRICT;

CREATE TABLE sync_executions (
    execution_id TEXT PRIMARY KEY CHECK (length(execution_id) > 0),
    plan_id TEXT NOT NULL REFERENCES sync_plans(plan_id) ON DELETE RESTRICT,
    task_id TEXT NOT NULL UNIQUE REFERENCES tasks(task_id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'succeeded', 'partially_succeeded', 'failed', 'cancelled', 'outcome_unknown')),
    started_at INTEGER CHECK (started_at >= 0),
    completed_at INTEGER CHECK (completed_at >= 0),
    success_count INTEGER NOT NULL CHECK (success_count >= 0),
    failed_count INTEGER NOT NULL CHECK (failed_count >= 0),
    skipped_count INTEGER NOT NULL CHECK (skipped_count >= 0),
    conflict_count INTEGER NOT NULL CHECK (conflict_count >= 0),
    correlation_id TEXT NOT NULL,
    error_reason TEXT
) STRICT;

CREATE TABLE sync_execution_items (
    execution_item_id TEXT PRIMARY KEY CHECK (length(execution_item_id) > 0),
    execution_id TEXT NOT NULL REFERENCES sync_executions(execution_id) ON DELETE RESTRICT,
    plan_item_id TEXT NOT NULL REFERENCES sync_plan_items(plan_item_id) ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'succeeded', 'failed', 'skipped', 'conflict', 'outcome_unknown')),
    attempt_count INTEGER NOT NULL CHECK (attempt_count >= 0),
    started_at INTEGER CHECK (started_at >= 0),
    completed_at INTEGER CHECK (completed_at >= 0),
    platform_request_id TEXT,
    result_version_id TEXT REFERENCES document_versions(version_id) ON DELETE RESTRICT,
    error_reason TEXT,
    error_detail_json TEXT,
    UNIQUE(execution_id, plan_item_id)
) STRICT;

CREATE TABLE conflicts (
    conflict_id TEXT PRIMARY KEY CHECK (length(conflict_id) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    local_file_id TEXT NOT NULL REFERENCES local_files(local_file_id) ON DELETE RESTRICT,
    document_id TEXT REFERENCES documents(document_id) ON DELETE RESTRICT,
    conflict_type TEXT NOT NULL CHECK (conflict_type IN ('content_diverged', 'delete_modify', 'rename_conflict', 'path_collision', 'case_collision', 'permission_conflict', 'metadata_conflict')),
    state TEXT NOT NULL CHECK (state IN ('detected', 'in_progress', 'resolved', 'superseded')),
    baseline_version_id TEXT REFERENCES document_versions(version_id) ON DELETE RESTRICT,
    platform_version_id TEXT REFERENCES document_versions(version_id) ON DELETE RESTRICT,
    baseline_checksum TEXT,
    platform_checksum TEXT,
    local_checksum TEXT,
    detected_at INTEGER NOT NULL CHECK (detected_at >= 0),
    resolved_at INTEGER CHECK (resolved_at >= 0),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0)
) STRICT;

CREATE TABLE conflict_resolutions (
    resolution_id TEXT PRIMARY KEY CHECK (length(resolution_id) > 0),
    conflict_id TEXT NOT NULL REFERENCES conflicts(conflict_id) ON DELETE RESTRICT,
    resolution_type TEXT NOT NULL CHECK (resolution_type IN ('use_platform', 'keep_local_rebase', 'manual_merge', 'external_tool_merge')),
    selected_by TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE RESTRICT,
    result_checksum_algorithm TEXT,
    result_checksum_value TEXT,
    new_baseline_version_id TEXT REFERENCES document_versions(version_id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    correlation_id TEXT NOT NULL
) STRICT;

CREATE TABLE durable_events (
    global_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE CHECK (length(event_id) > 0),
    workspace_id TEXT REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    workspace_sequence INTEGER CHECK (workspace_sequence > 0),
    event_type TEXT NOT NULL CHECK (length(event_type) > 0),
    object_type TEXT,
    object_id TEXT,
    payload_json TEXT NOT NULL,
    correlation_id TEXT,
    occurred_at INTEGER NOT NULL CHECK (occurred_at >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK ((workspace_id IS NULL AND workspace_sequence IS NULL) OR (workspace_id IS NOT NULL AND workspace_sequence IS NOT NULL)),
    UNIQUE(workspace_id, workspace_sequence)
) STRICT;

CREATE TABLE client_event_cursors (
    client_identity TEXT NOT NULL CHECK (length(client_identity) > 0),
    account_id TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE RESTRICT,
    last_ack_sequence INTEGER NOT NULL CHECK (last_ack_sequence >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    PRIMARY KEY(client_identity, account_id)
) WITHOUT ROWID, STRICT;

CREATE INDEX idx_workspaces_account_status ON workspaces(account_id, status);
CREATE INDEX idx_documents_parent_path ON documents(workspace_id, parent_document_id, platform_path);
CREATE UNIQUE INDEX idx_documents_current_normalized_path ON documents(workspace_id, normalized_platform_path) WHERE platform_state <> 'deleted';
CREATE INDEX idx_documents_platform_state ON documents(workspace_id, platform_state);
CREATE INDEX idx_document_versions_created ON document_versions(document_id, created_at DESC);
CREATE INDEX idx_document_versions_status ON document_versions(document_id, status);
CREATE INDEX idx_local_files_parent_path ON local_files(workspace_id, parent_local_file_id, relative_path);
CREATE INDEX idx_local_files_document ON local_files(document_id);
CREATE INDEX idx_file_states_display ON file_states(display_state);
CREATE INDEX idx_file_states_confirmation ON file_states(confirmation_state);
CREATE INDEX idx_file_states_conflict ON file_states(conflict_state);
CREATE INDEX idx_file_confirmations_history ON file_confirmations(local_file_id, created_at DESC);
CREATE UNIQUE INDEX idx_file_confirmations_active ON file_confirmations(local_file_id) WHERE invalidated_at IS NULL AND result = 'confirmed';
CREATE INDEX idx_sync_plans_workspace_state ON sync_plans(workspace_id, state, created_at DESC);
CREATE INDEX idx_tasks_workspace_state ON tasks(workspace_id, state, created_at DESC);
CREATE UNIQUE INDEX idx_conflicts_active ON conflicts(local_file_id) WHERE state IN ('detected', 'in_progress');
CREATE INDEX idx_durable_events_workspace ON durable_events(workspace_id, workspace_sequence);
CREATE INDEX idx_durable_events_type ON durable_events(event_type, global_sequence);
CREATE INDEX idx_durable_events_object ON durable_events(object_type, object_id, global_sequence DESC);
