CREATE TABLE ai_application_sessions (
 session_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL UNIQUE,
 application_id TEXT NOT NULL, authorization_digest TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('active','closed','cleaning','cleaned')),
 revision INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE ai_application_messages (
 session_id TEXT NOT NULL REFERENCES ai_application_sessions(session_id),
 client_message_id TEXT NOT NULL, content_digest TEXT NOT NULL, run_id TEXT NOT NULL UNIQUE,
 snapshot_json TEXT NOT NULL, context_json TEXT NOT NULL, attachments_json TEXT NOT NULL,
 revision INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 PRIMARY KEY(session_id,client_message_id)
);
CREATE TABLE ai_application_attachment_refs (
 session_id TEXT NOT NULL REFERENCES ai_application_sessions(session_id), attachment_id TEXT NOT NULL UNIQUE, metadata_json TEXT NOT NULL,
 PRIMARY KEY(session_id,attachment_id)
);
