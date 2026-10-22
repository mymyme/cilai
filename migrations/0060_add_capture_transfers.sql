CREATE TABLE capture_transfers (
    transfer_id TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL,
    scope_json TEXT NOT NULL,
    record_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    manifest_digest TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('receiving','received')),
    receipt_id TEXT UNIQUE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK((state='received' AND receipt_id IS NOT NULL) OR (state='receiving' AND receipt_id IS NULL)),
    UNIQUE(scope_key,record_id,revision)
);
CREATE TABLE capture_transfer_assets (
    transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
    asset_id TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    byte_length INTEGER NOT NULL CHECK(byte_length > 0),
    storage_key TEXT,
    PRIMARY KEY(transfer_id,asset_id)
);
CREATE INDEX capture_transfers_owner ON capture_transfers(scope_key,state,created_at);
