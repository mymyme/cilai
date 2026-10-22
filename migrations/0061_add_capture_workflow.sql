CREATE TABLE capture_transfer_chunks (
    transfer_id TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    chunk_index INTEGER NOT NULL CHECK(chunk_index >= 0),
    byte_length INTEGER NOT NULL CHECK(byte_length > 0 AND byte_length <= 1048576),
    sha256 TEXT NOT NULL,
    storage_key TEXT NOT NULL UNIQUE,
    PRIMARY KEY(transfer_id,asset_id,chunk_index),
    FOREIGN KEY(transfer_id,asset_id) REFERENCES capture_transfer_assets(transfer_id,asset_id)
);
CREATE TABLE capture_request_nonces (
    scope_key TEXT NOT NULL,
    nonce TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY(scope_key,nonce)
);
CREATE TABLE capture_record_metadata (
    transfer_id TEXT PRIMARY KEY REFERENCES capture_transfers(transfer_id),
    title TEXT NOT NULL,
    discussion_marked INTEGER NOT NULL CHECK(discussion_marked IN (0,1)),
    updated_at INTEGER NOT NULL
);
CREATE TABLE capture_processing (
    processing_id TEXT PRIMARY KEY,
    transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
    operation_id TEXT NOT NULL UNIQUE,
    model_id TEXT NOT NULL,
    asset_id TEXT NOT NULL,
    request_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared','not_submitted','submitted','completed','failed','unknown')),
    result_json TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE capture_analyses (
    analysis_id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL UNIQUE,
    transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
    version INTEGER NOT NULL CHECK(version > 0),
    source_text TEXT NOT NULL,
    source_sha256 TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared','completed','failed','interrupted','terminated')),
    source_run_id TEXT UNIQUE,
    result_json TEXT,
    error_reason TEXT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    UNIQUE(transfer_id,version)
);
CREATE TABLE capture_candidate_confirmations (
    analysis_id TEXT NOT NULL REFERENCES capture_analyses(analysis_id),
    candidate_id TEXT NOT NULL,
    disposition TEXT NOT NULL CHECK(disposition IN ('confirmed','deferred','rejected')),
    content_json TEXT NOT NULL,
    confirmed_at INTEGER NOT NULL,
    PRIMARY KEY(analysis_id,candidate_id)
);
CREATE TABLE capture_handoffs (
    handoff_id TEXT PRIMARY KEY,
    analysis_id TEXT NOT NULL REFERENCES capture_analyses(analysis_id),
    candidate_id TEXT NOT NULL,
    execution_round INTEGER NOT NULL CHECK(execution_round > 0),
    input_json TEXT NOT NULL,
    conversation_id TEXT,
    run_id TEXT,
    state TEXT NOT NULL CHECK(state IN ('prepared','submitted')),
    created_at INTEGER NOT NULL,
    UNIQUE(analysis_id,candidate_id,execution_round)
);

CREATE TABLE capture_shares (
    share_id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL UNIQUE,
    transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
    frozen_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared','submitting','completed','unknown','revoking','revoked')),
    progress_json TEXT NOT NULL DEFAULT '{}',
    error_reason TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
