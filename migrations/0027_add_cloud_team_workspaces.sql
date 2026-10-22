ALTER TABLE workspaces ADD COLUMN workspace_kind TEXT NOT NULL DEFAULT 'local'
    CHECK (workspace_kind IN ('local', 'cloud_team'));

ALTER TABLE workspace_platform_bindings ADD COLUMN team_space_id TEXT;
ALTER TABLE workspace_platform_bindings ADD COLUMN managed_root TEXT;
ALTER TABLE workspace_platform_bindings ADD COLUMN device_id TEXT;
ALTER TABLE workspace_platform_bindings ADD COLUMN baseline_revision TEXT;
ALTER TABLE workspace_platform_bindings ADD COLUMN change_cursor TEXT;
ALTER TABLE workspace_platform_bindings ADD COLUMN permission_role TEXT
    CHECK (permission_role IS NULL OR permission_role IN ('owner', 'editor', 'viewer'));
ALTER TABLE workspace_platform_bindings ADD COLUMN permission_revision TEXT;

CREATE UNIQUE INDEX idx_workspace_platform_bindings_team_space_device
    ON workspace_platform_bindings(account_id, team_space_id, device_id)
    WHERE team_space_id IS NOT NULL AND device_id IS NOT NULL AND binding_state <> 'disconnected';

CREATE UNIQUE INDEX idx_workspace_platform_bindings_managed_root_device
    ON workspace_platform_bindings(account_id, device_id, managed_root)
    WHERE device_id IS NOT NULL AND managed_root IS NOT NULL AND binding_state <> 'disconnected';

CREATE TABLE cloud_workspace_operations (
    client_operation_id TEXT PRIMARY KEY CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    operation_kind TEXT NOT NULL CHECK (operation_kind IN ('create_empty', 'initialize_local', 'attach_existing')),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    source_workspace_id TEXT REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    team_space_id TEXT,
    display_name TEXT NOT NULL CHECK (length(display_name) > 0),
    managed_root TEXT NOT NULL CHECK (length(managed_root) > 0),
    device_id TEXT NOT NULL CHECK (length(device_id) > 0),
    state TEXT NOT NULL CHECK (state IN ('prepared', 'service_unavailable', 'creating_remote', 'pulling_remote', 'uploading_initial', 'bound', 'failed')),
    failure_reason TEXT,
    workspace_id TEXT REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    CHECK (
        (operation_kind = 'create_empty' AND source_workspace_id IS NULL AND team_space_id IS NULL) OR
        (operation_kind = 'initialize_local' AND source_workspace_id IS NOT NULL AND team_space_id IS NULL) OR
        (operation_kind = 'attach_existing' AND source_workspace_id IS NULL AND team_space_id IS NOT NULL)
    )
) STRICT;

CREATE INDEX idx_cloud_workspace_operations_project
    ON cloud_workspace_operations(project_id, updated_at DESC);
