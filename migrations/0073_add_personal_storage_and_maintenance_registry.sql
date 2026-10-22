CREATE TABLE personal_maintenance_operation_registry (
    operation_id TEXT PRIMARY KEY NOT NULL,
    profile_id TEXT NOT NULL,
    operation_kind TEXT NOT NULL CHECK(operation_kind IN ('backup','restore','storage_cleanup')),
    preview_digest TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared','capturing','incomplete','ready_to_apply','applying','completed','cancelled')),
    updated_at INTEGER NOT NULL
) STRICT;
INSERT INTO personal_maintenance_operation_registry(operation_id,profile_id,operation_kind,preview_digest,state,updated_at)
SELECT operation_id,profile_id,operation_kind,preview_digest,state,updated_at FROM personal_data_operations;

CREATE TABLE personal_storage_operations (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES personal_maintenance_operation_registry(operation_id),
    profile_id TEXT NOT NULL,
    client_operation_id TEXT NOT NULL UNIQUE,
    request_fingerprint TEXT NOT NULL,
    preview_digest TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    candidate_json TEXT NOT NULL CHECK(json_valid(candidate_json)),
    state TEXT NOT NULL CHECK(state IN ('prepared','applying','completed','incomplete','cancelled')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX personal_storage_operations_profile ON personal_storage_operations(profile_id,created_at,operation_id);

CREATE TABLE personal_maintenance_fences_v73 (
    fence_id TEXT PRIMARY KEY NOT NULL,
    operation_id TEXT NOT NULL REFERENCES personal_maintenance_operation_registry(operation_id),
    profile_id TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    state TEXT NOT NULL CHECK(state IN ('draining','released')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
INSERT INTO personal_maintenance_fences_v73 SELECT * FROM personal_maintenance_fences;
DROP TABLE personal_maintenance_fences;
ALTER TABLE personal_maintenance_fences_v73 RENAME TO personal_maintenance_fences;
CREATE UNIQUE INDEX personal_maintenance_fences_active ON personal_maintenance_fences(profile_id) WHERE state='draining';
