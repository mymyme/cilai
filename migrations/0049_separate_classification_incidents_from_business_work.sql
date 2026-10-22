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

CREATE TEMP TABLE classification_incident_recoveries AS
SELECT attention.attention_case_id AS case_id,
       recovered.run_id AS recovered_run_id,
       json_extract(recovered.run_json,'$.completedAt') AS completed_at
FROM attention_cases attention
JOIN ai_runs failed ON failed.run_id=attention.agent_run_id
JOIN ai_conversations conversation ON conversation.conversation_id=failed.conversation_id
JOIN ai_runs recovered ON recovered.run_id=(
    SELECT candidate.run_id FROM ai_runs candidate
    JOIN conversation_work_classifications classification ON classification.run_id=candidate.run_id
    WHERE candidate.conversation_id=failed.conversation_id
      AND classification.conversation_id=failed.conversation_id
      AND classification.project_id=attention.project_id
      AND candidate.status='completed'
      AND json_extract(candidate.run_json,'$.classificationStatus')='applied'
      AND COALESCE(json_extract(candidate.run_json,'$.superseded'),0)=0
      AND json_extract(candidate.run_json,'$.startedAt')>=json_extract(failed.run_json,'$.completedAt')
      AND json_extract(candidate.run_json,'$.completedAt') IS NOT NULL
    ORDER BY json_extract(candidate.run_json,'$.completedAt'),candidate.run_id LIMIT 1
)
WHERE attention.case_type='agent_run_incident'
  AND attention.client_operation_id LIKE 'desktop-classification-incident-%'
  AND attention.task_id IS NULL AND attention.status='open'
  AND failed.status='completed'
  AND json_extract(failed.run_json,'$.classificationStatus')='failed'
  AND json_extract(conversation.conversation_json,'$.context.projectId')=attention.project_id;

INSERT OR IGNORE INTO attention_case_transitions(
    transition_id,attention_case_id,from_status,to_status,actor_label,resolution_summary,
    client_operation_id,request_fingerprint,created_at
)
SELECT 'incident-recovered:'||case_id||':'||recovered_run_id,case_id,'open','resolved','系统',
       '后续回合已成功保存工作归类：'||recovered_run_id||'；历史失败记录保留。',
       'incident-recovered:'||case_id||':'||recovered_run_id,
       'incident-recovered:'||case_id||':'||recovered_run_id,completed_at
FROM classification_incident_recoveries;

UPDATE attention_cases
SET status='resolved',state_revision=state_revision+1,
    resolution_summary=(SELECT '后续回合已成功保存工作归类：'||recovered_run_id||'；历史失败记录保留。' FROM classification_incident_recoveries WHERE case_id=attention_case_id),
    resolved_at=(SELECT completed_at FROM classification_incident_recoveries WHERE case_id=attention_case_id),
    updated_at=(SELECT completed_at FROM classification_incident_recoveries WHERE case_id=attention_case_id)
WHERE attention_case_id IN (SELECT case_id FROM classification_incident_recoveries) AND status='open';

INSERT OR IGNORE INTO durable_events(
    event_id,event_type,object_type,object_id,payload_json,correlation_id,occurred_at,created_at
)
SELECT 'incident-recovered:'||case_id||':'||recovered_run_id,'attention_case.resolved','attention_case',case_id,
       json_object('attentionCaseId',case_id,'recoveredRunId',recovered_run_id,'status','resolved'),
       'incident-recovered:'||case_id||':'||recovered_run_id,completed_at,completed_at
FROM classification_incident_recoveries;
DROP TABLE classification_incident_recoveries;
