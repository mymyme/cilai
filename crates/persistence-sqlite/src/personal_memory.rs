//! 当前资料记忆授权的轻量只读查询，不读取会话正文与原生数据库。
use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{
    PersonalMemoryAuthorization, PersonalMemoryEnableConfirmation, PersonalMemoryPreferences,
};
use rusqlite::{Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde_json::Value;
fn reject() -> PersistenceError {
    PersistenceError::with_reason(
        PersistenceErrorKind::Conflict,
        "personal_memory_authorization_invalid",
        "personal_memory_authorization_invalid",
    )
}
fn scalar<T: DeserializeOwned>(db: &Connection, id: &str) -> Result<Option<T>, PersistenceError> {
    let raw: Option<String> = db
        .query_row(
            "SELECT record_json FROM ai_runtime_entries WHERE record_id=?1",
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error("read memory authorization scalar"))?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&raw).map_err(|_| reject())?;
    if value["entryId"] != id || value["field"] != id || value["shape"] != "scalar" {
        return Err(reject());
    }
    serde_json::from_value(value["value"].clone())
        .map(Some)
        .map_err(|_| reject())
}
impl SqlitePersistence {
    /// # Errors
    /// 畸形或不可读取的授权记录明确失败，不解释为许可。
    pub fn personal_memory_authorization(
        &self,
    ) -> Result<PersonalMemoryAuthorization, PersistenceError> {
        let preferences =
            scalar::<PersonalMemoryPreferences>(&self.connection, "memoryPreferences")?
                .unwrap_or_default();
        let confirmation = scalar::<Option<PersonalMemoryEnableConfirmation>>(
            &self.connection,
            "memoryEnableConfirmation",
        )?
        .flatten();
        Ok(PersonalMemoryAuthorization {
            preferences,
            confirmation,
        })
    }
}
pub(crate) fn disable_for_reset(db: &Connection, now: i64) -> Result<(), PersistenceError> {
    let value = serde_json::json!({"entryId":"memoryPreferences","field":"memoryPreferences","shape":"scalar","value":{"enabled":false,"fromToolChats":false}});
    db.execute("INSERT INTO ai_runtime_entries(record_id,position,record_json,digest,updated_at) VALUES('memoryPreferences',0,?1,?2,?3) ON CONFLICT(record_id) DO UPDATE SET record_json=excluded.record_json,digest=excluded.digest,updated_at=excluded.updated_at",rusqlite::params![value.to_string(),crate::ai_business_ledger::digest(&value),now]).map_err(database_error("disable memory before explicit reset"))?;
    db.execute(
        "DELETE FROM ai_runtime_entries WHERE record_id='memoryEnableConfirmation'",
        [],
    )
    .map_err(database_error(
        "invalidate memory consent before explicit reset",
    ))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (std::path::PathBuf, SqlitePersistence) {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "cilai-memory-authorization-{}",
            uuid::Uuid::new_v4().simple()
        ));
        contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
        let db = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        (root, db)
    }
    fn insert(db: &Connection, id: &str, value: &Value) {
        db.execute("INSERT INTO ai_runtime_entries(record_id,position,record_json,digest,updated_at) VALUES(?1,0,?2,?3,1) ON CONFLICT(record_id) DO UPDATE SET record_json=excluded.record_json,digest=excluded.digest",rusqlite::params![id,value.to_string(),crate::ai_business_ledger::digest(value)]).unwrap();
    }
    #[test]
    fn absent_authorization_is_disabled_without_confirmation() {
        let (root, db) = fixture();
        let authorization = db.personal_memory_authorization().unwrap();
        assert!(!authorization.preferences.enabled);
        assert!(!authorization.preferences.from_tool_chats);
        assert!(authorization.confirmation.is_none());
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn other_connection_disabling_is_observed_without_loading_history() {
        let (root, db) = fixture();
        insert(
            &db.connection,
            "memoryPreferences",
            &serde_json::json!({"entryId":"memoryPreferences","field":"memoryPreferences","shape":"scalar","value":{"enabled":true,"fromToolChats":true}}),
        );
        insert(
            &db.connection,
            "memoryEnableConfirmation",
            &serde_json::json!({"entryId":"memoryEnableConfirmation","field":"memoryEnableConfirmation","shape":"scalar","value":{"consentDigest":"source-consent","fromToolChats":true,"confirmedAt":1}}),
        );
        let other = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        assert!(
            other
                .personal_memory_authorization()
                .unwrap()
                .preferences
                .enabled
        );
        disable_for_reset(&db.connection, 2).unwrap();
        let authorization = other.personal_memory_authorization().unwrap();
        assert!(!authorization.preferences.enabled);
        assert!(authorization.confirmation.is_none());
        drop(other);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn malformed_scalar_is_an_error_instead_of_authorization() {
        let (root, db) = fixture();
        insert(
            &db.connection,
            "memoryPreferences",
            &serde_json::json!({"entryId":"memoryPreferences","field":"different","shape":"scalar","value":{"enabled":true,"fromToolChats":true}}),
        );
        assert_eq!(
            db.personal_memory_authorization().unwrap_err().reason(),
            Some("personal_memory_authorization_invalid")
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}
