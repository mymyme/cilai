CREATE TABLE personal_data_operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    profile_id TEXT NOT NULL,
    operation_kind TEXT NOT NULL CHECK(operation_kind IN ('backup','restore')),
    client_operation_id TEXT NOT NULL UNIQUE,
    request_fingerprint TEXT NOT NULL,
    preview_digest TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    state TEXT NOT NULL CHECK(state IN ('prepared','capturing','incomplete','ready_to_apply','applying','completed','cancelled')),
    owner_instance TEXT,
    action_operation_id TEXT UNIQUE,
    action_fingerprint TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

CREATE INDEX personal_data_operations_profile ON personal_data_operations(profile_id,created_at,operation_id);
