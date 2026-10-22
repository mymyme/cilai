CREATE TABLE enterprise_capability_bindings (
    creation_project_id TEXT PRIMARY KEY REFERENCES capability_creation_projects(creation_project_id),
    tenant_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    package_digest TEXT NOT NULL,
    release_json TEXT NOT NULL CHECK(json_valid(release_json)),
    remaining_uses INTEGER NOT NULL DEFAULT 0,
    authorized_until INTEGER NOT NULL DEFAULT 0
) STRICT;
