//! 显式跨全部受管原生记忆作用域的删除账本和一次性维护写者许可。
use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{
    ModelExecutionOwner, PersonalMaintenanceFence, PersonalMaintenanceOperationStatus,
    PersonalMaintenanceState, PersonalMemoryResetOperation, PersonalMemoryResetOutcome,
    PersonalMemoryResetScopeCompleteParams, PersonalMemoryResetState,
    PersonalMemoryResetWriterParams, PersonalWriterLease, PersonalWriterMode, PersonalWriterState,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "personal_memory_reset_tests.rs"]
mod tests;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetScopeAuthorization {
    pub scope_id: String,
    pub native_home_relative_path: String,
    pub native_home_fingerprint: String,
    pub permit_token: Option<String>,
    pub writer_lease_id: Option<String>,
    pub writer_client_operation_id: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalMemoryResetRecord {
    pub operation: PersonalMemoryResetOperation,
    pub scopes: Vec<PersonalMemoryResetScopeAuthorization>,
    pub owner: Option<ModelExecutionOwner>,
    pub fence_id: Option<String>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}
fn reject(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}
fn encode(value: &impl Serialize) -> Result<String, PersistenceError> {
    serde_json::to_string(value).map_err(|_| reject("personal_memory_reset_record_invalid"))
}
fn state(value: &PersonalMemoryResetState) -> &'static str {
    match value {
        PersonalMemoryResetState::Prepared => "prepared",
        PersonalMemoryResetState::Applying => "applying",
        PersonalMemoryResetState::Completed => "completed",
        PersonalMemoryResetState::Incomplete => "incomplete",
        PersonalMemoryResetState::Cancelled => "cancelled",
    }
}
fn read(
    db: &Connection,
    profile: &str,
    id: &str,
) -> Result<Option<PersonalMemoryResetRecord>, PersistenceError> {
    let raw:Option<String>=db.query_row("SELECT record_json FROM personal_memory_reset_operations WHERE profile_id=?1 AND operation_id=?2",params![profile,id],|row|row.get(0)).optional().map_err(database_error("read personal memory reset operation"))?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|_| reject("personal_memory_reset_record_invalid"))
    })
    .transpose()
}
fn registry(db: &Connection, record: &PersonalMemoryResetRecord) -> Result<(), PersistenceError> {
    let op = &record.operation;
    crate::maintenance_operations::register(
        db,
        &PersonalMaintenanceOperationStatus {
            operation_id: op.operation_id.clone(),
            profile_id: op.profile_id.clone(),
            kind: "memory_reset".into(),
            state: state(&op.state).into(),
            preview_digest: op.preview_digest.clone(),
        },
        op.updated_at,
    )
}
fn save(db: &Connection, record: &PersonalMemoryResetRecord) -> Result<(), PersistenceError> {
    let op = &record.operation;
    db.execute("UPDATE personal_memory_reset_operations SET record_json=?3,state=?4,updated_at=?5 WHERE profile_id=?1 AND operation_id=?2",params![op.profile_id,op.operation_id,encode(record)?,state(&op.state),op.updated_at]).map_err(database_error("save personal memory reset result"))?;
    registry(db, record)
}
fn alive(owner: &ModelExecutionOwner) -> Result<bool, PersistenceError> {
    Ok(
        contexthub_platform_adapter::instance::process_start_fingerprint(owner.pid)
            .map_err(|_| reject("personal_memory_reset_owner_unconfirmed"))?
            .as_deref()
            == Some(owner.start_fingerprint.as_str()),
    )
}
fn checked_fence(
    db: &Connection,
    record: &PersonalMemoryResetRecord,
    owner: &ModelExecutionOwner,
    instance: &str,
) -> Result<(), PersistenceError> {
    if record.owner.as_ref() != Some(owner) || !alive(owner)? {
        return Err(reject("personal_memory_reset_owner_unconfirmed"));
    }
    let raw:Option<String>=db.query_row("SELECT record_json FROM personal_maintenance_fences WHERE profile_id=?1 AND operation_id=?2 AND fence_id=?3 AND state='draining'",params![record.operation.profile_id,record.operation.operation_id,record.fence_id],|row|row.get(0)).optional().map_err(database_error("read memory reset maintenance fence"))?;
    let fence: PersonalMaintenanceFence =
        serde_json::from_str(&raw.ok_or_else(|| reject("personal_memory_reset_permit_invalid"))?)
            .map_err(|_| reject("personal_memory_reset_permit_invalid"))?;
    if fence.owner != *owner
        || fence.instance_fence != instance
        || fence.state != PersonalMaintenanceState::Draining
    {
        return Err(reject("personal_memory_reset_permit_invalid"));
    }
    Ok(())
}
fn writer(
    db: &Connection,
    profile: &str,
    id: &str,
) -> Result<PersonalWriterLease, PersistenceError> {
    let raw: Option<String> = db
        .query_row(
            "SELECT record_json FROM personal_profile_writers WHERE profile_id=?1 AND lease_id=?2",
            params![profile, id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error("read memory reset writer receipt"))?;
    serde_json::from_str(&raw.ok_or_else(|| reject("personal_memory_reset_writer_unconfirmed"))?)
        .map_err(|_| reject("personal_memory_reset_writer_unconfirmed"))
}
fn writer_exited(lease: &PersonalWriterLease) -> Result<bool, PersistenceError> {
    if lease.state != PersonalWriterState::Released
        || lease.process_mode != PersonalWriterMode::LocalProcess
    {
        return Ok(false);
    }
    for process in &lease.processes {
        if contexthub_platform_adapter::instance::process_start_fingerprint(process.pid)
            .map_err(|_| reject("personal_memory_reset_writer_unconfirmed"))?
            .as_deref()
            == Some(process.start_fingerprint.as_str())
        {
            return Ok(false);
        }
    }
    Ok(true)
}
impl SqlitePersistence {
    /// # Errors
    /// 读取失败不能伪造空回执。
    pub fn personal_memory_reset_operation(
        &self,
        profile: &str,
        id: &str,
    ) -> Result<Option<PersonalMemoryResetRecord>, PersistenceError> {
        read(&self.connection, profile, id)
    }
    /// # Errors
    /// 列表读取失败明确报错。
    pub fn personal_memory_reset_operations(
        &self,
        profile: &str,
    ) -> Result<Vec<PersonalMemoryResetRecord>, PersistenceError> {
        let mut statement=self.connection.prepare("SELECT operation_id FROM personal_memory_reset_operations WHERE profile_id=?1 ORDER BY created_at DESC,operation_id").map_err(database_error("list memory reset operations"))?;
        let ids = statement
            .query_map([profile], |row| row.get::<_, String>(0))
            .map_err(database_error("query memory reset operations"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read memory reset operation ids"))?;
        ids.iter()
            .map(|id| {
                read(&self.connection, profile, id)?
                    .ok_or_else(|| reject("personal_memory_reset_record_invalid"))
            })
            .collect()
    }
    /// # Errors
    /// 相同准备标识不能改变已冻结的作用域。
    pub fn prepare_personal_memory_reset(
        &mut self,
        record: &PersonalMemoryResetRecord,
    ) -> Result<PersonalMemoryResetRecord, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin memory reset preparation"))?;
        let op = &record.operation;
        let ready:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_profile_compatibility WHERE profile_id=?1 AND state='ready' AND data_version=2)",[&op.profile_id],|row|row.get(0)).map_err(database_error("check memory reset profile readiness"))?;
        if !ready {
            return Err(reject("personal_memory_reset_profile_not_ready"));
        }
        let existing:Option<(String,String)>=tx.query_row("SELECT operation_id,request_fingerprint FROM personal_memory_reset_operations WHERE profile_id=?1 AND client_operation_id=?2",params![op.profile_id,record.client_operation_id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(database_error("read memory reset preparation"))?;
        if let Some((id, fingerprint)) = existing {
            if fingerprint != record.request_fingerprint {
                return Err(reject("operation_id_reused"));
            }
            return read(&tx, &op.profile_id, &id)?
                .ok_or_else(|| reject("personal_memory_reset_record_invalid"));
        }
        registry(&tx, record)?;
        tx.execute("INSERT INTO personal_memory_reset_operations(operation_id,profile_id,client_operation_id,request_fingerprint,preview_digest,state,record_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![op.operation_id,op.profile_id,record.client_operation_id,record.request_fingerprint,op.preview_digest,state(&op.state),encode(record)?,op.created_at,op.updated_at]).map_err(database_error("save memory reset preparation"))?;
        tx.commit()
            .map_err(database_error("commit memory reset preparation"))?;
        Ok(record.clone())
    }
    /// # Errors
    /// 状态或摘要已变化时保留原回执。
    pub fn transition_personal_memory_reset(
        &mut self,
        record: &PersonalMemoryResetRecord,
        expected: &PersonalMemoryResetState,
    ) -> Result<PersonalMemoryResetRecord, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin memory reset transition"))?;
        let old = read(
            &tx,
            &record.operation.profile_id,
            &record.operation.operation_id,
        )?
        .ok_or_else(|| reject("personal_memory_reset_operation_not_found"))?;
        if old.operation.state != *expected
            || old.operation.preview_digest != record.operation.preview_digest
        {
            return Err(reject("personal_memory_reset_operation_unconfirmed"));
        }
        if *expected == PersonalMemoryResetState::Prepared
            && record.operation.state == PersonalMemoryResetState::Applying
        {
            crate::personal_memory::disable_for_reset(&tx, record.operation.updated_at)?;
        }
        save(&tx, record)?;
        tx.commit()
            .map_err(database_error("commit memory reset transition"))?;
        Ok(record.clone())
    }
    /// # Errors
    /// 维护栅栏、拥有者或一次性许可不匹配时不登记、更不启动原生进程。
    pub fn prepare_personal_memory_reset_writer(
        &mut self,
        profile: &str,
        instance: &str,
        p: &PersonalMemoryResetWriterParams,
        now: i64,
    ) -> Result<PersonalWriterLease, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin authorized memory reset writer"))?;
        let permit = &p.permit;
        let mut record = read(&tx, profile, &permit.operation_id)?
            .ok_or_else(|| reject("personal_memory_reset_operation_not_found"))?;
        if record.operation.state != PersonalMemoryResetState::Applying
            || record.operation.preview_digest != permit.preview_digest
            || p.client_operation_id.is_empty()
            || p.client_operation_id.len() > 240
        {
            return Err(reject("personal_memory_reset_permit_invalid"));
        }
        checked_fence(&tx, &record, &permit.owner, instance)?;
        let index = record
            .scopes
            .iter()
            .position(|scope| scope.scope_id == permit.scope_id)
            .ok_or_else(|| reject("personal_memory_reset_permit_invalid"))?;
        let scope = &record.scopes[index];
        if scope.native_home_relative_path != permit.native_home_relative_path
            || scope.native_home_fingerprint != permit.native_home_fingerprint
            || scope.permit_token.as_ref() != Some(&permit.permit_token)
        {
            return Err(reject("personal_memory_reset_permit_invalid"));
        }
        let result = record
            .operation
            .scope_results
            .iter()
            .find(|result| result.scope_id == permit.scope_id)
            .ok_or_else(|| reject("personal_memory_reset_record_invalid"))?;
        if result.outcome != PersonalMemoryResetOutcome::Pending {
            return Err(reject("personal_memory_reset_operation_unconfirmed"));
        }
        if let Some(lease_id) = &scope.writer_lease_id {
            if scope.writer_client_operation_id.as_ref() != Some(&p.client_operation_id) {
                return Err(reject("personal_memory_reset_permit_consumed"));
            }
            return writer(&tx, profile, lease_id);
        }
        let lease = PersonalWriterLease {
            lease_id: format!("writer_{}", uuid::Uuid::new_v4().simple()),
            profile_id: profile.into(),
            instance_fence: instance.into(),
            kind: "native_memory_reset".into(),
            resource_id: permit.scope_id.clone(),
            resource_revision: permit.preview_digest.clone(),
            owner: permit.owner.clone(),
            launch_protocol: 1,
            process_mode: PersonalWriterMode::LocalProcess,
            processes: vec![],
            state: PersonalWriterState::Prepared,
            local_transport_finished: false,
            remote_outcome_unconfirmed: false,
            created_at: now,
            updated_at: now,
        };
        let fingerprint = encode(p)?;
        tx.execute("INSERT INTO personal_profile_writers(lease_id,profile_id,client_operation_id,request_fingerprint,record_json,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'prepared',?6,?6)",params![lease.lease_id,profile,p.client_operation_id,fingerprint,encode(&lease)?,now]).map_err(database_error("save authorized memory reset writer"))?;
        record.scopes[index].writer_lease_id = Some(lease.lease_id.clone());
        record.scopes[index].writer_client_operation_id = Some(p.client_operation_id.clone());
        record.operation.updated_at = now;
        save(&tx, &record)?;
        tx.commit()
            .map_err(database_error("commit authorized memory reset writer"))?;
        Ok(lease)
    }
    /// # Errors
    /// 对应维护、正式写者和官方回执不能核对时，不将作用域标记为已删除。
    pub fn complete_personal_memory_reset_scope(
        &mut self,
        profile: &str,
        instance: &str,
        p: &PersonalMemoryResetScopeCompleteParams,
        now: i64,
    ) -> Result<PersonalMemoryResetRecord, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin memory reset scope receipt"))?;
        let mut record = read(&tx, profile, &p.operation_id)?
            .ok_or_else(|| reject("personal_memory_reset_operation_not_found"))?;
        if record.operation.state != PersonalMemoryResetState::Applying
            || record.operation.preview_digest != p.preview_digest
        {
            return Err(reject("personal_memory_reset_operation_unconfirmed"));
        }
        checked_fence(&tx, &record, &p.owner, instance)?;
        let scope = record
            .scopes
            .iter()
            .find(|scope| scope.scope_id == p.scope_id)
            .ok_or_else(|| reject("personal_memory_reset_permit_invalid"))?;
        if p.writer_lease_id.is_some() && p.writer_lease_id != scope.writer_lease_id {
            return Err(reject("personal_memory_reset_writer_unconfirmed"));
        }
        let lease = scope
            .writer_lease_id
            .as_ref()
            .map(|id| writer(&tx, profile, id))
            .transpose()?;
        if let Some(lease) = &lease
            && (lease.kind != "native_memory_reset"
                || lease.resource_id != p.scope_id
                || lease.resource_revision != p.preview_digest
                || lease.owner != p.owner
                || lease.instance_fence != instance)
        {
            return Err(reject("personal_memory_reset_writer_unconfirmed"));
        }
        let exited = lease
            .as_ref()
            .map(writer_exited)
            .transpose()?
            .unwrap_or(false);
        let response = p
            .official_response
            .as_ref()
            .is_some_and(|value| value.as_object().is_some_and(serde_json::Map::is_empty));
        let launched = lease
            .as_ref()
            .is_some_and(|lease| !lease.processes.is_empty());
        let confirmed = response && p.failure_reason.is_none() && exited && launched;
        let no_writer = lease.is_none();
        let result = record
            .operation
            .scope_results
            .iter_mut()
            .find(|result| result.scope_id == p.scope_id)
            .ok_or_else(|| reject("personal_memory_reset_record_invalid"))?;
        if result.outcome == PersonalMemoryResetOutcome::Reset {
            if !confirmed {
                return Err(reject("personal_memory_reset_operation_unconfirmed"));
            }
            return Ok(record);
        }
        result.writer_lease_id.clone_from(&scope.writer_lease_id);
        result.official_response_received = response;
        result.process_exit_confirmed = exited;
        result.outcome = if confirmed {
            PersonalMemoryResetOutcome::Reset
        } else if no_writer && p.official_response.is_none() {
            PersonalMemoryResetOutcome::Failed
        } else {
            PersonalMemoryResetOutcome::Unconfirmed
        };
        result.reason = if confirmed {
            None
        } else {
            Some(
                if !launched {
                    "personal_memory_reset_not_launched"
                } else if !exited {
                    "personal_memory_reset_exit_unconfirmed"
                } else {
                    "personal_memory_reset_response_unconfirmed"
                }
                .into(),
            )
        };
        record.operation.updated_at = now;
        save(&tx, &record)?;
        tx.commit()
            .map_err(database_error("commit memory reset scope receipt"))?;
        Ok(record)
    }
    /// # Errors
    /// 写者仍运行或回执未确定时，不解除维护占用。
    pub fn finish_personal_memory_reset(
        &mut self,
        profile: &str,
        instance: &str,
        id: &str,
        digest: &str,
        owner: &ModelExecutionOwner,
        now: i64,
    ) -> Result<PersonalMemoryResetRecord, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin memory reset finish"))?;
        let mut record = read(&tx, profile, id)?
            .ok_or_else(|| reject("personal_memory_reset_operation_not_found"))?;
        if record.operation.preview_digest != digest {
            return Err(reject("personal_memory_reset_preview_changed"));
        }
        if matches!(
            record.operation.state,
            PersonalMemoryResetState::Completed | PersonalMemoryResetState::Incomplete
        ) {
            return Ok(record);
        }
        if record.operation.state != PersonalMemoryResetState::Applying {
            return Err(reject("personal_memory_reset_operation_unconfirmed"));
        }
        checked_fence(&tx, &record, owner, instance)?;
        for scope in &record.scopes {
            if let Some(id) = &scope.writer_lease_id
                && !writer_exited(&writer(&tx, profile, id)?)?
            {
                return Err(reject("personal_memory_reset_exit_unconfirmed"));
            }
        }
        for result in &mut record.operation.scope_results {
            if result.outcome == PersonalMemoryResetOutcome::Pending {
                result.outcome = PersonalMemoryResetOutcome::Unconfirmed;
                result.reason = Some("personal_memory_reset_response_unconfirmed".into());
            }
        }
        record.operation.state = if record
            .operation
            .scope_results
            .iter()
            .all(|result| result.outcome == PersonalMemoryResetOutcome::Reset)
        {
            PersonalMemoryResetState::Completed
        } else {
            PersonalMemoryResetState::Incomplete
        };
        record.operation.updated_at = now;
        record.operation.blockers = record
            .operation
            .scope_results
            .iter()
            .filter_map(|result| result.reason.clone())
            .collect();
        record.operation.blockers.sort();
        record.operation.blockers.dedup();
        save(&tx, &record)?;
        tx.commit()
            .map_err(database_error("commit memory reset finish"))?;
        Ok(record)
    }
}
