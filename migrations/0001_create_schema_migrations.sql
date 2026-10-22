CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY CHECK (version > 0),
    name TEXT NOT NULL UNIQUE CHECK (length(name) > 0),
    checksum TEXT NOT NULL CHECK (length(checksum) = 64),
    applied_at INTEGER NOT NULL CHECK (applied_at >= 0),
    app_version TEXT NOT NULL CHECK (length(app_version) > 0)
) STRICT;
