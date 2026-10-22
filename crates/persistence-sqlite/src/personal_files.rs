use super::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

impl SqlitePersistence {
    /// # Errors
    /// Returns an error when a file selection cannot be recorded.
    pub fn save_personal_file_grant(&self, grant: &Value) -> Result<(), PersistenceError> {
        self.connection.execute("INSERT INTO personal_file_grants(grant_id,creation_project_id,package_digest,metadata_json,expires_at) VALUES(?1,?2,?3,?4,?5)",
            params![grant["grantId"].as_str(),grant["creationProjectId"].as_str(),grant["packageDigest"].as_str(),grant.to_string(),grant["expiresAt"].as_i64()])
            .map_err(database_error("save personal file selection"))?;
        Ok(())
    }

    /// # Errors
    /// Returns an error for expired, consumed, or mismatched file selections.
    pub fn personal_file_grant(
        &mut self,
        id: &str,
        project: &str,
        digest: &str,
        now: i64,
        consume: bool,
    ) -> Result<Value, PersistenceError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin personal file access"))?;
        let found: Option<String> = transaction.query_row("SELECT metadata_json FROM personal_file_grants WHERE grant_id=?1 AND creation_project_id=?2 AND package_digest=?3 AND expires_at>?4 AND consumed=?5 AND write_digest IS NULL",
            params![id,project,digest,now,!consume], |row| row.get(0)).optional().map_err(database_error("read personal file selection"))?;
        let found = found.ok_or_else(|| {
            PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "personal_file_grant_unavailable",
                "file selection expired or consumed",
            )
        })?;
        if consume {
            transaction
                .execute(
                    "UPDATE personal_file_grants SET consumed=1 WHERE grant_id=?1",
                    [id],
                )
                .map_err(database_error("consume personal file selection"))?;
        }
        transaction
            .commit()
            .map_err(database_error("commit personal file selection"))?;
        serde_json::from_str(&found).map_err(|_| {
            PersistenceError::with_reason(
                PersistenceErrorKind::Configuration,
                "personal_file_grant_invalid",
                "invalid file selection",
            )
        })
    }

    /// # Errors
    /// Returns an error when the confirmed file write cannot be recorded.
    pub fn finish_personal_file_write(
        &self,
        id: &str,
        digest: &str,
    ) -> Result<(), PersistenceError> {
        self.connection.execute("UPDATE personal_file_grants SET write_digest=?2 WHERE grant_id=?1 AND consumed=1 AND write_digest IS NULL",params![id,digest])
            .map_err(database_error("record personal file write"))?;
        Ok(())
    }
}
