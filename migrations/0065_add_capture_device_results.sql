CREATE TABLE capture_device_result_permissions (
 permission_id TEXT PRIMARY KEY,
 owner_key TEXT NOT NULL,
 source_device_id TEXT NOT NULL,
 source_binding_version INTEGER NOT NULL CHECK(source_binding_version > 0),
 grant_id TEXT NOT NULL,
 pairing_id TEXT NOT NULL,
 receiver_cert_sha256 TEXT NOT NULL,
 source_public_key_sha256 TEXT NOT NULL,
 allow_read INTEGER NOT NULL CHECK(allow_read IN (0,1)),
 allow_retry_requests INTEGER NOT NULL CHECK(allow_retry_requests IN (0,1)),
 expires_at INTEGER NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('prepared','active','revoked')),
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
 frozen_json TEXT NOT NULL,
 request_sha256 TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 CHECK(allow_read=1 OR allow_retry_requests=1)
);
CREATE INDEX capture_device_result_permissions_owner_idx ON capture_device_result_permissions(owner_key,source_device_id,grant_id,state);
CREATE UNIQUE INDEX capture_device_result_permissions_active_idx ON capture_device_result_permissions(owner_key,grant_id) WHERE state='active';
CREATE TABLE capture_device_result_permission_transfers (
 permission_id TEXT NOT NULL REFERENCES capture_device_result_permissions(permission_id),
 transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
 record_id TEXT NOT NULL,
 record_revision INTEGER NOT NULL,
 manifest_sha256 TEXT NOT NULL,
 receipt_id TEXT NOT NULL,
 PRIMARY KEY(permission_id,transfer_id)
);
CREATE TABLE capture_device_result_requests (
 request_id TEXT PRIMARY KEY,
 owner_key TEXT NOT NULL,
 permission_id TEXT NOT NULL REFERENCES capture_device_result_permissions(permission_id),
 permission_revision INTEGER NOT NULL,
 source_device_id TEXT NOT NULL,
 transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
 asset_id TEXT NOT NULL,
 event_id TEXT,
 intent TEXT NOT NULL CHECK(intent IN ('transcribe','organize','summary')),
 expected_version INTEGER,
 request_sha256 TEXT NOT NULL,
 request_json TEXT NOT NULL,
 decision_state TEXT NOT NULL CHECK(decision_state IN ('pending','approved','rejected','cancelled')),
 operation_state TEXT NOT NULL CHECK(operation_state IN ('none','prepared','submitted','unknown','completed','failed','cancelled','not_submitted','conflict')),
 operation_id TEXT UNIQUE,
 model_id TEXT,
 frozen_json TEXT,
 processing_id TEXT,
 conversation_id TEXT,
 source_run_id TEXT UNIQUE,
 result_json TEXT,
 error_reason TEXT,
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE INDEX capture_device_result_requests_owner_idx ON capture_device_result_requests(owner_key,decision_state,operation_state,updated_at);
CREATE TABLE capture_device_result_pages (
 cursor_id TEXT PRIMARY KEY,
 owner_key TEXT NOT NULL,
 permission_id TEXT NOT NULL REFERENCES capture_device_result_permissions(permission_id),
 permission_revision INTEGER NOT NULL,
 page_kind TEXT NOT NULL,
 projection_sha256 TEXT NOT NULL,
 response_json TEXT NOT NULL,
 expires_at INTEGER NOT NULL
);
CREATE INDEX capture_device_result_pages_expiry_idx ON capture_device_result_pages(expires_at);
