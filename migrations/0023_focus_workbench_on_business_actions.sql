DROP TRIGGER trg_my_work_attention_insert;
DROP TRIGGER trg_my_work_attention_update;
DELETE FROM my_work_items WHERE source_type='agent_run_incident';

CREATE TRIGGER trg_my_work_attention_insert
AFTER INSERT ON attention_cases
WHEN NEW.status <> 'resolved' AND NEW.case_type <> 'agent_run_incident'
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
    WHERE project.project_id = NEW.project_id AND NEW.status <> 'resolved' AND NEW.case_type <> 'agent_run_incident';
END;
