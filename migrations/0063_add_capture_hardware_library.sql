CREATE TABLE capture_hardware_events (
 event_id TEXT PRIMARY KEY,
 owner_key TEXT NOT NULL,
 transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
 asset_id TEXT NOT NULL,
 source_device_id TEXT NOT NULL,
 occurred_at_ms INTEGER NOT NULL,
 full_text TEXT NOT NULL,
 transcript_sha256 TEXT NOT NULL,
 segments_json TEXT NOT NULL,
 title TEXT NOT NULL,
 summary TEXT NOT NULL,
 project_name TEXT NOT NULL,
 todos_json TEXT NOT NULL,
 content_version INTEGER NOT NULL DEFAULT 1 CHECK(content_version > 0),
 summary_template TEXT,
 source_operation_id TEXT NOT NULL,
 source_run_id TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 UNIQUE(transfer_id,asset_id)
);
CREATE INDEX capture_hardware_events_owner_idx ON capture_hardware_events(owner_key,occurred_at_ms,event_id);

CREATE TABLE capture_library_operations (
 operation_id TEXT PRIMARY KEY,
 owner_key TEXT NOT NULL,
 library_grant_id TEXT NOT NULL,
 capture_session_id TEXT NOT NULL,
 phone_device_id TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('organize','summary')),
 transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
 asset_id TEXT NOT NULL,
 event_id TEXT,
 expected_content_version INTEGER,
 request_sha256 TEXT NOT NULL,
 frozen_json TEXT NOT NULL,
 model_id TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('prepared','submitted','unknown','completed','failed','cancelled','not_submitted','conflict')),
 source_run_id TEXT UNIQUE,
 source_conversation_id TEXT,
 result_json TEXT,
 error_reason TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE INDEX capture_library_operations_owner_idx ON capture_library_operations(owner_key,state,updated_at);
