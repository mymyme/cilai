UPDATE business_tasks
SET assignee_type='collaboration',
    assignee_label='AI 对话协作',
    state_revision=state_revision+1
WHERE client_operation_id LIKE 'ai-task-create:%'
  AND assignee_type='agent';
