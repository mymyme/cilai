CREATE TABLE ai_application_thread_scopes (
    conversation_id TEXT PRIMARY KEY NOT NULL REFERENCES ai_conversation_thread_bindings(conversation_id) ON DELETE CASCADE,
    application_id TEXT NOT NULL,
    session_kind TEXT NOT NULL CHECK(session_kind IN ('application_page', 'application_task'))
);

CREATE TABLE ai_application_thread_runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES ai_application_thread_scopes(conversation_id) ON DELETE CASCADE
);
CREATE INDEX ai_application_thread_runs_conversation ON ai_application_thread_runs(conversation_id);
