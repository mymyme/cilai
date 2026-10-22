CREATE TABLE capability_creation_projects (
    creation_project_id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    business_goal TEXT NOT NULL,
    target_type TEXT NOT NULL CHECK (target_type IN ('local_mcp','web_application','web_mcp_application')),
    status TEXT NOT NULL CHECK (status IN ('planned','developing','validating','preview_ready','packaged','installed','failed')),
    conversation_id TEXT,
    managed_relative_path TEXT NOT NULL UNIQUE,
    requested_permissions_json TEXT NOT NULL CHECK (json_valid(requested_permissions_json) AND json_type(requested_permissions_json)='array'),
    acceptance_examples_json TEXT NOT NULL CHECK (json_valid(acceptance_examples_json) AND json_type(acceptance_examples_json)='array'),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 1),
    package_digest TEXT,
    error_reason TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX capability_creation_projects_updated
    ON capability_creation_projects(updated_at DESC, creation_project_id);

CREATE TABLE capability_creation_project_operations (
    client_operation_id TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    operation_kind TEXT NOT NULL CHECK (operation_kind='create'),
    creation_project_id TEXT NOT NULL REFERENCES capability_creation_projects(creation_project_id) ON DELETE RESTRICT,
    result_revision INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE ai_capability_project_deliveries (
    run_id TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending','historical','ignored','invalid','failed','created')),
    source_json TEXT NOT NULL CHECK (json_valid(source_json)),
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json))
) STRICT;

CREATE INDEX ai_capability_project_deliveries_pending
    ON ai_capability_project_deliveries(status,run_id);

INSERT INTO ai_capability_project_deliveries(run_id,status,source_json)
SELECT run_id,'historical',json_object('runId',run_id)
FROM ai_runs
WHERE status='completed';
