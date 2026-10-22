CREATE TABLE personal_capability_management_operations (
    client_operation_id TEXT PRIMARY KEY NOT NULL,
    request_fingerprint TEXT NOT NULL,
    creation_project_id TEXT NOT NULL REFERENCES capability_creation_projects(creation_project_id) ON DELETE RESTRICT,
    action TEXT NOT NULL CHECK (action IN ('enable','disable','uninstall')),
    result_revision INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX personal_capability_management_project_idx
ON personal_capability_management_operations(creation_project_id);
