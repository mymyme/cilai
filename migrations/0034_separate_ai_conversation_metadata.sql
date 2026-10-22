CREATE TABLE ai_runtime_revision (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0)
);
INSERT INTO ai_runtime_revision SELECT state_key,state_revision FROM ai_runtime_state;

CREATE TABLE ai_conversations (
    conversation_id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    conversation_json TEXT NOT NULL CHECK (json_valid(conversation_json))
);
INSERT INTO ai_conversations(conversation_id,position,conversation_json)
SELECT json_extract(item.value,'$.conversationId'),CAST(item.key AS INTEGER),item.value
FROM ai_runtime_state state,json_each(state.state_json,'$.conversations') item
WHERE state.state_key='default';

ALTER TABLE ai_runtime_state DROP COLUMN state_revision;
UPDATE ai_runtime_state SET state_json=json_remove(state_json,'$.conversations');
