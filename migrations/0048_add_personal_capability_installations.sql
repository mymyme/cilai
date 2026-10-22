CREATE TABLE personal_capability_installations (
    creation_project_id TEXT PRIMARY KEY,
    package_digest TEXT NOT NULL,
    installed_relative_path TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    state_revision INTEGER NOT NULL,
    installed_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (creation_project_id)
        REFERENCES capability_creation_projects(creation_project_id)
        ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_personal_capability_installations_path
ON personal_capability_installations(installed_relative_path);

CREATE TABLE capability_creation_project_operations_v2 (
    client_operation_id TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    operation_kind TEXT NOT NULL CHECK (operation_kind IN ('create','install')),
    creation_project_id TEXT NOT NULL REFERENCES capability_creation_projects(creation_project_id) ON DELETE RESTRICT,
    result_revision INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

INSERT INTO capability_creation_project_operations_v2(
    client_operation_id,
    request_fingerprint,
    operation_kind,
    creation_project_id,
    result_revision,
    created_at
)
SELECT
    client_operation_id,
    request_fingerprint,
    operation_kind,
    creation_project_id,
    result_revision,
    created_at
FROM capability_creation_project_operations;

DROP TABLE capability_creation_project_operations;

ALTER TABLE capability_creation_project_operations_v2
RENAME TO capability_creation_project_operations;
