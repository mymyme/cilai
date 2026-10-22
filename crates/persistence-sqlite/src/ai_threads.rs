use super::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};

fn rejected(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, PersistenceError> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or_else(|| rejected("ai_thread_request_invalid"))
}

fn binding(db: &Connection, conversation_id: &str) -> Result<Option<Value>, PersistenceError> {
    db.query_row(
        "SELECT binding_revision,generation,native_thread_id,protocol_version,status,owner_instance_id,active_run_id,writer_parked,runtime_scope_json FROM ai_conversation_thread_bindings WHERE conversation_id=?1",
        [conversation_id],
        |row| Ok(json!({
            "conversationId": conversation_id,
            "bindingRevision": row.get::<_, i64>(0)?, "generation": row.get::<_, i64>(1)?,
            "nativeThreadId": row.get::<_, Option<String>>(2)?, "protocolVersion": row.get::<_, i64>(3)?,
            "status": row.get::<_, String>(4)?, "ownerInstanceId": row.get::<_, Option<String>>(5)?,
            "activeRunId": row.get::<_, Option<String>>(6)?,
            "writerParked": row.get::<_, bool>(7)?,
            "runtimeScope": row.get::<_,Option<String>>(8)?.map(|raw|serde_json::from_str::<Value>(&raw)).transpose().map_err(|_|rusqlite::Error::InvalidQuery)?,
        })),
    ).optional().map_err(database_error("read AI thread binding"))
}

fn parse_json(value: &str) -> Result<Value, PersistenceError> {
    serde_json::from_str(value).map_err(|_| rejected("ai_thread_storage_invalid"))
}

fn owns(binding: &Value, request: &Value) -> bool {
    binding["ownerInstanceId"].as_str().is_some()
        && binding["ownerInstanceId"] == request["ownerInstanceId"]
        && binding["activeRunId"] == request["runId"]
}

fn operation(db: &Connection, id: &str) -> Result<Option<Value>, PersistenceError> {
    db.query_row(
        "SELECT request_json,state,receipt_json,generation FROM ai_thread_operations WHERE operation_id=?1",
        [id],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, i64>(3)?)),
    ).optional().map_err(database_error("read AI thread operation"))?
        .map(|(request, state, receipt, generation)| Ok(json!({
            "request": parse_json(&request)?, "state": state,
            "receipt": receipt.as_deref().map(parse_json).transpose()?, "generation": generation,
        }))).transpose()
}

fn touch(
    tx: &Transaction<'_>,
    conversation_id: &str,
    status: &str,
    now: i64,
) -> Result<(), PersistenceError> {
    tx.execute("UPDATE ai_conversation_thread_bindings SET binding_revision=binding_revision+1,status=?2,updated_at=?3 WHERE conversation_id=?1", params![conversation_id,status,now])
        .map_err(database_error("update AI thread revision"))?;
    Ok(())
}

impl SqlitePersistence {
    /// Read the authoritative binding and delivery journal without changing AI state.
    ///
    /// # Errors
    /// Returns an error if the stored journal cannot be read or decoded.
    pub fn ai_thread_state(&self, conversation_id: &str) -> Result<Value, PersistenceError> {
        let current = binding(&self.connection, conversation_id)?;
        let mut statement = self.connection.prepare("SELECT operation_id FROM ai_thread_operations WHERE conversation_id=?1 ORDER BY created_at,operation_id")
            .map_err(database_error("list AI thread operations"))?;
        let ids = statement
            .query_map([conversation_id], |row| row.get::<_, String>(0))
            .map_err(database_error("list AI thread operations"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read AI thread operations"))?;
        let operations = ids
            .iter()
            .map(|id| {
                operation(&self.connection, id)?
                    .ok_or_else(|| rejected("ai_thread_storage_invalid"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut statement = self.connection.prepare("SELECT generation,message_id,content_digest,native_thread_id,native_turn_id,operation_id,client_message_id FROM ai_thread_message_links WHERE conversation_id=?1 ORDER BY accepted_at,operation_id")
            .map_err(database_error("list AI thread message mappings"))?;
        let messages = statement.query_map([conversation_id], |row| Ok(json!({
            "generation": row.get::<_, i64>(0)?, "messageId": row.get::<_, String>(1)?,
            "contentDigest": row.get::<_, String>(2)?, "nativeThreadId": row.get::<_, String>(3)?,
            "nativeTurnId": row.get::<_, String>(4)?, "operationId": row.get::<_, String>(5)?, "clientMessageId": row.get::<_, String>(6)?,
        }))).map_err(database_error("list AI thread message mappings"))?
            .collect::<Result<Vec<_>, _>>().map_err(database_error("read AI thread message mappings"))?;
        let mut statement=self.connection.prepare("SELECT generation,native_thread_id,runtime_scope_json FROM ai_thread_generations WHERE conversation_id=?1 ORDER BY generation").map_err(database_error("read native thread generations"))?;
        let generations=statement.query_map([conversation_id],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,Option<String>>(2)?))).map_err(database_error("query native thread generations"))?.map(|row|{let (generation,native_id,scope)=row.map_err(database_error("decode native thread generation"))?;Ok(json!({"generation":generation,"nativeThreadId":native_id,"runtimeScope":scope.as_deref().map(parse_json).transpose()?}))}).collect::<Result<Vec<_>,PersistenceError>>()?;
        Ok(
            json!({ "binding": current, "operations": operations, "messages": messages,"generations":generations }),
        )
    }

    /// Reserve one native operation before it is sent. Replays never dispatch again.
    ///
    /// # Errors
    /// Rejects conflicting owners, revisions, reused operation identities and unresolved delivery.
    #[allow(clippy::too_many_lines)]
    pub fn prepare_ai_thread_operation(
        &mut self,
        request: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let conversation_id = text(request, "conversationId")?;
        let operation_id = text(request, "operationId")?;
        let run_id = text(request, "runId")?;
        let owner = text(request, "ownerInstanceId")?;
        let kind = text(request, "kind")?;
        if !matches!(
            kind,
            "create" | "resume" | "fork" | "inject" | "send" | "steer" | "compact" | "delete"
        ) || !request["input"].is_object()
        {
            return Err(rejected("ai_thread_request_invalid"));
        }
        let expected = request["expectedBindingRevision"]
            .as_i64()
            .ok_or_else(|| rejected("ai_thread_request_invalid"))?;
        let protocol = request["protocolVersion"]
            .as_i64()
            .filter(|v| *v > 0)
            .ok_or_else(|| rejected("ai_thread_request_invalid"))?;
        let tx = self
            .connection
            .transaction()
            .map_err(database_error("begin AI thread operation"))?;
        if let Some(prior) = operation(&tx, operation_id)? {
            let mut compared = request.clone();
            if prior["state"] == "acknowledged"
                && prior["request"]["input"].get("content").is_none()
                && let Some(input) = compared["input"].as_object_mut()
            {
                input.remove("content");
                input.remove("history");
            }
            if prior["request"] != compared {
                return Err(rejected("ai_thread_operation_conflict"));
            }
            return Ok(
                json!({ "binding": binding(&tx, conversation_id)?, "operation": prior, "replayed": true }),
            );
        }
        let run_conversation: Option<String> = tx
            .query_row(
                "SELECT json_extract(run_json,'$.conversationId') FROM ai_business_runs WHERE run_id=?1",
                [run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error("validate AI thread run"))?;
        let application_scope = request
            .get("applicationScope")
            .filter(|value| !value.is_null());
        if let Some(scope) = application_scope {
            let application_id = text(scope, "applicationId")?;
            let session_kind = text(scope, "sessionKind")?;
            let valid_namespace = match session_kind {
                "application_page" => conversation_id.starts_with("application:"),
                "application_task" => conversation_id.starts_with("application-task:"),
                _ => false,
            };
            if !valid_namespace
                || !run_id.starts_with("app_run_")
                || run_conversation.is_some()
                || !application_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            {
                return Err(rejected("ai_application_thread_scope_invalid"));
            }
            let stored: Option<(String, String)> = tx.query_row("SELECT application_id,session_kind FROM ai_application_thread_scopes WHERE conversation_id=?1", [conversation_id], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(database_error("validate application thread scope"))?;
            if stored
                .as_ref()
                .is_some_and(|(app, kind)| app != application_id || kind != session_kind)
            {
                return Err(rejected("ai_application_thread_scope_conflict"));
            }
            let stored_run: Option<String> = tx
                .query_row(
                    "SELECT conversation_id FROM ai_application_thread_runs WHERE run_id=?1",
                    [run_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error("validate application thread run"))?;
            if stored_run
                .as_deref()
                .is_some_and(|id| id != conversation_id)
            {
                return Err(rejected("ai_thread_run_mismatch"));
            }
        } else if run_conversation.as_deref() != Some(conversation_id) {
            return Err(rejected("ai_thread_run_mismatch"));
        }
        let current = binding(&tx, conversation_id)?;
        let runtime_scope = request["input"]
            .get("runtimeScope")
            .filter(|v| !v.is_null());
        let personal_v2:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_profile_compatibility WHERE singleton=1 AND profile_id IS NOT NULL AND state='ready')",[],|row|row.get(0)).map_err(database_error("read personal native thread policy"))?;
        if personal_v2 && runtime_scope.is_none() {
            return Err(rejected("model_thread_scope_required"));
        }
        if let Some(scope) = runtime_scope {
            let scope: contexthub_protocol::ModelThreadRuntimeScope =
                serde_json::from_value(scope.clone())
                    .map_err(|_| rejected("model_thread_scope_invalid"))?;
            if !matches!(scope.runtime_kind.as_str(), "workbench" | "application")
                || (scope.runtime_kind == "application") != application_scope.is_some()
                || scope.runtime_home_scope
                    != contexthub_protocol::model_runtime_home_scope(
                        &scope.profile_id,
                        &scope.connection_id,
                        scope.auth_generation,
                    )
                || scope.config_revision == 0
                || scope.auth_generation == 0
            {
                return Err(rejected("model_thread_scope_invalid"));
            }
            let (profile_id, ready): (Option<String>, String) = tx
                .query_row(
                    "SELECT profile_id,state FROM personal_profile_compatibility WHERE singleton=1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(database_error("read native thread profile"))?;
            if profile_id.as_deref() != Some(&scope.profile_id) || ready != "ready" {
                return Err(rejected("model_thread_profile_mismatch"));
            }
            crate::model_connections::validate_native_scope(
                &tx,
                &scope,
                text(&request["input"], "modelExecutionLeaseId")?,
                conversation_id,
                owner,
                &request["input"]["writerProcess"],
            )?;
        }
        let generation;
        if let Some(current) = current {
            if current["bindingRevision"] != expected {
                return Err(rejected("thread_binding_conflict"));
            }
            if current["status"] == "retired" || current["protocolVersion"] != protocol {
                return Err(rejected("history_migration_required"));
            }
            let scope_changed =
                current["runtimeScope"] != runtime_scope.cloned().unwrap_or(Value::Null);
            if scope_changed && !current["nativeThreadId"].is_null() {
                if kind != "fork" {
                    return Err(rejected("model_thread_scope_changed"));
                }
                if request["input"]["historyConsentConfirmed"] != true {
                    return Err(rejected("model_thread_history_consent_required"));
                }
            }
            if current["status"] == "blocked"
                && (kind != "create" || !current["nativeThreadId"].is_null())
            {
                return Err(rejected("history_migration_required"));
            }
            let handoff = if current["writerParked"] == true
                && kind == "resume"
                && current["ownerInstanceId"] == request["ownerInstanceId"]
            {
                let previous: Option<String> = tx.query_row(
                    "SELECT json_extract(request_json,'$.input.writerProcess') FROM ai_thread_operations WHERE conversation_id=?1 AND run_id=?2 AND operation_kind IN ('create','resume','fork') AND state='acknowledged' ORDER BY created_at DESC LIMIT 1",
                    params![conversation_id,current["activeRunId"].as_str()], |row| row.get::<_, Option<String>>(0)
                ).optional().map_err(database_error("verify parked writer identity"))?.flatten();
                previous
                    .as_deref()
                    .map(parse_json)
                    .transpose()?
                    .is_some_and(|identity| {
                        identity.is_object()
                            && identity["pid"].is_u64()
                            && identity["startFingerprint"].is_string()
                            && identity == request["input"]["writerProcess"]
                    })
            } else {
                false
            };
            if current["activeRunId"].is_string() && !owns(&current, request) && !handoff {
                return Err(rejected("thread_owner_busy"));
            }
            let unresolved: i64 = tx.query_row("SELECT count(*) FROM ai_thread_operations WHERE conversation_id=?1 AND state IN ('dispatched','uncertain')", [conversation_id], |row| row.get(0))
                .map_err(database_error("check AI thread pending delivery"))?;
            if unresolved != 0 {
                return Err(rejected("message_delivery_uncertain"));
            }
            if (kind == "create" && !current["nativeThreadId"].is_null())
                || (!matches!(kind, "create" | "resume") && current["nativeThreadId"].is_null())
            {
                return Err(rejected("ai_thread_operation_invalid"));
            }
            generation = current["generation"]
                .as_i64()
                .ok_or_else(|| rejected("ai_thread_storage_invalid"))?;
            tx.execute("UPDATE ai_conversation_thread_bindings SET owner_instance_id=?2,active_run_id=?3,writer_parked=0 WHERE conversation_id=?1", params![conversation_id,owner,run_id])
                .map_err(database_error("reserve AI thread writer"))?;
            touch(&tx, conversation_id, "preparing", now)?;
        } else {
            if expected != 0 || kind != "create" {
                return Err(rejected("thread_binding_conflict"));
            }
            generation = 1;
            tx.execute("INSERT INTO ai_conversation_thread_bindings(conversation_id,binding_revision,generation,protocol_version,status,owner_instance_id,active_run_id,updated_at) VALUES(?1,1,1,?2,'preparing',?3,?4,?5)", params![conversation_id,protocol,owner,run_id,now])
                .map_err(database_error("create AI thread binding"))?;
        }
        if let Some(scope) = application_scope {
            tx.execute("INSERT INTO ai_application_thread_scopes(conversation_id,application_id,session_kind) VALUES(?1,?2,?3) ON CONFLICT(conversation_id) DO NOTHING",params![conversation_id,text(scope,"applicationId")?,text(scope,"sessionKind")?]).map_err(database_error("register application thread scope"))?;
            tx.execute("INSERT INTO ai_application_thread_runs(run_id,conversation_id) VALUES(?1,?2) ON CONFLICT(run_id) DO NOTHING",params![run_id,conversation_id]).map_err(database_error("register application thread run"))?;
        }
        if runtime_scope.is_some() {
            tx.execute("UPDATE ai_conversation_thread_bindings SET model_execution_lease_id=?2 WHERE conversation_id=?1",params![conversation_id,text(&request["input"],"modelExecutionLeaseId")?]).map_err(database_error("bind native model lease"))?;
        }
        if matches!(kind, "send" | "steer") {
            let message_id = text(&request["input"], "clientMessageId")?;
            text(&request["input"], "messageId")?;
            text(&request["input"], "contentDigest")?;
            let already_received: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ai_thread_message_links WHERE conversation_id=?1 AND generation=?2 AND client_message_id=?3)", params![conversation_id,generation,message_id], |row| row.get(0))
                .map_err(database_error("check AI message delivery"))?;
            if already_received {
                return Err(rejected("ai_message_already_accepted"));
            }
        }
        tx.execute("INSERT INTO ai_thread_operations(operation_id,conversation_id,generation,run_id,operation_kind,request_json,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,'prepared',?7,?7)", params![operation_id,conversation_id,generation,run_id,kind,request.to_string(),now])
            .map_err(database_error("prepare AI thread operation"))?;
        let result = json!({ "binding": binding(&tx, conversation_id)?, "operation": operation(&tx, operation_id)?, "replayed": false });
        tx.commit()
            .map_err(database_error("commit AI thread operation"))?;
        Ok(result)
    }

    /// Record dispatch or a matching native receipt in the same transaction as its binding.
    ///
    /// # Errors
    /// Rejects foreign owners, invalid transitions, mismatched receipts and duplicate messages.
    #[allow(clippy::too_many_lines)]
    pub fn acknowledge_ai_thread_operation(
        &mut self,
        request: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let conversation_id = text(request, "conversationId")?;
        let operation_id = text(request, "operationId")?;
        let event = text(request, "event")?;
        let tx = self
            .connection
            .transaction()
            .map_err(database_error("begin AI thread receipt"))?;
        let current =
            binding(&tx, conversation_id)?.ok_or_else(|| rejected("thread_binding_conflict"))?;
        let prior = operation(&tx, operation_id)?
            .ok_or_else(|| rejected("ai_thread_operation_not_found"))?;
        if prior["request"]["conversationId"] != conversation_id
            || prior["request"]["runId"] != request["runId"]
            || prior["request"]["ownerInstanceId"] != request["ownerInstanceId"]
        {
            return Err(rejected("ai_thread_operation_conflict"));
        }
        let target = match event {
            "dispatch" => "dispatched",
            "acknowledge" => "acknowledged",
            "reject" => "rejected",
            "uncertain" => "uncertain",
            _ => return Err(rejected("ai_thread_request_invalid")),
        };
        if prior["state"] == target {
            if prior["receipt"] != request["receipt"] {
                return Err(rejected("ai_thread_receipt_conflict"));
            }
            return Ok(json!({ "binding": current, "operation": prior, "replayed": true }));
        }
        let reconciliation = request["reconciliation"] == true
            && prior["state"] == "uncertain"
            && current["activeRunId"].is_null()
            && target == "acknowledged"
            && current["generation"] == prior["generation"];
        if request["reconciliation"] == true && !reconciliation {
            return Err(rejected("ai_thread_reconciliation_invalid"));
        }
        if !owns(&current, request) && !reconciliation {
            return Err(rejected("thread_owner_busy"));
        }
        let allowed = matches!(
            (prior["state"].as_str(), target),
            (Some("prepared"), "dispatched" | "rejected")
                | (
                    Some("dispatched"),
                    "acknowledged" | "rejected" | "uncertain"
                )
                | (Some("uncertain"), "acknowledged" | "rejected")
        );
        if !allowed {
            return Err(rejected("ai_thread_transition_invalid"));
        }
        if target == "acknowledged" {
            let receipt = &request["receipt"];
            let kind = text(&prior["request"], "kind")?;
            if reconciliation && !matches!(kind, "send" | "steer" | "resume") {
                return Err(rejected("ai_thread_receipt_not_queryable"));
            }
            let native_thread = text(receipt, "nativeThreadId")?;
            if matches!(kind, "create" | "fork") {
                if kind == "fork" && receipt["sourceThreadId"] != current["nativeThreadId"] {
                    return Err(rejected("ai_thread_receipt_conflict"));
                }
                let generation = current["generation"]
                    .as_i64()
                    .ok_or_else(|| rejected("ai_thread_storage_invalid"))?
                    + i64::from(kind == "fork");
                let runtime_scope = prior["request"]["input"]
                    .get("runtimeScope")
                    .filter(|v| !v.is_null())
                    .map(Value::to_string);
                tx.execute("INSERT INTO ai_thread_generations(conversation_id,generation,native_thread_id,source_thread_id,source_operation_id,created_at,runtime_scope_json) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![conversation_id,generation,native_thread,current["nativeThreadId"].as_str(),operation_id,now,runtime_scope])
                    .map_err(database_error("publish AI thread generation"))?;
                tx.execute("UPDATE ai_conversation_thread_bindings SET generation=?2,native_thread_id=?3,runtime_scope_json=?4 WHERE conversation_id=?1", params![conversation_id,generation,native_thread,runtime_scope])
                    .map_err(database_error("bind AI native thread"))?;
            } else if current["nativeThreadId"] != native_thread {
                return Err(rejected("ai_thread_receipt_conflict"));
            }
            if matches!(kind, "send" | "steer") {
                let input = &prior["request"]["input"];
                let message_id = text(input, "messageId")?;
                if receipt["clientMessageId"] != input["clientMessageId"] {
                    return Err(rejected("ai_thread_receipt_conflict"));
                }
                tx.execute("INSERT INTO ai_thread_message_links(conversation_id,generation,message_id,content_digest,native_thread_id,native_turn_id,operation_id,accepted_at,client_message_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![conversation_id,current["generation"].as_i64(),message_id,text(input,"contentDigest")?,native_thread,text(receipt,"nativeTurnId")?,operation_id,now,text(input,"clientMessageId")?])
                    .map_err(database_error("acknowledge AI message delivery"))?;
                if prior["request"]["applicationScope"].is_null() {
                    tx.execute("INSERT INTO ai_run_native_refs(run_id,native_thread_id,native_turn_id) VALUES(?1,?2,?3) ON CONFLICT(run_id) DO UPDATE SET native_thread_id=excluded.native_thread_id,native_turn_id=excluded.native_turn_id", params![text(&prior["request"],"runId")?,native_thread,text(receipt,"nativeTurnId")?]).map_err(database_error("record native run mapping"))?;
                }
                tx.execute(
                    "DELETE FROM ai_input_outbox WHERE message_id=?1",
                    [message_id],
                )
                .map_err(database_error("complete input outbox delivery"))?;
            }
        }
        tx.execute("UPDATE ai_thread_operations SET state=?2,receipt_json=?3,updated_at=?4 WHERE operation_id=?1", params![operation_id,target,request["receipt"].to_string(),now])
            .map_err(database_error("save AI thread receipt"))?;
        let native: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM ai_storage_activation WHERE storage_key='native')",
                [],
                |row| row.get(0),
            )
            .map_err(database_error("read AI storage mode"))?;
        if target == "acknowledged" && native {
            tx.execute("UPDATE ai_thread_operations SET request_json=json_remove(request_json,'$.input.content','$.input.history') WHERE operation_id=?1",[operation_id]).map_err(database_error("complete native input journal"))?;
        }
        touch(
            &tx,
            conversation_id,
            if target == "uncertain" {
                "reconciling"
            } else {
                "active"
            },
            now,
        )?;
        if reconciliation {
            let unresolved: i64 = tx.query_row("SELECT count(*) FROM ai_thread_operations WHERE conversation_id=?1 AND state IN ('dispatched','uncertain')", [conversation_id], |row| row.get(0))
                .map_err(database_error("check reconciled AI thread"))?;
            touch(
                &tx,
                conversation_id,
                if unresolved == 0 {
                    "ready"
                } else {
                    "reconciling"
                },
                now,
            )?;
        }
        let result = json!({ "binding": binding(&tx, conversation_id)?, "operation": operation(&tx, operation_id)?, "replayed": false });
        tx.commit()
            .map_err(database_error("commit AI thread receipt"))?;
        Ok(result)
    }

    /// Record native cleanup after the conversation has been retired.
    ///
    /// # Errors
    /// Rejects active bindings, foreign native threads and invalid receipt transitions.
    pub fn record_ai_thread_cleanup(
        &mut self,
        request: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let conversation = text(request, "conversationId")?;
        let native_thread = text(request, "nativeThreadId")?;
        let event = text(request, "event")?;
        let tx = self
            .connection
            .transaction()
            .map_err(database_error("begin thread cleanup"))?;
        let current =
            binding(&tx, conversation)?.ok_or_else(|| rejected("thread_binding_conflict"))?;
        if current["status"] != "retired" || !current["activeRunId"].is_null() {
            return Err(rejected("thread_owner_busy"));
        }
        let generation: Option<i64> = tx.query_row("SELECT generation FROM ai_thread_generations WHERE conversation_id=?1 AND native_thread_id=?2",params![conversation,native_thread],|row|row.get(0)).optional().map_err(database_error("validate thread cleanup"))?;
        let generation = generation.ok_or_else(|| rejected("ai_thread_receipt_conflict"))?;
        let id = format!("cleanup:{conversation}:{native_thread}");
        let state = match event {
            "dispatch" => "dispatched",
            "acknowledge" => "acknowledged",
            "uncertain" => "uncertain",
            _ => return Err(rejected("ai_thread_request_invalid")),
        };
        if let Some(prior) = operation(&tx, &id)? {
            if prior["state"] == state {
                return Ok(json!({"binding":current,"operation":prior,"replayed":true}));
            }
            if prior["state"] != "dispatched" || event == "dispatch" {
                return Err(rejected("ai_thread_transition_invalid"));
            }
            tx.execute("UPDATE ai_thread_operations SET state=?2,receipt_json=?3,updated_at=?4 WHERE operation_id=?1",params![id,state,json!({"nativeThreadId":native_thread}).to_string(),now]).map_err(database_error("save thread cleanup receipt"))?;
        } else {
            if event != "dispatch" {
                return Err(rejected("ai_thread_transition_invalid"));
            }
            let saved = json!({"conversationId":conversation,"operationId":id,"kind":"delete","nativeThreadId":native_thread});
            tx.execute("INSERT INTO ai_thread_operations(operation_id,conversation_id,generation,run_id,operation_kind,request_json,state,created_at,updated_at) VALUES(?1,?2,?3,'cleanup','delete',?4,'dispatched',?5,?5)",params![id,conversation,generation,saved.to_string(),now]).map_err(database_error("record thread cleanup"))?;
        }
        let result = json!({"binding":current,"operation":operation(&tx,&id)?,"replayed":false});
        tx.commit()
            .map_err(database_error("commit thread cleanup"))?;
        Ok(result)
    }

    /// Release a stopped writer, or retain an idle writer for the same process.
    ///
    /// # Errors
    /// Rejects mismatched ownership and release without termination evidence.
    pub fn release_ai_thread_writer(
        &mut self,
        request: &Value,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let conversation_id = text(request, "conversationId")?;
        let park = request["retainWriter"] == true;
        if request["processReaped"] != true && !park {
            return Err(rejected("thread_owner_busy"));
        }
        if park && request["processReaped"] == true {
            return Err(rejected("ai_thread_request_invalid"));
        }
        let tx = self
            .connection
            .transaction()
            .map_err(database_error("begin AI thread release"))?;
        let current =
            binding(&tx, conversation_id)?.ok_or_else(|| rejected("thread_binding_conflict"))?;
        if !owns(&current, request) {
            return Err(rejected("thread_owner_busy"));
        }
        if park {
            let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ai_thread_operations WHERE conversation_id=?1 AND state<>'acknowledged' AND state<>'rejected')", [conversation_id], |row| row.get(0))
                .map_err(database_error("verify parked writer delivery"))?;
            let terminal: Option<String> = tx.query_row("SELECT json_extract(run_json,'$.status') FROM ai_business_runs WHERE run_id=?1",[text(request,"runId")?],|row|row.get::<_, Option<String>>(0))
                .optional().map_err(database_error("verify parked writer terminal run"))?.flatten();
            if pending
                || current["nativeThreadId"].is_null()
                || !matches!(current["status"].as_str(), Some("ready" | "active"))
                || !matches!(terminal.as_deref(), Some("completed" | "failed"))
            {
                return Err(rejected("thread_owner_busy"));
            }
            tx.execute("UPDATE ai_conversation_thread_bindings SET writer_parked=1 WHERE conversation_id=?1",[conversation_id])
                .map_err(database_error("park native writer"))?;
            touch(&tx, conversation_id, "ready", now)?;
            let result = json!({"binding":binding(&tx,conversation_id)?});
            tx.commit()
                .map_err(database_error("commit parked native writer"))?;
            return Ok(result);
        }
        tx.execute("UPDATE ai_thread_operations SET state='uncertain',updated_at=?2 WHERE conversation_id=?1 AND state='dispatched'", params![conversation_id,now])
            .map_err(database_error("preserve uncertain AI deliveries"))?;
        tx.execute("UPDATE ai_thread_operations SET state='rejected',receipt_json='null',updated_at=?2 WHERE conversation_id=?1 AND state='prepared'", params![conversation_id,now])
            .map_err(database_error("close unsent AI operations"))?;
        let unresolved: i64 = tx.query_row("SELECT count(*) FROM ai_thread_operations WHERE conversation_id=?1 AND state='uncertain'", [conversation_id], |row| row.get(0))
            .map_err(database_error("check uncertain AI deliveries"))?;
        let expected_history: i64 = tx.query_row("SELECT coalesce(sum(json_extract(request_json,'$.input.migrationItemCount')),0) FROM ai_thread_operations WHERE conversation_id=?1 AND generation=?2 AND operation_kind='create' AND state='acknowledged'",params![conversation_id,current["generation"].as_i64()],|row|row.get(0))
            .map_err(database_error("check AI history migration"))?;
        let imported_history: i64 = tx.query_row("SELECT coalesce(sum(json_extract(request_json,'$.input.count')),0) FROM ai_thread_operations WHERE conversation_id=?1 AND generation=?2 AND operation_kind='inject' AND state='acknowledged'",params![conversation_id,current["generation"].as_i64()],|row|row.get(0))
            .map_err(database_error("check AI imported history"))?;
        let rejected_fork: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ai_thread_operations WHERE conversation_id=?1 AND run_id=?2 AND operation_kind='fork' AND state='rejected')",params![conversation_id,current["activeRunId"].as_str()],|row|row.get(0))
            .map_err(database_error("check AI edit branch"))?;
        let status = if unresolved > 0 {
            "reconciling"
        } else if current["nativeThreadId"].is_null()
            || imported_history != expected_history
            || rejected_fork
        {
            "blocked"
        } else {
            "ready"
        };
        touch(&tx, conversation_id, status, now)?;
        tx.execute("UPDATE ai_conversation_thread_bindings SET owner_instance_id=NULL,active_run_id=NULL,writer_parked=0 WHERE conversation_id=?1", [conversation_id])
            .map_err(database_error("release AI thread writer"))?;
        let result = json!({ "binding": binding(&tx, conversation_id)? });
        tx.commit()
            .map_err(database_error("commit AI thread release"))?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestDatabase {
        path: std::path::PathBuf,
    }
    impl TestDatabase {
        fn new(label: &str) -> Self {
            static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "cilai-{label}-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir(&root).unwrap();
            Self {
                path: root.join("state.sqlite"),
            }
        }
    }
    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
        }
    }

    fn prepare(id: &str, kind: &str, revision: i64) -> Value {
        json!({"conversationId":"c","operationId":id,"runId":"r","ownerInstanceId":"host-1","expectedBindingRevision":revision,"protocolVersion":1,"kind":kind,"input":{"messageId":"m","clientMessageId":"m","contentDigest":"digest"}})
    }
    fn receipt(id: &str, event: &str, value: Value) -> Value {
        {
            let mut result = json!({"conversationId":"c","operationId":id,"runId":"r","ownerInstanceId":"host-1","event":event});
            result["receipt"] = value;
            result
        }
    }
    fn seeded() -> (TestDatabase, SqlitePersistence) {
        let fixture = TestDatabase::new("thread-journal");
        let mut db = SqlitePersistence::open(&fixture.path, "0.1.0").unwrap();
        db.replace_ai_runtime_state(0, &json!({"conversations":[{"conversationId":"c"}],"messages":[{"messageId":"m","conversationId":"c","role":"user","content":"hello"}],"runs":[{"runId":"r","conversationId":"c"},{"runId":"other","conversationId":"c"}]}), 1).unwrap();
        (fixture, db)
    }
    fn created(db: &mut SqlitePersistence) {
        db.prepare_ai_thread_operation(&prepare("create", "create", 0), 2)
            .unwrap();
        db.acknowledge_ai_thread_operation(&receipt("create", "dispatch", Value::Null), 3)
            .unwrap();
        db.acknowledge_ai_thread_operation(
            &receipt("create", "acknowledge", json!({"nativeThreadId":"t"})),
            4,
        )
        .unwrap();
    }

    #[test]
    fn parked_writer_handoff_requires_same_process_and_completed_delivery() {
        let (_fixture, mut db) = seeded();
        let identity = json!({"pid":123,"startFingerprint":"os-start-1"});
        let mut create = prepare("create", "create", 0);
        create["input"]["writerProcess"] = identity.clone();
        db.prepare_ai_thread_operation(&create, 2).unwrap();
        db.acknowledge_ai_thread_operation(&receipt("create", "dispatch", Value::Null), 3)
            .unwrap();
        db.acknowledge_ai_thread_operation(
            &receipt("create", "acknowledge", json!({"nativeThreadId":"t"})),
            4,
        )
        .unwrap();
        let park = json!({"conversationId":"c","runId":"r","ownerInstanceId":"host-1","processReaped":false,"retainWriter":true});
        assert!(db.release_ai_thread_writer(&park, 5).is_err());
        db.connection.execute("UPDATE ai_runs SET run_json=json_set(run_json,'$.status','completed') WHERE run_id='r'", []).unwrap();
        let state = db.release_ai_thread_writer(&park, 6).unwrap();
        assert_eq!(state["binding"]["writerParked"], true);
        assert_eq!(state["binding"]["activeRunId"], "r");
        let mut next = prepare(
            "next",
            "resume",
            state["binding"]["bindingRevision"].as_i64().unwrap(),
        );
        next["runId"] = json!("other");
        next["input"]["writerProcess"] = identity;
        for (field, value) in [
            ("pid", json!(124)),
            ("startFingerprint", json!("reused-pid")),
        ] {
            let mut wrong = next.clone();
            wrong["input"]["writerProcess"][field] = value;
            assert_eq!(
                db.prepare_ai_thread_operation(&wrong, 7)
                    .unwrap_err()
                    .reason(),
                Some("thread_owner_busy")
            );
        }
        let mut foreign = next.clone();
        foreign["ownerInstanceId"] = json!("host-2");
        assert_eq!(
            db.prepare_ai_thread_operation(&foreign, 7)
                .unwrap_err()
                .reason(),
            Some("thread_owner_busy")
        );
        db.prepare_ai_thread_operation(&next, 8).unwrap();
        let binding = db.ai_thread_state("c").unwrap()["binding"].clone();
        assert_eq!(binding["activeRunId"], "other");
        assert_eq!(binding["writerParked"], false);
        let mut old_release = park;
        old_release["retainWriter"] = json!(false);
        old_release["processReaped"] = json!(true);
        assert!(db.release_ai_thread_writer(&old_release, 9).is_err());
    }

    #[test]
    fn pending_delivery_cannot_be_parked() {
        let (_fixture, mut db) = seeded();
        created(&mut db);
        db.connection.execute("UPDATE ai_runs SET run_json=json_set(run_json,'$.status','completed') WHERE run_id='r'", []).unwrap();
        let revision = db.ai_thread_state("c").unwrap()["binding"]["bindingRevision"]
            .as_i64()
            .unwrap();
        db.prepare_ai_thread_operation(&prepare("send", "send", revision), 5)
            .unwrap();
        let park = json!({"conversationId":"c","runId":"r","ownerInstanceId":"host-1","processReaped":false,"retainWriter":true});
        assert_eq!(
            db.release_ai_thread_writer(&park, 6).unwrap_err().reason(),
            Some("thread_owner_busy")
        );
    }

    #[test]
    fn journal_serializes_writers_and_never_replays_unknown_deliveries() {
        let (_fixture, mut db) = seeded();
        created(&mut db);
        let rev = db.ai_thread_state("c").unwrap()["binding"]["bindingRevision"]
            .as_i64()
            .unwrap();
        let mut other = prepare("other", "resume", rev);
        other["runId"] = json!("other");
        assert_eq!(
            db.prepare_ai_thread_operation(&other, 5)
                .unwrap_err()
                .reason(),
            Some("thread_owner_busy")
        );
        let send = prepare("send", "send", rev);
        db.prepare_ai_thread_operation(&send, 6).unwrap();
        assert_eq!(
            db.prepare_ai_thread_operation(&send, 7).unwrap()["replayed"],
            true
        );
        let mut conflict = send.clone();
        conflict["input"]["contentDigest"] = json!("changed");
        assert_eq!(
            db.prepare_ai_thread_operation(&conflict, 7)
                .unwrap_err()
                .reason(),
            Some("ai_thread_operation_conflict")
        );
        db.acknowledge_ai_thread_operation(&receipt("send", "dispatch", Value::Null), 8)
            .unwrap();
        let mut release = json!({"conversationId":"c","runId":"r","ownerInstanceId":"host-1","processReaped":false});
        assert!(db.release_ai_thread_writer(&release, 9).is_err());
        release["processReaped"] = json!(true);
        assert_eq!(
            db.release_ai_thread_writer(&release, 10).unwrap()["binding"]["status"],
            "reconciling"
        );
        let state = db.ai_thread_state("c").unwrap();
        assert_eq!(state["messages"], json!([]));
        let retry = prepare(
            "retry",
            "resume",
            state["binding"]["bindingRevision"].as_i64().unwrap(),
        );
        assert_eq!(
            db.prepare_ai_thread_operation(&retry, 11)
                .unwrap_err()
                .reason(),
            Some("message_delivery_uncertain")
        );
    }

    #[test]
    fn receipt_links_multiple_messages_to_one_turn_and_rejects_mismatch() {
        let (_fixture, mut db) = seeded();
        created(&mut db);
        for (index, kind) in [(1, "send"), (2, "steer")] {
            let id = format!("message-{index}");
            let rev = db.ai_thread_state("c").unwrap()["binding"]["bindingRevision"]
                .as_i64()
                .unwrap();
            let mut p = prepare(&id, kind, rev);
            p["input"]["messageId"] = json!(id);
            p["input"]["clientMessageId"] = json!(id);
            db.prepare_ai_thread_operation(&p, 5).unwrap();
            db.acknowledge_ai_thread_operation(&receipt(&id, "dispatch", Value::Null), 6)
                .unwrap();
            let wrong = receipt(
                &id,
                "acknowledge",
                json!({"nativeThreadId":"other","nativeTurnId":"turn","clientMessageId":id}),
            );
            assert!(db.acknowledge_ai_thread_operation(&wrong, 7).is_err());
            let ack = receipt(
                &id,
                "acknowledge",
                json!({"nativeThreadId":"t","nativeTurnId":"turn","clientMessageId":id}),
            );
            db.acknowledge_ai_thread_operation(&ack, 8).unwrap();
            assert_eq!(
                db.acknowledge_ai_thread_operation(&ack, 9).unwrap()["replayed"],
                true
            );
        }
        assert_eq!(
            db.ai_thread_state("c").unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let release = json!({"conversationId":"c","runId":"r","ownerInstanceId":"host-1","processReaped":true});
        assert_eq!(
            db.release_ai_thread_writer(&release, 10).unwrap()["binding"]["status"],
            "ready"
        );
    }
    #[test]
    fn interrupted_migration_never_becomes_ready() {
        let (_fixture, mut db) = seeded();
        let mut create = prepare("migration", "create", 0);
        create["input"]["migrationItemCount"] = json!(40);
        db.prepare_ai_thread_operation(&create, 2).unwrap();
        db.acknowledge_ai_thread_operation(&receipt("migration", "dispatch", Value::Null), 3)
            .unwrap();
        db.acknowledge_ai_thread_operation(
            &receipt("migration", "acknowledge", json!({"nativeThreadId":"t"})),
            4,
        )
        .unwrap();
        let result = db.release_ai_thread_writer(&json!({"conversationId":"c","runId":"r","ownerInstanceId":"host-1","processReaped":true}),5).unwrap();
        assert_eq!(result["binding"]["status"], "blocked");
    }

    #[test]
    fn ordinary_state_save_cannot_erase_journal_and_deletion_waits_for_writer() {
        let (_fixture, mut db) = seeded();
        created(&mut db);
        let before = db.ai_thread_state("c").unwrap();
        let (revision, state) = db.ai_runtime_state().unwrap();
        db.replace_ai_runtime_state(revision, &state, 10).unwrap();
        assert_eq!(db.ai_thread_state("c").unwrap(), before);
        let (revision, mut state) = db.ai_runtime_state().unwrap();
        state["conversations"] = json!([]);
        assert_eq!(
            db.replace_ai_runtime_state(revision, &state, 11)
                .unwrap_err()
                .reason(),
            Some("thread_owner_busy")
        );
        db.release_ai_thread_writer(&json!({"conversationId":"c","runId":"r","ownerInstanceId":"host-1","processReaped":true}),12).unwrap();
        db.replace_ai_runtime_state(revision, &state, 13).unwrap();
        assert_eq!(
            db.ai_thread_state("c").unwrap()["binding"]["status"],
            "retired"
        );
    }

    #[test]
    fn fork_receipt_atomically_publishes_a_new_generation() {
        let (_fixture, mut db) = seeded();
        created(&mut db);
        let rev = db.ai_thread_state("c").unwrap()["binding"]["bindingRevision"]
            .as_i64()
            .unwrap();
        db.prepare_ai_thread_operation(&prepare("fork", "fork", rev), 5)
            .unwrap();
        db.acknowledge_ai_thread_operation(&receipt("fork", "dispatch", Value::Null), 6)
            .unwrap();
        let wrong = receipt(
            "fork",
            "acknowledge",
            json!({"nativeThreadId":"fork","sourceThreadId":"foreign"}),
        );
        assert!(db.acknowledge_ai_thread_operation(&wrong, 7).is_err());
        assert_eq!(
            db.ai_thread_state("c").unwrap()["binding"]["nativeThreadId"],
            "t"
        );
        db.acknowledge_ai_thread_operation(
            &receipt(
                "fork",
                "acknowledge",
                json!({"nativeThreadId":"fork","sourceThreadId":"t"}),
            ),
            8,
        )
        .unwrap();
        assert_eq!(db.ai_thread_state("c").unwrap()["binding"]["generation"], 2);
    }
    #[test]
    fn rejected_edit_never_resumes_the_superseded_answer() {
        let (_fixture, mut db) = seeded();
        created(&mut db);
        let rev = db.ai_thread_state("c").unwrap()["binding"]["bindingRevision"]
            .as_i64()
            .unwrap();
        db.prepare_ai_thread_operation(&prepare("fork", "fork", rev), 5)
            .unwrap();
        db.acknowledge_ai_thread_operation(&receipt("fork", "dispatch", Value::Null), 6)
            .unwrap();
        db.acknowledge_ai_thread_operation(
            &receipt("fork", "reject", json!({"reason":"denied"})),
            7,
        )
        .unwrap();
        let released = db.release_ai_thread_writer(&json!({"conversationId":"c","runId":"r","ownerInstanceId":"host-1","processReaped":true}),8).unwrap();
        assert_eq!(released["binding"]["status"], "blocked");
        let retry = prepare(
            "resume",
            "resume",
            released["binding"]["bindingRevision"].as_i64().unwrap(),
        );
        assert_eq!(
            db.prepare_ai_thread_operation(&retry, 9)
                .unwrap_err()
                .reason(),
            Some("history_migration_required")
        );
    }

    #[test]
    fn native_cleanup_requires_retired_owned_generation_and_keeps_receipts() {
        let (_fixture, mut db) = seeded();
        created(&mut db);
        let request = json!({"conversationId":"c","nativeThreadId":"t","event":"dispatch"});
        assert!(db.record_ai_thread_cleanup(&request, 5).is_err());
        db.release_ai_thread_writer(&json!({"conversationId":"c","runId":"r","ownerInstanceId":"host-1","processReaped":true}),6).unwrap();
        let (revision, mut state) = db.ai_runtime_state().unwrap();
        state["conversations"] = json!([]);
        db.replace_ai_runtime_state(revision, &state, 7).unwrap();
        let mut foreign = request.clone();
        foreign["nativeThreadId"] = json!("other");
        assert!(db.record_ai_thread_cleanup(&foreign, 8).is_err());
        db.record_ai_thread_cleanup(&request, 9).unwrap();
        let mut ack = request;
        ack["event"] = json!("acknowledge");
        assert_eq!(
            db.record_ai_thread_cleanup(&ack, 10).unwrap()["operation"]["state"],
            "acknowledged"
        );
        assert_eq!(
            db.ai_thread_state("c").unwrap()["binding"]["status"],
            "retired"
        );
    }
    #[test]
    fn application_scope_cannot_claim_formal_threads_or_change_its_owner_application() {
        let (_fixture, mut db) = seeded();
        let mut request = prepare("app-create", "create", 0);
        request["applicationScope"] =
            json!({"applicationId":"app-a","sessionKind":"application_page"});
        request["runId"] = json!("app_run_one");
        assert_eq!(
            db.prepare_ai_thread_operation(&request, 2)
                .unwrap_err()
                .reason(),
            Some("ai_application_thread_scope_invalid")
        );
        request["conversationId"] = json!("application:page");
        db.prepare_ai_thread_operation(&request, 3).unwrap();
        let mut other = request.clone();
        other["operationId"] = json!("app-other");
        other["applicationScope"]["applicationId"] = json!("app-b");
        assert_eq!(
            db.prepare_ai_thread_operation(&other, 4)
                .unwrap_err()
                .reason(),
            Some("ai_application_thread_scope_conflict")
        );
        other["applicationScope"] = Value::Null;
        assert_eq!(
            db.prepare_ai_thread_operation(&other, 5)
                .unwrap_err()
                .reason(),
            Some("ai_thread_run_mismatch")
        );
        assert_eq!(db.ai_thread_state("c").unwrap()["binding"], Value::Null);
    }

    #[test]
    fn acknowledged_application_input_does_not_enter_formal_history_references() {
        let (_fixture, mut db) = seeded();
        let mut request = prepare("app-create", "create", 0);
        request["applicationScope"] =
            json!({"applicationId":"app-a","sessionKind":"application_page"});
        request["conversationId"] = json!("application:page");
        request["runId"] = json!("app_run_one");
        db.prepare_ai_thread_operation(&request, 2).unwrap();
        let mut ack = json!({"conversationId":"application:page","runId":"app_run_one","operationId":"app-create","ownerInstanceId":"host-1","event":"acknowledge","receipt":{"nativeThreadId":"app-native"}});
        ack["event"] = json!("dispatch");
        ack["receipt"] = Value::Null;
        db.acknowledge_ai_thread_operation(&ack, 3).unwrap();
        ack["event"] = json!("acknowledge");
        ack["receipt"] = json!({"nativeThreadId":"app-native"});
        db.acknowledge_ai_thread_operation(&ack, 4).unwrap();
        request["operationId"] = json!("app-send");
        request["kind"] = json!("send");
        request["expectedBindingRevision"] =
            db.ai_thread_state("application:page").unwrap()["binding"]["bindingRevision"].clone();
        db.prepare_ai_thread_operation(&request, 5).unwrap();
        ack["operationId"] = json!("app-send");
        ack["event"] = json!("dispatch");
        ack["receipt"] = Value::Null;
        db.acknowledge_ai_thread_operation(&ack, 6).unwrap();
        ack["event"] = json!("acknowledge");
        ack["receipt"] =
            json!({"nativeThreadId":"app-native","nativeTurnId":"app-turn","clientMessageId":"m"});
        db.acknowledge_ai_thread_operation(&ack, 7).unwrap();
        assert_eq!(
            db.ai_thread_state("application:page").unwrap()["messages"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let count: i64 = db
            .connection
            .query_row(
                "SELECT count(*) FROM ai_run_native_refs WHERE run_id='app_run_one'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        let (_, state) = db.ai_runtime_state().unwrap();
        assert_eq!(state["conversations"].as_array().unwrap().len(), 1);
        assert_eq!(state["conversations"][0]["conversationId"], "c");
    }
}
