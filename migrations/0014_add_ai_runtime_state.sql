CREATE TABLE ai_runtime_state (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    state_json TEXT NOT NULL CHECK (json_valid(state_json)),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

INSERT INTO ai_runtime_state(state_key, state_revision, state_json, updated_at)
VALUES ('default', 0, '{"agents":[],"conversations":[],"messages":[],"runs":[],"permissionRequests":[],"automationRules":[]}', 0);

