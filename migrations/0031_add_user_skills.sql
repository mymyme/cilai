CREATE TABLE user_skills (
    skill_id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    managed_relative_path TEXT NOT NULL UNIQUE,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    automatic INTEGER NOT NULL DEFAULT 0 CHECK (automatic IN (0, 1)),
    state_revision INTEGER NOT NULL DEFAULT 1 CHECK (state_revision > 0),
    content_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_user_skills_updated_at
    ON user_skills(updated_at DESC, skill_id ASC);

CREATE TABLE user_skill_operations (
    client_operation_id TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    operation_kind TEXT NOT NULL,
    skill_id TEXT NOT NULL,
    result_revision INTEGER,
    created_at INTEGER NOT NULL
);
