use super::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct SkillInstallationRecord {
    pub run_id: String,
    pub status: String,
    pub source_json: String,
    pub result_json: Option<String>,
}

/// 为尚未登记的已完成运行建立技能交付事实。
///
/// 按运行在列表中的顺序处理；回复取该运行最后一条助手消息，没有回复的运行暂不登记，
/// 等回复写入后的下一次保存再补上。
pub(super) fn enqueue(transaction: &Transaction<'_>) -> Result<(), PersistenceError> {
    let pending = {
        let mut statement = transaction
            .prepare(
                "SELECT run.run_id,run.run_json FROM ai_business_runs run
                 WHERE run.status='completed'
                   AND NOT EXISTS (SELECT 1 FROM ai_skill_installations known WHERE known.run_id=run.run_id)
                 ORDER BY run.position,run.run_id",
            )
            .map_err(database_error("prepare skill installation candidates"))?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error("query skill installation candidates"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read skill installation candidates"))?
    };
    for (id, run) in pending {
        let run: Value = serde_json::from_str(&run).map_err(|_| invalid_entity())?;
        let message: Option<String> = transaction
            .query_row(
                "SELECT message_json FROM ai_business_messages WHERE run_id=?1 AND role='assistant'
                 ORDER BY position DESC,message_id DESC LIMIT 1",
                [&id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error("read skill installation reply"))?;
        let Some(message) = message else {
            continue;
        };
        let message: Value = serde_json::from_str(&message).map_err(|_| invalid_entity())?;
        let message = super::ai_state::hydrate_message(transaction, message)?;
        let conversation: Option<String> = transaction
            .query_row(
                "SELECT conversation_json FROM ai_business_conversations WHERE conversation_id=?1",
                [run["conversationId"].as_str().unwrap_or_default()],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error("read skill installation conversation"))?;
        let conversation = conversation
            .map(|encoded| serde_json::from_str::<Value>(&encoded).map_err(|_| invalid_entity()))
            .transpose()?;
        let source = json!({
            "runId": id, "conversationId": run["conversationId"], "superseded": run["superseded"],
            "workspaceRoot": conversation.as_ref().map(|conversation| &conversation["context"]["workspaceRoot"]),
            "message": message, "artifacts": run["artifacts"],
        });
        transaction.execute("INSERT INTO ai_skill_installations(run_id,status,source_json) VALUES (?1,'pending',?2)", params![id,source.to_string()])
            .map_err(database_error("enqueue skill installation"))?;
    }
    Ok(())
}

fn invalid_entity() -> PersistenceError {
    PersistenceError::with_reason(
        PersistenceErrorKind::Configuration,
        "ai_state_invalid",
        "Invalid AI state",
    )
}

impl SqlitePersistence {
    /// 读取已完成运行对应的技能处理事实。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn skill_installation(
        &self,
        run_id: &str,
    ) -> Result<Option<SkillInstallationRecord>, PersistenceError> {
        self.connection.query_row("SELECT run_id,status,source_json,result_json FROM ai_skill_installations WHERE run_id=?1", [run_id], |row| Ok(SkillInstallationRecord {
            run_id: row.get(0)?, status: row.get(1)?, source_json: row.get(2)?, result_json: row.get(3)?,
        })).optional().map_err(database_error("read skill installation"))
    }

    /// 读取等待服务处理的运行。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn pending_skill_installations(&self) -> Result<Vec<String>, PersistenceError> {
        let mut statement = self.connection.prepare("SELECT run_id FROM ai_skill_installations WHERE status='pending' ORDER BY rowid LIMIT 8")
            .map_err(database_error("prepare pending skill installations"))?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(database_error("query pending skill installations"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read pending skill installations"))
    }

    /// 保存技能处理结果，失败结果仅在员工重试时重新处理。
    ///
    /// # Errors
    /// 数据库不可写时返回错误。
    pub fn finish_skill_installation(
        &self,
        run_id: &str,
        status: &str,
        result: &Value,
    ) -> Result<(), PersistenceError> {
        self.connection
            .execute(
                "UPDATE ai_skill_installations SET status=?2,result_json=?3 WHERE run_id=?1",
                params![run_id, status, result.to_string()],
            )
            .map_err(database_error("finish skill installation"))?;
        Ok(())
    }

    /// 核对源运行仍存在且可作为技能交付来源。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn skill_source_run_available(&self, run_id: &str) -> Result<bool, PersistenceError> {
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM ai_business_runs WHERE run_id=?1 AND status='completed' AND COALESCE(json_extract(run_json,'$.superseded'),0)=0)", [run_id], |row| row.get(0))
            .map_err(database_error("check skill source run"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_marks_existing_completed_runs_historical_without_queuing_them() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE ai_runtime_state(state_json TEXT); CREATE TABLE ai_conversations(conversation_id TEXT,conversation_json TEXT);").unwrap();
        let state = json!({"runs":[{"runId":"old","conversationId":"conversation","status":"completed","artifacts":[]},
            {"runId":"active","status":"running"}], "messages":[{"runId":"old","role":"assistant","messageId":"reply","content":"技能内容"}]});
        connection
            .execute(
                "INSERT INTO ai_runtime_state VALUES (?1)",
                [state.to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ai_conversations VALUES ('conversation',?1)",
                [json!({"context":{"workspaceRoot":"/tmp/project"}}).to_string()],
            )
            .unwrap();
        connection
            .execute_batch(include_str!(
                "../../../migrations/0035_add_ai_skill_installations.sql"
            ))
            .unwrap();
        let (status, source): (String, String) = connection
            .query_row(
                "SELECT status,source_json FROM ai_skill_installations WHERE run_id='old'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "historical");
        let source: Value = serde_json::from_str(&source).unwrap();
        assert_eq!(source["message"]["messageId"], "reply");
        assert_eq!(source["workspaceRoot"], "/tmp/project");
        let pending: i64 = connection
            .query_row(
                "SELECT count(*) FROM ai_skill_installations WHERE status='pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
    }
}
