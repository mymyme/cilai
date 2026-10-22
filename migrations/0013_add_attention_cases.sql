CREATE TABLE attention_cases (
    attention_case_id TEXT PRIMARY KEY CHECK (length(attention_case_id) > 0),
    case_type TEXT NOT NULL CHECK (case_type IN ('project_blocker', 'decision_request', 'agent_run_incident')),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    phase_id TEXT REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    task_id TEXT REFERENCES business_tasks(business_task_id) ON DELETE RESTRICT,
    deliverable_id TEXT REFERENCES deliverables(deliverable_id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(title) > 0),
    detail TEXT NOT NULL CHECK (length(detail) > 0),
    status TEXT NOT NULL CHECK (status IN ('open', 'acknowledged', 'resolved')),
    responsibility_type TEXT NOT NULL CHECK (responsibility_type IN ('user', 'collaboration', 'project_owner')),
    responsibility_label TEXT NOT NULL CHECK (length(responsibility_label) > 0),
    reporter_label TEXT NOT NULL CHECK (length(reporter_label) > 0),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high', 'critical')),
    due_at INTEGER CHECK (due_at >= 0),
    agent_run_id TEXT,
    agent_label TEXT,
    resolution_summary TEXT,
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    resolved_at INTEGER CHECK (resolved_at >= 0),
    CHECK ((case_type = 'agent_run_incident') = (agent_run_id IS NOT NULL)),
    CHECK ((case_type = 'agent_run_incident') = (agent_label IS NOT NULL)),
    CHECK ((status = 'resolved') = (resolved_at IS NOT NULL)),
    CHECK ((status = 'resolved') = (resolution_summary IS NOT NULL))
) STRICT;

CREATE UNIQUE INDEX idx_attention_cases_agent_run
    ON attention_cases(project_id, agent_run_id)
    WHERE case_type = 'agent_run_incident';
CREATE INDEX idx_attention_cases_project_status
    ON attention_cases(project_id, status, priority, updated_at DESC, attention_case_id);
CREATE INDEX idx_attention_cases_phase_status
    ON attention_cases(phase_id, status, updated_at DESC, attention_case_id)
    WHERE phase_id IS NOT NULL;

CREATE TABLE attention_case_transitions (
    transition_id TEXT PRIMARY KEY CHECK (length(transition_id) > 0),
    attention_case_id TEXT NOT NULL REFERENCES attention_cases(attention_case_id) ON DELETE RESTRICT,
    from_status TEXT NOT NULL CHECK (from_status IN ('open', 'acknowledged')),
    to_status TEXT NOT NULL CHECK (to_status IN ('acknowledged', 'resolved')),
    actor_label TEXT NOT NULL CHECK (length(actor_label) > 0),
    resolution_summary TEXT NOT NULL,
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0)
) STRICT;

CREATE INDEX idx_attention_case_transitions_case
    ON attention_case_transitions(attention_case_id, created_at, transition_id);

DROP TRIGGER trg_my_work_task_insert;
DROP TRIGGER trg_my_work_task_update;
DROP TRIGGER trg_my_work_deliverable_version_insert;
DROP TRIGGER trg_my_work_deliverable_version_update;
DROP TRIGGER trg_my_work_phase_update;

CREATE TABLE my_work_items_v13 (
    work_item_id TEXT PRIMARY KEY CHECK (length(work_item_id) > 0),
    source_type TEXT NOT NULL CHECK (source_type IN ('business_task', 'deliverable_version', 'project_phase', 'project_blocker', 'decision_request', 'agent_run_incident')),
    source_id TEXT NOT NULL CHECK (length(source_id) > 0),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    project_name TEXT NOT NULL CHECK (length(project_name) > 0),
    primary_workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    phase_id TEXT REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    phase_name TEXT,
    task_id TEXT REFERENCES business_tasks(business_task_id) ON DELETE RESTRICT,
    deliverable_id TEXT REFERENCES deliverables(deliverable_id) ON DELETE RESTRICT,
    deliverable_title TEXT,
    attention_case_id TEXT REFERENCES attention_cases(attention_case_id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(title) > 0),
    action_type TEXT NOT NULL CHECK (action_type IN ('task_triage', 'task_execute', 'task_review', 'deliverable_review', 'phase_review', 'blocker_resolve', 'decision_respond', 'agent_incident_review')),
    focus_group TEXT NOT NULL CHECK (focus_group IN ('todo', 'in_progress', 'review', 'decision')),
    responsibility_type TEXT NOT NULL CHECK (responsibility_type IN ('user', 'agent', 'collaboration', 'project_owner')),
    responsibility_label TEXT NOT NULL CHECK (length(responsibility_label) > 0),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high', 'critical')),
    due_at INTEGER CHECK (due_at >= 0),
    source_revision INTEGER NOT NULL CHECK (source_revision >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE(source_type, source_id),
    CHECK ((phase_id IS NULL) = (phase_name IS NULL)),
    CHECK ((deliverable_id IS NULL) = (deliverable_title IS NULL)),
    CHECK ((source_type IN ('project_blocker', 'decision_request', 'agent_run_incident')) = (attention_case_id IS NOT NULL))
) STRICT;

INSERT INTO my_work_items_v13(
    work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id,
    phase_id, phase_name, task_id, deliverable_id, deliverable_title, attention_case_id,
    title, action_type, focus_group, responsibility_type, responsibility_label,
    priority, due_at, source_revision, updated_at
)
SELECT
    work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id,
    phase_id, phase_name, task_id, deliverable_id, deliverable_title, NULL,
    title, action_type, focus_group, responsibility_type, responsibility_label,
    priority, due_at, source_revision, updated_at
FROM my_work_items;

DROP TABLE my_work_items;
ALTER TABLE my_work_items_v13 RENAME TO my_work_items;

CREATE INDEX idx_my_work_items_focus_order
    ON my_work_items(focus_group, priority, due_at, updated_at DESC, work_item_id);
CREATE INDEX idx_my_work_items_project
    ON my_work_items(project_id, focus_group, updated_at DESC, work_item_id);
CREATE INDEX idx_my_work_items_responsibility
    ON my_work_items(responsibility_type, responsibility_label, focus_group, updated_at DESC);

CREATE TRIGGER trg_my_work_task_insert
AFTER INSERT ON business_tasks
WHEN NEW.status NOT IN ('completed', 'terminated')
BEGIN
    INSERT INTO my_work_items(
        work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, phase_id, phase_name,
        task_id, deliverable_id, deliverable_title, title, action_type, focus_group,
        responsibility_type, responsibility_label, priority, due_at, source_revision, updated_at
    )
    SELECT
        'task:' || NEW.business_task_id, 'business_task', NEW.business_task_id,
        NEW.project_id, project.display_name, project_workspace.workspace_id, NEW.phase_id, phase.name, NEW.business_task_id,
        phase.required_deliverable_id, deliverable.title, NEW.title,
        CASE WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'task_triage' WHEN NEW.status = 'in_progress' THEN 'task_execute' WHEN NEW.status = 'review' THEN 'task_review' END,
        CASE WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'todo' WHEN NEW.status = 'in_progress' THEN 'in_progress' WHEN NEW.status = 'review' THEN 'review' END,
        NEW.assignee_type, NEW.assignee_label, NEW.priority, NEW.due_at, NEW.state_revision, NEW.updated_at
    FROM projects project
    JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    JOIN project_phases phase ON phase.phase_id = NEW.phase_id
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = phase.required_deliverable_id
    WHERE project.project_id = NEW.project_id;
END;

CREATE TRIGGER trg_my_work_task_update
AFTER UPDATE OF status, title, assignee_type, assignee_label, priority, due_at, state_revision, updated_at ON business_tasks
BEGIN
    DELETE FROM my_work_items WHERE work_item_id = 'task:' || NEW.business_task_id;
    INSERT INTO my_work_items(
        work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, phase_id, phase_name,
        task_id, deliverable_id, deliverable_title, title, action_type, focus_group,
        responsibility_type, responsibility_label, priority, due_at, source_revision, updated_at
    )
    SELECT
        'task:' || NEW.business_task_id, 'business_task', NEW.business_task_id,
        NEW.project_id, project.display_name, project_workspace.workspace_id, NEW.phase_id, phase.name, NEW.business_task_id,
        phase.required_deliverable_id, deliverable.title, NEW.title,
        CASE WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'task_triage' WHEN NEW.status = 'in_progress' THEN 'task_execute' WHEN NEW.status = 'review' THEN 'task_review' END,
        CASE WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'todo' WHEN NEW.status = 'in_progress' THEN 'in_progress' WHEN NEW.status = 'review' THEN 'review' END,
        NEW.assignee_type, NEW.assignee_label, NEW.priority, NEW.due_at, NEW.state_revision, NEW.updated_at
    FROM projects project
    JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    JOIN project_phases phase ON phase.phase_id = NEW.phase_id
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = phase.required_deliverable_id
    WHERE project.project_id = NEW.project_id AND NEW.status NOT IN ('completed', 'terminated');
END;

CREATE TRIGGER trg_my_work_deliverable_version_insert
AFTER INSERT ON deliverable_versions
WHEN NEW.status = 'candidate'
BEGIN
    INSERT INTO my_work_items(
        work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id,
        deliverable_id, deliverable_title, title, action_type, focus_group,
        responsibility_type, responsibility_label, priority, source_revision, updated_at
    )
    SELECT
        'deliverable-version:' || NEW.version_id, 'deliverable_version', NEW.version_id,
        deliverable.project_id, project.display_name, project_workspace.workspace_id, deliverable.deliverable_id, deliverable.title,
        '验收交付物：' || deliverable.title, 'deliverable_review', 'review',
        'user', deliverable.owner_label, 'high', NEW.version_no, NEW.created_at
    FROM deliverables deliverable
    JOIN projects project ON project.project_id = deliverable.project_id
    JOIN project_workspaces project_workspace ON project_workspace.project_id = deliverable.project_id AND project_workspace.role = 'primary'
    WHERE deliverable.deliverable_id = NEW.deliverable_id;
END;

CREATE TRIGGER trg_my_work_deliverable_version_update
AFTER UPDATE OF status ON deliverable_versions
BEGIN
    DELETE FROM my_work_items WHERE work_item_id = 'deliverable-version:' || NEW.version_id;
END;

CREATE TRIGGER trg_my_work_phase_update
AFTER UPDATE OF status, state_revision, updated_at ON project_phases
BEGIN
    DELETE FROM my_work_items WHERE work_item_id = 'phase:' || NEW.phase_id;
    INSERT INTO my_work_items(
        work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, phase_id, phase_name,
        deliverable_id, deliverable_title, title, action_type, focus_group,
        responsibility_type, responsibility_label, priority, source_revision, updated_at
    )
    SELECT
        'phase:' || NEW.phase_id, 'project_phase', NEW.phase_id,
        NEW.project_id, project.display_name, project_workspace.workspace_id, NEW.phase_id, NEW.name,
        NEW.required_deliverable_id, deliverable.title, '验收阶段：' || NEW.name,
        'phase_review', 'decision', 'project_owner', '项目负责人', 'critical', NEW.state_revision, NEW.updated_at
    FROM projects project
    JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = NEW.required_deliverable_id
    WHERE project.project_id = NEW.project_id AND NEW.status = 'review';
END;

CREATE TRIGGER trg_my_work_attention_insert
AFTER INSERT ON attention_cases
WHEN NEW.status <> 'resolved'
BEGIN
    INSERT INTO my_work_items(
        work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id,
        phase_id, phase_name, task_id, deliverable_id, deliverable_title, attention_case_id,
        title, action_type, focus_group, responsibility_type, responsibility_label,
        priority, due_at, source_revision, updated_at
    )
    SELECT
        'attention:' || NEW.attention_case_id, NEW.case_type, NEW.attention_case_id,
        NEW.project_id, project.display_name, project_workspace.workspace_id,
        NEW.phase_id, phase.name, NEW.task_id, NEW.deliverable_id, deliverable.title, NEW.attention_case_id,
        CASE NEW.case_type
            WHEN 'project_blocker' THEN '解除阻塞：' || NEW.title
            WHEN 'decision_request' THEN '作出决策：' || NEW.title
            WHEN 'agent_run_incident' THEN '处理智能体异常：' || NEW.title
        END,
        CASE NEW.case_type WHEN 'project_blocker' THEN 'blocker_resolve' WHEN 'decision_request' THEN 'decision_respond' WHEN 'agent_run_incident' THEN 'agent_incident_review' END,
        CASE WHEN NEW.case_type = 'decision_request' THEN 'decision' WHEN NEW.case_type = 'agent_run_incident' AND NEW.status = 'acknowledged' THEN 'in_progress' ELSE 'todo' END,
        NEW.responsibility_type, NEW.responsibility_label, NEW.priority, NEW.due_at, NEW.state_revision, NEW.updated_at
    FROM projects project
    JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN project_phases phase ON phase.phase_id = NEW.phase_id
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = NEW.deliverable_id
    WHERE project.project_id = NEW.project_id;
END;

CREATE TRIGGER trg_my_work_attention_update
AFTER UPDATE OF status, title, responsibility_type, responsibility_label, priority, due_at, state_revision, updated_at ON attention_cases
BEGIN
    DELETE FROM my_work_items WHERE work_item_id = 'attention:' || NEW.attention_case_id;
    INSERT INTO my_work_items(
        work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id,
        phase_id, phase_name, task_id, deliverable_id, deliverable_title, attention_case_id,
        title, action_type, focus_group, responsibility_type, responsibility_label,
        priority, due_at, source_revision, updated_at
    )
    SELECT
        'attention:' || NEW.attention_case_id, NEW.case_type, NEW.attention_case_id,
        NEW.project_id, project.display_name, project_workspace.workspace_id,
        NEW.phase_id, phase.name, NEW.task_id, NEW.deliverable_id, deliverable.title, NEW.attention_case_id,
        CASE NEW.case_type
            WHEN 'project_blocker' THEN '解除阻塞：' || NEW.title
            WHEN 'decision_request' THEN '作出决策：' || NEW.title
            WHEN 'agent_run_incident' THEN '处理智能体异常：' || NEW.title
        END,
        CASE NEW.case_type WHEN 'project_blocker' THEN 'blocker_resolve' WHEN 'decision_request' THEN 'decision_respond' WHEN 'agent_run_incident' THEN 'agent_incident_review' END,
        CASE WHEN NEW.case_type = 'decision_request' THEN 'decision' WHEN NEW.case_type = 'agent_run_incident' AND NEW.status = 'acknowledged' THEN 'in_progress' ELSE 'todo' END,
        NEW.responsibility_type, NEW.responsibility_label, NEW.priority, NEW.due_at, NEW.state_revision, NEW.updated_at
    FROM projects project
    JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN project_phases phase ON phase.phase_id = NEW.phase_id
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = NEW.deliverable_id
    WHERE project.project_id = NEW.project_id AND NEW.status <> 'resolved';
END;
