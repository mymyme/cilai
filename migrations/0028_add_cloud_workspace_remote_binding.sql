CREATE TABLE cloud_workspace_operations_v2 (
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
    remote_binding_id TEXT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    CHECK (
        (operation_kind = 'create_empty' AND source_workspace_id IS NULL) OR
        (operation_kind = 'initialize_local' AND source_workspace_id IS NOT NULL) OR
        (operation_kind = 'attach_existing' AND source_workspace_id IS NULL AND team_space_id IS NOT NULL)
    )
) STRICT;

INSERT INTO cloud_workspace_operations_v2(
    client_operation_id,request_fingerprint,operation_kind,project_id,source_workspace_id,
    team_space_id,display_name,managed_root,device_id,state,failure_reason,workspace_id,
    created_at,updated_at
)
SELECT
    client_operation_id,request_fingerprint,operation_kind,project_id,source_workspace_id,
    team_space_id,display_name,managed_root,device_id,state,failure_reason,workspace_id,
    created_at,updated_at
FROM cloud_workspace_operations;

DROP TABLE cloud_workspace_operations;
ALTER TABLE cloud_workspace_operations_v2 RENAME TO cloud_workspace_operations;

CREATE INDEX idx_cloud_workspace_operations_project
    ON cloud_workspace_operations(project_id, updated_at DESC);

CREATE UNIQUE INDEX idx_cloud_workspace_operations_remote_binding
    ON cloud_workspace_operations(remote_binding_id)
    WHERE remote_binding_id IS NOT NULL;
