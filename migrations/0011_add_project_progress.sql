CREATE TABLE phase_plans (
    plan_id TEXT PRIMARY KEY CHECK (length(plan_id) > 0),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    version_no INTEGER NOT NULL CHECK (version_no > 0),
    status TEXT NOT NULL CHECK (status IN ('draft', 'active', 'superseded')),
    summary TEXT NOT NULL,
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    activation_client_operation_id TEXT UNIQUE,
    activation_request_fingerprint TEXT,
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    activated_at INTEGER CHECK (activated_at >= 0),
    UNIQUE(project_id, version_no)
) STRICT;

CREATE UNIQUE INDEX idx_phase_plans_active
    ON phase_plans(project_id)
    WHERE status = 'active';

CREATE TABLE project_phases (
    phase_id TEXT PRIMARY KEY CHECK (length(phase_id) > 0),
    plan_id TEXT NOT NULL REFERENCES phase_plans(plan_id) ON DELETE RESTRICT,
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    name TEXT NOT NULL CHECK (length(name) > 0),
    goal TEXT NOT NULL CHECK (length(goal) > 0),
    acceptance_criteria TEXT NOT NULL CHECK (length(acceptance_criteria) > 0),
    required_deliverable_id TEXT REFERENCES deliverables(deliverable_id) ON DELETE RESTRICT,
    sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
    is_main INTEGER NOT NULL CHECK (is_main IN (0, 1)),
    status TEXT NOT NULL CHECK (status IN ('planned', 'in_progress', 'review', 'completed', 'paused', 'terminated')),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE(plan_id, sort_order)
) STRICT;

CREATE INDEX idx_project_phases_plan_order
    ON project_phases(plan_id, sort_order, phase_id);

CREATE TABLE phase_dependencies (
    plan_id TEXT NOT NULL REFERENCES phase_plans(plan_id) ON DELETE RESTRICT,
    predecessor_phase_id TEXT NOT NULL REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    successor_phase_id TEXT NOT NULL REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    dependency_type TEXT NOT NULL CHECK (dependency_type IN ('finish_to_start')),
    PRIMARY KEY(plan_id, predecessor_phase_id, successor_phase_id),
    CHECK (predecessor_phase_id <> successor_phase_id)
) WITHOUT ROWID, STRICT;

CREATE TABLE business_tasks (
    business_task_id TEXT PRIMARY KEY CHECK (length(business_task_id) > 0),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    phase_id TEXT NOT NULL REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(title) > 0),
    objective TEXT NOT NULL CHECK (length(objective) > 0),
    input_context TEXT NOT NULL,
    expected_output TEXT NOT NULL CHECK (length(expected_output) > 0),
    completion_criteria TEXT NOT NULL CHECK (length(completion_criteria) > 0),
    assignee_type TEXT NOT NULL CHECK (assignee_type IN ('user', 'agent', 'collaboration')),
    assignee_label TEXT NOT NULL CHECK (length(assignee_label) > 0),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high', 'critical')),
    due_at INTEGER CHECK (due_at >= 0),
    status TEXT NOT NULL CHECK (status IN ('candidate', 'todo', 'in_progress', 'review', 'rework', 'completed', 'terminated')),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

CREATE INDEX idx_business_tasks_phase_status
    ON business_tasks(phase_id, status, priority, updated_at DESC, business_task_id);
CREATE INDEX idx_business_tasks_assignee
    ON business_tasks(assignee_type, assignee_label, status, updated_at DESC);

CREATE TABLE business_task_transitions (
    transition_id TEXT PRIMARY KEY CHECK (length(transition_id) > 0),
    business_task_id TEXT NOT NULL REFERENCES business_tasks(business_task_id) ON DELETE RESTRICT,
    from_status TEXT NOT NULL,
    to_status TEXT NOT NULL,
    actor_label TEXT NOT NULL CHECK (length(actor_label) > 0),
    comment TEXT NOT NULL,
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE INDEX idx_business_task_transitions_task
    ON business_task_transitions(business_task_id, created_at, transition_id);

CREATE TABLE phase_transitions (
    transition_id TEXT PRIMARY KEY CHECK (length(transition_id) > 0),
    phase_id TEXT NOT NULL REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    from_status TEXT NOT NULL,
    to_status TEXT NOT NULL,
    actor_label TEXT NOT NULL CHECK (length(actor_label) > 0),
    comment TEXT NOT NULL,
    acceptance_criteria_snapshot TEXT,
    accepted_deliverable_version_id TEXT REFERENCES deliverable_versions(version_id) ON DELETE RESTRICT,
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE INDEX idx_phase_transitions_phase
    ON phase_transitions(phase_id, created_at, transition_id);
