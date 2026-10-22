CREATE TABLE personal_package_imports (
    import_id TEXT PRIMARY KEY NOT NULL,
    profile_id TEXT NOT NULL,
    prepare_operation_id TEXT NOT NULL UNIQUE,
    prepare_fingerprint TEXT NOT NULL,
    preview_digest TEXT NOT NULL,
    preview_json TEXT NOT NULL CHECK(json_valid(preview_json)),
    candidate_relative_path TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK(state IN ('prepared','committed','cancelled')),
    commit_operation_id TEXT UNIQUE,
    commit_fingerprint TEXT,
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
    prepared_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK((state='committed' AND commit_operation_id IS NOT NULL AND commit_fingerprint IS NOT NULL AND result_json IS NOT NULL)
       OR (state<>'committed' AND commit_operation_id IS NULL AND commit_fingerprint IS NULL AND result_json IS NULL))
) STRICT;

CREATE INDEX personal_package_imports_profile ON personal_package_imports(profile_id,prepared_at,import_id);
