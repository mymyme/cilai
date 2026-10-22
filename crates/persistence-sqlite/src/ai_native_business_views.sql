DROP VIEW ai_business_conversations;
DROP VIEW ai_business_messages;
DROP VIEW ai_business_runs;
CREATE VIEW ai_business_conversations AS
SELECT ledger.record_id AS conversation_id,ledger.position,
       json_set(ledger.record_json,'$.title',native.title,'$.archived',json(CASE native.archived WHEN 1 THEN 'true' ELSE 'false' END)) AS conversation_json
FROM ai_conversation_ledger ledger JOIN ai_native_thread_index native ON native.conversation_id=ledger.record_id;
CREATE VIEW ai_business_messages AS
SELECT record_id AS message_id,position,record_json AS message_json,conversation_id,run_id,json_extract(record_json,'$.role') AS role
FROM ai_message_ledger;
CREATE VIEW ai_business_runs AS
SELECT record_id AS run_id,position,record_json AS run_json,conversation_id,status
FROM ai_run_ledger;
CREATE TRIGGER retired_ai_conversations_insert BEFORE INSERT ON ai_conversations BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_conversations_update BEFORE UPDATE ON ai_conversations BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_conversations_delete BEFORE DELETE ON ai_conversations BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_messages_insert BEFORE INSERT ON ai_messages BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_messages_update BEFORE UPDATE ON ai_messages BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_messages_delete BEFORE DELETE ON ai_messages BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_runs_insert BEFORE INSERT ON ai_runs BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_runs_update BEFORE UPDATE ON ai_runs BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_runs_delete BEFORE DELETE ON ai_runs BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_runtime_state_insert BEFORE INSERT ON ai_runtime_state BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_runtime_state_update BEFORE UPDATE ON ai_runtime_state BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_runtime_state_delete BEFORE DELETE ON ai_runtime_state BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_runtime_revision_insert BEFORE INSERT ON ai_runtime_revision BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_runtime_revision_update BEFORE UPDATE ON ai_runtime_revision BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
CREATE TRIGGER retired_ai_runtime_revision_delete BEFORE DELETE ON ai_runtime_revision BEGIN SELECT RAISE(ABORT,'ai_legacy_storage_retired'); END;
