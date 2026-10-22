PRAGMA legacy_alter_table = ON;

ALTER TABLE workspaces RENAME TO workspaces_v2;

CREATE TABLE workspaces (
    workspace_id TEXT PRIMARY KEY CHECK (length(workspace_id) > 0),
    display_name TEXT NOT NULL CHECK (length(display_name) > 0),
    local_root TEXT NOT NULL CHECK (length(local_root) > 0),
    local_root_identity TEXT,
    local_status TEXT NOT NULL CHECK (local_status IN ('creating', 'scanning', 'ready', 'attention_required', 'unavailable', 'invalid')),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    last_local_scan_at INTEGER CHECK (last_local_scan_at >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE(local_root)
) STRICT;

INSERT INTO workspaces(
    workspace_id,
    display_name,
    local_root,
    local_root_identity,
    local_status,
    state_revision,
    last_local_scan_at,
    created_at,
    updated_at
)
SELECT
    workspace_id,
    display_name,
    local_root,
    local_root_identity,
    CASE status
        WHEN 'invalid' THEN 'invalid'
        WHEN 'attention_required' THEN 'attention_required'
        WHEN 'conflict' THEN 'attention_required'
        ELSE 'ready'
    END,
    state_revision,
    last_local_scan_at,
    created_at,
    updated_at
FROM workspaces_v2;

CREATE TABLE workspace_platform_bindings (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    account_id TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE RESTRICT,
    organization_id TEXT CHECK (organization_id IS NULL OR length(organization_id) > 0),
    platform_workspace_id TEXT CHECK (platform_workspace_id IS NULL OR length(platform_workspace_id) > 0),
    project_id TEXT NOT NULL CHECK (length(project_id) > 0),
    platform_root TEXT NOT NULL,
    binding_state TEXT NOT NULL CHECK (binding_state IN ('binding', 'bound', 'unauthorized', 'disconnected', 'invalid')),
    platform_snapshot_revision TEXT,
    last_platform_check_at INTEGER CHECK (last_platform_check_at >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

INSERT INTO workspace_platform_bindings(
    workspace_id,
    account_id,
    organization_id,
    platform_workspace_id,
    project_id,
    platform_root,
    binding_state,
    platform_snapshot_revision,
    last_platform_check_at,
    created_at,
    updated_at
)
SELECT
    workspace_id,
    account_id,
    NULL,
    NULL,
    project_id,
    platform_root,
    CASE status
        WHEN 'binding' THEN 'binding'
        WHEN 'unauthorized' THEN 'unauthorized'
        WHEN 'invalid' THEN 'invalid'
        ELSE 'bound'
    END,
    platform_snapshot_revision,
    last_platform_check_at,
    created_at,
    updated_at
FROM workspaces_v2;

DROP TABLE workspaces_v2;

CREATE INDEX idx_workspaces_local_status ON workspaces(local_status);
CREATE UNIQUE INDEX idx_workspaces_local_root_identity
    ON workspaces(local_root_identity)
    WHERE local_root_identity IS NOT NULL;
CREATE INDEX idx_workspace_platform_bindings_account_state
    ON workspace_platform_bindings(account_id, binding_state);
CREATE UNIQUE INDEX idx_workspace_platform_bindings_active_target
    ON workspace_platform_bindings(account_id, platform_workspace_id, platform_root)
    WHERE binding_state <> 'disconnected';

PRAGMA legacy_alter_table = OFF;
