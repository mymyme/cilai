PRAGMA legacy_alter_table = ON;

CREATE TABLE local_scan_snapshots (
    snapshot_id TEXT PRIMARY KEY CHECK (length(snapshot_id) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    snapshot_kind TEXT NOT NULL CHECK (snapshot_kind IN ('full', 'incremental')),
    generation INTEGER NOT NULL CHECK (generation > 0),
    observed_at INTEGER NOT NULL CHECK (observed_at >= 0),
    completed_at INTEGER NOT NULL CHECK (completed_at >= observed_at),
    entry_count INTEGER NOT NULL CHECK (entry_count >= 0),
    issue_count INTEGER NOT NULL CHECK (issue_count >= 0),
    UNIQUE(workspace_id, generation)
) STRICT;

CREATE TABLE platform_snapshots (
    snapshot_id TEXT PRIMARY KEY CHECK (length(snapshot_id) > 0),
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    platform_revision TEXT NOT NULL CHECK (length(platform_revision) > 0),
    cursor TEXT,
    snapshot_kind TEXT NOT NULL CHECK (snapshot_kind IN ('full', 'incremental')),
    received_at INTEGER NOT NULL CHECK (received_at >= 0),
    platform_time INTEGER CHECK (platform_time >= 0),
    item_count INTEGER NOT NULL CHECK (item_count >= 0),
    request_id TEXT,
    UNIQUE(workspace_id, platform_revision)
) STRICT;

ALTER TABLE local_files RENAME TO local_files_v3;

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
    first_seen_snapshot_id TEXT REFERENCES local_scan_snapshots(snapshot_id) ON DELETE RESTRICT,
    last_seen_snapshot_id TEXT REFERENCES local_scan_snapshots(snapshot_id) ON DELETE RESTRICT,
    missing_since_snapshot_id TEXT REFERENCES local_scan_snapshots(snapshot_id) ON DELETE RESTRICT,
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

INSERT INTO local_files(
    local_file_id,
    workspace_id,
    document_id,
    parent_local_file_id,
    relative_path,
    normalized_relative_path,
    file_identity,
    materialization_state,
    file_type,
    size_bytes,
    modified_at_fs,
    observed_at,
    local_checksum_algorithm,
    local_checksum_value,
    scan_state,
    state_revision,
    created_at,
    updated_at
)
SELECT
    local_file_id,
    workspace_id,
    document_id,
    parent_local_file_id,
    relative_path,
    normalized_relative_path,
    file_identity,
    materialization_state,
    file_type,
    size_bytes,
    modified_at_fs,
    observed_at,
    local_checksum_algorithm,
    local_checksum_value,
    scan_state,
    state_revision,
    created_at,
    updated_at
FROM local_files_v3;

DROP TABLE local_files_v3;

CREATE TABLE local_file_observations (
    snapshot_id TEXT NOT NULL REFERENCES local_scan_snapshots(snapshot_id) ON DELETE RESTRICT,
    local_file_id TEXT NOT NULL REFERENCES local_files(local_file_id) ON DELETE RESTRICT,
    relative_path TEXT NOT NULL,
    normalized_relative_path TEXT NOT NULL,
    file_identity TEXT,
    file_type TEXT NOT NULL CHECK (file_type IN ('file', 'directory', 'symlink')),
    size_bytes INTEGER CHECK (size_bytes >= 0),
    modified_at_fs INTEGER CHECK (modified_at_fs >= 0),
    checksum_algorithm TEXT,
    checksum_value TEXT,
    observed_at INTEGER NOT NULL CHECK (observed_at >= 0),
    PRIMARY KEY(snapshot_id, local_file_id)
) WITHOUT ROWID, STRICT;

CREATE UNIQUE INDEX idx_local_files_current_normalized_path
    ON local_files(workspace_id, normalized_relative_path)
    WHERE materialization_state <> 'missing';
CREATE INDEX idx_local_files_current_identity
    ON local_files(workspace_id, file_identity)
    WHERE file_identity IS NOT NULL AND materialization_state <> 'missing';
CREATE INDEX idx_local_files_parent_path
    ON local_files(workspace_id, parent_local_file_id, relative_path);
CREATE INDEX idx_local_files_document ON local_files(document_id);
CREATE INDEX idx_local_file_observations_path
    ON local_file_observations(snapshot_id, normalized_relative_path);
CREATE INDEX idx_local_scan_snapshots_workspace
    ON local_scan_snapshots(workspace_id, generation DESC);
CREATE INDEX idx_platform_snapshots_workspace
    ON platform_snapshots(workspace_id, received_at DESC);

PRAGMA legacy_alter_table = OFF;
