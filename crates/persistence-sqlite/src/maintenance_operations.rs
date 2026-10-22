//! 各资料维护业务在同一事务内登记最小操作状态，写者栅栏不依赖具体业务结果格式。
use crate::{PersistenceError, PersistenceErrorKind, database_error};
use contexthub_protocol::PersonalMaintenanceOperationStatus;
use rusqlite::{Connection, OptionalExtension, params};
pub(crate) fn register(
    db: &Connection,
    operation: &PersonalMaintenanceOperationStatus,
    now: i64,
) -> Result<(), PersistenceError> {
    let count=db.execute("INSERT INTO personal_maintenance_operation_registry(operation_id,profile_id,operation_kind,preview_digest,state,updated_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(operation_id) DO UPDATE SET preview_digest=excluded.preview_digest,state=excluded.state,updated_at=excluded.updated_at WHERE personal_maintenance_operation_registry.profile_id=excluded.profile_id AND personal_maintenance_operation_registry.operation_kind=excluded.operation_kind",params![operation.operation_id,operation.profile_id,operation.kind,operation.preview_digest,operation.state,now]).map_err(database_error("save maintenance operation registry"))?;
    if count != 1 {
        return Err(PersistenceError::with_reason(
            PersistenceErrorKind::Conflict,
            "personal_maintenance_operation_conflict",
            "personal_maintenance_operation_conflict",
        ));
    }
    Ok(())
}
pub(crate) fn read(
    db: &Connection,
    profile: &str,
    id: &str,
) -> Result<PersonalMaintenanceOperationStatus, PersistenceError> {
    db.query_row("SELECT operation_id,profile_id,operation_kind,state,preview_digest FROM personal_maintenance_operation_registry WHERE profile_id=?1 AND operation_id=?2",params![profile,id],|row|Ok(PersonalMaintenanceOperationStatus{operation_id:row.get(0)?,profile_id:row.get(1)?,kind:row.get(2)?,state:row.get(3)?,preview_digest:row.get(4)?})).optional().map_err(database_error("read maintenance operation registry"))?.ok_or_else(||PersistenceError::with_reason(PersistenceErrorKind::Conflict,"personal_maintenance_operation_not_found","personal_maintenance_operation_not_found"))
}

/// 当前发行件支持的个人备份业务数据库版本；不修改归档中已记录的源版本。
///
/// # Panics
/// 发行件的迁移列表为空或版本超出 u32 时停止，不能为归档伪造版本。
#[must_use]
pub fn personal_data_schema_version() -> u32 {
    u32::try_from(
        crate::MIGRATIONS
            .last()
            .expect("schema migrations are nonempty")
            .version,
    )
    .expect("compiled migration version is within u32")
}
