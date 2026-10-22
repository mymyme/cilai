CREATE TABLE ai_skill_installations (
    run_id TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending','historical','ignored','invalid','failed','installed')),
    source_json TEXT NOT NULL CHECK (json_valid(source_json)),
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json))
) STRICT;

CREATE INDEX ai_skill_installations_pending ON ai_skill_installations(status,run_id);

INSERT INTO ai_skill_installations(run_id,status,source_json)
WITH completed AS MATERIALIZED (
    SELECT json_extract(run.value,'$.runId') AS run_id,
           json_extract(run.value,'$.conversationId') AS conversation_id,
           json_extract(run.value,'$.superseded') AS superseded,
           COALESCE(json_extract(run.value,'$.artifacts'),'[]') AS artifacts
    FROM ai_runtime_state state, json_each(state.state_json,'$.runs') run
    WHERE json_extract(run.value,'$.status')='completed'
      AND json_extract(run.value,'$.runId') IS NOT NULL
), replies AS MATERIALIZED (
    SELECT json_extract(message.value,'$.runId') AS run_id, message.value AS message_json,
           ROW_NUMBER() OVER (PARTITION BY json_extract(message.value,'$.runId') ORDER BY CAST(message.key AS INTEGER) DESC) AS position
    FROM ai_runtime_state state, json_each(state.state_json,'$.messages') message
    WHERE json_extract(message.value,'$.role')='assistant'
)
SELECT run.run_id, 'historical',
       json_object(
           'runId',run.run_id,
           'conversationId',run.conversation_id,
           'superseded',run.superseded,
           'workspaceRoot',json_extract(conversation.conversation_json,'$.context.workspaceRoot'),
           'artifacts',json(run.artifacts),
           'message',json(reply.message_json))
FROM completed run
LEFT JOIN replies reply ON reply.run_id=run.run_id AND reply.position=1
LEFT JOIN ai_conversations conversation ON conversation.conversation_id=run.conversation_id;
