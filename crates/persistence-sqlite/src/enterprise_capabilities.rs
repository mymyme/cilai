use super::{PersistenceError, SqlitePersistence, database_error};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

impl SqlitePersistence {
    /// # Errors
    /// Returns an error when the enterprise binding cannot be read.
    pub fn enterprise_capability_binding(
        &self,
        project: &str,
    ) -> Result<Option<Value>, PersistenceError> {
        self.connection.query_row("SELECT tenant_id,installation_id,package_digest,release_json FROM enterprise_capability_bindings WHERE creation_project_id=?1",[project],|row| {
            Ok(json!({"tenantId":row.get::<_,String>(0)?,"installationId":row.get::<_,String>(1)?,"packageDigest":row.get::<_,String>(2)?,"releaseJson":row.get::<_,String>(3)?}))
        }).optional().map_err(database_error("read enterprise capability binding"))
    }
}
