CREATE TABLE personal_file_grants (
    grant_id TEXT PRIMARY KEY,
    creation_project_id TEXT NOT NULL REFERENCES capability_creation_projects(creation_project_id),
    package_digest TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json)),
    expires_at INTEGER NOT NULL,
    consumed INTEGER NOT NULL DEFAULT 0 CHECK(consumed IN (0,1)),
    write_digest TEXT
) STRICT;
