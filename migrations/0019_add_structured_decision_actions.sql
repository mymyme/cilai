ALTER TABLE attention_cases ADD COLUMN decision_action TEXT
    CHECK (decision_action IN ('approved', 'change_requested', 'rejected', 'delegated'));
ALTER TABLE attention_cases ADD COLUMN decision_action_summary TEXT;
ALTER TABLE attention_cases ADD COLUMN decision_actor_label TEXT;
ALTER TABLE attention_cases ADD COLUMN decision_delegated_to TEXT;
ALTER TABLE attention_cases ADD COLUMN decision_action_at INTEGER CHECK (decision_action_at >= 0);

DROP INDEX idx_attention_case_transitions_case;

CREATE TABLE attention_case_transitions_v19 (
    transition_id TEXT PRIMARY KEY CHECK (length(transition_id) > 0),
    attention_case_id TEXT NOT NULL REFERENCES attention_cases(attention_case_id) ON DELETE RESTRICT,
    from_status TEXT NOT NULL CHECK (from_status IN ('open', 'acknowledged')),
    to_status TEXT NOT NULL CHECK (to_status IN ('open', 'acknowledged', 'resolved')),
    actor_label TEXT NOT NULL CHECK (length(actor_label) > 0),
    resolution_summary TEXT NOT NULL,
    decision_action TEXT CHECK (decision_action IN ('approved', 'change_requested', 'rejected', 'delegated')),
    delegated_to TEXT,
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK (to_status <> 'open' OR decision_action = 'delegated'),
    CHECK (decision_action = 'delegated' OR delegated_to IS NULL),
    CHECK (decision_action <> 'delegated' OR length(delegated_to) > 0)
) STRICT;

INSERT INTO attention_case_transitions_v19(
    transition_id, attention_case_id, from_status, to_status, actor_label,
    resolution_summary, decision_action, delegated_to,
    client_operation_id, request_fingerprint, created_at
)
SELECT
    transition_id, attention_case_id, from_status, to_status, actor_label,
    resolution_summary, NULL, NULL,
    client_operation_id, request_fingerprint, created_at
FROM attention_case_transitions;

DROP TABLE attention_case_transitions;
ALTER TABLE attention_case_transitions_v19 RENAME TO attention_case_transitions;

CREATE INDEX idx_attention_case_transitions_case
    ON attention_case_transitions(attention_case_id, created_at, transition_id);
