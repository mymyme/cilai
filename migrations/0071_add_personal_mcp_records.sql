CREATE TABLE personal_mcp_records (
    profile_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('connection','change','lease','cleanup')),
    record_id TEXT NOT NULL,
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(profile_id,kind,record_id)
) STRICT;

CREATE INDEX personal_mcp_records_recent ON personal_mcp_records(profile_id,kind,updated_at,record_id);
