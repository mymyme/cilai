//! 退出前只读核对全部正式占用；不从进程消失推定写者已释放。
use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::ModelExecutionLease;
use serde_json::Value;
impl SqlitePersistence {
    /// # Errors
    /// 任一占用记录畸形、查询失败时不批准服务退出。
    pub fn personal_shutdown_blocking_reasons(&self) -> Result<Vec<String>, PersistenceError> {
        let mut reasons = vec![];
        let writers: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM personal_profile_writers WHERE state<>'released')",
                [],
                |row| row.get(0),
            )
            .map_err(database_error("check service shutdown profile writers"))?;
        if writers {
            reasons.push("profile_writer_active".into());
        }
        let applying:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM personal_maintenance_operation_registry WHERE state IN ('applying','capturing'))",[],|row|row.get(0)).map_err(database_error("check service shutdown maintenance"))?;
        if applying {
            reasons.push("personal_maintenance_operation_unconfirmed".into());
        }
        let mut statement = self.connection.prepare("SELECT record_json FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:lease:*'").map_err(database_error("read service shutdown model leases"))?;
        for raw in statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error("query service shutdown model leases"))?
        {
            let wrapper: Value = serde_json::from_str(
                &raw.map_err(database_error("read service shutdown model lease"))?,
            )
            .map_err(|_| invalid())?;
            let lease: ModelExecutionLease =
                serde_json::from_value(wrapper["value"].clone()).map_err(|_| invalid())?;
            if !lease.released {
                reasons.push("model_execution_active".into());
                break;
            }
        }
        Ok(reasons)
    }
}
fn invalid() -> PersistenceError {
    PersistenceError::with_reason(
        PersistenceErrorKind::Conflict,
        "app_server_writers_unconfirmed",
        "app_server_writers_unconfirmed",
    )
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;
