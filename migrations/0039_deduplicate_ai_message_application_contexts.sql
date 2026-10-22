-- 消息携带的企业应用上下文（含全部工具定义与参数结构）按内容只存一份。
-- 同一应用在多轮对话里反复被带上时，每条消息原先都各存一整份，占去消息数据的绝大部分。
-- 消息正文里改记上下文编号，读出时由存储层还原，运行时看到的消息内容不变。
CREATE TABLE ai_application_contexts (
    context_id INTEGER PRIMARY KEY,
    context_json TEXT NOT NULL UNIQUE CHECK (json_valid(context_json))
);

INSERT OR IGNORE INTO ai_application_contexts(context_json)
SELECT json_extract(message_json,'$.applicationContext')
FROM ai_messages
WHERE json_type(message_json,'$.applicationContext')='object'
ORDER BY position,message_id;

UPDATE ai_messages
SET message_json=json_set(
    json_remove(message_json,'$.applicationContext'),
    '$.applicationContextId',
    (SELECT context.context_id FROM ai_application_contexts context
     WHERE context.context_json=json_extract(ai_messages.message_json,'$.applicationContext'))
)
WHERE json_type(message_json,'$.applicationContext')='object';
