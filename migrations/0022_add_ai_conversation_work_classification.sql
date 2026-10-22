DROP TRIGGER trg_my_work_task_insert;
DROP TRIGGER trg_my_work_task_update;
DROP TRIGGER trg_my_work_deliverable_version_insert;
DROP TRIGGER trg_my_work_deliverable_version_update;
DROP TRIGGER trg_my_work_phase_update;
DROP TRIGGER trg_my_work_attention_insert;
DROP TRIGGER trg_my_work_attention_update;

CREATE TABLE business_tasks_v22 (
    business_task_id TEXT PRIMARY KEY CHECK (length(business_task_id) > 0),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    phase_id TEXT REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(title) > 0),
    objective TEXT NOT NULL CHECK (length(objective) > 0),
    input_context TEXT NOT NULL,
    expected_output TEXT NOT NULL CHECK (length(expected_output) > 0),
    completion_criteria TEXT NOT NULL CHECK (length(completion_criteria) > 0),
    assignee_type TEXT NOT NULL CHECK (assignee_type IN ('user', 'agent', 'collaboration')),
    assignee_label TEXT NOT NULL CHECK (length(assignee_label) > 0),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high', 'critical')),
    due_at INTEGER CHECK (due_at >= 0),
    status TEXT NOT NULL CHECK (status IN ('candidate', 'todo', 'in_progress', 'review', 'decision', 'rework', 'completed', 'terminated')),
    state_revision INTEGER NOT NULL CHECK (state_revision >= 0),
    client_operation_id TEXT NOT NULL UNIQUE CHECK (length(client_operation_id) > 0),
    request_fingerprint TEXT NOT NULL CHECK (length(request_fingerprint) > 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    attempt_no INTEGER NOT NULL DEFAULT 1 CHECK (attempt_no > 0)
) STRICT;

INSERT INTO business_tasks_v22 SELECT * FROM business_tasks;
DROP TABLE business_tasks;
ALTER TABLE business_tasks_v22 RENAME TO business_tasks;

CREATE INDEX idx_business_tasks_phase_status
    ON business_tasks(phase_id, status, priority, updated_at DESC, business_task_id);
CREATE INDEX idx_business_tasks_project_status
    ON business_tasks(project_id, status, updated_at DESC, business_task_id);
CREATE INDEX idx_business_tasks_assignee
    ON business_tasks(assignee_type, assignee_label, status, updated_at DESC);

CREATE TABLE conversation_task_bindings (
    conversation_id TEXT PRIMARY KEY CHECK (length(conversation_id) > 0),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    business_task_id TEXT NOT NULL REFERENCES business_tasks(business_task_id) ON DELETE RESTRICT,
    last_run_id TEXT NOT NULL CHECK (length(last_run_id) > 0),
    source_revision INTEGER NOT NULL CHECK (source_revision >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0)
) STRICT;

CREATE INDEX idx_conversation_task_bindings_task
    ON conversation_task_bindings(business_task_id, updated_at DESC, conversation_id);

CREATE TABLE conversation_work_classifications (
    classification_id TEXT PRIMARY KEY CHECK (length(classification_id) > 0),
    run_id TEXT NOT NULL UNIQUE CHECK (length(run_id) > 0),
    conversation_id TEXT NOT NULL CHECK (length(conversation_id) > 0),
    project_id TEXT REFERENCES projects(project_id) ON DELETE RESTRICT,
    business_task_id TEXT REFERENCES business_tasks(business_task_id) ON DELETE RESTRICT,
    classification_action TEXT NOT NULL CHECK (classification_action IN ('apply', 'keep', 'unclassified')),
    task_action TEXT NOT NULL CHECK (task_action IN ('keep', 'link', 'create', 'none')),
    stage TEXT CHECK (stage IN ('todo', 'in_progress', 'review', 'decision')),
    reason TEXT NOT NULL CHECK (length(reason) > 0),
    confidence REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    raw_json TEXT NOT NULL CHECK (json_valid(raw_json)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK ((classification_action = 'unclassified') = (business_task_id IS NULL)),
    CHECK ((classification_action = 'unclassified') = (stage IS NULL)),
    CHECK ((classification_action = 'unclassified') = (task_action = 'none'))
) STRICT;

CREATE INDEX idx_conversation_work_classifications_conversation
    ON conversation_work_classifications(conversation_id, created_at DESC, classification_id);

CREATE TABLE my_work_items_v22 (
    work_item_id TEXT PRIMARY KEY CHECK (length(work_item_id) > 0),
    source_type TEXT NOT NULL CHECK (source_type IN ('business_task', 'deliverable_version', 'project_phase', 'project_blocker', 'decision_request', 'agent_run_incident')),
    source_id TEXT NOT NULL CHECK (length(source_id) > 0),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    project_name TEXT NOT NULL CHECK (length(project_name) > 0),
    primary_workspace_id TEXT REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    phase_id TEXT REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    phase_name TEXT,
    task_id TEXT REFERENCES business_tasks(business_task_id) ON DELETE RESTRICT,
    deliverable_id TEXT REFERENCES deliverables(deliverable_id) ON DELETE RESTRICT,
    deliverable_title TEXT,
    attention_case_id TEXT REFERENCES attention_cases(attention_case_id) ON DELETE RESTRICT,
    title TEXT NOT NULL CHECK (length(title) > 0),
    action_type TEXT NOT NULL CHECK (action_type IN ('task_triage', 'task_execute', 'task_review', 'task_decision', 'deliverable_review', 'phase_review', 'blocker_resolve', 'decision_respond', 'agent_incident_review')),
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

INSERT INTO my_work_items_v22 SELECT * FROM my_work_items;
DROP TABLE my_work_items;
ALTER TABLE my_work_items_v22 RENAME TO my_work_items;

CREATE INDEX idx_my_work_items_focus_order ON my_work_items(focus_group, priority, due_at, updated_at DESC, work_item_id);
CREATE INDEX idx_my_work_items_project ON my_work_items(project_id, focus_group, updated_at DESC, work_item_id);
CREATE INDEX idx_my_work_items_responsibility ON my_work_items(responsibility_type, responsibility_label, focus_group, updated_at DESC);

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
        CASE WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'task_triage' WHEN NEW.status = 'in_progress' THEN 'task_execute' WHEN NEW.status = 'review' THEN 'task_review' WHEN NEW.status = 'decision' THEN 'task_decision' END,
        CASE WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'todo' ELSE NEW.status END,
        NEW.assignee_type, NEW.assignee_label, NEW.priority, NEW.due_at, NEW.state_revision, NEW.updated_at
    FROM projects project
    LEFT JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN project_phases phase ON phase.phase_id = NEW.phase_id
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
        CASE WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'task_triage' WHEN NEW.status = 'in_progress' THEN 'task_execute' WHEN NEW.status = 'review' THEN 'task_review' WHEN NEW.status = 'decision' THEN 'task_decision' END,
        CASE WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'todo' ELSE NEW.status END,
        NEW.assignee_type, NEW.assignee_label, NEW.priority, NEW.due_at, NEW.state_revision, NEW.updated_at
    FROM projects project
    LEFT JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN project_phases phase ON phase.phase_id = NEW.phase_id
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = phase.required_deliverable_id
    WHERE project.project_id = NEW.project_id AND NEW.status NOT IN ('completed', 'terminated');
END;

CREATE TRIGGER trg_my_work_deliverable_version_insert
AFTER INSERT ON deliverable_versions
WHEN NEW.status = 'candidate'
BEGIN
    INSERT INTO my_work_items(work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, deliverable_id, deliverable_title, title, action_type, focus_group, responsibility_type, responsibility_label, priority, source_revision, updated_at)
    SELECT 'deliverable-version:' || NEW.version_id, 'deliverable_version', NEW.version_id, deliverable.project_id, project.display_name, project_workspace.workspace_id, deliverable.deliverable_id, deliverable.title, '验收交付物：' || deliverable.title, 'deliverable_review', 'review', 'user', deliverable.owner_label, 'high', NEW.version_no, NEW.created_at
    FROM deliverables deliverable
    JOIN projects project ON project.project_id = deliverable.project_id
    LEFT JOIN project_workspaces project_workspace ON project_workspace.project_id = deliverable.project_id AND project_workspace.role = 'primary'
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
    INSERT INTO my_work_items(work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, phase_id, phase_name, deliverable_id, deliverable_title, title, action_type, focus_group, responsibility_type, responsibility_label, priority, source_revision, updated_at)
    SELECT 'phase:' || NEW.phase_id, 'project_phase', NEW.phase_id, NEW.project_id, project.display_name, project_workspace.workspace_id, NEW.phase_id, NEW.name, NEW.required_deliverable_id, deliverable.title, '验收阶段：' || NEW.name, 'phase_review', 'decision', 'project_owner', '项目负责人', 'critical', NEW.state_revision, NEW.updated_at
    FROM projects project
    LEFT JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = NEW.required_deliverable_id
    WHERE project.project_id = NEW.project_id AND NEW.status = 'review';
END;

CREATE TRIGGER trg_my_work_attention_insert
AFTER INSERT ON attention_cases
WHEN NEW.status <> 'resolved'
BEGIN
    INSERT INTO my_work_items(work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, phase_id, phase_name, task_id, deliverable_id, deliverable_title, attention_case_id, title, action_type, focus_group, responsibility_type, responsibility_label, priority, due_at, source_revision, updated_at)
    SELECT 'attention:' || NEW.attention_case_id, NEW.case_type, NEW.attention_case_id, NEW.project_id, project.display_name, project_workspace.workspace_id, NEW.phase_id, phase.name, NEW.task_id, NEW.deliverable_id, deliverable.title, NEW.attention_case_id,
        CASE NEW.case_type WHEN 'project_blocker' THEN '解除阻塞：' || NEW.title WHEN 'decision_request' THEN '作出决策：' || NEW.title WHEN 'agent_run_incident' THEN '处理智能体异常：' || NEW.title END,
        CASE NEW.case_type WHEN 'project_blocker' THEN 'blocker_resolve' WHEN 'decision_request' THEN 'decision_respond' WHEN 'agent_run_incident' THEN 'agent_incident_review' END,
        CASE WHEN NEW.case_type = 'decision_request' THEN 'decision' WHEN NEW.case_type = 'agent_run_incident' AND NEW.status = 'acknowledged' THEN 'in_progress' ELSE 'todo' END,
        NEW.responsibility_type, NEW.responsibility_label, NEW.priority, NEW.due_at, NEW.state_revision, NEW.updated_at
    FROM projects project
    LEFT JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN project_phases phase ON phase.phase_id = NEW.phase_id
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = NEW.deliverable_id
    WHERE project.project_id = NEW.project_id;
END;

CREATE TRIGGER trg_my_work_attention_update
AFTER UPDATE OF status, title, responsibility_type, responsibility_label, priority, due_at, state_revision, updated_at ON attention_cases
BEGIN
    DELETE FROM my_work_items WHERE work_item_id = 'attention:' || NEW.attention_case_id;
    INSERT INTO my_work_items(work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, phase_id, phase_name, task_id, deliverable_id, deliverable_title, attention_case_id, title, action_type, focus_group, responsibility_type, responsibility_label, priority, due_at, source_revision, updated_at)
    SELECT 'attention:' || NEW.attention_case_id, NEW.case_type, NEW.attention_case_id, NEW.project_id, project.display_name, project_workspace.workspace_id, NEW.phase_id, phase.name, NEW.task_id, NEW.deliverable_id, deliverable.title, NEW.attention_case_id,
        CASE NEW.case_type WHEN 'project_blocker' THEN '解除阻塞：' || NEW.title WHEN 'decision_request' THEN '作出决策：' || NEW.title WHEN 'agent_run_incident' THEN '处理智能体异常：' || NEW.title END,
        CASE NEW.case_type WHEN 'project_blocker' THEN 'blocker_resolve' WHEN 'decision_request' THEN 'decision_respond' WHEN 'agent_run_incident' THEN 'agent_incident_review' END,
        CASE WHEN NEW.case_type = 'decision_request' THEN 'decision' WHEN NEW.case_type = 'agent_run_incident' AND NEW.status = 'acknowledged' THEN 'in_progress' ELSE 'todo' END,
        NEW.responsibility_type, NEW.responsibility_label, NEW.priority, NEW.due_at, NEW.state_revision, NEW.updated_at
    FROM projects project
    LEFT JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN project_phases phase ON phase.phase_id = NEW.phase_id
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = NEW.deliverable_id
    WHERE project.project_id = NEW.project_id AND NEW.status <> 'resolved';
END;
