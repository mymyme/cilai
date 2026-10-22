CREATE TABLE my_work_items (
    work_item_id TEXT PRIMARY KEY CHECK (length(work_item_id) > 0),
    source_type TEXT NOT NULL CHECK (source_type IN ('business_task', 'deliverable_version', 'project_phase')),
    source_id TEXT NOT NULL CHECK (length(source_id) > 0),
    project_id TEXT NOT NULL REFERENCES projects(project_id) ON DELETE RESTRICT,
    project_name TEXT NOT NULL CHECK (length(project_name) > 0),
    primary_workspace_id TEXT NOT NULL REFERENCES workspaces(workspace_id) ON DELETE RESTRICT,
    phase_id TEXT REFERENCES project_phases(phase_id) ON DELETE RESTRICT,
    phase_name TEXT,
    task_id TEXT REFERENCES business_tasks(business_task_id) ON DELETE RESTRICT,
    deliverable_id TEXT REFERENCES deliverables(deliverable_id) ON DELETE RESTRICT,
    deliverable_title TEXT,
    title TEXT NOT NULL CHECK (length(title) > 0),
    action_type TEXT NOT NULL CHECK (action_type IN ('task_triage', 'task_execute', 'task_review', 'deliverable_review', 'phase_review')),
    focus_group TEXT NOT NULL CHECK (focus_group IN ('todo', 'in_progress', 'review', 'decision')),
    responsibility_type TEXT NOT NULL CHECK (responsibility_type IN ('user', 'agent', 'collaboration', 'project_owner')),
    responsibility_label TEXT NOT NULL CHECK (length(responsibility_label) > 0),
    priority TEXT NOT NULL CHECK (priority IN ('low', 'medium', 'high', 'critical')),
    due_at INTEGER CHECK (due_at >= 0),
    source_revision INTEGER NOT NULL CHECK (source_revision >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE(source_type, source_id),
    CHECK ((phase_id IS NULL) = (phase_name IS NULL)),
    CHECK ((deliverable_id IS NULL) = (deliverable_title IS NULL))
) STRICT;

CREATE INDEX idx_my_work_items_focus_order
    ON my_work_items(focus_group, priority, due_at, updated_at DESC, work_item_id);
CREATE INDEX idx_my_work_items_project
    ON my_work_items(project_id, focus_group, updated_at DESC, work_item_id);
CREATE INDEX idx_my_work_items_responsibility
    ON my_work_items(responsibility_type, responsibility_label, focus_group, updated_at DESC);

INSERT INTO my_work_items(
    work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, phase_id, phase_name,
    task_id, deliverable_id, deliverable_title, title, action_type, focus_group,
    responsibility_type, responsibility_label, priority, due_at, source_revision, updated_at
)
SELECT
    'task:' || task.business_task_id,
    'business_task',
    task.business_task_id,
    task.project_id,
    project.display_name,
    project_workspace.workspace_id,
    task.phase_id,
    phase.name,
    task.business_task_id,
    phase.required_deliverable_id,
    deliverable.title,
    task.title,
    CASE
        WHEN task.status IN ('candidate', 'todo', 'rework') THEN 'task_triage'
        WHEN task.status = 'in_progress' THEN 'task_execute'
        WHEN task.status = 'review' THEN 'task_review'
    END,
    CASE
        WHEN task.status IN ('candidate', 'todo', 'rework') THEN 'todo'
        WHEN task.status = 'in_progress' THEN 'in_progress'
        WHEN task.status = 'review' THEN 'review'
    END,
    task.assignee_type,
    task.assignee_label,
    task.priority,
    task.due_at,
    task.state_revision,
    task.updated_at
FROM business_tasks task
JOIN projects project ON project.project_id = task.project_id
JOIN project_workspaces project_workspace ON project_workspace.project_id = task.project_id AND project_workspace.role = 'primary'
JOIN project_phases phase ON phase.phase_id = task.phase_id
LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = phase.required_deliverable_id
WHERE task.status NOT IN ('completed', 'terminated');

INSERT INTO my_work_items(
    work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id,
    deliverable_id, deliverable_title, title, action_type, focus_group,
    responsibility_type, responsibility_label, priority, source_revision, updated_at
)
SELECT
    'deliverable-version:' || version.version_id,
    'deliverable_version',
    version.version_id,
    deliverable.project_id,
    project.display_name,
    project_workspace.workspace_id,
    deliverable.deliverable_id,
    deliverable.title,
    '验收交付物：' || deliverable.title,
    'deliverable_review',
    'review',
    'user',
    deliverable.owner_label,
    'high',
    version.version_no,
    version.created_at
FROM deliverable_versions version
JOIN deliverables deliverable ON deliverable.deliverable_id = version.deliverable_id
JOIN projects project ON project.project_id = deliverable.project_id
JOIN project_workspaces project_workspace ON project_workspace.project_id = deliverable.project_id AND project_workspace.role = 'primary'
WHERE version.status = 'candidate';

INSERT INTO my_work_items(
    work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, phase_id, phase_name,
    deliverable_id, deliverable_title, title, action_type, focus_group,
    responsibility_type, responsibility_label, priority, source_revision, updated_at
)
SELECT
    'phase:' || phase.phase_id,
    'project_phase',
    phase.phase_id,
    phase.project_id,
    project.display_name,
    project_workspace.workspace_id,
    phase.phase_id,
    phase.name,
    phase.required_deliverable_id,
    deliverable.title,
    '验收阶段：' || phase.name,
    'phase_review',
    'decision',
    'project_owner',
    '项目负责人',
    'critical',
    phase.state_revision,
    phase.updated_at
FROM project_phases phase
JOIN projects project ON project.project_id = phase.project_id
JOIN project_workspaces project_workspace ON project_workspace.project_id = phase.project_id AND project_workspace.role = 'primary'
LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = phase.required_deliverable_id
WHERE phase.status = 'review';

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
        CASE
            WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'task_triage'
            WHEN NEW.status = 'in_progress' THEN 'task_execute'
            WHEN NEW.status = 'review' THEN 'task_review'
        END,
        CASE
            WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'todo'
            WHEN NEW.status = 'in_progress' THEN 'in_progress'
            WHEN NEW.status = 'review' THEN 'review'
        END,
        NEW.assignee_type, NEW.assignee_label, NEW.priority, NEW.due_at,
        NEW.state_revision, NEW.updated_at
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
        CASE
            WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'task_triage'
            WHEN NEW.status = 'in_progress' THEN 'task_execute'
            WHEN NEW.status = 'review' THEN 'task_review'
        END,
        CASE
            WHEN NEW.status IN ('candidate', 'todo', 'rework') THEN 'todo'
            WHEN NEW.status = 'in_progress' THEN 'in_progress'
            WHEN NEW.status = 'review' THEN 'review'
        END,
        NEW.assignee_type, NEW.assignee_label, NEW.priority, NEW.due_at,
        NEW.state_revision, NEW.updated_at
    FROM projects project
    JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    JOIN project_phases phase ON phase.phase_id = NEW.phase_id
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = phase.required_deliverable_id
    WHERE project.project_id = NEW.project_id
      AND NEW.status NOT IN ('completed', 'terminated');
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
        'phase_review', 'decision', 'project_owner', '项目负责人', 'critical',
        NEW.state_revision, NEW.updated_at
    FROM projects project
    JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = NEW.required_deliverable_id
    WHERE project.project_id = NEW.project_id
      AND NEW.status = 'review';
END;
