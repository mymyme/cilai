CREATE TABLE conversation_work_classifications_v25 (
    classification_id TEXT PRIMARY KEY CHECK (length(classification_id) > 0),
    run_id TEXT NOT NULL UNIQUE CHECK (length(run_id) > 0),
    conversation_id TEXT NOT NULL CHECK (length(conversation_id) > 0),
    project_id TEXT REFERENCES projects(project_id) ON DELETE RESTRICT,
    business_task_id TEXT REFERENCES business_tasks(business_task_id) ON DELETE RESTRICT,
    classification_action TEXT NOT NULL CHECK (classification_action IN ('apply', 'keep', 'unclassified')),
    task_action TEXT NOT NULL CHECK (task_action IN ('keep', 'link', 'create', 'none')),
    conversation_status TEXT CHECK (conversation_status IN ('active', 'completed')),
    stage TEXT CHECK (stage IN ('todo', 'in_progress', 'review', 'decision')),
    outcome_title TEXT CHECK (outcome_title IS NULL OR length(outcome_title) > 0),
    outcome_summary TEXT CHECK (outcome_summary IS NULL OR length(outcome_summary) > 0),
    reason TEXT NOT NULL CHECK (length(reason) > 0),
    confidence REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    raw_json TEXT NOT NULL CHECK (json_valid(raw_json)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK ((classification_action = 'unclassified') = (business_task_id IS NULL)),
    CHECK ((classification_action = 'unclassified') = (conversation_status IS NULL)),
    CHECK ((classification_action = 'unclassified') = (outcome_title IS NULL)),
    CHECK ((classification_action = 'unclassified') = (outcome_summary IS NULL)),
    CHECK (conversation_status IS NULL OR (conversation_status = 'active') = (stage IS NOT NULL)),
    CHECK ((classification_action = 'unclassified') = (task_action = 'none'))
) STRICT;

INSERT INTO conversation_work_classifications_v25(
    classification_id,run_id,conversation_id,project_id,business_task_id,
    classification_action,task_action,conversation_status,stage,outcome_title,
    outcome_summary,reason,confidence,raw_json,created_at
)
SELECT
    classification_id,run_id,conversation_id,project_id,business_task_id,
    classification_action,task_action,
    CASE WHEN classification_action='unclassified' THEN NULL ELSE 'active' END,
    stage,
    CASE WHEN classification_action='unclassified' THEN NULL ELSE reason END,
    CASE WHEN classification_action='unclassified' THEN NULL ELSE reason END,
    reason,confidence,raw_json,created_at
FROM conversation_work_classifications;

DROP TABLE conversation_work_classifications;
ALTER TABLE conversation_work_classifications_v25 RENAME TO conversation_work_classifications;

CREATE INDEX idx_conversation_work_classifications_conversation
ON conversation_work_classifications(conversation_id, created_at DESC, classification_id);

CREATE INDEX idx_conversation_work_classifications_task_status
ON conversation_work_classifications(
    business_task_id,
    conversation_id,
    created_at DESC,
    classification_id DESC,
    conversation_status
);
