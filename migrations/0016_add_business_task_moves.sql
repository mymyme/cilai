CREATE TABLE business_task_moves (
    move_id TEXT PRIMARY KEY CHECK (length(move_id) > 0),
    business_task_id TEXT NOT NULL REFERENCES business_tasks(business_task_id) ON DELETE RESTRICT,
    from_phase_id TEXT NOT NULL REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    to_phase_id TEXT NOT NULL REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    dependency_impact TEXT NOT NULL CHECK (length(dependency_impact) > 0),
    deliverable_impact TEXT NOT NULL CHECK (length(deliverable_impact) > 0),
    acceptance_impact TEXT NOT NULL CHECK (length(acceptance_impact) > 0),
    actor_label TEXT NOT NULL CHECK (length(actor_label) > 0),
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK (from_phase_id <> to_phase_id)
) STRICT;

CREATE INDEX idx_business_task_moves_task
    ON business_task_moves(business_task_id, created_at, move_id);
