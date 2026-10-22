CREATE TABLE ai_conversation_thread_bindings (
    conversation_id TEXT PRIMARY KEY NOT NULL,
    binding_revision INTEGER NOT NULL CHECK (binding_revision > 0),
    generation INTEGER NOT NULL CHECK (generation > 0),
    native_thread_id TEXT,
    protocol_version INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('preparing','ready','active','reconciling','blocked','retired')),
    owner_instance_id TEXT,
    active_run_id TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE ai_thread_operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES ai_conversation_thread_bindings(conversation_id),
    generation INTEGER NOT NULL,
    run_id TEXT NOT NULL,
    operation_kind TEXT NOT NULL CHECK (operation_kind IN ('create','resume','fork','inject','send','steer','compact','delete')),
    request_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('prepared','dispatched','acknowledged','rejected','uncertain')),
    receipt_json TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX ai_thread_operations_conversation ON ai_thread_operations(conversation_id, generation, created_at);

CREATE TABLE ai_thread_generations (
    conversation_id TEXT NOT NULL REFERENCES ai_conversation_thread_bindings(conversation_id),
    generation INTEGER NOT NULL,
    native_thread_id TEXT NOT NULL UNIQUE,
    source_thread_id TEXT,
    source_operation_id TEXT NOT NULL REFERENCES ai_thread_operations(operation_id),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, generation)
);

CREATE TABLE ai_thread_message_links (
    conversation_id TEXT NOT NULL REFERENCES ai_conversation_thread_bindings(conversation_id),
    generation INTEGER NOT NULL,
    message_id TEXT NOT NULL,
    client_message_id TEXT NOT NULL,
    content_digest TEXT NOT NULL,
    native_thread_id TEXT NOT NULL,
    native_turn_id TEXT NOT NULL,
    operation_id TEXT NOT NULL UNIQUE REFERENCES ai_thread_operations(operation_id),
    accepted_at INTEGER NOT NULL,
    PRIMARY KEY (conversation_id, generation, client_message_id)
);
CREATE INDEX ai_thread_message_links_turn ON ai_thread_message_links(native_thread_id, native_turn_id);
