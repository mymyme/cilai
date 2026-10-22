use super::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Map, Value};
use std::collections::HashSet;

fn invalid_state() -> PersistenceError {
    PersistenceError::with_reason(
        PersistenceErrorKind::Configuration,
        "ai_state_invalid",
        "Invalid AI state",
    )
}

/// 本地记忆的"上次已保存"与库里实际不一致。按增量继续写只会把差异固化，必须明确失败。
fn baseline_mismatch(detail: &str) -> PersistenceError {
    PersistenceError::with_reason(
        PersistenceErrorKind::Conflict,
        "ai_state_delta_mismatch",
        format!("AI state delta does not match stored {detail}"),
    )
}

/// 按条目独立存储的一类 AI 状态。
struct EntityTable {
    /// 在聚合状态里的字段名。
    field: &'static str,
    /// 条目内容里的编号字段。
    id_field: &'static str,
    table: &'static str,
    id_column: &'static str,
    json_column: &'static str,
}

const CONVERSATIONS: EntityTable = EntityTable {
    field: "conversations",
    id_field: "conversationId",
    table: "ai_conversations",
    id_column: "conversation_id",
    json_column: "conversation_json",
};
const MESSAGES: EntityTable = EntityTable {
    field: "messages",
    id_field: "messageId",
    table: "ai_messages",
    id_column: "message_id",
    json_column: "message_json",
};
const RUNS: EntityTable = EntityTable {
    field: "runs",
    id_field: "runId",
    table: "ai_runs",
    id_column: "run_id",
    json_column: "run_json",
};
const ENTITY_TABLES: [&EntityTable; 3] = [&CONVERSATIONS, &MESSAGES, &RUNS];
const CONTEXT_FIELD: &str = "applicationContext";
const CONTEXT_REF: &str = "applicationContextId";

/// 一类条目的变化：`upserts` 为（位置，完整内容），`total` 为应用后应有的条目总数。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiStateEntityChanges {
    pub upserts: Vec<(i64, Value)>,
    pub removed_ids: Vec<String>,
    pub total: i64,
}

/// AI 运行状态的一次变更。`full` 为真时各类条目完整给出，库里多出的条目一律删除。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiStateChanges {
    pub full: bool,
    pub payload: Option<Value>,
    pub conversations: AiStateEntityChanges,
    pub messages: AiStateEntityChanges,
    pub runs: AiStateEntityChanges,
}

impl AiStateChanges {
    /// 由完整状态构造一次全量同步。
    ///
    /// # Errors
    /// 状态不是对象或缺少任一类条目列表时返回错误。
    pub fn full_from_state(state: &Value) -> Result<Self, PersistenceError> {
        let mut payload = state.as_object().cloned().ok_or_else(invalid_state)?;
        let mut take = |table: &EntityTable| -> Result<AiStateEntityChanges, PersistenceError> {
            let items = payload.remove(table.field).ok_or_else(invalid_state)?;
            let Value::Array(items) = items else {
                return Err(invalid_state());
            };
            let total = i64::try_from(items.len()).map_err(|_| invalid_state())?;
            let upserts = items
                .into_iter()
                .enumerate()
                .map(|(position, item)| {
                    i64::try_from(position)
                        .map(|position| (position, item))
                        .map_err(|_| invalid_state())
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(AiStateEntityChanges {
                upserts,
                removed_ids: Vec::new(),
                total,
            })
        };
        let conversations = take(&CONVERSATIONS)?;
        let messages = take(&MESSAGES)?;
        let runs = take(&RUNS)?;
        Ok(Self {
            full: true,
            payload: Some(Value::Object(payload)),
            conversations,
            messages,
            runs,
        })
    }

    fn entities(&self, table: &EntityTable) -> &AiStateEntityChanges {
        match table.field {
            "conversations" => &self.conversations,
            "messages" => &self.messages,
            _ => &self.runs,
        }
    }
}

pub(super) fn revision(connection: &Connection) -> Result<i64, PersistenceError> {
    connection
        .query_row(
            "SELECT state_revision FROM ai_runtime_revision WHERE state_key='default'",
            [],
            |row| row.get(0),
        )
        .map_err(database_error("read AI state revision"))
}

pub(super) fn advance_revision(
    transaction: &Transaction<'_>,
    expected: i64,
) -> Result<i64, PersistenceError> {
    let next = expected.checked_add(1).ok_or_else(invalid_state)?;
    let changed = transaction.execute("UPDATE ai_runtime_revision SET state_revision=?1 WHERE state_key='default' AND state_revision=?2", params![next, expected])
        .map_err(database_error("advance AI state revision"))?;
    if changed != 1 {
        return Err(revision_conflict());
    }
    Ok(next)
}

fn revision_conflict() -> PersistenceError {
    PersistenceError::with_reason(
        PersistenceErrorKind::Conflict,
        "ai_state_revision_conflict",
        "AI runtime state changed before commit",
    )
}

/// 在调用方的事务内应用一次变更，并在提交前核对各类条目总数。
pub(super) fn apply_changes(
    transaction: &Transaction<'_>,
    changes: &AiStateChanges,
    now: i64,
) -> Result<(), PersistenceError> {
    if changes.full && changes.payload.is_none() {
        return Err(invalid_state());
    }
    for table in ENTITY_TABLES {
        apply_entities(transaction, table, changes.entities(table), changes.full)?;
    }
    if let Some(payload) = changes.payload.as_ref() {
        let mut payload = payload.as_object().cloned().ok_or_else(invalid_state)?;
        for table in ENTITY_TABLES {
            // 全量同步时由完整状态拆出，已不含条目列表；增量里出现条目列表说明调用方把
            // 整份状态当成了其余字段，照写会让同一条目在两处各存一份。
            if payload.remove(table.field).is_some() {
                return Err(invalid_state());
            }
        }
        let encoded = serde_json::to_string(&payload).map_err(|_| invalid_state())?;
        transaction
            .execute(
                "UPDATE ai_runtime_state SET state_json=?1,updated_at=?2 WHERE state_key='default'",
                params![encoded, now],
            )
            .map_err(database_error("save AI runtime payload"))?;
    }
    super::skill_installations::enqueue(transaction)?;
    super::capability_creation::enqueue(transaction)?;
    super::incident_recovery::reconcile(transaction)?;
    Ok(())
}

fn entity_id<'a>(table: &EntityTable, value: &'a Value) -> Result<&'a str, PersistenceError> {
    value
        .get(table.id_field)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(invalid_state)
}

fn apply_entities(
    transaction: &Transaction<'_>,
    table: &EntityTable,
    changes: &AiStateEntityChanges,
    full: bool,
) -> Result<(), PersistenceError> {
    let mut upserted = HashSet::new();
    let upsert = format!(
        "INSERT INTO {table}({id},position,{json}) VALUES (?1,?2,?3) \
         ON CONFLICT({id}) DO UPDATE SET position=excluded.position,{json}=excluded.{json} \
         WHERE position<>excluded.position OR {json}<>excluded.{json}",
        table = table.table,
        id = table.id_column,
        json = table.json_column,
    );
    let mut statement = transaction
        .prepare_cached(&upsert)
        .map_err(database_error("prepare AI entity save"))?;
    for (position, value) in &changes.upserts {
        if *position < 0 || !value.is_object() {
            return Err(invalid_state());
        }
        let id = entity_id(table, value)?;
        if !upserted.insert(id) {
            return Err(invalid_state());
        }
        let previous: Option<String> = transaction
            .query_row(
                &format!(
                    "SELECT {} FROM {} WHERE {}=?1",
                    table.json_column, table.table, table.id_column
                ),
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error("read AI text integrity baseline"))?;
        if let Some(previous) = previous {
            let previous: Value = serde_json::from_str(&previous).map_err(|_| invalid_state())?;
            if super::text_integrity::introduces_replacement(&previous, value) {
                return Err(super::ai_business_ledger::reject(
                    "ai_text_corruption_detected",
                ));
            }
        }
        let encoded = if table.field == MESSAGES.field {
            stored_message(transaction, value)?
        } else {
            serde_json::to_string(value).map_err(|_| invalid_state())?
        };
        statement
            .execute(params![id, position, encoded])
            .map_err(database_error("save AI entity"))?;
    }
    drop(statement);
    if full {
        if !changes.removed_ids.is_empty() {
            return Err(invalid_state());
        }
        let existing = {
            let mut statement = transaction
                .prepare(&format!("SELECT {} FROM {}", table.id_column, table.table))
                .map_err(database_error("prepare AI entity IDs"))?;
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(database_error("read AI entity IDs"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error("decode AI entity IDs"))?
        };
        for id in existing {
            if !upserted.contains(id.as_str()) {
                delete_entity(transaction, table, &id)?;
            }
        }
    } else {
        for id in &changes.removed_ids {
            if id.is_empty() || upserted.contains(id.as_str()) {
                return Err(invalid_state());
            }
            if delete_entity(transaction, table, id)? != 1 {
                return Err(baseline_mismatch(table.field));
            }
        }
    }
    let stored: i64 = transaction
        .query_row(
            &format!("SELECT count(*) FROM {}", table.table),
            [],
            |row| row.get(0),
        )
        .map_err(database_error("count AI entities"))?;
    if stored != changes.total {
        return Err(baseline_mismatch(table.field));
    }
    if table.field == MESSAGES.field && (full || !changes.removed_ids.is_empty()) {
        transaction
            .execute(
                "DELETE FROM ai_application_contexts WHERE context_id NOT IN (
                    SELECT json_extract(message_json,'$.applicationContextId') FROM ai_messages
                    WHERE json_extract(message_json,'$.applicationContextId') IS NOT NULL)",
                [],
            )
            .map_err(database_error("remove unused AI application contexts"))?;
    }
    Ok(())
}

/// 消息的存储形式：应用上下文按内容只存一份，消息里记其编号。
fn stored_message(
    transaction: &Transaction<'_>,
    message: &Value,
) -> Result<String, PersistenceError> {
    // 编号字段只由存储层写入；调用方带着它来，说明传入的是存储形式而非消息本身
    if message.get(CONTEXT_REF).is_some() {
        return Err(invalid_state());
    }
    let Some(context) = message
        .get(CONTEXT_FIELD)
        .filter(|context| context.is_object())
    else {
        return serde_json::to_string(message).map_err(|_| invalid_state());
    };
    let context = serde_json::to_string(context).map_err(|_| invalid_state())?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO ai_application_contexts(context_json) VALUES (?1)",
            [&context],
        )
        .map_err(database_error("save AI application context"))?;
    let context_id: i64 = transaction
        .query_row(
            "SELECT context_id FROM ai_application_contexts WHERE context_json=?1",
            [&context],
            |row| row.get(0),
        )
        .map_err(database_error("read AI application context"))?;
    let mut stored = message.as_object().cloned().ok_or_else(invalid_state)?;
    stored.remove(CONTEXT_FIELD);
    stored.insert(CONTEXT_REF.to_owned(), Value::from(context_id));
    serde_json::to_string(&stored).map_err(|_| invalid_state())
}

/// 读出存储形式的消息时把应用上下文放回原处。
pub(super) fn hydrate_message(
    transaction: &Transaction<'_>,
    mut message: Value,
) -> Result<Value, PersistenceError> {
    let Some(context_id) = message.get(CONTEXT_REF).and_then(Value::as_i64) else {
        return Ok(message);
    };
    let context: String = transaction
        .query_row(
            "SELECT context_json FROM ai_application_contexts WHERE context_id=?1",
            [context_id],
            |row| row.get(0),
        )
        .map_err(database_error("read AI application context"))?;
    let context: Value = serde_json::from_str(&context).map_err(|_| invalid_state())?;
    let object = message.as_object_mut().ok_or_else(invalid_state)?;
    object.remove(CONTEXT_REF);
    object.insert(CONTEXT_FIELD.to_owned(), context);
    Ok(message)
}

fn delete_entity(
    transaction: &Transaction<'_>,
    table: &EntityTable,
    id: &str,
) -> Result<usize, PersistenceError> {
    if table.field == CONVERSATIONS.field {
        let busy: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM ai_conversation_thread_bindings WHERE conversation_id=?1 AND active_run_id IS NOT NULL)", [id], |row| row.get(0))
            .map_err(database_error("check AI thread writer before deletion"))?;
        if busy {
            return Err(PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "thread_owner_busy",
                "AI thread writer still active",
            ));
        }
        transaction.execute("UPDATE ai_conversation_thread_bindings SET status='retired',binding_revision=binding_revision+1 WHERE conversation_id=?1", [id])
            .map_err(database_error("retire deleted AI thread binding"))?;
    }
    transaction
        .execute(
            &format!("DELETE FROM {} WHERE {}=?1", table.table, table.id_column),
            [id],
        )
        .map_err(database_error("remove AI entity"))
}

fn read_entities(
    transaction: &Transaction<'_>,
    table: &EntityTable,
) -> Result<Vec<Value>, PersistenceError> {
    let mut statement = transaction
        .prepare(&format!(
            "SELECT {} FROM {} ORDER BY position,{}",
            table.json_column, table.table, table.id_column
        ))
        .map_err(database_error("prepare AI entities"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(database_error("read AI entities"))?;
    rows.map(|row| {
        let value =
            serde_json::from_str::<Value>(&row.map_err(database_error("decode AI entity"))?)
                .map_err(|_| invalid_state())?;
        if table.field == MESSAGES.field {
            hydrate_message(transaction, value)
        } else {
            Ok(value)
        }
    })
    .collect()
}

impl SqlitePersistence {
    /// # Errors
    /// 状态、会话或修订无法读取时返回错误。
    pub fn ai_runtime_state(&self) -> Result<(i64, Value), PersistenceError> {
        if self.native_ai_storage_active()? {
            return Err(super::ai_business_ledger::reject(
                "ai_legacy_storage_retired",
            ));
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(database_error("begin AI state read"))?;
        let revision = revision(&transaction)?;
        let encoded: String = transaction
            .query_row(
                "SELECT state_json FROM ai_runtime_state WHERE state_key='default'",
                [],
                |row| row.get(0),
            )
            .map_err(database_error("read AI payload"))?;
        let mut state: Map<String, Value> = match serde_json::from_str(&encoded) {
            Ok(Value::Object(state)) => state,
            _ => return Err(invalid_state()),
        };
        for table in ENTITY_TABLES {
            let entities = read_entities(&transaction, table)?;
            state.insert(table.field.to_owned(), Value::Array(entities));
        }
        transaction
            .commit()
            .map_err(database_error("commit AI state read"))?;
        Ok((revision, Value::Object(state)))
    }

    /// 以完整状态整体替换 AI 运行状态。
    ///
    /// # Errors
    /// 修订冲突、状态不合法或数据库写入失败时返回错误。
    pub fn replace_ai_runtime_state(
        &mut self,
        expected_revision: i64,
        state: &Value,
        updated_at: i64,
    ) -> Result<i64, PersistenceError> {
        self.apply_ai_runtime_changes(
            expected_revision,
            &AiStateChanges::full_from_state(state)?,
            updated_at,
        )
    }

    /// 应用一次 AI 运行状态变更，只改写其中给出的条目。
    ///
    /// # Errors
    /// 修订冲突、基线不一致、状态不合法或数据库写入失败时返回错误。
    pub fn apply_ai_runtime_changes(
        &mut self,
        expected_revision: i64,
        changes: &AiStateChanges,
        updated_at: i64,
    ) -> Result<i64, PersistenceError> {
        if self.native_ai_storage_active()? {
            return Err(super::ai_business_ledger::reject(
                "ai_legacy_storage_retired",
            ));
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin AI state update"))?;
        let next_revision = advance_revision(&transaction, expected_revision)?;
        apply_changes(&transaction, changes, updated_at)?;
        transaction
            .commit()
            .map_err(database_error("commit AI state update"))?;
        Ok(next_revision)
    }

    /// # Errors
    /// 修订冲突、目标会话缺失或数据库提交失败时返回错误。
    pub fn set_ai_conversations_archived(
        &mut self,
        expected: i64,
        ids: &[String],
        archived: bool,
        now: i64,
    ) -> Result<(i64, Vec<Value>, Vec<String>), PersistenceError> {
        if self.native_ai_storage_active()? {
            return Err(super::ai_business_ledger::reject(
                "ai_legacy_storage_retired",
            ));
        }
        if ids.is_empty()
            || ids.len() > 1000
            || ids.iter().any(|id| id.is_empty() || id.len() > 160)
        {
            return Err(invalid_state());
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin AI archive"))?;
        let current = revision(&transaction)?;
        if current != expected {
            return Err(revision_conflict());
        }
        let mut conversations = Vec::new();
        let mut changed = Vec::new();
        let mut seen = HashSet::new();
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            let encoded: String = transaction
                .query_row(
                    "SELECT conversation_json FROM ai_conversations WHERE conversation_id=?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(database_error("read archived conversation"))?
                .ok_or_else(|| {
                    PersistenceError::with_reason(
                        PersistenceErrorKind::Conflict,
                        "conversation_not_found",
                        "Conversation does not exist",
                    )
                })?;
            let mut conversation: Value =
                serde_json::from_str(&encoded).map_err(|_| invalid_state())?;
            let existing = conversation
                .get("archived")
                .and_then(Value::as_bool)
                .ok_or_else(invalid_state)?;
            if existing != archived {
                conversation["archived"] = Value::Bool(archived);
                conversation["updatedAt"] = Value::from(now);
                transaction
                    .execute(
                        "UPDATE ai_conversations SET conversation_json=?1 WHERE conversation_id=?2",
                        params![conversation.to_string(), id],
                    )
                    .map_err(database_error("update conversation archive"))?;
                changed.push(id.clone());
            }
            conversations.push(conversation);
        }
        let next = if changed.is_empty() {
            current
        } else {
            advance_revision(&transaction, expected)?
        };
        transaction
            .commit()
            .map_err(database_error("commit AI archive"))?;
        Ok((next, conversations, changed))
    }
}
