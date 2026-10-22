use super::ai_business_ledger::{apply_batch, digest, reject};
use super::{PersistenceError, SqlitePersistence, database_error};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, PersistenceError> {
    value[field]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| reject("ai_history_request_invalid"))
}

impl SqlitePersistence {
    pub(super) fn native_ai_storage_active(&self) -> Result<bool, PersistenceError> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ai_storage_activation WHERE storage_key='native')",
                [],
                |r| r.get(0),
            )
            .map_err(database_error("read AI storage activation"))
    }

    /// 只读取当前存储中的进程元数据，用于确认旧材料缓存没有运行中的读取者。
    /// # Errors
    /// 存储状态或进程元数据不可读时返回错误。
    pub fn ai_material_reader_pids(&self) -> Result<Vec<u32>, PersistenceError> {
        let query = if self.native_ai_storage_active()? {
            "SELECT record_json FROM ai_run_ledger"
        } else {
            "SELECT run_json FROM ai_runs"
        };
        let mut statement = self
            .connection
            .prepare(query)
            .map_err(database_error("read material cache readers"))?;
        let values = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error("read material cache readers"))?;
        let mut pids = Vec::new();
        for value in values {
            let value = value.map_err(database_error("read material cache reader"))?;
            let record: Value =
                serde_json::from_str(&value).map_err(|_| reject("ai_run_metadata_invalid"))?;
            if let Some(pid) = record.get("pid").filter(|pid| !pid.is_null()) {
                pids.push(
                    pid.as_u64()
                        .and_then(|pid| u32::try_from(pid).ok())
                        .filter(|pid| *pid > 0)
                        .ok_or_else(|| reject("ai_run_metadata_invalid"))?,
                );
            }
        }
        Ok(pids)
    }

    /// 迁移准备、记录、启用均由应用服务串行执行。
    /// # Errors
    /// 历史改变、核对不足、回执冲突或备份失败时拒绝切换。
    pub fn ai_history(
        &mut self,
        action: &str,
        payload: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        match action {
            "source" => self.ai_history_source(),
            "backup" => self.ai_history_backup(),
            "record" => self.ai_history_record(payload, now),
            "activate" => self.ai_history_activate(payload, now),
            "index" => self.ai_history_index(payload, now),
            "bind" => self.ai_history_bind(payload, now),
            "handoff" => self.import_application_handoff(payload, now),
            "recoverText" => self.recover_ai_text(payload, now),
            "recoveryBackup" => self.backup_ai_text_recovery(),
            "contextGet" | "contextPrepare" | "contextCommit" | "contextDelete" => {
                self.ai_context_import(action, payload, now)
            }
            "memoryGet" | "memorySave" | "memoryReset" => self.ai_workbench_memory(action, payload),
            "memoryLegacyRead" => self.read_legacy_memory_metadata(),
            _ => Err(reject("ai_history_action_invalid")),
        }
    }

    fn ai_workbench_memory(
        &mut self,
        action: &str,
        payload: &Value,
    ) -> Result<Value, PersistenceError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin workbench memory transaction"))?;
        let known: Option<(i64, String)> = transaction
            .query_row(
                "SELECT revision,value_json FROM ai_memory_state WHERE state_key='account'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(database_error("read workbench memory"))?;
        let (revision, value) = match known {
            Some((revision, json)) => (
                revision,
                serde_json::from_str::<Value>(&json)
                    .map_err(|_| reject("ai_memory_state_invalid"))?,
            ),
            None => (
                0,
                json!({"sources":{},"phase":"empty","publication":null,"error":null}),
            ),
        };
        if action == "memoryGet" {
            return Ok(json!({"revision":revision,"value":value}));
        }
        if action == "memorySave" && payload["revision"].as_i64() != Some(revision) {
            return Err(reject("ai_memory_revision_conflict"));
        }
        let next = if action == "memoryReset" {
            json!({"sources":{},"phase":"empty","publication":null,"error":null})
        } else {
            payload["value"].clone()
        };
        if !next["sources"].is_object()
            || !next["phase"].is_string()
            // 完整来源与不可变发布保存在工作台；为 64 MiB 传输帧预留外层协议空间。
            || next.to_string().len() > 63 * 1024 * 1024
        {
            return Err(reject("ai_memory_state_invalid"));
        }
        transaction.execute("INSERT INTO ai_memory_state(state_key,revision,value_json) VALUES('account',?1,?2) ON CONFLICT(state_key) DO UPDATE SET revision=excluded.revision,value_json=excluded.value_json",params![revision+1,next.to_string()]).map_err(database_error("save workbench memory"))?;
        transaction
            .commit()
            .map_err(database_error("commit workbench memory"))?;
        Ok(json!({"revision":revision+1,"value":next}))
    }

    fn ai_context_import(
        &self,
        action: &str,
        payload: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let scope = text(payload, "scope")?;
        let thread = text(payload, "threadId")?;
        if action == "contextDelete" {
            let deleted = self
                .connection
                .execute(
                    "DELETE FROM ai_context_imports WHERE scope=?1 AND thread_id=?2",
                    params![scope, thread],
                )
                .map_err(database_error("delete workbench context import"))?;
            return Ok(json!({"deleted":deleted > 0}));
        }
        let known: Option<(String, String, String)> = self.connection.query_row(
            "SELECT source_digest,source_json,status FROM ai_context_imports WHERE scope=?1 AND thread_id=?2",
            params![scope,thread], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional().map_err(database_error("read workbench context import"))?;
        if action == "contextGet" {
            return known.map_or(Ok(Value::Null), |(hash, source, status)| {
                Ok(json!({"digest":hash,"turns":serde_json::from_str::<Value>(&source).map_err(|_|reject("ai_context_import_invalid"))?,"status":status}))
            });
        }
        let turns = payload["turns"]
            .as_array()
            .ok_or_else(|| reject("ai_context_import_invalid"))?;
        let source =
            serde_json::to_value(turns).map_err(|_| reject("ai_context_import_invalid"))?;
        let hash = digest(&source);
        if let Some((known_hash, _, status)) = known {
            if hash != known_hash {
                return Err(reject("ai_context_import_conflict"));
            }
            if action == "contextCommit" && status == "prepared" {
                self.connection.execute("UPDATE ai_context_imports SET status='committed',updated_at=?3 WHERE scope=?1 AND thread_id=?2",params![scope,thread,now]).map_err(database_error("commit workbench context import"))?;
                return Ok(json!({"status":"committed"}));
            }
            return Ok(json!({"status":status,"replayed":true}));
        }
        if action != "contextPrepare" {
            return Err(reject("ai_context_import_not_prepared"));
        }
        self.connection.execute("INSERT INTO ai_context_imports(scope,thread_id,source_digest,source_json,status,updated_at) VALUES(?1,?2,?3,?4,'prepared',?5)",params![scope,thread,hash,source.to_string(),now]).map_err(database_error("prepare workbench context import"))?;
        Ok(json!({"status":"prepared","replayed":false}))
    }

    /// # Errors
    /// 历史业务交付来源无法读取时返回错误。
    pub fn ai_history_delivery_sources(&self) -> Result<Vec<Value>, PersistenceError> {
        let mut rows = Vec::new();
        for (kind, table) in [
            ("skill", "ai_skill_installations"),
            ("capability", "ai_capability_project_deliveries"),
        ] {
            let mut statement = self
                .connection
                .prepare(&format!("SELECT run_id,source_json FROM {table}"))
                .map_err(database_error("read legacy delivery sources"))?;
            let values = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(database_error("read legacy delivery sources"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error("decode legacy delivery sources"))?;
            for (run, source) in values {
                rows.push(json!({"kind":kind,"runId":run,"source":serde_json::from_str::<Value>(&source).map_err(|_|reject("ai_history_delivery_invalid"))?}));
            }
        }
        Ok(rows)
    }

    fn ai_history_source(&self) -> Result<Value, PersistenceError> {
        if self.native_ai_storage_active()? {
            return Ok(json!({"active":true,"ledger":self.ai_business_ledger()?}));
        }
        let (_, source) = self.ai_runtime_state()?;
        let mut stmt = self.connection.prepare("SELECT conversation_id,source_digest,native_thread_id,status,receipt_json FROM ai_history_migrations ORDER BY conversation_id").map_err(database_error("read AI migration receipts"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(database_error("read AI migration receipts"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("decode AI migration receipts"))?;
        let migrations = rows.into_iter().map(|(id,hash,thread,status,receipt)| Ok(json!({"conversationId":id,"sourceDigest":hash,"nativeThreadId":thread,"status":status,"receipt":serde_json::from_str::<Value>(&receipt).map_err(|_| reject("ai_history_storage_invalid"))?}))).collect::<Result<Vec<_>,PersistenceError>>()?;
        Ok(
            json!({"active":false,"sourceDigest":digest(&source),"state":source,"migrations":migrations}),
        )
    }

    fn ai_history_backup(&self) -> Result<Value, PersistenceError> {
        if self.native_ai_storage_active()? {
            return Err(reject("ai_history_already_active"));
        }
        let parent = self
            .database_path
            .parent()
            .ok_or_else(|| reject("ai_history_backup_path_invalid"))?;
        let directory = parent.join("ai-history-backup");
        std::fs::create_dir_all(&directory).map_err(|_| reject("ai_history_backup_failed"))?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| reject("ai_history_clock_invalid"))?
            .as_nanos();
        let path = directory.join(format!("{stamp}.sqlite"));
        self.connection
            .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])
            .map_err(database_error("backup AI business database"))?;
        Ok(json!({"databasePath":path,"directory":directory}))
    }

    fn ai_history_record(&mut self, value: &Value, now: i64) -> Result<Value, PersistenceError> {
        if self.native_ai_storage_active()? {
            return Err(reject("ai_history_already_active"));
        }
        let id = text(value, "conversationId")?;
        let hash = text(value, "sourceDigest")?;
        let status = text(value, "status")?;
        if !matches!(status, "prepared" | "importing" | "verified" | "failed") {
            return Err(reject("ai_history_status_invalid"));
        }
        let existing: Option<(String,Option<String>,String)> = self.connection.query_row("SELECT source_digest,native_thread_id,status FROM ai_history_migrations WHERE conversation_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(database_error("read migration record"))?;
        if let Some((known, thread, previous)) = existing
            && (known != hash
                || (thread.is_some() && thread.as_deref() != value["nativeThreadId"].as_str())
                || (previous == "verified" && status != "verified"))
        {
            return Err(reject("ai_history_receipt_conflict"));
        }
        if status == "verified"
            && (value["nativeThreadId"].as_str().is_none()
                || value["receipt"]["verifiedDigest"] != hash)
        {
            return Err(reject("ai_history_verification_required"));
        }
        let count = value["messageCount"]
            .as_i64()
            .filter(|n| *n >= 0)
            .ok_or_else(|| reject("ai_history_request_invalid"))?;
        self.connection.execute("INSERT INTO ai_history_migrations(conversation_id,source_digest,native_thread_id,status,message_count,receipt_json,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(conversation_id) DO UPDATE SET native_thread_id=excluded.native_thread_id,status=excluded.status,message_count=excluded.message_count,receipt_json=excluded.receipt_json,updated_at=excluded.updated_at", params![id,hash,value["nativeThreadId"].as_str(),status,count,value["receipt"].to_string(),now]).map_err(database_error("record AI history migration"))?;
        Ok(json!({"recorded":true}))
    }

    fn ai_history_index(&self, value: &Value, now: i64) -> Result<Value, PersistenceError> {
        let id = text(value, "conversationId")?;
        let thread = text(value, "nativeThreadId")?;
        let bound:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM ai_conversation_thread_bindings WHERE conversation_id=?1 AND native_thread_id=?2 AND status<>'retired')",params![id,thread],|row|row.get(0)).map_err(database_error("verify native index binding"))?;
        if !bound {
            return Err(reject("thread_binding_conflict"));
        }
        let title = value["title"]
            .as_str()
            .ok_or_else(|| reject("ai_history_request_invalid"))?;
        let archived = value["archived"]
            .as_bool()
            .ok_or_else(|| reject("ai_history_request_invalid"))?;
        self.connection.execute("INSERT INTO ai_native_thread_index(conversation_id,native_thread_id,title,archived,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(conversation_id) DO UPDATE SET native_thread_id=excluded.native_thread_id,title=excluded.title,archived=excluded.archived,updated_at=excluded.updated_at", params![id,thread,title,archived,now]).map_err(database_error("update native AI index"))?;
        Ok(json!({"indexed":true}))
    }

    fn ai_history_bind(&mut self, value: &Value, now: i64) -> Result<Value, PersistenceError> {
        let id = text(value, "conversationId")?;
        text(value, "nativeThreadId")?;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin native conversation binding"))?;
        let exists: bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM ai_conversation_thread_bindings WHERE conversation_id=?1)",[id],|r|r.get(0)).map_err(database_error("check native conversation binding"))?;
        if exists {
            return Err(reject("thread_binding_conflict"));
        }
        super::application_handoff::bind(&transaction, value, now)?;
        transaction
            .commit()
            .map_err(database_error("commit native conversation binding"))?;
        Ok(json!({"bound":true}))
    }

    fn ai_history_activate(&mut self, value: &Value, now: i64) -> Result<Value, PersistenceError> {
        if self.native_ai_storage_active()? {
            return Err(reject("ai_history_already_active"));
        }
        let (_, source) = self.ai_runtime_state()?;
        let hash = digest(&source);
        if value["sourceDigest"] != hash {
            return Err(reject("ai_history_source_changed"));
        }
        let conversations = source["conversations"]
            .as_array()
            .ok_or_else(|| reject("ai_history_source_invalid"))?;
        validate_projection(&source, value)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin AI history cutover"))?;
        for conversation in conversations {
            let id = text(conversation, "conversationId")?;
            let verified: Option<(String,String)> = transaction.query_row("SELECT native_thread_id,receipt_json FROM ai_history_migrations WHERE conversation_id=?1 AND status='verified'", [id], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(database_error("check AI migration verification"))?;
            let (thread, encoded) =
                verified.ok_or_else(|| reject("ai_history_verification_required"))?;
            let receipt: Value =
                serde_json::from_str(&encoded).map_err(|_| reject("ai_history_storage_invalid"))?;
            for (collection, key, receipt_field) in [
                ("messages", "messageId", "messages"),
                ("runs", "runId", "runs"),
            ] {
                let expected = source[collection]
                    .as_array()
                    .ok_or_else(|| reject("ai_history_source_invalid"))?
                    .iter()
                    .filter(|item| item["conversationId"] == id)
                    .collect::<Vec<_>>();
                let mapped = receipt[receipt_field]
                    .as_array()
                    .ok_or_else(|| reject("ai_history_receipt_invalid"))?;
                if mapped.len() != expected.len()
                    || expected.iter().any(|item| {
                        mapped.iter().filter(|link| link[key] == item[key]).count() != 1
                    })
                {
                    return Err(reject("ai_history_receipt_incomplete"));
                }
            }
            let reuse = receipt["reuseExisting"] == true;
            let generation: i64 = if reuse {
                transaction.query_row("SELECT generation FROM ai_conversation_thread_bindings WHERE conversation_id=?1 AND native_thread_id=?2",params![id,thread],|row|row.get(0)).map_err(database_error("verify reused native thread binding"))?
            } else {
                transaction.query_row("SELECT COALESCE(MAX(generation),0)+1 FROM ai_thread_generations WHERE conversation_id=?1",[id],|row|row.get(0)).map_err(database_error("read migrated thread generation"))?
            };
            if !reuse {
                let operation = format!("migration:{id}:{generation}");
                transaction.execute("INSERT INTO ai_conversation_thread_bindings(conversation_id,binding_revision,generation,native_thread_id,protocol_version,status,updated_at) VALUES(?1,1,?2,?3,1,'ready',?4) ON CONFLICT(conversation_id) DO UPDATE SET binding_revision=binding_revision+1,generation=excluded.generation,native_thread_id=excluded.native_thread_id,status='ready',active_run_id=NULL,owner_instance_id=NULL,updated_at=excluded.updated_at", params![id,generation,thread,now]).map_err(database_error("activate migrated thread binding"))?;
                let request =
                    json!({"conversationId":id,"kind":"create","runId":operation,"input":{}});
                transaction.execute("INSERT INTO ai_thread_operations(operation_id,conversation_id,generation,run_id,operation_kind,request_json,state,receipt_json,created_at,updated_at) VALUES(?1,?2,?3,?1,'create',?4,'acknowledged',?5,?6,?6)",params![operation,id,generation,request.to_string(),json!({"nativeThreadId":thread}).to_string(),now]).map_err(database_error("record migrated thread identity"))?;
                transaction.execute("INSERT INTO ai_thread_generations(conversation_id,generation,native_thread_id,source_operation_id,created_at) VALUES(?1,?2,?3,?4,?5)",params![id,generation,thread,operation,now]).map_err(database_error("record migrated generation"))?;
            }
            for link in receipt["messages"]
                .as_array()
                .ok_or_else(|| reject("ai_history_receipt_invalid"))?
            {
                let message_id = text(link, "messageId")?;
                transaction.execute("INSERT INTO ai_message_native_refs(message_id,native_thread_id,native_turn_id,native_item_id,body_format) VALUES(?1,?2,?3,?4,?5)",params![message_id,thread,text(link,"nativeTurnId")?,link["nativeItemId"].as_str().unwrap_or(message_id),link["bodyFormat"].as_str().unwrap_or("plain")]).map_err(database_error("record migrated native item"))?;
                if link["role"] != "user" || reuse {
                    continue;
                }
                let op = format!("migration:{message_id}");
                let request = json!({"conversationId":id,"kind":"send","runId":link["runId"],"input":{"messageId":message_id,"clientMessageId":message_id,"contentDigest":link["contentDigest"]}});
                let ack = json!({"nativeThreadId":thread,"nativeTurnId":link["nativeTurnId"],"clientMessageId":message_id});
                transaction.execute("INSERT INTO ai_thread_operations(operation_id,conversation_id,generation,run_id,operation_kind,request_json,state,receipt_json,created_at,updated_at) VALUES(?1,?2,?3,?4,'send',?5,'acknowledged',?6,?7,?7)",params![op,id,generation,text(link,"runId")?,request.to_string(),ack.to_string(),now]).map_err(database_error("record migrated message receipt"))?;
                transaction.execute("INSERT INTO ai_thread_message_links(conversation_id,generation,message_id,client_message_id,content_digest,native_thread_id,native_turn_id,operation_id,accepted_at) VALUES(?1,?2,?3,?3,?4,?5,?6,?7,?8)",params![id,generation,message_id,text(link,"contentDigest")?,thread,text(link,"nativeTurnId")?,op,now]).map_err(database_error("record migrated message mapping"))?;
            }
            for run in receipt["runs"]
                .as_array()
                .ok_or_else(|| reject("ai_history_receipt_invalid"))?
            {
                transaction.execute("INSERT INTO ai_run_native_refs(run_id,native_thread_id,native_turn_id) VALUES(?1,?2,?3) ON CONFLICT(run_id) DO UPDATE SET native_thread_id=excluded.native_thread_id,native_turn_id=excluded.native_turn_id",params![text(run,"runId")?,thread,text(run,"nativeTurnId")?]).map_err(database_error("record migrated run mapping"))?;
            }
            transaction.execute("INSERT INTO ai_native_thread_index(conversation_id,native_thread_id,title,archived,updated_at) VALUES(?1,?2,?3,?4,?5)",params![id,thread,conversation["title"].as_str().unwrap_or_default(),conversation["archived"].as_bool().unwrap_or(false),now]).map_err(database_error("index migrated thread"))?;
        }
        migrate_deliveries(&transaction, value)?;
        let receipt = apply_batch(&transaction, &value["ledger"], now)?;
        transaction
            .execute_batch(include_str!("ai_native_business_views.sql"))
            .map_err(database_error("switch native AI business views"))?;
        transaction.execute("UPDATE ai_thread_operations SET request_json=json_remove(request_json,'$.input.content','$.input.history') WHERE state='acknowledged'",[]).map_err(database_error("retire accepted input copies"))?;
        transaction.execute("INSERT INTO ai_storage_activation(storage_key,source_digest,activated_at) VALUES('native',?1,?2)",params![hash,now]).map_err(database_error("activate native AI storage"))?;
        transaction
            .commit()
            .map_err(database_error("commit AI history cutover"))?;
        Ok(receipt)
    }
}

fn validate_projection(source: &Value, value: &Value) -> Result<(), PersistenceError> {
    let records = value["ledger"]["records"]
        .as_array()
        .ok_or_else(|| reject("ai_history_ledger_invalid"))?;
    for (collection, key) in [
        ("conversations", "conversationId"),
        ("messages", "messageId"),
        ("runs", "runId"),
    ] {
        let source_items = source[collection]
            .as_array()
            .ok_or_else(|| reject("ai_history_source_invalid"))?;
        let projected = records
            .iter()
            .filter(|record| record["collection"] == collection)
            .collect::<Vec<_>>();
        if projected.len() != source_items.len() {
            return Err(reject("ai_history_ledger_incomplete"));
        }
        for (position, item) in source_items.iter().enumerate() {
            let matches = projected
                .iter()
                .filter(|record| {
                    record["id"] == item[key]
                        && record["value"][key] == item[key]
                        && record["position"] == position
                        && record["expectedDigest"].is_null()
                })
                .count();
            if matches != 1 {
                return Err(reject("ai_history_ledger_incomplete"));
            }
        }
    }
    Ok(())
}

fn migrate_deliveries(
    transaction: &rusqlite::Transaction<'_>,
    value: &Value,
) -> Result<(), PersistenceError> {
    for delivery in value["legacyDeliveries"]
        .as_array()
        .ok_or_else(|| reject("ai_history_delivery_invalid"))?
    {
        let table = match delivery["kind"].as_str() {
            Some("skill") => "ai_skill_installations",
            Some("capability") => "ai_capability_project_deliveries",
            _ => return Err(reject("ai_history_delivery_invalid")),
        };
        let previous: String = transaction
            .query_row(
                &format!("SELECT source_json FROM {table} WHERE run_id=?1"),
                [text(delivery, "runId")?],
                |row| row.get(0),
            )
            .map_err(database_error("verify legacy delivery source"))?;
        let previous: Value =
            serde_json::from_str(&previous).map_err(|_| reject("ai_history_delivery_invalid"))?;
        if digest(&previous) != delivery["sourceDigest"]
            || delivery["source"]["message"].get("content").is_some()
        {
            return Err(reject("ai_history_delivery_changed"));
        }
        transaction.execute(&format!("UPDATE {table} SET source_json=?1,result_json=CASE WHEN result_json IS NULL THEN NULL ELSE json_set(result_json,'$.displayContent',NULL) END WHERE run_id=?2"),params![delivery["source"].to_string(),text(delivery,"runId")?]).map_err(database_error("migrate structured delivery"))?;
    }
    Ok(())
}

#[cfg(test)]
mod cache_reader_tests {
    use super::*;

    #[test]
    fn cache_readers_switch_to_native_metadata_after_activation() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch(r#"CREATE TABLE ai_runs(run_json TEXT); CREATE TABLE ai_run_ledger(record_json TEXT); CREATE TABLE ai_storage_activation(storage_key TEXT);
            INSERT INTO ai_runs VALUES('{"pid":12}'); INSERT INTO ai_run_ledger VALUES('{"pid":34}');"#).unwrap();
        let db = SqlitePersistence {
            connection,
            database_path: ":memory:".into(),
            file_content_index: None,
        };
        assert_eq!(db.ai_material_reader_pids().unwrap(), vec![12]);
        db.connection
            .execute("INSERT INTO ai_storage_activation VALUES('native')", [])
            .unwrap();
        db.connection.execute("DROP TABLE ai_runs", []).unwrap();
        assert_eq!(db.ai_material_reader_pids().unwrap(), vec![34]);
        db.connection
            .execute(
                r#"INSERT INTO ai_run_ledger VALUES('{"pid":"invalid"}')"#,
                [],
            )
            .unwrap();
        assert!(db.ai_material_reader_pids().is_err());
    }
}
