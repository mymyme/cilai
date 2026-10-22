//! 仅处理独占关闭被明确拒绝且离线 helper 尚未启动的恢复准备。
use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{
    PersonalDataIssue, PersonalDataOperation, PersonalDataOperationKind,
    PersonalDataOperationState, PersonalMaintenanceFence, PersonalMaintenanceOperationStatus,
    PersonalMaintenanceState, PersonalRestoreAbortBeforeHelperParams,
};
use rusqlite::{OptionalExtension, params};
fn reject(code: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, code, code)
}

impl SqlitePersistence {
    /// # Errors
    /// 仅活跃同实例的同拥有者能结束未执行准备；未知关闭或已应用状态保持原状。
    pub fn abort_personal_restore_before_helper(
        &mut self,
        profile: &str,
        instance: &str,
        p: &PersonalRestoreAbortBeforeHelperParams,
        now: i64,
    ) -> Result<PersonalDataOperation, PersistenceError> {
        self.verify_personal_data_quiescence(profile, &p.operation_id, instance, &p.quiescence)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin restore abort before helper"))?;
        let ready:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_profile_compatibility WHERE profile_id=?1 AND instance_fence=?2 AND state='ready')",params![profile,instance],|row|row.get(0)).map_err(database_error("verify live restore owner instance"))?;
        let raw:Option<String>=tx.query_row("SELECT record_json FROM personal_maintenance_fences WHERE profile_id=?1 AND operation_id=?2 AND fence_id=?3 AND state='draining'",params![profile,p.operation_id,p.quiescence.token],|row|row.get(0)).optional().map_err(database_error("read restore abort fence"))?;
        let fence: PersonalMaintenanceFence = serde_json::from_str(
            &raw.ok_or_else(|| reject("personal_data_quiescence_unconfirmed"))?,
        )
        .map_err(|_| reject("personal_data_record_invalid"))?;
        let owner_alive =
            contexthub_platform_adapter::instance::process_start_fingerprint(fence.owner.pid)
                .map_err(|_| reject("personal_data_quiescence_unconfirmed"))?
                .as_deref()
                == Some(fence.owner.start_fingerprint.as_str());
        if !ready
            || !owner_alive
            || fence.owner != p.quiescence.owner
            || fence.instance_fence != instance
            || fence.state != PersonalMaintenanceState::Draining
        {
            return Err(reject("personal_maintenance_owner_changed"));
        }
        if !crate::profile_writers::active_writers(&tx, profile)?.is_empty() {
            return Err(reject("personal_data_writer_active"));
        }
        let raw:String=tx.query_row("SELECT record_json FROM personal_data_operations WHERE profile_id=?1 AND operation_id=?2",params![profile,p.operation_id],|row|row.get(0)).map_err(database_error("read restore abort operation"))?;
        let mut op: PersonalDataOperation =
            serde_json::from_str(&raw).map_err(|_| reject("personal_data_record_invalid"))?;
        if op.kind != PersonalDataOperationKind::Restore
            || op.preview_digest != p.preview_digest
            || op.profile_id != profile
            || op.operation_id != p.operation_id
        {
            return Err(reject("personal_data_preview_changed"));
        }
        if op.state == PersonalDataOperationState::Incomplete
            && op.blockers.len() == 1
            && op.blockers[0].code == "personal_data_restore_exclusive_shutdown_refused"
        {
            return Ok(op);
        }
        if op.state != PersonalDataOperationState::ReadyToApply {
            return Err(reject("personal_data_operation_unconfirmed"));
        }
        let registry = crate::maintenance_operations::read(&tx, profile, &p.operation_id)?;
        if registry.kind != "restore"
            || registry.state != "ready_to_apply"
            || registry.preview_digest != p.preview_digest
        {
            return Err(reject("personal_data_operation_unconfirmed"));
        }
        op.state = PersonalDataOperationState::Incomplete;
        op.updated_at = now;
        op.blockers = vec![PersonalDataIssue {
            code: "personal_data_restore_exclusive_shutdown_refused".into(),
            category: None,
            object_id: None,
            relative_path: None,
        }];
        let raw = serde_json::to_string(&op).map_err(|_| reject("personal_data_record_invalid"))?;
        tx.execute("UPDATE personal_data_operations SET state='incomplete',record_json=?3,updated_at=?4 WHERE profile_id=?1 AND operation_id=?2 AND state='ready_to_apply'",params![profile,p.operation_id,raw,now]).map_err(database_error("record refused exclusive restore shutdown"))?;
        crate::maintenance_operations::register(
            &tx,
            &PersonalMaintenanceOperationStatus {
                operation_id: op.operation_id.clone(),
                profile_id: profile.into(),
                kind: "restore".into(),
                state: "incomplete".into(),
                preview_digest: op.preview_digest.clone(),
            },
            now,
        )?;
        tx.commit()
            .map_err(database_error("commit restore abort before helper"))?;
        Ok(op)
    }
}

#[cfg(test)]
#[path = "restore_abort_tests.rs"]
mod tests;
