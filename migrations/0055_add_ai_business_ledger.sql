CREATE TABLE ai_conversation_ledger (
    record_id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    digest TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE ai_message_ledger (
    record_id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json) AND json_type(record_json,'$.content') IS NULL),
    digest TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    conversation_id TEXT GENERATED ALWAYS AS(json_extract(record_json,'$.conversationId')) VIRTUAL,
    run_id TEXT GENERATED ALWAYS AS(json_extract(record_json,'$.runId')) VIRTUAL
);
CREATE INDEX ai_message_ledger_conversation ON ai_message_ledger(conversation_id);
CREATE INDEX ai_message_ledger_run ON ai_message_ledger(run_id);
CREATE TABLE ai_run_ledger (
    record_id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)
        AND json_type(record_json,'$.output') IS NULL
        AND json_type(record_json,'$.partialOutput') IS NULL
        AND json_type(record_json,'$.events') IS NULL
        AND json_type(record_json,'$.logTail') IS NULL
        AND json_type(record_json,'$.previousRevisions') IS NULL
        AND json_type(record_json,'$.regenerationDraft') IS NULL),
    digest TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    conversation_id TEXT GENERATED ALWAYS AS(json_extract(record_json,'$.conversationId')) VIRTUAL,
    status TEXT GENERATED ALWAYS AS(json_extract(record_json,'$.status')) VIRTUAL
);
CREATE INDEX ai_run_ledger_conversation ON ai_run_ledger(conversation_id);
CREATE INDEX ai_run_ledger_status ON ai_run_ledger(status);
CREATE TABLE ai_runtime_entries (
    record_id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    digest TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE ai_ledger_commands (
    command_id TEXT PRIMARY KEY NOT NULL,
    request_digest TEXT NOT NULL,
    result_json TEXT NOT NULL CHECK(json_valid(result_json)),
    applied_at INTEGER NOT NULL
);
CREATE TABLE ai_history_migrations (
    conversation_id TEXT PRIMARY KEY NOT NULL,
    source_digest TEXT NOT NULL,
    native_thread_id TEXT,
    status TEXT NOT NULL CHECK(status IN ('prepared','importing','verified','failed')),
    message_count INTEGER NOT NULL,
    receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)),
    updated_at INTEGER NOT NULL
);
CREATE TABLE ai_input_outbox (
    message_id TEXT PRIMARY KEY NOT NULL,
    content TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE ai_native_thread_index (
    conversation_id TEXT PRIMARY KEY NOT NULL,
    native_thread_id TEXT NOT NULL,
    title TEXT NOT NULL,
    archived INTEGER NOT NULL CHECK(archived IN (0,1)),
    updated_at INTEGER NOT NULL
);
CREATE TABLE ai_storage_activation (
    storage_key TEXT PRIMARY KEY NOT NULL CHECK(storage_key='native'),
    source_digest TEXT NOT NULL,
    activated_at INTEGER NOT NULL
);
CREATE TABLE ai_run_native_refs (
    run_id TEXT PRIMARY KEY NOT NULL,
    native_thread_id TEXT NOT NULL,
    native_turn_id TEXT NOT NULL
);
CREATE TABLE ai_message_native_refs (
    message_id TEXT PRIMARY KEY NOT NULL,
    native_thread_id TEXT NOT NULL,
    native_turn_id TEXT NOT NULL,
    native_item_id TEXT NOT NULL,
    body_format TEXT NOT NULL CHECK(body_format IN ('plain','native'))
);
CREATE VIEW ai_business_conversations AS SELECT * FROM ai_conversations;
CREATE VIEW ai_business_messages AS SELECT * FROM ai_messages;
CREATE VIEW ai_business_runs AS SELECT * FROM ai_runs;
