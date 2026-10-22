CREATE TABLE personal_application_data_slots (
    profile_id TEXT NOT NULL,
    slot_id TEXT NOT NULL,
    creation_project_id TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(profile_id,slot_id)
) STRICT;
CREATE INDEX personal_application_data_slots_project ON personal_application_data_slots(profile_id,creation_project_id);
CREATE TABLE personal_application_data_pointers (
    profile_id TEXT NOT NULL,
    creation_project_id TEXT NOT NULL,
    slot_id TEXT NOT NULL,
    PRIMARY KEY(profile_id,creation_project_id),
    FOREIGN KEY(profile_id,slot_id) REFERENCES personal_application_data_slots(profile_id,slot_id)
) STRICT;
CREATE TABLE personal_application_data_operations (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES personal_maintenance_operation_registry(operation_id),
    profile_id TEXT NOT NULL,
    creation_project_id TEXT NOT NULL,
    client_operation_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(profile_id,client_operation_id)
) STRICT;
CREATE INDEX personal_application_data_operations_project ON personal_application_data_operations(profile_id,creation_project_id,created_at);
