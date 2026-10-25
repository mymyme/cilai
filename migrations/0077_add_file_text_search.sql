CREATE TABLE file_search_documents (
    document_id INTEGER PRIMARY KEY,
    checksum_value TEXT NOT NULL UNIQUE CHECK (length(checksum_value) = 64),
    status TEXT NOT NULL CHECK (status IN ('indexed', 'binary', 'unsupported_encoding', 'too_large'))
) STRICT;

CREATE VIRTUAL TABLE file_search_text USING fts5(content, tokenize='trigram', detail=none);
INSERT INTO file_search_text(file_search_text, rank) VALUES('secure-delete', 1);

CREATE INDEX local_files_search_version ON local_files(local_checksum_value)
WHERE materialization_state<>'missing' AND local_checksum_algorithm='sha256';
