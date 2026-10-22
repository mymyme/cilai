use super::ai_business_ledger::{apply_batch, reject};
use super::{PersistenceError, SqlitePersistence, database_error};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, PersistenceError> {
    value[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| reject("application_handoff_invalid"))
}

pub(super) fn bind(
    transaction: &Transaction<'_>,
    value: &Value,
    now: i64,
) -> Result<(), PersistenceError> {
    let id = text(value, "conversationId")?;
    let thread = text(value, "nativeThreadId")?;
    let operation = format!("conversation-create:{id}");
    transaction.execute("INSERT INTO ai_conversation_thread_bindings(conversation_id,binding_revision,generation,native_thread_id,protocol_version,status,updated_at) VALUES(?1,1,1,?2,1,'ready',?3)",params![id,thread,now]).map_err(database_error("bind native conversation"))?;
    transaction.execute("INSERT INTO ai_thread_operations(operation_id,conversation_id,generation,run_id,operation_kind,request_json,state,receipt_json,created_at,updated_at) VALUES(?1,?2,1,?1,'create',?3,'acknowledged',?4,?5,?5)",params![operation,id,json!({"conversationId":id,"kind":"create","runId":operation,"input":{}}).to_string(),json!({"nativeThreadId":thread}).to_string(),now]).map_err(database_error("record native conversation creation"))?;
    transaction.execute("INSERT INTO ai_thread_generations(conversation_id,generation,native_thread_id,source_operation_id,created_at) VALUES(?1,1,?2,?3,?4)",params![id,thread,operation,now]).map_err(database_error("record native conversation generation"))?;
    transaction.execute("INSERT INTO ai_native_thread_index(conversation_id,native_thread_id,title,archived,updated_at) VALUES(?1,?2,?3,0,?4)",params![id,thread,text(value,"title")?,now]).map_err(database_error("index native conversation"))?;
    Ok(())
}

fn validate_messages(
    records: &[Value],
    messages: &[Value],
    id: &str,
    output_digest: &str,
) -> Result<(), PersistenceError> {
    for role in ["user", "assistant"] {
        let links = messages
            .iter()
            .filter(|message| message["role"] == role)
            .collect::<Vec<_>>();
        let [link] = links.as_slice() else {
            return Err(reject("application_handoff_records_invalid"));
        };
        let record = records
            .iter()
            .find(|record| record["collection"] == "messages" && record["id"] == link["messageId"])
            .ok_or_else(|| reject("application_handoff_records_invalid"))?;
        if record["value"]["conversationId"] != id
            || record["value"]["role"] != role
            || record["value"]["runId"] != Value::Null
            || record["value"]["contentDigest"] != link["contentDigest"]
            || (role == "assistant" && link["contentDigest"] != output_digest)
        {
            return Err(reject("application_handoff_records_invalid"));
        }
    }
    Ok(())
}

impl SqlitePersistence {
    pub(super) fn import_application_handoff(
        &mut self,
        value: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        if !self.native_ai_storage_active()? {
            return Err(reject("application_handoff_storage_unavailable"));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin application handoff"))?;
        let ledger = &value["ledger"];
        let command = text(ledger, "commandId")?;
        let replay: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ai_ledger_commands WHERE command_id=?1)",
                [command],
                |row| row.get(0),
            )
            .map_err(database_error("read handoff command"))?;
        if replay {
            let result = apply_batch(&transaction, ledger, now)?;
            transaction
                .commit()
                .map_err(database_error("confirm handoff replay"))?;
            return Ok(result);
        }
        let id = text(value, "conversationId")?;
        let handoff_id = text(value, "handoffId")?;
        let raw:String = transaction.query_row("SELECT record_json FROM ai_runtime_entries WHERE record_id='conversationSession:applicationHandoffs'",[],|row|row.get(0)).optional().map_err(database_error("read handoff source"))?.ok_or_else(||reject("application_handoff_missing"))?;
        let source: Value =
            serde_json::from_str(&raw).map_err(|_| reject("application_handoff_invalid"))?;
        let queue = source["value"]
            .as_array()
            .ok_or_else(|| reject("application_handoff_invalid"))?;
        let handoff = queue
            .iter()
            .find(|entry| entry["handoffId"] == handoff_id)
            .ok_or_else(|| reject("application_handoff_missing"))?;
        let output_digest = format!("{:x}", Sha256::digest(text(handoff, "output")?.as_bytes()));
        if handoff["outputDigest"] != output_digest {
            return Err(reject("application_handoff_output_changed"));
        }
        let exists:bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM ai_conversation_ledger WHERE record_id=?1 OR json_extract(record_json,'$.applicationHandoff.handoffId')=?2)",params![id,handoff_id],|row|row.get(0)).map_err(database_error("check completed handoff"))?;
        if exists {
            return Err(reject("application_handoff_already_completed"));
        }
        let records = ledger["records"]
            .as_array()
            .filter(|records| records.len() == 4)
            .ok_or_else(|| reject("application_handoff_records_invalid"))?;
        let conversation = records
            .iter()
            .find(|record| record["collection"] == "conversations" && record["id"] == id)
            .ok_or_else(|| reject("application_handoff_records_invalid"))?;
        let origin = &conversation["value"]["applicationHandoff"];
        if origin["handoffId"] != handoff_id
            || origin["applicationId"] != handoff["applicationId"]
            || origin["runId"] != handoff["runId"]
            || origin["outputDigest"] != output_digest
        {
            return Err(reject("application_handoff_records_invalid"));
        }
        let remaining = queue
            .iter()
            .filter(|entry| entry["handoffId"] != handoff_id)
            .cloned()
            .collect::<Vec<_>>();
        let changed_queue = records
            .iter()
            .find(|record| {
                record["collection"] == "entries"
                    && record["id"] == "conversationSession:applicationHandoffs"
            })
            .ok_or_else(|| reject("application_handoff_records_invalid"))?;
        if changed_queue["value"]["value"] != json!(remaining) {
            return Err(reject("application_handoff_records_invalid"));
        }
        let messages = value["messages"]
            .as_array()
            .filter(|messages| messages.len() == 2)
            .ok_or_else(|| reject("application_handoff_records_invalid"))?;
        validate_messages(records, messages, id, &output_digest)?;
        bind(&transaction, value, now)?;
        let thread = text(value, "nativeThreadId")?;
        for link in messages {
            let message = text(link, "messageId")?;
            let turn = text(link, "nativeTurnId")?;
            transaction.execute("INSERT INTO ai_message_native_refs(message_id,native_thread_id,native_turn_id,native_item_id,body_format) VALUES(?1,?2,?3,?1,'plain')",params![message,thread,turn]).map_err(database_error("bind handoff history item"))?;
            if link["role"] == "user" {
                let operation = format!("handoff:{message}");
                let request = json!({"conversationId":id,"kind":"send","runId":turn,"input":{"messageId":message,"clientMessageId":message,"contentDigest":link["contentDigest"]}});
                let receipt =
                    json!({"nativeThreadId":thread,"nativeTurnId":turn,"clientMessageId":message});
                transaction.execute("INSERT INTO ai_thread_operations(operation_id,conversation_id,generation,run_id,operation_kind,request_json,state,receipt_json,created_at,updated_at) VALUES(?1,?2,1,?3,'send',?4,'acknowledged',?5,?6,?6)",params![operation,id,turn,request.to_string(),receipt.to_string(),now]).map_err(database_error("record handoff message receipt"))?;
                transaction.execute("INSERT INTO ai_thread_message_links(conversation_id,generation,message_id,client_message_id,content_digest,native_thread_id,native_turn_id,operation_id,accepted_at) VALUES(?1,1,?2,?2,?3,?4,?5,?6,?7)",params![id,message,text(link,"contentDigest")?,thread,turn,operation,now]).map_err(database_error("record handoff message mapping"))?;
            }
        }
        let result = apply_batch(&transaction, ledger, now)?;
        transaction
            .commit()
            .map_err(database_error("commit application handoff"))?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handoff_commit_rolls_back_all_records_and_replays_once() {
        let root = std::env::temp_dir().join(format!(
            "handoff-storage-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("test.db"), "test").unwrap();
        db.connection.execute("INSERT INTO ai_storage_activation(storage_key,source_digest,activated_at) VALUES('native','test',1)",[]).unwrap();
        db.connection
            .execute_batch(include_str!("ai_native_business_views.sql"))
            .unwrap();
        let hash = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
        let queue_id = "conversationSession:applicationHandoffs";
        let queued = json!({"handoffId":"handoff-run","applicationId":"air","runId":"run","output":"原回答","outputDigest":hash("原回答")});
        let entry = json!({"entryId":queue_id,"field":"conversationSession","member":"applicationHandoffs","shape":"map","value":[queued]});
        let seed = json!({"commandId":"seed","records":[{"collection":"entries","id":queue_id,"position":0,"expectedDigest":null,"value":entry}]});
        let transaction = db.connection.transaction().unwrap();
        let seeded = apply_batch(&transaction, &seed, 1).unwrap();
        transaction.commit().unwrap();
        let mut next = entry;
        next["value"] = json!([]);
        let origin = json!({"handoffId":"handoff-run","applicationId":"air","runId":"run","outputDigest":hash("原回答")});
        let mut records = vec![
            json!({"collection":"conversations","id":"conversation","position":0,"expectedDigest":null,"value":{"conversationId":"conversation","applicationHandoff":origin,"context":{"workspaceRoot":""},"createdAt":1,"updatedAt":1}}),
            json!({"collection":"entries","id":queue_id,"position":0,"expectedDigest":seeded["records"][0]["digest"],"value":next}),
        ];
        let mut links = vec![];
        for (index, role, body) in [(0, "user", "原问题"), (1, "assistant", "原回答")] {
            let id = format!("message-{index}");
            let digest = hash(body);
            records.push(json!({"collection":"messages","id":id,"position":index,"expectedDigest":null,"value":{"messageId":id,"conversationId":"conversation","role":role,"runId":null,"contentDigest":digest,"contentBytes":body.len(),"createdAt":1}}));
            links.push(json!({"messageId":id,"role":role,"runId":format!("history:{id}"),"nativeTurnId":format!("history:{id}"),"contentDigest":digest}));
        }
        let request = json!({"handoffId":"handoff-run","conversationId":"conversation","nativeThreadId":"thread","title":"转交验收","messages":links,"ledger":{"commandId":"commit","records":records,"pendingInputs":[]}});
        let mut invalid = request.clone();
        invalid["ledger"]["records"][1]["expectedDigest"] = json!("changed");
        assert!(db.import_application_handoff(&invalid, 2).is_err());
        for table in [
            "ai_conversation_ledger",
            "ai_message_ledger",
            "ai_message_native_refs",
            "ai_conversation_thread_bindings",
            "ai_native_thread_index",
        ] {
            assert_eq!(
                db.connection
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            db.ai_business_ledger().unwrap()["entries"][0]["value"]["value"][0]["output"],
            "原回答"
        );
        let result = db.import_application_handoff(&request, 3).unwrap();
        assert_eq!(db.import_application_handoff(&request, 4).unwrap(), result);
        let ledger = db.ai_business_ledger().unwrap();
        assert_eq!(ledger["conversations"].as_array().unwrap().len(), 1);
        assert_eq!(ledger["messages"].as_array().unwrap().len(), 2);
        assert_eq!(ledger["messageRefs"].as_array().unwrap().len(), 2);
        assert!(
            ledger["entries"][0]["value"]["value"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(ledger["outbox"].as_array().unwrap().is_empty());
    }
}
