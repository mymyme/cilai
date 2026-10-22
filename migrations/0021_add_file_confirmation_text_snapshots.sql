CREATE TABLE file_confirmation_text_snapshots (
    confirmation_id TEXT PRIMARY KEY REFERENCES file_confirmations(confirmation_id) ON DELETE RESTRICT,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    local_file_id TEXT NOT NULL REFERENCES local_files(local_file_id) ON DELETE RESTRICT,
    content_revision TEXT NOT NULL CHECK (length(content_revision) = 64),
    text_content TEXT NOT NULL,
    content_bytes INTEGER NOT NULL CHECK (content_bytes >= 0 AND content_bytes <= 4194304),
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE INDEX idx_file_confirmation_text_snapshots_file
    ON file_confirmation_text_snapshots(workspace_id, local_file_id, created_at DESC, confirmation_id DESC);
