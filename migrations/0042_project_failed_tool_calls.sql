INSERT OR IGNORE INTO attention_cases(
    attention_case_id,case_type,project_id,phase_id,task_id,deliverable_id,
    title,detail,status,responsibility_type,responsibility_label,reporter_label,
    priority,due_at,agent_run_id,agent_label,resolution_summary,state_revision,
    client_operation_id,request_fingerprint,created_at,updated_at,resolved_at
)
SELECT
    'tool-failure-incident-' || run.run_id,
    'agent_run_incident',
    classification.project_id,
    CASE WHEN EXISTS (
        SELECT 1 FROM project_phases phase
        WHERE phase.phase_id=json_extract(conversation.conversation_json,'$.context.phaseId')
          AND phase.project_id=classification.project_id
    ) THEN json_extract(conversation.conversation_json,'$.context.phaseId') ELSE NULL END,
    classification.business_task_id,
    NULL,
    '处理工具调用失败：' || COALESCE(
        NULLIF(TRIM(json_extract(conversation.conversation_json,'$.title')),''),
        'AI 对话'
    ),
    COALESCE(
        NULLIF(TRIM((
            SELECT json_extract(event.value,'$.summary')
            FROM json_each(run.run_json,'$.events') event
            WHERE json_extract(event.value,'$.phase')='reply'
              AND json_extract(event.value,'$.kind')='mcp'
              AND json_extract(event.value,'$.eventType')='item.completed'
            ORDER BY CAST(json_extract(event.value,'$.sequence') AS INTEGER) DESC,
                     CAST(event.key AS INTEGER) DESC
            LIMIT 1
        )),''),
        '工具调用失败，需要查看原对话处理。'
    ),
    'open',
    'user',
    '当前用户',
    '词来 桌面端',
    'high',
    NULL,
    run.run_id,
    COALESCE(NULLIF(TRIM(json_extract(run.run_json,'$.model')),''),'AI 助手'),
    NULL,
    0,
    'desktop-tool-failure-incident-' || run.run_id,
    'tool-failure-incident-' || run.run_id,
    COALESCE(json_extract(run.run_json,'$.completedAt'),classification.created_at,0),
    COALESCE(json_extract(run.run_json,'$.completedAt'),classification.created_at,0),
    NULL
FROM conversation_work_classifications classification
JOIN ai_runs run ON run.run_id=classification.run_id
JOIN ai_conversations conversation ON conversation.conversation_id=classification.conversation_id
JOIN projects project ON project.project_id=classification.project_id
JOIN business_tasks task ON task.business_task_id=classification.business_task_id
WHERE classification.business_task_id IS NOT NULL
  AND classification.conversation_status='active'
  AND COALESCE(json_extract(conversation.conversation_json,'$.archived'),0)=0
  AND NOT EXISTS (
      SELECT 1 FROM conversation_work_classifications newer
      WHERE newer.conversation_id=classification.conversation_id
        AND (
            newer.created_at>classification.created_at
            OR (
                newer.created_at=classification.created_at
                AND newer.classification_id>classification.classification_id
            )
        )
  )
  AND (
      SELECT json_extract(event.value,'$.status')
      FROM json_each(run.run_json,'$.events') event
      WHERE json_extract(event.value,'$.phase')='reply'
        AND json_extract(event.value,'$.kind')='mcp'
        AND json_extract(event.value,'$.eventType')='item.completed'
      ORDER BY CAST(json_extract(event.value,'$.sequence') AS INTEGER) DESC,
               CAST(event.key AS INTEGER) DESC
      LIMIT 1
  )='failed'
  AND NOT EXISTS (
      SELECT 1 FROM attention_cases attention
      WHERE attention.project_id=classification.project_id
        AND attention.agent_run_id=run.run_id
        AND attention.case_type='agent_run_incident'
  );
