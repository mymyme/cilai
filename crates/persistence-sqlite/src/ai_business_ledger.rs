//! 业务元数据按条写入，以条目摘要检测冲突，以命令编号处理重复提交。
use super::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{MODEL_ENTRY_OWNER, MODEL_ENTRY_PREFIX};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const COLLECTIONS: [(&str, &str); 4] = [
    ("conversations", "ai_conversation_ledger"),
    ("messages", "ai_message_ledger"),
    ("runs", "ai_run_ledger"),
    ("entries", "ai_runtime_entries"),
];

pub(super) fn reject(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}

pub(super) fn digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, PersistenceError> {
    value[field]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or_else(|| reject("ai_ledger_request_invalid"))
}

fn table(collection: &str) -> Result<&'static str, PersistenceError> {
    COLLECTIONS
        .iter()
        .find(|(name, _)| *name == collection)
        .map(|(_, table)| *table)
        .ok_or_else(|| reject("ai_ledger_collection_invalid"))
}

fn validate_metadata(collection: &str, id: &str, value: &Value) -> Result<(), PersistenceError> {
    let object = value
        .as_object()
        .ok_or_else(|| reject("ai_ledger_record_invalid"))?;
    let (key, forbidden): (&str, &[&str]) = match collection {
        "conversations" => ("conversationId", &["title", "archived"]),
        "messages" => ("messageId", &["content"]),
        "runs" => (
            "runId",
            &[
                "output",
                "partialOutput",
                "events",
                "logTail",
                "previousRevisions",
                "regenerationDraft",
            ],
        ),
        "entries" => ("entryId", &[]),
        _ => return Err(reject("ai_ledger_collection_invalid")),
    };
    if value[key] != id || forbidden.iter().any(|field| object.contains_key(*field)) {
        return Err(reject("ai_ledger_record_invalid"));
    }
    Ok(())
}

fn verify_text_integrity(
    transaction: &Transaction<'_>,
    table: &str,
    id: &str,
    value: &Value,
) -> Result<(), PersistenceError> {
    let previous: Option<String> = transaction
        .query_row(
            &format!("SELECT record_json FROM {table} WHERE record_id=?1"),
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error("read ledger text integrity baseline"))?;
    if let Some(previous) = previous {
        let previous: Value =
            serde_json::from_str(&previous).map_err(|_| reject("ai_ledger_storage_invalid"))?;
        if super::text_integrity::introduces_replacement(&previous, value) {
            return Err(reject("ai_text_corruption_detected"));
        }
    }
    Ok(())
}

pub(super) fn apply_batch(
    transaction: &Transaction<'_>,
    request: &Value,
    now: i64,
) -> Result<Value, PersistenceError> {
    apply_scoped_batch(transaction, request, now, false)
}

pub(super) fn apply_model_batch(
    transaction: &Transaction<'_>,
    request: &Value,
    now: i64,
) -> Result<Value, PersistenceError> {
    apply_scoped_batch(transaction, request, now, true)
}

fn validate_writer_scope(request: &Value, model_writer: bool) -> Result<(), PersistenceError> {
    let command_id = text(request, "commandId")?;
    if command_id.starts_with(MODEL_ENTRY_PREFIX) != model_writer {
        return Err(reject("ai_ledger_writer_scope_invalid"));
    }
    for record in request["records"]
        .as_array()
        .ok_or_else(|| reject("ai_ledger_request_invalid"))?
    {
        let protected = record["collection"] == "entries"
            && record["id"]
                .as_str()
                .is_some_and(|id| id.starts_with(MODEL_ENTRY_PREFIX));
        let model_owner = record["value"]["owner"] == MODEL_ENTRY_OWNER;
        if model_writer {
            if !protected || (!record["value"].is_null() && !model_owner) {
                return Err(reject("ai_ledger_writer_scope_invalid"));
            }
        } else if protected || model_owner {
            return Err(reject("ai_ledger_writer_scope_invalid"));
        }
    }
    if model_writer
        && ["pendingInputs", "deliveryInputs"].iter().any(|key| {
            request
                .get(*key)
                .is_some_and(|v| !v.as_array().is_some_and(Vec::is_empty))
        })
    {
        return Err(reject("ai_ledger_writer_scope_invalid"));
    }
    Ok(())
}

fn apply_scoped_batch(
    transaction: &Transaction<'_>,
    request: &Value,
    now: i64,
    model_writer: bool,
) -> Result<Value, PersistenceError> {
    validate_writer_scope(request, model_writer)?;
    let command_id = text(request, "commandId")?;
    let request_digest = digest(request);
    let previous: Option<(String, String)> = transaction
        .query_row(
            "SELECT request_digest,result_json FROM ai_ledger_commands WHERE command_id=?1",
            [command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(database_error("read ledger command"))?;
    if let Some((known, result)) = previous {
        if known != request_digest {
            return Err(reject("ai_ledger_command_conflict"));
        }
        return serde_json::from_str(&result).map_err(|_| reject("ai_ledger_storage_invalid"));
    }
    let records = request["records"]
        .as_array()
        .filter(|records| records.len() <= 50_000)
        .ok_or_else(|| reject("ai_ledger_request_invalid"))?;
    let mut seen = HashSet::new();
    let mut receipts = Vec::new();
    for record in records {
        let collection = text(record, "collection")?;
        let table = table(collection)?;
        let id = text(record, "id")?;
        if !seen.insert((collection, id)) {
            return Err(reject("ai_ledger_duplicate_record"));
        }
        let expected = record
            .get("expectedDigest")
            .ok_or_else(|| reject("ai_ledger_request_invalid"))?;
        if !expected.is_null() && !expected.is_string() {
            return Err(reject("ai_ledger_request_invalid"));
        }
        let current: Option<String> = transaction
            .query_row(
                &format!("SELECT digest FROM {table} WHERE record_id=?1"),
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error("read ledger record digest"))?;
        if current.as_deref() != expected.as_str() {
            return Err(reject("ai_ledger_record_conflict"));
        }
        let value = record
            .get("value")
            .ok_or_else(|| reject("ai_ledger_request_invalid"))?;
        if value.is_null() {
            delete_record(transaction, collection, table, id)?;
            receipts.push(json!({"collection":collection,"id":id,"digest":null}));
        } else {
            validate_metadata(collection, id, value)?;
            verify_text_integrity(transaction, table, id, value)?;
            let position = record["position"]
                .as_i64()
                .filter(|p| *p >= 0)
                .ok_or_else(|| reject("ai_ledger_position_invalid"))?;
            if collection == "messages" {
                transaction.execute("DELETE FROM ai_message_native_refs WHERE message_id=?1 AND EXISTS(SELECT 1 FROM ai_message_ledger WHERE record_id=?1 AND json_extract(record_json,'$.contentDigest')<>?2)",params![id,value["contentDigest"].as_str()]).map_err(database_error("retire changed message reference"))?;
            }
            let encoded = value.to_string();
            let record_digest = digest(&json!({"position":position,"value":value}));
            transaction.execute(&format!(
                "INSERT INTO {table}(record_id,position,record_json,digest,updated_at) VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(record_id) DO UPDATE SET position=excluded.position,record_json=excluded.record_json,digest=excluded.digest,updated_at=excluded.updated_at"
            ), params![id,position,encoded,record_digest,now]).map_err(database_error("write ledger record"))?;
            receipts.push(json!({"collection":collection,"id":id,"digest":record_digest}));
        }
    }
    if !model_writer {
        apply_pending_inputs(transaction, request, now)?;
        retire_accepted_inputs(transaction)?;
        apply_deliveries(transaction, request)?;
    }
    if !model_writer
        && transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ai_storage_activation WHERE storage_key='native')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(database_error("read AI storage mode"))?
    {
        super::incident_recovery::reconcile(transaction)?;
    }
    let result = json!({"commandId":command_id,"records":receipts});
    transaction.execute(
        "INSERT INTO ai_ledger_commands(command_id,request_digest,result_json,applied_at) VALUES(?1,?2,?3,?4)",
        params![command_id,request_digest,result.to_string(),now],
    ).map_err(database_error("record ledger command"))?;
    Ok(result)
}

fn delete_record(
    transaction: &Transaction<'_>,
    collection: &str,
    table: &str,
    id: &str,
) -> Result<(), PersistenceError> {
    if collection == "conversations" {
        let busy:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM ai_conversation_thread_bindings WHERE conversation_id=?1 AND active_run_id IS NOT NULL)",[id],|row|row.get(0)).map_err(database_error("check deleted thread writer"))?;
        if busy {
            return Err(reject("thread_owner_busy"));
        }
        transaction.execute("UPDATE ai_conversation_thread_bindings SET status='retired',binding_revision=binding_revision+1 WHERE conversation_id=?1",[id]).map_err(database_error("retire native conversation binding"))?;
        transaction
            .execute(
                "DELETE FROM ai_native_thread_index WHERE conversation_id=?1",
                [id],
            )
            .map_err(database_error("remove native conversation index"))?;
    }
    if collection == "messages" {
        transaction
            .execute("DELETE FROM ai_input_outbox WHERE message_id=?1", [id])
            .map_err(database_error("remove unsent input"))?;
        transaction
            .execute(
                "DELETE FROM ai_message_native_refs WHERE message_id=?1",
                [id],
            )
            .map_err(database_error("remove native message reference"))?;
    }
    if collection == "runs" {
        transaction
            .execute("DELETE FROM ai_run_native_refs WHERE run_id=?1", [id])
            .map_err(database_error("remove native run reference"))?;
    }
    transaction
        .execute(&format!("DELETE FROM {table} WHERE record_id=?1"), [id])
        .map_err(database_error("delete ledger record"))?;
    Ok(())
}

fn apply_pending_inputs(
    transaction: &Transaction<'_>,
    request: &Value,
    now: i64,
) -> Result<(), PersistenceError> {
    if let Some(inputs) = request.get("pendingInputs") {
        for input in inputs
            .as_array()
            .ok_or_else(|| reject("ai_ledger_request_invalid"))?
        {
            let id = text(input, "messageId")?;
            let body = input["content"]
                .as_str()
                .ok_or_else(|| reject("ai_ledger_input_invalid"))?;
            let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
            if input["contentDigest"] != hash {
                return Err(reject("ai_ledger_input_invalid"));
            }
            let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM ai_message_ledger WHERE record_id=?1 AND json_extract(record_json,'$.contentDigest')=?2 AND json_extract(record_json,'$.role')='user')", params![id,hash], |row| row.get(0)).map_err(database_error("validate unsent input"))?;
            if !exists {
                return Err(reject("ai_ledger_input_invalid"));
            }
            let accepted = input_accepted(
                transaction,
                id,
                body,
                &hash,
                input["nativeContentDigest"].as_str(),
            )?;
            if !accepted {
                transaction.execute("INSERT INTO ai_input_outbox(message_id,content,content_digest,created_at) VALUES(?1,?2,?3,?4) ON CONFLICT(message_id) DO UPDATE SET content=excluded.content,content_digest=excluded.content_digest", params![id,body,hash,now]).map_err(database_error("preserve unsent input"))?;
            }
        }
    }
    Ok(())
}

fn input_accepted(
    transaction: &Transaction<'_>,
    id: &str,
    body: &str,
    hash: &str,
    native_digest: Option<&str>,
) -> Result<bool, PersistenceError> {
    let steer_digest = digest(&json!({"content":body}));
    transaction.query_row("SELECT EXISTS(SELECT 1 FROM ai_thread_message_links WHERE message_id=?1 AND content_digest IN (?2,?3,?4)) OR EXISTS(SELECT 1 FROM ai_message_native_refs r JOIN ai_message_ledger m ON m.record_id=r.message_id WHERE r.message_id=?1 AND json_extract(m.record_json,'$.contentDigest')=?2)", params![id,hash,native_digest,steer_digest], |row| row.get(0)).map_err(database_error("check input delivery"))
}

fn retire_accepted_inputs(transaction: &Transaction<'_>) -> Result<(), PersistenceError> {
    let inputs = {
        let mut statement = transaction
            .prepare("SELECT message_id,content,content_digest FROM ai_input_outbox")
            .map_err(database_error("read pending inputs"))?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error("read pending inputs"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read pending inputs"))?
    };
    for (id, body, hash) in inputs {
        if input_accepted(transaction, &id, &body, &hash, None)? {
            transaction
                .execute("DELETE FROM ai_input_outbox WHERE message_id=?1", [&id])
                .map_err(database_error("clear accepted input copy"))?;
        }
    }
    Ok(())
}

fn apply_deliveries(
    transaction: &Transaction<'_>,
    request: &Value,
) -> Result<(), PersistenceError> {
    let Some(deliveries) = request.get("deliveries") else {
        return Ok(());
    };
    for delivery in deliveries
        .as_array()
        .ok_or_else(|| reject("ai_ledger_delivery_invalid"))?
    {
        let run_id = text(delivery, "runId")?;
        let message_id = text(delivery, "messageId")?;
        let encoded: String = transaction.query_row("SELECT record_json FROM ai_message_ledger WHERE record_id=?1 AND run_id=?2 AND json_extract(record_json,'$.role')='assistant' AND json_extract(record_json,'$.contentDigest')=?3",params![message_id,run_id,text(delivery,"contentDigest")?],|row|row.get(0)).map_err(database_error("validate native delivery message"))?;
        let message: Value =
            serde_json::from_str(&encoded).map_err(|_| reject("ai_ledger_storage_invalid"))?;
        let encoded: String = transaction
            .query_row(
                "SELECT record_json FROM ai_run_ledger WHERE record_id=?1 AND status='completed'",
                [run_id],
                |row| row.get(0),
            )
            .map_err(database_error("validate native delivery run"))?;
        let run: Value =
            serde_json::from_str(&encoded).map_err(|_| reject("ai_ledger_storage_invalid"))?;
        let encoded: String = transaction
            .query_row(
                "SELECT record_json FROM ai_conversation_ledger WHERE record_id=?1",
                [text(&run, "conversationId")?],
                |row| row.get(0),
            )
            .map_err(database_error("read native delivery context"))?;
        let conversation: Value =
            serde_json::from_str(&encoded).map_err(|_| reject("ai_ledger_storage_invalid"))?;
        if !delivery["skillDelivery"].is_object() || !delivery["capabilityDelivery"].is_object() {
            return Err(reject("ai_ledger_delivery_invalid"));
        }
        let source = json!({"runId":run_id,"conversationId":run["conversationId"],"superseded":run["superseded"],"workspaceRoot":conversation["context"]["workspaceRoot"],"message":message,"artifacts":run["artifacts"],"skillIds":run["skillIds"],"skillDelivery":delivery["skillDelivery"],"capabilityDelivery":delivery["capabilityDelivery"]});
        transaction.execute("INSERT OR IGNORE INTO ai_skill_installations(run_id,status,source_json) VALUES(?1,'pending',?2)",params![run_id,source.to_string()]).map_err(database_error("enqueue native skill delivery"))?;
        transaction.execute("INSERT OR IGNORE INTO ai_capability_project_deliveries(run_id,status,source_json) VALUES(?1,'pending',?2)",params![run_id,source.to_string()]).map_err(database_error("enqueue native capability delivery"))?;
    }
    Ok(())
}

fn records(connection: &Connection, collection: &str) -> Result<Value, PersistenceError> {
    let table = table(collection)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT record_id,position,record_json,digest FROM {table} ORDER BY position,record_id"
        ))
        .map_err(database_error("prepare ledger read"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(database_error("read ledger"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("decode ledger"))?;
    rows.into_iter()
        .map(|(id, position, value, hash)| {
            let value: Value =
                serde_json::from_str(&value).map_err(|_| reject("ai_ledger_storage_invalid"))?;
            Ok(json!({"id":id,"position":position,"value":value,"digest":hash}))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

impl SqlitePersistence {
    /// 原子提交业务条目；同一命令重放返回原回执，不依赖全局修订。
    /// # Errors
    /// 条目冲突、重复编号、正文误入台账或数据库写入失败时返回错误。
    pub fn apply_ai_business_batch(
        &mut self,
        request: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin ledger batch"))?;
        let result = apply_batch(&transaction, request, now)?;
        transaction
            .commit()
            .map_err(database_error("commit ledger batch"))?;
        Ok(result)
    }

    /// 在一致读取事务内返回业务条目及各自摘要。
    /// # Errors
    /// 数据库或条目内容不可读时返回错误。
    pub fn ai_business_ledger(&self) -> Result<Value, PersistenceError> {
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(database_error("begin ledger read"))?;
        let mut result = serde_json::Map::new();
        for (collection, _) in COLLECTIONS {
            result.insert(collection.into(), records(&transaction, collection)?);
        }
        let mut statement = transaction.prepare("SELECT message_id,content,content_digest FROM ai_input_outbox ORDER BY created_at,message_id").map_err(database_error("read input outbox"))?;
        let outbox = statement.query_map([], |row| Ok(json!({"messageId":row.get::<_,String>(0)?,"content":row.get::<_,String>(1)?,"contentDigest":row.get::<_,String>(2)?}))).map_err(database_error("read input outbox"))?.collect::<Result<Vec<_>,_>>().map_err(database_error("decode input outbox"))?;
        drop(statement);
        result.insert("outbox".into(), Value::Array(outbox));
        let mut statement = transaction.prepare("SELECT run_id,native_thread_id,native_turn_id FROM ai_run_native_refs ORDER BY run_id").map_err(database_error("read native run mappings"))?;
        let references = statement.query_map([], |row| Ok(json!({"runId":row.get::<_,String>(0)?,"nativeThreadId":row.get::<_,String>(1)?,"nativeTurnId":row.get::<_,String>(2)?}))).map_err(database_error("read native run mappings"))?.collect::<Result<Vec<_>,_>>().map_err(database_error("decode native run mappings"))?;
        drop(statement);
        result.insert("runRefs".into(), Value::Array(references));
        let mut statement = transaction.prepare("SELECT message_id,native_thread_id,native_turn_id,native_item_id,body_format FROM ai_message_native_refs ORDER BY message_id").map_err(database_error("read native message mappings"))?;
        let references = statement.query_map([], |row| Ok(json!({"messageId":row.get::<_,String>(0)?,"nativeThreadId":row.get::<_,String>(1)?,"nativeTurnId":row.get::<_,String>(2)?,"nativeItemId":row.get::<_,String>(3)?,"bodyFormat":row.get::<_,String>(4)?}))).map_err(database_error("read native message mappings"))?.collect::<Result<Vec<_>,_>>().map_err(database_error("decode native message mappings"))?;
        drop(statement);
        result.insert("messageRefs".into(), Value::Array(references));
        transaction
            .commit()
            .map_err(database_error("commit ledger read"))?;
        let mut native = Vec::new();
        for record in result["conversations"]
            .as_array()
            .ok_or_else(|| reject("ai_ledger_storage_invalid"))?
        {
            let id = text(record, "id")?;
            native.push(json!({"conversationId":id,"journal":self.ai_thread_state(id)?}));
        }
        result.insert("native".into(), Value::Array(native));
        Ok(Value::Object(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> SqlitePersistence {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(include_str!(
                "../../../migrations/0055_add_ai_business_ledger.sql"
            ))
            .unwrap();
        SqlitePersistence {
            connection,
            database_path: ":memory:".into(),
            file_content_index: None,
        }
    }
    fn batch(command: &str, id: &str, expected: &Value, text: &str) -> Value {
        json!({"commandId":command,"records":[{"collection":"entries","id":id,"position":0,
            "expectedDigest":expected,"value":{"entryId":id,"draft":text}}]})
    }
    #[test]
    fn independent_records_and_command_replay_do_not_conflict() {
        let mut db = db();
        let a = batch("a", "draft-a", &Value::Null, "甲");
        let receipt = db.apply_ai_business_batch(&a, 1).unwrap();
        db.apply_ai_business_batch(&batch("b", "draft-b", &Value::Null, "乙"), 2)
            .unwrap();
        assert_eq!(db.apply_ai_business_batch(&a, 3).unwrap(), receipt);
        let mut update = batch("c", "draft-a", &receipt["records"][0]["digest"], "新甲");
        db.apply_ai_business_batch(&update, 4).unwrap();
        update["commandId"] = json!("stale");
        assert_eq!(
            db.apply_ai_business_batch(&update, 5).unwrap_err().reason(),
            Some("ai_ledger_record_conflict")
        );
        assert_eq!(
            db.ai_business_ledger().unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn mixed_batch_is_atomic_and_transcript_bodies_are_rejected() {
        let mut db = db();
        let mut request = batch("invalid", "draft", &Value::Null, "草稿");
        request["records"]
            .as_array_mut()
            .unwrap()
            .push(json!({"collection":"messages","id":"m","position":0,
            "expectedDigest":null,"value":{"messageId":"m","content":"不应进入台账"}}));
        assert!(db.apply_ai_business_batch(&request, 1).is_err());
        assert_eq!(db.ai_business_ledger().unwrap()["entries"], json!([]));
        assert!(
            db.apply_ai_business_batch(&batch("ok", "draft", &Value::Null, "草稿"), 2)
                .is_ok()
        );
        assert_eq!(
            db.apply_ai_business_batch(&batch("ok", "other", &Value::Null, "不同"), 3)
                .unwrap_err()
                .reason(),
            Some("ai_ledger_command_conflict")
        );
    }
    #[test]
    fn verified_native_message_reference_retires_old_temporary_input_copy() {
        let mut db = db();
        db.connection
            .execute_batch(
                "CREATE TABLE ai_thread_message_links(message_id TEXT,content_digest TEXT);",
            )
            .unwrap();
        let body = "已由原生线程核对的原文";
        let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
        db.connection.execute("INSERT INTO ai_message_native_refs(message_id,native_thread_id,native_turn_id,native_item_id,body_format) VALUES('m','thread','turn','item','native')",[]).unwrap();
        let request = json!({"commandId":"verified","records":[{"collection":"messages","id":"m","position":0,"expectedDigest":null,"value":{"messageId":"m","role":"user","contentDigest":hash}}],"pendingInputs":[{"messageId":"m","content":body,"contentDigest":hash,"nativeContentDigest":"old-delivery-used-another-context-digest"}]});
        let receipt = db.apply_ai_business_batch(&request, 1).unwrap();
        assert_eq!(db.ai_business_ledger().unwrap()["outbox"], json!([]));
        db.connection.execute("INSERT INTO ai_input_outbox(message_id,content,content_digest,created_at) VALUES('m',?1,?2,1)",params![body,hash]).unwrap();
        db.apply_ai_business_batch(&json!({"commandId":"reconcile","records":[]}), 2)
            .unwrap();
        assert_eq!(db.ai_business_ledger().unwrap()["outbox"], json!([]));
        let edited = "尚未投递的新内容";
        let edited_hash = format!("{:x}", Sha256::digest(edited.as_bytes()));
        let edit_request = json!({"commandId":"edit","records":[{"collection":"messages","id":"m","position":0,"expectedDigest":receipt["records"][0]["digest"],"value":{"messageId":"m","role":"user","contentDigest":edited_hash}}],"pendingInputs":[{"messageId":"m","content":edited,"contentDigest":edited_hash}]});
        db.apply_ai_business_batch(&edit_request, 3).unwrap();
        assert_eq!(
            db.ai_business_ledger().unwrap()["outbox"][0]["content"],
            edited
        );
        assert_eq!(
            db.connection
                .query_row(
                    "SELECT COUNT(*) FROM ai_message_native_refs WHERE message_id='m'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn acknowledged_steer_does_not_recreate_a_temporary_body_after_confirmation() {
        let mut db = db();
        db.connection
            .execute_batch(
                "CREATE TABLE ai_thread_message_links(message_id TEXT,content_digest TEXT);",
            )
            .unwrap();
        let body = "补充输入";
        let hash = format!("{:x}", Sha256::digest(body.as_bytes()));
        let accepted_digest = digest(&json!({"content":body}));
        db.connection
            .execute(
                "INSERT INTO ai_thread_message_links(message_id,content_digest) VALUES('steer',?1)",
                [&accepted_digest],
            )
            .unwrap();
        let request = json!({"commandId":"steer","records":[{"collection":"messages","id":"steer","position":0,"expectedDigest":null,"value":{"messageId":"steer","role":"user","contentDigest":hash}}],"pendingInputs":[{"messageId":"steer","content":body,"contentDigest":hash,"nativeContentDigest":digest(&json!({"content":body,"attachments":[],"referencedMaterials":[]}))}]});
        db.apply_ai_business_batch(&request, 1).unwrap();
        assert_eq!(db.ai_business_ledger().unwrap()["outbox"], json!([]));
        db.connection.execute("INSERT INTO ai_input_outbox(message_id,content,content_digest,created_at) VALUES('steer',?1,?2,1)", params![body,hash]).unwrap();
        db.apply_ai_business_batch(&json!({"commandId":"cleanup","records":[]}), 2)
            .unwrap();
        assert_eq!(db.ai_business_ledger().unwrap()["outbox"], json!([]));
    }
}
