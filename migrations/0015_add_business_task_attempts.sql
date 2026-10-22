ALTER TABLE business_tasks
ADD COLUMN attempt_no INTEGER NOT NULL DEFAULT 1 CHECK (attempt_no > 0);

ALTER TABLE business_task_transitions
ADD COLUMN attempt_no INTEGER NOT NULL DEFAULT 1 CHECK (attempt_no > 0);

CREATE INDEX business_task_transitions_attempt_idx
ON business_task_transitions(business_task_id, attempt_no, created_at);
