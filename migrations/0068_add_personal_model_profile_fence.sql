CREATE TABLE personal_profile_compatibility (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    data_version INTEGER NOT NULL CHECK(data_version = 2),
    profile_id TEXT,
    instance_fence TEXT,
    manifest_digest TEXT,
    state TEXT NOT NULL CHECK(state IN ('pending', 'ready'))
);
INSERT INTO personal_profile_compatibility(singleton, data_version, state) VALUES(1, 2, 'pending');
ALTER TABLE ai_conversation_thread_bindings ADD COLUMN runtime_scope_json TEXT;
ALTER TABLE ai_conversation_thread_bindings ADD COLUMN model_execution_lease_id TEXT;
ALTER TABLE ai_thread_generations ADD COLUMN runtime_scope_json TEXT;
