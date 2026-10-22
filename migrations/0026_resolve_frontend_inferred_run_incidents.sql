INSERT INTO attention_case_transitions(
    transition_id, attention_case_id, from_status, to_status, actor_label,
    resolution_summary, decision_action, delegated_to,
    client_operation_id, request_fingerprint, created_at
)
SELECT
    'migration-26-resolve:' || attention_case_id,
    attention_case_id,
    status,
    'resolved',
    '系统',
    '历史运行仍保留；该异常由前端按对话当前任务推断归属，不再作为任务阻塞依据。',
    NULL,
    NULL,
    'migration-26-resolve:' || attention_case_id,
    request_fingerprint,
    updated_at
FROM attention_cases
WHERE case_type = 'agent_run_incident'
  AND status <> 'resolved'
  AND client_operation_id LIKE 'desktop-agent-incident-%';

UPDATE attention_cases
SET status = 'resolved',
    resolution_summary = '历史运行仍保留；该异常由前端按对话当前任务推断归属，不再作为任务阻塞依据。',
    state_revision = state_revision + 1,
    resolved_at = updated_at
WHERE case_type = 'agent_run_incident'
  AND status <> 'resolved'
  AND client_operation_id LIKE 'desktop-agent-incident-%';
