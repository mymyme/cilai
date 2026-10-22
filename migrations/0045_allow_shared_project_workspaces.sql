PRAGMA legacy_alter_table = ON;

CREATE TABLE project_workspaces_v45 (
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('primary', 'supporting')),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    PRIMARY KEY(project_id, workspace_id)
) WITHOUT ROWID, STRICT;

INSERT INTO project_workspaces_v45 SELECT * FROM project_workspaces;

DROP TABLE project_workspaces;

ALTER TABLE project_workspaces_v45 RENAME TO project_workspaces;

CREATE INDEX idx_fk_project_workspaces_project_id ON project_workspaces(project_id);

CREATE UNIQUE INDEX idx_project_primary_workspace
    ON project_workspaces(project_id)
    WHERE role = 'primary';

CREATE INDEX idx_project_workspaces_workspace ON project_workspaces(workspace_id);

CREATE TABLE cloud_workspace_operations_v45 (
    client_operation_id TEXT PRIMARY KEY CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    operation_kind TEXT NOT NULL CHECK (operation_kind IN ('create_empty', 'initialize_local', 'attach_existing')),
    project_id TEXT NOT NULL CHECK (length(project_id) > 0),
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
    expected_manifest_digest TEXT,
    expected_object_count INTEGER,
    baseline_revision TEXT,
    remote_stage TEXT,
    remote_operation_id TEXT,
    CHECK (
        (operation_kind = 'create_empty' AND source_workspace_id IS NULL) OR
        (operation_kind = 'initialize_local' AND source_workspace_id IS NOT NULL) OR
        (operation_kind = 'attach_existing' AND source_workspace_id IS NULL AND team_space_id IS NOT NULL)
    )
) STRICT;

INSERT INTO cloud_workspace_operations_v45 SELECT * FROM cloud_workspace_operations;

DROP TABLE cloud_workspace_operations;

ALTER TABLE cloud_workspace_operations_v45 RENAME TO cloud_workspace_operations;

CREATE UNIQUE INDEX idx_cloud_workspace_one_pending_per_project
    ON cloud_workspace_operations(project_id)
    WHERE state IN ('prepared','creating_remote','pulling_remote','uploading_initial');

CREATE INDEX idx_cloud_workspace_operations_project
    ON cloud_workspace_operations(project_id, updated_at DESC);

CREATE UNIQUE INDEX idx_cloud_workspace_operations_remote_binding
    ON cloud_workspace_operations(remote_binding_id)
    WHERE remote_binding_id IS NOT NULL;

CREATE INDEX idx_fk_cloud_workspace_operations_source_workspace_id ON cloud_workspace_operations(source_workspace_id);

CREATE INDEX idx_fk_cloud_workspace_operations_workspace_id ON cloud_workspace_operations(workspace_id);

CREATE TABLE direct_cloud_workspace_operations_v45 (
    client_operation_id TEXT PRIMARY KEY CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    project_id TEXT NOT NULL CHECK (length(project_id) > 0),
    team_space_id TEXT NOT NULL CHECK (length(team_space_id) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

INSERT INTO direct_cloud_workspace_operations_v45 SELECT * FROM direct_cloud_workspace_operations;

DROP TABLE direct_cloud_workspace_operations;

ALTER TABLE direct_cloud_workspace_operations_v45 RENAME TO direct_cloud_workspace_operations;

CREATE INDEX idx_direct_cloud_workspace_operations_project
    ON direct_cloud_workspace_operations(project_id, created_at DESC);

CREATE INDEX idx_fk_direct_cloud_workspace_operations_workspace_id ON direct_cloud_workspace_operations(workspace_id);

PRAGMA legacy_alter_table = OFF;
