CREATE TABLE projects (
    project_id TEXT PRIMARY KEY CHECK (length(project_id) > 0),
    display_name TEXT NOT NULL CHECK (length(display_name) > 0),
    goal TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'paused', 'completed', 'archived')),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

CREATE TABLE project_workspaces (
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    workspace_id TEXT NOT NULL UNIQUE REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    role TEXT NOT NULL CHECK (role IN ('primary', 'supporting')),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    PRIMARY KEY(project_id, workspace_id)
) WITHOUT ROWID, STRICT;

CREATE UNIQUE INDEX idx_project_primary_workspace
    ON project_workspaces(project_id)
    WHERE role = 'primary';

CREATE TABLE deliverables (
    deliverable_id TEXT PRIMARY KEY CHECK (length(deliverable_id) > 0),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(title) > 0),
    deliverable_type TEXT NOT NULL CHECK (length(deliverable_type) > 0),
    owner_label TEXT NOT NULL CHECK (length(owner_label) > 0),
    acceptance_criteria TEXT NOT NULL CHECK (length(acceptance_criteria) > 0),
    status TEXT NOT NULL CHECK (status IN ('draft', 'candidate', 'accepted', 'revision_needed')),
    current_accepted_version_id TEXT,
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

CREATE INDEX idx_deliverables_project
    ON deliverables(project_id, updated_at DESC, deliverable_id);

CREATE TABLE deliverable_versions (
    version_id TEXT PRIMARY KEY CHECK (length(version_id) > 0),
    deliverable_id TEXT NOT NULL REFERENCES deliverables(deliverable_id) ON DELETE RESTRICT,
    version_no INTEGER NOT NULL CHECK (version_no > 0),
    status TEXT NOT NULL CHECK (status IN ('candidate', 'accepted', 'rejected', 'superseded')),
    summary TEXT NOT NULL,
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    accepted_at INTEGER CHECK (accepted_at >= 0),
    rejected_at INTEGER CHECK (rejected_at >= 0),
    UNIQUE(deliverable_id, version_no)
) STRICT;

CREATE INDEX idx_deliverable_versions_deliverable
    ON deliverable_versions(deliverable_id, version_no DESC);

CREATE TABLE deliverable_file_bindings (
    version_id TEXT NOT NULL REFERENCES deliverable_versions(version_id) ON DELETE RESTRICT,
    local_file_id TEXT NOT NULL REFERENCES local_files(local_file_id) ON DELETE RESTRICT,
    workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    relative_path_at_binding TEXT NOT NULL CHECK (length(relative_path_at_binding) > 0),
    file_revision_at_binding INTEGER NOT NULL CHECK (file_revision_at_binding >= 0),
    checksum_algorithm TEXT NOT NULL CHECK (length(checksum_algorithm) > 0),
    checksum_value TEXT NOT NULL CHECK (length(checksum_value) > 0),
    binding_role TEXT NOT NULL CHECK (binding_role IN ('primary', 'supporting')),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    PRIMARY KEY(version_id, local_file_id)
) WITHOUT ROWID, STRICT;

CREATE INDEX idx_deliverable_bindings_file
    ON deliverable_file_bindings(local_file_id, version_id);

CREATE TABLE deliverable_acceptances (
    acceptance_id TEXT PRIMARY KEY CHECK (length(acceptance_id) > 0),
    deliverable_id TEXT NOT NULL REFERENCES deliverables(deliverable_id) ON DELETE RESTRICT,
    version_id TEXT NOT NULL REFERENCES deliverable_versions(version_id) ON DELETE RESTRICT,
    decision TEXT NOT NULL CHECK (decision IN ('accepted', 'rejected')),
    reviewer_label TEXT NOT NULL CHECK (length(reviewer_label) > 0),
    comment TEXT NOT NULL,
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE INDEX idx_deliverable_acceptances_version
    ON deliverable_acceptances(version_id, created_at DESC);
