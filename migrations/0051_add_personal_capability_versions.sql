CREATE TABLE personal_capability_versions (
    creation_project_id TEXT NOT NULL REFERENCES capability_creation_projects(creation_project_id),
    package_digest TEXT NOT NULL,
    version TEXT NOT NULL,
    project_json TEXT NOT NULL CHECK(json_valid(project_json)),
    build_json TEXT NOT NULL CHECK(json_valid(build_json)),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(creation_project_id, package_digest),
    UNIQUE(creation_project_id, version)
) STRICT;

CREATE TABLE personal_capability_lifecycle_operations (
    operation_id TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    creation_project_id TEXT NOT NULL REFERENCES capability_creation_projects(creation_project_id),
    kind TEXT NOT NULL CHECK(kind IN ('build','upgrade')),
    package_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;
