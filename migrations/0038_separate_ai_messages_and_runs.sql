-- 消息与运行记录从聚合状态中拆出，按条目独立存储。
-- 聚合状态每次保存都整段重写，历史越长单次写入越慢；拆开后保存只需改动发生变化的条目。
-- 会话编号、运行编号、角色、状态由正文派生，供技能交付、事件恢复和我的工作按条件查询。
CREATE TABLE ai_messages (
    message_id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    message_json TEXT NOT NULL CHECK (json_valid(message_json)),
    conversation_id TEXT GENERATED ALWAYS AS (json_extract(message_json,'$.conversationId')) VIRTUAL,
    run_id TEXT GENERATED ALWAYS AS (json_extract(message_json,'$.runId')) VIRTUAL,
    role TEXT GENERATED ALWAYS AS (json_extract(message_json,'$.role')) VIRTUAL
);
CREATE INDEX ai_messages_by_run ON ai_messages(run_id);
CREATE INDEX ai_messages_by_conversation ON ai_messages(conversation_id);

CREATE TABLE ai_runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    position INTEGER NOT NULL,
    run_json TEXT NOT NULL CHECK (json_valid(run_json)),
    conversation_id TEXT GENERATED ALWAYS AS (json_extract(run_json,'$.conversationId')) VIRTUAL,
    status TEXT GENERATED ALWAYS AS (json_extract(run_json,'$.status')) VIRTUAL
);
CREATE INDEX ai_runs_by_conversation ON ai_runs(conversation_id);
CREATE INDEX ai_runs_by_status ON ai_runs(status);

INSERT INTO ai_messages(message_id,position,message_json)
SELECT json_extract(item.value,'$.messageId'),CAST(item.key AS INTEGER),item.value
FROM ai_runtime_state state,json_each(state.state_json,'$.messages') item
WHERE state.state_key='default';

INSERT INTO ai_runs(run_id,position,run_json)
SELECT json_extract(item.value,'$.runId'),CAST(item.key AS INTEGER),item.value
FROM ai_runtime_state state,json_each(state.state_json,'$.runs') item
WHERE state.state_key='default';

UPDATE ai_runtime_state SET state_json=json_remove(state_json,'$.messages','$.runs');
