CREATE TABLE shared_file_content_index (
    file_identity TEXT PRIMARY KEY CHECK (length(file_identity) > 0),
    fingerprint TEXT NOT NULL,
    checksum_value TEXT NOT NULL CHECK (length(checksum_value) = 64)
) STRICT;
