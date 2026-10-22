CREATE TABLE personal_namespace_storage_operations (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES personal_maintenance_operation_registry(operation_id),
    profile_id TEXT NOT NULL,
    client_operation_id TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    log_segment_id TEXT,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    state TEXT NOT NULL CHECK(state IN ('prepared','applying','completed','incomplete','cancelled')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(profile_id,client_operation_id)
) STRICT;
CREATE INDEX personal_namespace_storage_profile ON personal_namespace_storage_operations(profile_id,created_at);

CREATE UNIQUE INDEX personal_namespace_storage_segment ON personal_namespace_storage_operations(profile_id,log_segment_id) WHERE log_segment_id IS NOT NULL AND state IN ('prepared','applying');
