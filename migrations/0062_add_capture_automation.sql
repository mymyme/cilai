CREATE TABLE capture_auto_configs (
    owner_key TEXT PRIMARY KEY,
    config_json TEXT NOT NULL,
    enabled_after_rowid INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE capture_auto_pipelines (
    pipeline_id TEXT PRIMARY KEY,
    transfer_id TEXT NOT NULL UNIQUE REFERENCES capture_transfers(transfer_id),
    owner_key TEXT NOT NULL,
    config_json TEXT,
    state TEXT NOT NULL CHECK(state IN ('not_configured','queued','speech','analysis','completed','failed','unknown')),
    progress_json TEXT NOT NULL DEFAULT '{}',
    error_reason TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX capture_auto_pipelines_owner ON capture_auto_pipelines(owner_key,state,created_at);
