CREATE TABLE personal_maintenance_operation_registry_v74 (
    operation_id TEXT PRIMARY KEY NOT NULL,
    profile_id TEXT NOT NULL,
    operation_kind TEXT NOT NULL CHECK(operation_kind IN ('backup','restore','storage_cleanup','memory_reset','personal_capability_data_change')),
    preview_digest TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared','capturing','incomplete','ready_to_apply','applying','completed','cancelled')),
    updated_at INTEGER NOT NULL
) STRICT;
INSERT INTO personal_maintenance_operation_registry_v74 SELECT * FROM personal_maintenance_operation_registry;
DROP TABLE personal_maintenance_operation_registry;
ALTER TABLE personal_maintenance_operation_registry_v74 RENAME TO personal_maintenance_operation_registry;

CREATE TABLE personal_memory_reset_operations (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES personal_maintenance_operation_registry(operation_id),
    profile_id TEXT NOT NULL,
    client_operation_id TEXT NOT NULL UNIQUE,
    request_fingerprint TEXT NOT NULL,
    preview_digest TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('prepared','applying','completed','incomplete','cancelled')),
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX personal_memory_reset_operations_profile ON personal_memory_reset_operations(profile_id,created_at,operation_id);
