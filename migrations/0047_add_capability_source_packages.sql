ALTER TABLE capability_creation_projects
ADD COLUMN source_digest TEXT;

ALTER TABLE capability_creation_projects
ADD COLUMN source_file_count INTEGER NOT NULL DEFAULT 0;

CREATE TABLE capability_creation_builds (
    build_id TEXT PRIMARY KEY,
    creation_project_id TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    package_digest TEXT NOT NULL,
    package_relative_path TEXT NOT NULL,
    file_count INTEGER NOT NULL,
    total_bytes INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (creation_project_id)
        REFERENCES capability_creation_projects(creation_project_id)
        ON DELETE CASCADE,
    UNIQUE (creation_project_id, source_digest)
);

CREATE INDEX idx_capability_creation_builds_project_created
ON capability_creation_builds(creation_project_id, created_at DESC);
