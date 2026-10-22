CREATE TABLE ai_context_imports (
    scope TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    source_json TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('prepared', 'committed')),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(scope, thread_id)
);
CREATE TABLE ai_memory_state (
    state_key TEXT PRIMARY KEY CHECK(state_key='account'),
    revision INTEGER NOT NULL,
    value_json TEXT NOT NULL
);
