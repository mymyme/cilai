use super::ai_business_ledger::{apply_batch, digest, reject};
use super::{PersistenceError, SqlitePersistence, database_error};
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::HashSet;

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, PersistenceError> {
    value[key]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| reject("ai_text_recovery_invalid"))
}

impl SqlitePersistence {
    pub(super) fn backup_ai_text_recovery(&self) -> Result<Value, PersistenceError> {
        let directory = self
            .database_path
            .parent()
            .ok_or_else(|| reject("ai_text_recovery_backup_failed"))?
            .join("text-recovery-backups");
        std::fs::create_dir_all(&directory)
            .map_err(|_| reject("ai_text_recovery_backup_failed"))?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| reject("ai_text_recovery_backup_failed"))?
            .as_nanos();
        let path = directory.join(format!("{stamp}.sqlite"));
        self.connection
            .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
            .map_err(database_error("backup text recovery database"))?;
        Ok(json!({"databasePath":path}))
    }

    pub(super) fn recover_ai_text(
        &mut self,
        value: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        if !self.native_ai_storage_active()? {
            return Err(reject("ai_text_recovery_native_required"));
        }
        if digest(&self.ai_business_ledger()?) != text(value, "expectedLedgerDigest")? {
            return Err(reject("ai_text_recovery_source_changed"));
        }
        let transitions = value["transitions"]
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| reject("ai_text_recovery_invalid"))?;
        let operation = text(value, "operationId")?;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin text recovery"))?;
        let receipt = apply_batch(&transaction, &value["ledger"], now)?;
        let mut seen = HashSet::new();
        for change in transitions {
            let id = text(change, "conversationId")?;
            if !seen.insert(id) {
                return Err(reject("ai_text_recovery_invalid"));
            }
            let old_thread = text(change, "previousThreadId")?;
            let new_thread = text(change, "nativeThreadId")?;
            if old_thread == new_thread {
                return Err(reject("ai_text_recovery_invalid"));
            }
            let current: (String, i64, Option<String>) = transaction.query_row("SELECT native_thread_id,generation,active_run_id FROM ai_conversation_thread_bindings WHERE conversation_id=?1", [id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(database_error("verify text recovery binding"))?;
            if current.0 != old_thread || current.2.is_some() {
                return Err(reject("ai_text_recovery_binding_changed"));
            }
            let pending: i64 = transaction.query_row("SELECT count(*) FROM ai_thread_operations WHERE conversation_id=?1 AND state IN ('prepared','dispatched','uncertain')",[id],|r|r.get(0)).map_err(database_error("verify history has no pending operation"))?;
            if pending != 0 {
                return Err(reject("ai_text_recovery_active_writer"));
            }
            let generation = current
                .1
                .checked_add(1)
                .ok_or_else(|| reject("ai_text_recovery_invalid"))?;
            transaction.execute("UPDATE ai_conversation_thread_bindings SET native_thread_id=?2,generation=?3,binding_revision=binding_revision+1,status='ready',owner_instance_id=NULL,updated_at=?4 WHERE conversation_id=?1",params![id,new_thread,generation,now]).map_err(database_error("bind restored native history"))?;
            let create_operation = format!("text-recovery:{operation}:{id}");
            let create_request =
                json!({"conversationId":id,"kind":"create","runId":create_operation,"input":{}});
            transaction.execute("INSERT INTO ai_thread_operations(operation_id,conversation_id,generation,run_id,operation_kind,request_json,state,receipt_json,created_at,updated_at) VALUES(?1,?2,?3,?1,'create',?4,'acknowledged',?5,?6,?6)",params![create_operation,id,generation,create_request.to_string(),json!({"nativeThreadId":new_thread}).to_string(),now]).map_err(database_error("record restored thread identity"))?;
            transaction.execute("INSERT INTO ai_thread_generations(conversation_id,generation,native_thread_id,source_operation_id,created_at) VALUES(?1,?2,?3,?4,?5)",params![id,generation,new_thread,format!("text-recovery:{operation}:{id}"),now]).map_err(database_error("record restored history generation"))?;
            transaction.execute("UPDATE ai_native_thread_index SET native_thread_id=?2,title=?3,archived=?4,updated_at=?5 WHERE conversation_id=?1",params![id,new_thread,text(change,"title")?,change["archived"].as_bool().ok_or_else(||reject("ai_text_recovery_invalid"))?,now]).map_err(database_error("index restored native history"))?;
            let runs = change["runs"]
                .as_array()
                .ok_or_else(|| reject("ai_text_recovery_invalid"))?;
            let expected: i64 = transaction.query_row("SELECT count(*) FROM ai_run_ledger WHERE json_extract(record_json,'$.conversationId')=?1",[id],|r|r.get(0)).map_err(database_error("count restored history runs"))?;
            if usize::try_from(expected).ok() != Some(runs.len()) {
                return Err(reject("ai_text_recovery_incomplete"));
            }
            let mut run_ids = HashSet::new();
            for run in runs {
                let run_id = text(run, "runId")?;
                let belongs: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM ai_run_ledger WHERE record_id=?1 AND json_extract(record_json,'$.conversationId')=?2)",params![run_id,id],|r|r.get(0)).map_err(database_error("verify restored run identity"))?;
                if !belongs || !run_ids.insert(run_id) {
                    return Err(reject("ai_text_recovery_incomplete"));
                }
                let changed = transaction.execute("UPDATE ai_run_native_refs SET native_thread_id=?2,native_turn_id=?3 WHERE run_id=?1 AND native_thread_id=?4",params![text(run,"runId")?,new_thread,text(run,"nativeTurnId")?,old_thread]).map_err(database_error("restore native run reference"))?;
                if changed != 1 {
                    return Err(reject("ai_text_recovery_incomplete"));
                }
            }
            let messages = change["messages"]
                .as_array()
                .ok_or_else(|| reject("ai_text_recovery_invalid"))?;
            let expected: i64 = transaction.query_row("SELECT count(*) FROM ai_message_ledger WHERE json_extract(record_json,'$.conversationId')=?1",[id],|r|r.get(0)).map_err(database_error("count restored history messages"))?;
            if usize::try_from(expected).ok() != Some(messages.len()) {
                return Err(reject("ai_text_recovery_incomplete"));
            }
            let mut message_ids = HashSet::new();
            for message in messages {
                let message_id = text(message, "messageId")?;
                let belongs: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM ai_message_ledger WHERE record_id=?1 AND json_extract(record_json,'$.conversationId')=?2)",params![message_id,id],|r|r.get(0)).map_err(database_error("verify restored message identity"))?;
                if !belongs || !message_ids.insert(message_id) {
                    return Err(reject("ai_text_recovery_incomplete"));
                }
                transaction.execute("INSERT INTO ai_message_native_refs(message_id,native_thread_id,native_turn_id,native_item_id,body_format) VALUES(?1,?2,?3,?1,'plain') ON CONFLICT(message_id) DO UPDATE SET native_thread_id=excluded.native_thread_id,native_turn_id=excluded.native_turn_id,native_item_id=excluded.native_item_id,body_format=excluded.body_format",params![message_id,new_thread,text(message,"nativeTurnId")?]).map_err(database_error("restore native message reference"))?;
                if message["role"] == "user" {
                    let op = format!("text-recovery:{operation}:{message_id}");
                    let request = json!({"conversationId":id,"kind":"send","runId":message["runId"],"input":{"messageId":message_id,"clientMessageId":message_id,"contentDigest":message["contentDigest"]}});
                    let ack = json!({"nativeThreadId":new_thread,"nativeTurnId":message["nativeTurnId"],"clientMessageId":message_id});
                    transaction.execute("INSERT INTO ai_thread_operations(operation_id,conversation_id,generation,run_id,operation_kind,request_json,state,receipt_json,created_at,updated_at) VALUES(?1,?2,?3,?4,'send',?5,'acknowledged',?6,?7,?7)",params![op,id,generation,text(message,"runId")?,request.to_string(),ack.to_string(),now]).map_err(database_error("record restored message receipt"))?;
                    transaction.execute("INSERT INTO ai_thread_message_links(conversation_id,generation,message_id,client_message_id,content_digest,native_thread_id,native_turn_id,operation_id,accepted_at) VALUES(?1,?2,?3,?3,?4,?5,?6,?7,?8)",params![id,generation,message_id,text(message,"contentDigest")?,new_thread,text(message,"nativeTurnId")?,op,now]).map_err(database_error("record restored message mapping"))?;
                }
            }
        }
        transaction
            .commit()
            .map_err(database_error("commit text recovery"))?;
        Ok(receipt)
    }
}
