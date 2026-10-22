CREATE TABLE capture_analysis_submissions (
 analysis_id TEXT PRIMARY KEY REFERENCES capture_analyses(analysis_id),
 state TEXT NOT NULL CHECK(state IN ('claimed','not_submitted','submitted')),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
