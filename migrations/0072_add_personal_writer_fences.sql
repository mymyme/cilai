CREATE TABLE personal_profile_writers (
    lease_id TEXT PRIMARY KEY NOT NULL,
    profile_id TEXT NOT NULL,
    client_operation_id TEXT NOT NULL UNIQUE,
    request_fingerprint TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    state TEXT NOT NULL CHECK(state IN ('prepared','active','released')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX personal_profile_writers_active ON personal_profile_writers(profile_id,state,lease_id);

CREATE TABLE personal_maintenance_fences (
    fence_id TEXT PRIMARY KEY NOT NULL,
    operation_id TEXT NOT NULL REFERENCES personal_data_operations(operation_id),
    profile_id TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    state TEXT NOT NULL CHECK(state IN ('draining','released')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE UNIQUE INDEX personal_maintenance_fences_active ON personal_maintenance_fences(profile_id) WHERE state='draining';
