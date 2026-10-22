DROP TRIGGER trg_my_work_attention_insert;
DROP TRIGGER trg_my_work_attention_update;

CREATE TRIGGER trg_my_work_attention_insert
AFTER INSERT ON attention_cases
WHEN NEW.status <> 'resolved'
 AND (
    NEW.case_type <> 'agent_run_incident'
    OR (
        NEW.task_id IS NULL
        AND NEW.client_operation_id LIKE 'desktop-classification-incident-%'
    )
 )
BEGIN
    INSERT INTO my_work_items(work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id, phase_id, phase_name, task_id, deliverable_id, deliverable_title, attention_case_id, title, action_type, focus_group, responsibility_type, responsibility_label, priority, due_at, source_revision, updated_at)
    SELECT 'attention:' || NEW.attention_case_id, NEW.case_type, NEW.attention_case_id, NEW.project_id, project.display_name, project_workspace.workspace_id, NEW.phase_id, phase.name, NEW.task_id, NEW.deliverable_id, deliverable.title, NEW.attention_case_id,
        CASE NEW.case_type WHEN 'project_blocker' THEN '解除阻塞：' || NEW.title WHEN 'decision_request' THEN '作出决策：' || NEW.title WHEN 'agent_run_incident' THEN NEW.title END,
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
        CASE NEW.case_type WHEN 'project_blocker' THEN '解除阻塞：' || NEW.title WHEN 'decision_request' THEN '作出决策：' || NEW.title WHEN 'agent_run_incident' THEN NEW.title END,
        CASE NEW.case_type WHEN 'project_blocker' THEN 'blocker_resolve' WHEN 'decision_request' THEN 'decision_respond' WHEN 'agent_run_incident' THEN 'agent_incident_review' END,
        CASE WHEN NEW.case_type = 'decision_request' THEN 'decision' WHEN NEW.case_type = 'agent_run_incident' AND NEW.status = 'acknowledged' THEN 'in_progress' ELSE 'todo' END,
        NEW.responsibility_type, NEW.responsibility_label, NEW.priority, NEW.due_at, NEW.state_revision, NEW.updated_at
    FROM projects project
    LEFT JOIN project_workspaces project_workspace ON project_workspace.project_id = NEW.project_id AND project_workspace.role = 'primary'
    LEFT JOIN project_phases phase ON phase.phase_id = NEW.phase_id
    LEFT JOIN deliverables deliverable ON deliverable.deliverable_id = NEW.deliverable_id
    WHERE project.project_id = NEW.project_id
      AND NEW.status <> 'resolved'
      AND (
        NEW.case_type <> 'agent_run_incident'
        OR (
            NEW.task_id IS NULL
            AND NEW.client_operation_id LIKE 'desktop-classification-incident-%'
        )
      );
END;

INSERT OR IGNORE INTO attention_cases(
    attention_case_id, case_type, project_id, phase_id, task_id, deliverable_id,
    title, detail, status, responsibility_type, responsibility_label, reporter_label,
    priority, due_at, agent_run_id, agent_label, resolution_summary, state_revision,
    client_operation_id, request_fingerprint, created_at, updated_at, resolved_at
)
SELECT
    'classification-incident-' || run.run_id,
    'agent_run_incident',
    json_extract(conversation.conversation_json, '$.context.projectId'),
    CASE WHEN EXISTS (
        SELECT 1 FROM project_phases phase
        WHERE phase.phase_id = json_extract(conversation.conversation_json, '$.context.phaseId')
    ) THEN json_extract(conversation.conversation_json, '$.context.phaseId') ELSE NULL END,
    NULL,
    NULL,
    '恢复工作归类：' || COALESCE(NULLIF(TRIM(json_extract(conversation.conversation_json, '$.title')), ''), '待归类对话'),
    COALESCE(NULLIF(TRIM(json_extract(run.run_json, '$.classificationError')), ''), '工作归类失败，本次回复已保留，任务状态未更新。'),
    'open',
    'user',
    '当前用户',
    '词来 桌面端',
    'high',
    NULL,
    run.run_id,
    'AI 工作归类',
    NULL,
    0,
    'desktop-classification-incident-' || run.run_id,
    'classification-incident-' || run.run_id,
    COALESCE(json_extract(run.run_json, '$.completedAt'), json_extract(run.run_json, '$.startedAt'), 0),
    COALESCE(json_extract(run.run_json, '$.completedAt'), json_extract(run.run_json, '$.startedAt'), 0),
    NULL
FROM ai_runs run
JOIN ai_conversations conversation ON conversation.conversation_id = run.conversation_id
JOIN projects project ON project.project_id = json_extract(conversation.conversation_json, '$.context.projectId')
WHERE run.status = 'completed'
  AND json_extract(run.run_json, '$.classificationStatus') = 'failed'
  AND COALESCE(json_extract(conversation.conversation_json, '$.archived'), 0) = 0
  AND NOT EXISTS (
    SELECT 1 FROM attention_cases attention
    WHERE attention.project_id = project.project_id
      AND attention.agent_run_id = run.run_id
      AND attention.case_type = 'agent_run_incident'
  );

DELETE FROM my_work_items
WHERE source_type = 'agent_run_incident'
  AND NOT EXISTS (
    SELECT 1 FROM attention_cases attention
    WHERE attention.attention_case_id = my_work_items.attention_case_id
      AND attention.status <> 'resolved'
      AND attention.task_id IS NULL
      AND attention.client_operation_id LIKE 'desktop-classification-incident-%'
  );

INSERT OR REPLACE INTO my_work_items(
    work_item_id, source_type, source_id, project_id, project_name, primary_workspace_id,
    phase_id, phase_name, task_id, deliverable_id, deliverable_title, attention_case_id,
    title, action_type, focus_group, responsibility_type, responsibility_label, priority,
    due_at, source_revision, updated_at
)
SELECT
    'attention:' || attention.attention_case_id,
    attention.case_type,
    attention.attention_case_id,
    attention.project_id,
    project.display_name,
    project_workspace.workspace_id,
    attention.phase_id,
    phase.name,
    NULL,
    NULL,
    NULL,
    attention.attention_case_id,
    attention.title,
    'agent_incident_review',
    CASE WHEN attention.status = 'acknowledged' THEN 'in_progress' ELSE 'todo' END,
    attention.responsibility_type,
    attention.responsibility_label,
    attention.priority,
    attention.due_at,
    attention.state_revision,
    attention.updated_at
FROM attention_cases attention
JOIN projects project ON project.project_id = attention.project_id
LEFT JOIN project_workspaces project_workspace ON project_workspace.project_id = attention.project_id AND project_workspace.role = 'primary'
LEFT JOIN project_phases phase ON phase.phase_id = attention.phase_id
WHERE attention.case_type = 'agent_run_incident'
  AND attention.status <> 'resolved'
  AND attention.task_id IS NULL
  AND attention.client_operation_id LIKE 'desktop-classification-incident-%';
