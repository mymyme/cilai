ALTER TABLE cloud_workspace_operations ADD COLUMN expected_manifest_digest TEXT;
ALTER TABLE cloud_workspace_operations ADD COLUMN expected_object_count INTEGER;
ALTER TABLE cloud_workspace_operations ADD COLUMN baseline_revision TEXT;
ALTER TABLE cloud_workspace_operations ADD COLUMN remote_stage TEXT;
ALTER TABLE cloud_workspace_operations ADD COLUMN remote_operation_id TEXT;

CREATE TABLE cloud_workspace_transfer_entries (
    client_operation_id TEXT NOT NULL REFERENCES cloud_workspace_operations(client_operation_id) ON DELETE CASCADE,
    local_relative_path TEXT NOT NULL,
    remote_relative_path TEXT NOT NULL,
    entry_kind TEXT NOT NULL CHECK (entry_kind IN ('folder','file')),
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    checksum_sha256 TEXT NOT NULL,
    remote_node_id TEXT,
    upload_id TEXT,
    transfer_state TEXT NOT NULL CHECK (transfer_state IN ('pending','remote_requested','upload_ready','uploading','complete_requested','completed')),
    remote_operation_id TEXT,
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    PRIMARY KEY(client_operation_id, remote_relative_path)
) STRICT;

CREATE INDEX idx_cloud_workspace_transfer_pending
    ON cloud_workspace_transfer_entries(client_operation_id, transfer_state, remote_relative_path);
