ALTER TABLE capture_hardware_events RENAME TO capture_hardware_events_previous;

CREATE TABLE capture_hardware_events (
 event_id TEXT PRIMARY KEY,
 owner_key TEXT NOT NULL,
 transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
 asset_id TEXT,
 text_material_id TEXT,
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
,
 CHECK((asset_id IS NOT NULL AND text_material_id IS NULL) OR (asset_id IS NULL AND text_material_id IS NOT NULL))
);

INSERT INTO capture_hardware_events(event_id,owner_key,transfer_id,asset_id,source_device_id,occurred_at_ms,full_text,transcript_sha256,segments_json,title,summary,project_name,todos_json,content_version,summary_template,source_operation_id,source_run_id,created_at,updated_at) SELECT event_id,owner_key,transfer_id,asset_id,source_device_id,occurred_at_ms,full_text,transcript_sha256,segments_json,title,summary,project_name,todos_json,content_version,summary_template,source_operation_id,source_run_id,created_at,updated_at FROM capture_hardware_events_previous;

DROP TABLE capture_hardware_events_previous;

ALTER TABLE capture_library_operations RENAME TO capture_library_operations_previous;

CREATE TABLE capture_library_operations (
 operation_id TEXT PRIMARY KEY,
 owner_key TEXT NOT NULL,
 library_grant_id TEXT NOT NULL,
 capture_session_id TEXT NOT NULL,
 phone_device_id TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('organize','summary')),
 transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
 asset_id TEXT,
 text_material_id TEXT,
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
,
 CHECK((asset_id IS NOT NULL AND text_material_id IS NULL) OR (asset_id IS NULL AND text_material_id IS NOT NULL))
);

INSERT INTO capture_library_operations(operation_id,owner_key,library_grant_id,capture_session_id,phone_device_id,kind,transfer_id,asset_id,event_id,expected_content_version,request_sha256,frozen_json,model_id,state,source_run_id,source_conversation_id,result_json,error_reason,created_at,updated_at) SELECT operation_id,owner_key,library_grant_id,capture_session_id,phone_device_id,kind,transfer_id,asset_id,event_id,expected_content_version,request_sha256,frozen_json,model_id,state,source_run_id,source_conversation_id,result_json,error_reason,created_at,updated_at FROM capture_library_operations_previous;

DROP TABLE capture_library_operations_previous;

ALTER TABLE capture_device_result_requests RENAME TO capture_device_result_requests_previous;

CREATE TABLE capture_device_result_requests (
 request_id TEXT PRIMARY KEY,
 owner_key TEXT NOT NULL,
 permission_id TEXT NOT NULL REFERENCES capture_device_result_permissions(permission_id),
 permission_revision INTEGER NOT NULL,
 source_device_id TEXT NOT NULL,
 transfer_id TEXT NOT NULL REFERENCES capture_transfers(transfer_id),
 asset_id TEXT,
 text_material_id TEXT,
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
,
 CHECK((asset_id IS NOT NULL AND text_material_id IS NULL) OR (asset_id IS NULL AND text_material_id IS NOT NULL))
);

INSERT INTO capture_device_result_requests(request_id,owner_key,permission_id,permission_revision,source_device_id,transfer_id,asset_id,event_id,intent,expected_version,request_sha256,request_json,decision_state,operation_state,operation_id,model_id,frozen_json,processing_id,conversation_id,source_run_id,result_json,error_reason,revision,created_at,updated_at) SELECT request_id,owner_key,permission_id,permission_revision,source_device_id,transfer_id,asset_id,event_id,intent,expected_version,request_sha256,request_json,decision_state,operation_state,operation_id,model_id,frozen_json,processing_id,conversation_id,source_run_id,result_json,error_reason,revision,created_at,updated_at FROM capture_device_result_requests_previous;

DROP TABLE capture_device_result_requests_previous;

CREATE INDEX capture_hardware_events_owner_idx ON capture_hardware_events(owner_key,occurred_at_ms,event_id);

CREATE UNIQUE INDEX capture_hardware_events_text_idx ON capture_hardware_events(transfer_id,text_material_id) WHERE text_material_id IS NOT NULL;

CREATE INDEX capture_library_operations_owner_idx ON capture_library_operations(owner_key,state,updated_at);

CREATE INDEX capture_device_result_requests_owner_idx ON capture_device_result_requests(owner_key,decision_state,operation_state,updated_at);
