//! 固定命名空间清理的持久动作许可；未知执行不重发，不凭执行器退出伪造完成。
use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{
    ModelExecutionOwner, PersonalDataQuiescence, PersonalMaintenanceFence,
    PersonalMaintenanceOperationStatus, PersonalMaintenanceState, PersonalNamespaceActionAckParams,
    PersonalNamespaceActionOutcome, PersonalNamespaceActionParams, PersonalNamespaceActionPermit,
    PersonalNamespaceActionState, PersonalNamespaceStorageConfirmParams,
    PersonalNamespaceStorageOperation, PersonalNamespaceStorageParams,
    PersonalNamespaceStorageStartParams, PersonalNamespaceStorageStartResult,
    PersonalStorageOperationState, PersonalWriterLease, PersonalWriterMode, PersonalWriterState,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceActionAuthorization {
    pub action_id: String,
    pub permit_token: String,
    pub writer_lease_id: Option<String>,
    pub acknowledgement: Option<PersonalNamespaceActionAckParams>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalNamespaceStorageRecord {
    pub operation: PersonalNamespaceStorageOperation,
    pub authorizations: Vec<PersonalNamespaceActionAuthorization>,
    pub quiescence: Option<PersonalDataQuiescence>,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}
fn reject(code: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, code, code)
}
fn encode(v: &impl Serialize) -> Result<String, PersistenceError> {
    serde_json::to_string(v).map_err(|_| reject("personal_namespace_storage_record_invalid"))
}
fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, PersistenceError> {
    serde_json::from_value(v).map_err(|_| reject("personal_namespace_storage_request_invalid"))
}
fn value(v: impl Serialize) -> Result<Value, PersistenceError> {
    serde_json::to_value(v).map_err(|_| reject("personal_namespace_storage_record_invalid"))
}
fn state(s: &PersonalStorageOperationState) -> &'static str {
    match s {
        PersonalStorageOperationState::Prepared => "prepared",
        PersonalStorageOperationState::Applying => "applying",
        PersonalStorageOperationState::Completed => "completed",
        PersonalStorageOperationState::Incomplete => "incomplete",
        PersonalStorageOperationState::Cancelled => "cancelled",
    }
}
fn read(
    db: &Connection,
    profile: &str,
    id: &str,
) -> Result<PersonalNamespaceStorageRecord, PersistenceError> {
    let raw:Option<(String,String,String,String)>=db.query_row("SELECT record_json,state,client_operation_id,request_fingerprint FROM personal_namespace_storage_operations WHERE profile_id=?1 AND operation_id=?2",params![profile,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(database_error("read namespace cleanup receipt"))?;
    let (raw, sql_state, client, fingerprint) =
        raw.ok_or_else(|| reject("personal_namespace_storage_not_found"))?;
    let record: PersonalNamespaceStorageRecord = serde_json::from_str(&raw)
        .map_err(|_| reject("personal_namespace_storage_record_invalid"))?;
    if record.operation.profile_id != profile
        || record.operation.operation_id != id
        || state(&record.operation.state) != sql_state
        || record.client_operation_id != client
        || record.request_fingerprint != fingerprint
    {
        return Err(reject("personal_namespace_storage_record_invalid"));
    }
    validate_record(&record)?;
    Ok(record)
}
fn validate_record(r: &PersonalNamespaceStorageRecord) -> Result<(), PersistenceError> {
    if r.operation.actions.is_empty()
        || r.operation.actions.len() > 3
        || r.operation.actions.len() != r.authorizations.len()
    {
        return Err(reject("personal_namespace_storage_record_invalid"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut kinds = std::collections::BTreeSet::new();
    let mut tokens = std::collections::BTreeSet::new();
    for (a, permit) in r.operation.actions.iter().zip(&r.authorizations) {
        if a.action_id != permit.action_id
            || !ids.insert(&a.action_id)
            || !kinds.insert(&a.kind)
            || permit.permit_token.is_empty()
            || !tokens.insert(&permit.permit_token)
        {
            return Err(reject("personal_namespace_storage_record_invalid"));
        }
        let valid = match a.state {
            PersonalNamespaceActionState::Planned => {
                permit.writer_lease_id.is_none()
                    && permit.acknowledgement.is_none()
                    && a.outcome.is_none()
            }
            PersonalNamespaceActionState::Started | PersonalNamespaceActionState::Unconfirmed => {
                permit.writer_lease_id.is_some()
                    && permit.acknowledgement.is_none()
                    && a.outcome.is_none()
            }
            PersonalNamespaceActionState::Acknowledged => {
                permit.writer_lease_id.is_some()
                    && permit.acknowledgement.as_ref().is_some_and(|ack| {
                        ack.operation_id == r.operation.operation_id
                            && ack.action_id == a.action_id
                            && ack.shell_owner == r.operation.shell_owner
                            && ack.permit_token == permit.permit_token
                            && Some(&ack.outcome) == a.outcome.as_ref()
                            && ack.reason_code == a.reason_code
                            && ack.after_cache_bytes == a.after_cache_bytes
                    })
            }
        };
        if !valid {
            return Err(reject("personal_namespace_storage_record_invalid"));
        }
    }
    if matches!(
        r.operation.state,
        PersonalStorageOperationState::Prepared | PersonalStorageOperationState::Cancelled
    ) && r
        .operation
        .actions
        .iter()
        .any(|a| a.state != PersonalNamespaceActionState::Planned)
    {
        return Err(reject("personal_namespace_storage_record_invalid"));
    }
    if r.operation.state == PersonalStorageOperationState::Completed
        && r.operation.actions.iter().any(|a| {
            a.state != PersonalNamespaceActionState::Acknowledged
                || a.outcome != Some(PersonalNamespaceActionOutcome::Succeeded)
        })
    {
        return Err(reject("personal_namespace_storage_record_invalid"));
    }
    Ok(())
}
fn registry(db: &Connection, r: &PersonalNamespaceStorageRecord) -> Result<(), PersistenceError> {
    let op = &r.operation;
    crate::maintenance_operations::register(
        db,
        &PersonalMaintenanceOperationStatus {
            operation_id: op.operation_id.clone(),
            profile_id: op.profile_id.clone(),
            kind: "storage_cleanup".into(),
            state: state(&op.state).into(),
            preview_digest: op.preview_digest.clone(),
        },
        op.updated_at,
    )
}
fn save(db: &Connection, r: &PersonalNamespaceStorageRecord) -> Result<(), PersistenceError> {
    validate_record(r)?;
    db.execute("UPDATE personal_namespace_storage_operations SET record_json=?3,state=?4,updated_at=?5 WHERE profile_id=?1 AND operation_id=?2",params![r.operation.profile_id,r.operation.operation_id,encode(r)?,state(&r.operation.state),r.operation.updated_at]).map_err(database_error("save namespace cleanup receipt"))?;
    registry(db, r)
}
fn alive(owner: &ModelExecutionOwner) -> Result<bool, PersistenceError> {
    if owner.pid == 0 || owner.instance_id.is_empty() || owner.start_fingerprint.is_empty() {
        return Err(reject("personal_namespace_storage_owner_invalid"));
    }
    Ok(
        contexthub_platform_adapter::instance::process_start_fingerprint(owner.pid)
            .map_err(|_| reject("personal_namespace_storage_owner_unconfirmed"))?
            .as_deref()
            == Some(owner.start_fingerprint.as_str()),
    )
}
fn recorded_fence(
    db: &Connection,
    r: &PersonalNamespaceStorageRecord,
) -> Result<PersonalMaintenanceFence, PersistenceError> {
    let p = r
        .quiescence
        .as_ref()
        .ok_or_else(|| reject("personal_namespace_storage_proof_missing"))?;
    let raw:Option<String>=db.query_row("SELECT record_json FROM personal_maintenance_fences WHERE profile_id=?1 AND operation_id=?2 AND fence_id=?3 AND state='draining'",params![r.operation.profile_id,r.operation.operation_id,p.token],|row|row.get(0)).optional().map_err(database_error("check namespace maintenance fence"))?;
    let f: PersonalMaintenanceFence = serde_json::from_str(
        &raw.ok_or_else(|| reject("personal_namespace_storage_fence_changed"))?,
    )
    .map_err(|_| reject("personal_namespace_storage_fence_changed"))?;
    if f.profile_id != r.operation.profile_id
        || f.operation_id != r.operation.operation_id
        || f.fence_id != p.token
        || f.owner != p.owner
        || f.state != PersonalMaintenanceState::Draining
    {
        return Err(reject("personal_namespace_storage_fence_changed"));
    }
    Ok(f)
}
fn fence(
    db: &Connection,
    r: &PersonalNamespaceStorageRecord,
    instance: &str,
) -> Result<(), PersistenceError> {
    let f = recorded_fence(db, r)?;
    if f.instance_fence != instance || !alive(&f.owner)? {
        return Err(reject("personal_namespace_storage_fence_changed"));
    }
    Ok(())
}
fn confirm(
    r: &PersonalNamespaceStorageRecord,
    p: &PersonalNamespaceStorageConfirmParams,
) -> Result<(), PersistenceError> {
    if r.operation.preview_digest != p.preview_digest {
        return Err(reject("personal_namespace_storage_preview_changed"));
    }
    Ok(())
}
fn action_index(
    r: &PersonalNamespaceStorageRecord,
    p: &PersonalNamespaceActionParams,
) -> Result<usize, PersistenceError> {
    if r.operation.shell_owner != p.shell_owner || !alive(&p.shell_owner)? {
        return Err(reject("personal_namespace_storage_owner_unconfirmed"));
    }
    let i = r
        .authorizations
        .iter()
        .position(|a| a.action_id == p.action_id && a.permit_token == p.permit_token)
        .ok_or_else(|| reject("personal_namespace_storage_permit_invalid"))?;
    if r.operation
        .actions
        .get(i)
        .is_none_or(|a| a.action_id != p.action_id)
    {
        return Err(reject("personal_namespace_storage_record_invalid"));
    }
    Ok(i)
}
impl SqlitePersistence {
    /// # Errors
    /// 错误、未知范围或损坏记录不能当作空列表。
    pub fn personal_namespace_storage_operations(
        &self,
        profile: &str,
    ) -> Result<Vec<PersonalNamespaceStorageOperation>, PersistenceError> {
        let mut stmt=self.connection.prepare("SELECT operation_id FROM personal_namespace_storage_operations WHERE profile_id=?1 ORDER BY created_at DESC,operation_id").map_err(database_error("list namespace cleanup"))?;
        let ids = stmt
            .query_map([profile], |r| r.get::<_, String>(0))
            .map_err(database_error("query namespace cleanup"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read namespace cleanup ids"))?;
        ids.iter()
            .map(|id| Ok(read(&self.connection, profile, id)?.operation))
            .collect()
    }
    /// # Errors
    /// 幂等键重用或回执不可读时拒绝。
    pub fn personal_namespace_storage_by_client(
        &self,
        profile: &str,
        client: &str,
        fingerprint: &str,
    ) -> Result<Option<PersonalNamespaceStorageRecord>, PersistenceError> {
        let row:Option<(String,String)>=self.connection.query_row("SELECT operation_id,request_fingerprint FROM personal_namespace_storage_operations WHERE profile_id=?1 AND client_operation_id=?2",params![profile,client],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(database_error("read namespace cleanup client"))?;
        match row {
            Some((id, f)) if f == fingerprint => Ok(Some(read(&self.connection, profile, &id)?)),
            Some(_) => Err(reject("operation_id_reused")),
            None => Ok(None),
        }
    }
    /// # Errors
    /// 不改变同一次操作已确认的固定范围。
    pub fn prepare_personal_namespace_storage(
        &mut self,
        r: &PersonalNamespaceStorageRecord,
        instance: &str,
    ) -> Result<PersonalNamespaceStorageRecord, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin namespace cleanup preview"))?;
        validate_record(r)?;
        crate::profile_writers::require_writer_admission(&tx, &r.operation.profile_id, instance)?;
        if !alive(&r.operation.shell_owner)? {
            return Err(reject("personal_namespace_storage_owner_unconfirmed"));
        }
        registry(&tx, r)?;
        tx.execute("INSERT INTO personal_namespace_storage_operations(operation_id,profile_id,client_operation_id,request_fingerprint,record_json,state,created_at,updated_at,log_segment_id) VALUES(?1,?2,?3,?4,?5,'prepared',?6,?6,?7)",params![r.operation.operation_id,r.operation.profile_id,r.client_operation_id,r.request_fingerprint,encode(r)?,r.operation.created_at,r.operation.log_segment_id]).map_err(database_error("save namespace cleanup preview"))?;
        tx.commit()
            .map_err(database_error("commit namespace cleanup preview"))?;
        Ok(r.clone())
    }
    /// # Errors
    /// 真实维护证明、同资料和当前服务代次必须同时成立；未知执行不重复发许可。
    pub fn start_personal_namespace_storage(
        &mut self,
        p: &PersonalNamespaceStorageStartParams,
        profile: &str,
        instance: &str,
        now: i64,
    ) -> Result<PersonalNamespaceStorageStartResult, PersistenceError> {
        self.verify_personal_data_quiescence(profile, &p.operation_id, instance, &p.quiescence)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin namespace cleanup"))?;
        let mut r = read(&tx, profile, &p.operation_id)?;
        confirm(
            &r,
            &PersonalNamespaceStorageConfirmParams {
                operation_id: p.operation_id.clone(),
                preview_digest: p.preview_digest.clone(),
            },
        )?;
        if r.operation.state != PersonalStorageOperationState::Prepared
            || !alive(&r.operation.shell_owner)?
        {
            return Err(reject("personal_namespace_storage_operation_unconfirmed"));
        }
        r.quiescence = Some(p.quiescence.clone());
        fence(&tx, &r, instance)?;
        if !crate::profile_writers::active_writers(&tx, profile)?.is_empty() {
            return Err(reject("personal_data_writer_active"));
        }
        r.operation.state = PersonalStorageOperationState::Applying;
        r.operation.updated_at = now;
        let permits = r
            .operation
            .actions
            .iter()
            .zip(&r.authorizations)
            .map(|(a, p)| PersonalNamespaceActionPermit {
                action_id: a.action_id.clone(),
                permit_token: p.permit_token.clone(),
                kind: a.kind.clone(),
                shell_owner: r.operation.shell_owner.clone(),
            })
            .collect();
        save(&tx, &r)?;
        tx.commit()
            .map_err(database_error("commit namespace cleanup start"))?;
        Ok(PersonalNamespaceStorageStartResult {
            operation: r.operation,
            permits,
        })
    }
    /// # Errors
    /// 此方法只改变持久回执；不会启动、重试或删除任何文件。
    pub fn personal_namespace_storage_rpc(
        &mut self,
        method: &str,
        params: Value,
        profile: &str,
        instance: &str,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin namespace action receipt"))?;
        let result = match method {
            "personalNamespaceStorage/get" | "personalNamespaceStorage/reconcile" => {
                let p: PersonalNamespaceStorageParams = parse(params)?;
                value(read(&tx, profile, &p.operation_id)?.operation)?
            }
            "personalNamespaceStorage/beginAction" => {
                begin_action(&tx, &parse(params)?, profile, instance, now)?
            }
            "personalNamespaceStorage/ackAction" => {
                ack_action(&tx, &parse(params)?, profile, instance, now)?
            }
            "personalNamespaceStorage/finish"
            | "personalNamespaceStorage/cancel"
            | "personalNamespaceStorage/resolveInterrupted" => {
                finish(&tx, method, &parse(params)?, profile, now)?
            }
            _ => return Err(reject("personal_namespace_storage_method_unknown")),
        };
        tx.commit()
            .map_err(database_error("commit namespace action receipt"))?;
        Ok(result)
    }
}
fn begin_action(
    tx: &rusqlite::Transaction<'_>,
    p: &PersonalNamespaceActionParams,
    profile: &str,
    instance: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut r = read(tx, profile, &p.operation_id)?;
    let i = action_index(&r, p)?;
    fence(tx, &r, instance)?;
    if r.operation.state != PersonalStorageOperationState::Applying
        || r.operation.actions[i].state != PersonalNamespaceActionState::Planned
    {
        return Err(reject("personal_namespace_storage_action_already_started"));
    }
    checked_processes(p)?;
    if crate::profile_writers::active_writers(tx, profile)?
        .iter()
        .any(|l| l.kind != "namespace_action" || l.resource_id != r.operation.operation_id)
    {
        return Err(reject("personal_data_writer_active"));
    }
    let lease = PersonalWriterLease {
        lease_id: format!("writer_{}", uuid::Uuid::new_v4().simple()),
        profile_id: profile.into(),
        instance_fence: instance.into(),
        kind: "namespace_action".into(),
        resource_id: r.operation.operation_id.clone(),
        resource_revision: r.operation.preview_digest.clone(),
        owner: p.shell_owner.clone(),
        launch_protocol: 1,
        process_mode: PersonalWriterMode::RemoteRequest,
        processes: p.execution_processes.clone(),
        state: PersonalWriterState::Active,
        local_transport_finished: false,
        remote_outcome_unconfirmed: false,
        created_at: now,
        updated_at: now,
    };
    tx.execute("INSERT INTO personal_profile_writers(lease_id,profile_id,client_operation_id,request_fingerprint,record_json,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'active',?6,?6)",params![lease.lease_id,profile,format!("namespace:{}:{}",p.operation_id,p.action_id),r.operation.preview_digest,encode(&lease)?,now]).map_err(database_error("register namespace action writer"))?;
    r.authorizations[i].writer_lease_id = Some(lease.lease_id);
    r.operation.actions[i].state = PersonalNamespaceActionState::Started;
    r.operation.actions[i].started_at = Some(now);
    r.operation.updated_at = now;
    save(tx, &r)?;
    value(r.operation.actions[i].clone())
}
fn ack_action(
    tx: &rusqlite::Transaction<'_>,
    p: &PersonalNamespaceActionAckParams,
    profile: &str,
    _instance: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut r = read(tx, profile, &p.operation_id)?;
    let i = action_index(
        &r,
        &PersonalNamespaceActionParams {
            operation_id: p.operation_id.clone(),
            action_id: p.action_id.clone(),
            permit_token: p.permit_token.clone(),
            shell_owner: p.shell_owner.clone(),
            execution_processes: vec![],
        },
    )?;
    if let Some(old) = &r.authorizations[i].acknowledgement {
        if old == p {
            return value(r.operation.actions[i].clone());
        }
        return Err(reject("personal_namespace_storage_ack_changed"));
    }
    let origin_fence = recorded_fence(tx, &r)?;
    if r.operation.actions[i].state != PersonalNamespaceActionState::Started
        || !matches!(
            r.operation.state,
            PersonalStorageOperationState::Applying | PersonalStorageOperationState::Incomplete
        )
    {
        return Err(reject("personal_namespace_storage_action_not_started"));
    }
    if p.reason_code.as_ref().is_some_and(|s| {
        s.is_empty()
            || s.len() > 120
            || !s
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    }) || (p.outcome == PersonalNamespaceActionOutcome::Succeeded && p.reason_code.is_some())
        || (p.outcome == PersonalNamespaceActionOutcome::Failed && p.reason_code.is_none())
    {
        return Err(reject("personal_namespace_storage_ack_invalid"));
    }
    if r.operation.actions[i].kind
        != contexthub_protocol::PersonalNamespaceStorageKind::BrowserHttpCache
        && p.after_cache_bytes.is_some()
    {
        return Err(reject("personal_namespace_storage_ack_invalid"));
    }
    if p.after_cache_bytes
        .is_some_and(|v| v > 9_007_199_254_740_991)
        || (r.operation.actions[i].kind
            == contexthub_protocol::PersonalNamespaceStorageKind::BrowserHttpCache
            && p.outcome == PersonalNamespaceActionOutcome::Succeeded
            && p.after_cache_bytes.is_none())
    {
        return Err(reject("personal_namespace_storage_ack_invalid"));
    }
    let lease_id = r.authorizations[i]
        .writer_lease_id
        .as_ref()
        .ok_or_else(|| reject("personal_namespace_storage_writer_unconfirmed"))?;
    let raw: String = tx
        .query_row(
            "SELECT record_json FROM personal_profile_writers WHERE profile_id=?1 AND lease_id=?2",
            params![profile, lease_id],
            |row| row.get(0),
        )
        .map_err(database_error("read namespace action writer"))?;
    let mut lease: PersonalWriterLease = serde_json::from_str(&raw)
        .map_err(|_| reject("personal_namespace_storage_writer_unconfirmed"))?;
    if lease.kind != "namespace_action"
        || lease.resource_id != p.operation_id
        || lease.owner != p.shell_owner
        || lease.instance_fence != origin_fence.instance_fence
        || lease.state != PersonalWriterState::Active
    {
        return Err(reject("personal_namespace_storage_writer_unconfirmed"));
    }
    lease.state = PersonalWriterState::Released;
    lease.local_transport_finished = true;
    lease.remote_outcome_unconfirmed = false;
    lease.updated_at = now;
    tx.execute("UPDATE personal_profile_writers SET record_json=?3,state='released',updated_at=?4 WHERE profile_id=?1 AND lease_id=?2",params![profile,lease_id,encode(&lease)?,now]).map_err(database_error("acknowledge namespace action writer"))?;
    r.authorizations[i].acknowledgement = Some(p.clone());
    let a = &mut r.operation.actions[i];
    a.state = PersonalNamespaceActionState::Acknowledged;
    a.outcome = Some(p.outcome.clone());
    a.reason_code.clone_from(&p.reason_code);
    a.after_cache_bytes = p.after_cache_bytes;
    a.acknowledged_at = Some(now);
    r.operation.updated_at = now;
    save(tx, &r)?;
    value(r.operation.actions[i].clone())
}
fn finish(
    tx: &rusqlite::Transaction<'_>,
    method: &str,
    p: &PersonalNamespaceStorageConfirmParams,
    profile: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    let mut r = read(tx, profile, &p.operation_id)?;
    confirm(&r, p)?;
    if matches!(
        r.operation.state,
        PersonalStorageOperationState::Completed | PersonalStorageOperationState::Cancelled
    ) {
        return value(r.operation);
    }
    if method == "personalNamespaceStorage/cancel" {
        if r.operation
            .actions
            .iter()
            .any(|a| a.state != PersonalNamespaceActionState::Planned)
        {
            return Err(reject("personal_namespace_storage_action_unconfirmed"));
        }
        r.operation.state = PersonalStorageOperationState::Cancelled;
    } else {
        if method == "personalNamespaceStorage/resolveInterrupted" {
            resolve_actions(tx, &mut r, now)?;
        }
        if r.operation.state == PersonalStorageOperationState::Prepared
            && method != "personalNamespaceStorage/resolveInterrupted"
        {
            return Err(reject("personal_namespace_storage_not_started"));
        }
        r.operation.state = if r.operation.actions.iter().all(|a| {
            a.state == PersonalNamespaceActionState::Acknowledged
                && a.outcome == Some(PersonalNamespaceActionOutcome::Succeeded)
        }) {
            PersonalStorageOperationState::Completed
        } else {
            PersonalStorageOperationState::Incomplete
        };
        r.operation.blockers = r
            .operation
            .actions
            .iter()
            .filter_map(|a| match a.state {
                PersonalNamespaceActionState::Started
                | PersonalNamespaceActionState::Unconfirmed => {
                    Some("personal_namespace_storage_action_unconfirmed".into())
                }
                PersonalNamespaceActionState::Planned => {
                    Some("personal_namespace_storage_action_not_started".into())
                }
                PersonalNamespaceActionState::Acknowledged
                    if a.outcome == Some(PersonalNamespaceActionOutcome::Failed) =>
                {
                    Some(
                        a.reason_code
                            .clone()
                            .unwrap_or_else(|| "personal_namespace_storage_action_failed".into()),
                    )
                }
                PersonalNamespaceActionState::Acknowledged => None,
            })
            .collect();
    }
    r.operation.updated_at = now;
    save(tx, &r)?;
    value(r.operation)
}

fn checked_processes(p: &PersonalNamespaceActionParams) -> Result<(), PersistenceError> {
    if p.execution_processes.is_empty()
        || p.execution_processes.len() > 256
        || !p.execution_processes.iter().any(|process| {
            process.pid == p.shell_owner.pid
                && process.start_fingerprint == p.shell_owner.start_fingerprint
        })
    {
        return Err(reject("personal_namespace_storage_processes_unconfirmed"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for process in &p.execution_processes {
        if !seen.insert(process.pid)
            || contexthub_platform_adapter::instance::process_start_fingerprint(process.pid)
                .map_err(|_| reject("personal_namespace_storage_processes_unconfirmed"))?
                .as_deref()
                != Some(process.start_fingerprint.as_str())
        {
            return Err(reject("personal_namespace_storage_processes_unconfirmed"));
        }
    }
    Ok(())
}
fn resolve_actions(
    tx: &rusqlite::Transaction<'_>,
    r: &mut PersonalNamespaceStorageRecord,
    now: i64,
) -> Result<(), PersistenceError> {
    if alive(&r.operation.shell_owner)?
        && r.operation
            .actions
            .iter()
            .any(|a| a.state == PersonalNamespaceActionState::Started)
    {
        return Err(reject("personal_namespace_storage_owner_alive"));
    }
    for (action, authorization) in r.operation.actions.iter_mut().zip(&r.authorizations) {
        if action.state != PersonalNamespaceActionState::Started {
            continue;
        }
        if action.kind == contexthub_protocol::PersonalNamespaceStorageKind::BrowserHttpCache {
            return Err(reject(
                "personal_namespace_storage_browser_transport_unconfirmed",
            ));
        }
        let id = authorization
            .writer_lease_id
            .as_ref()
            .ok_or_else(|| reject("personal_namespace_storage_writer_unconfirmed"))?;
        let raw:String=tx.query_row("SELECT record_json FROM personal_profile_writers WHERE profile_id=?1 AND lease_id=?2",params![r.operation.profile_id,id],|row|row.get(0)).map_err(database_error("read interrupted namespace writer"))?;
        let mut lease: PersonalWriterLease = serde_json::from_str(&raw)
            .map_err(|_| reject("personal_namespace_storage_writer_unconfirmed"))?;
        if lease.kind != "namespace_action"
            || lease.resource_id != r.operation.operation_id
            || lease.owner != r.operation.shell_owner
            || lease.state != PersonalWriterState::Active
            || lease.processes.is_empty()
        {
            return Err(reject("personal_namespace_storage_writer_unconfirmed"));
        }
        for process in &lease.processes {
            if contexthub_platform_adapter::instance::process_start_fingerprint(process.pid)
                .map_err(|_| reject("personal_namespace_storage_processes_unconfirmed"))?
                .as_deref()
                == Some(process.start_fingerprint.as_str())
            {
                return Err(reject("personal_namespace_storage_process_alive"));
            }
        }
        lease.state = PersonalWriterState::Released;
        lease.local_transport_finished = true;
        lease.remote_outcome_unconfirmed = true;
        lease.updated_at = now;
        tx.execute("UPDATE personal_profile_writers SET state='released',record_json=?3,updated_at=?4 WHERE profile_id=?1 AND lease_id=?2",params![r.operation.profile_id,id,encode(&lease)?,now]).map_err(database_error("record interrupted namespace writer exit"))?;
        action.state = PersonalNamespaceActionState::Unconfirmed;
        action.reason_code = Some("personal_namespace_storage_action_unconfirmed".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "personal_namespace_storage_tests.rs"]
mod tests;
