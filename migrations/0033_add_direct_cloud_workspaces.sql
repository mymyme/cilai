ALTER TABLE workspace_platform_bindings ADD COLUMN access_mode TEXT NOT NULL DEFAULT 'local_mirror'
    CHECK (access_mode IN ('local_mirror', 'remote_direct'));

UPDATE workspace_platform_bindings
SET access_mode = 'remote_direct'
WHERE team_space_id IS NOT NULL
  AND binding_state = 'bound';

CREATE TABLE direct_cloud_workspace_operations (
    client_operation_id TEXT PRIMARY KEY CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    team_space_id TEXT NOT NULL CHECK (length(team_space_id) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE INDEX idx_direct_cloud_workspace_operations_project
    ON direct_cloud_workspace_operations(project_id, created_at DESC);
