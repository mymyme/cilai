//! 跨宿主资料写者登记；维护先关闭新准入，再分别核对已登记写者。
use crate::{PersistenceError, PersistenceErrorKind, database_error};
use contexthub_protocol::{
    ModelExecutionOwner, ModelExecutionProcess, PersonalMaintenanceAcquireParams,
    PersonalMaintenanceFence, PersonalMaintenanceParams, PersonalMaintenanceState,
    PersonalWriterAbortParams, PersonalWriterLease, PersonalWriterLeaseParams, PersonalWriterMode,
    PersonalWriterPrepareParams, PersonalWriterRegisterParams, PersonalWriterState,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(test)]
#[path = "profile_writers_tests.rs"]
mod tests;

impl crate::SqlitePersistence {
    /// # Errors
    /// 资料未就绪、占用未确认、维护冲突或回执无法保存时拒绝。
    pub fn personal_writer_rpc(
        &mut self,
        method: &str,
        params: Value,
        profile: &str,
        instance_fence: &str,
        now: i64,
    ) -> Result<Value, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin personal writer operation"))?;
        let result = if method.starts_with("personalMaintenance/") {
            maintenance_rpc(&tx, method, params, profile, instance_fence, now)?
        } else {
            writer_rpc(&tx, method, params, profile, instance_fence, now)?
        };
        tx.commit()
            .map_err(database_error("commit personal writer operation"))?;
        Ok(result)
    }
}

fn reject(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}
fn encoded(value: &impl serde::Serialize) -> Result<String, PersistenceError> {
    serde_json::to_string(value).map_err(|_| reject("personal_writer_record_invalid"))
}
fn hash(value: &impl serde::Serialize) -> Result<String, PersistenceError> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(encoded(value)?.as_bytes())
    ))
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 240 && !value.contains('\0')
}
fn current_profile(
    tx: &Transaction<'_>,
    profile: &str,
    fence: &str,
) -> Result<(), PersistenceError> {
    let ready:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_profile_compatibility WHERE profile_id=?1 AND instance_fence=?2 AND state='ready' AND data_version=2)",params![profile,fence],|r|r.get(0)).map_err(database_error("read writer profile readiness"))?;
    if ready {
        Ok(())
    } else {
        Err(reject("personal_writer_profile_not_ready"))
    }
}
fn fingerprint(pid: u32) -> Result<Option<String>, PersistenceError> {
    contexthub_platform_adapter::instance::process_start_fingerprint(pid)
        .map_err(|_| reject("personal_writer_process_unconfirmed"))
}
fn owner_alive(owner: &ModelExecutionOwner) -> Result<bool, PersistenceError> {
    if owner.pid == 0 || !text(&owner.instance_id) || !text(&owner.start_fingerprint) {
        return Err(reject("personal_writer_owner_invalid"));
    }
    Ok(fingerprint(owner.pid)?.as_deref() == Some(owner.start_fingerprint.as_str()))
}
fn store(tx: &Transaction<'_>, lease: &PersonalWriterLease) -> Result<(), PersistenceError> {
    let state = match lease.state {
        PersonalWriterState::Prepared => "prepared",
        PersonalWriterState::Active => "active",
        PersonalWriterState::Released => "released",
    };
    tx.execute("UPDATE personal_profile_writers SET record_json=?3,state=?4,updated_at=?5 WHERE profile_id=?1 AND lease_id=?2",params![lease.profile_id,lease.lease_id,encoded(lease)?,state,lease.updated_at]).map_err(database_error("save personal writer"))?;
    Ok(())
}
fn read(
    tx: &Transaction<'_>,
    profile: &str,
    id: &str,
) -> Result<PersonalWriterLease, PersistenceError> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT record_json FROM personal_profile_writers WHERE profile_id=?1 AND lease_id=?2",
            params![profile, id],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error("read personal writer"))?;
    let lease: PersonalWriterLease =
        serde_json::from_str(&raw.ok_or_else(|| reject("personal_writer_not_found"))?)
            .map_err(|_| reject("personal_writer_record_invalid"))?;
    if lease.profile_id != profile || lease.lease_id != id {
        return Err(reject("personal_writer_record_invalid"));
    }
    Ok(lease)
}

/// # Errors
/// 拒绝未就绪资料、未知服务代次和处于维护期的任何新写者。
pub fn require_writer_admission(
    tx: &Transaction<'_>,
    profile: &str,
    instance_fence: &str,
) -> Result<(), PersistenceError> {
    current_profile(tx, profile, instance_fence)?;
    let maintenance:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_maintenance_fences WHERE profile_id=?1 AND state='draining')",[profile],|r|r.get(0)).map_err(database_error("check personal maintenance fence"))?;
    if maintenance {
        return Err(reject("personal_data_maintenance_active"));
    }
    Ok(())
}

/// # Errors
/// 新写者在同一业务事务内取得准入；幂等操作不能改变其资源和拥有者。
pub fn prepare_writer(
    tx: &Transaction<'_>,
    p: &PersonalWriterPrepareParams,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<PersonalWriterLease, PersistenceError> {
    current_profile(tx, profile, instance_fence)?;
    let request = hash(p)?;
    let old:Option<(String,String)>=tx.query_row("SELECT lease_id,request_fingerprint FROM personal_profile_writers WHERE profile_id=?1 AND client_operation_id=?2",params![profile,p.client_operation_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(database_error("read writer operation"))?;
    if let Some((id, expected)) = old {
        if expected != request {
            return Err(reject("operation_id_reused"));
        }
        return read(tx, profile, &id);
    }
    require_writer_admission(tx, profile, instance_fence)?;
    if !text(&p.client_operation_id)
        || !text(&p.resource_id)
        || !text(&p.resource_revision)
        || !matches!(
            p.kind.as_str(),
            "personal_mcp"
                | "user_mcp"
                | "attachment"
                | "artifact"
                | "application_bridge"
                | "local_operation"
                | "browser_download"
                | "native_memory"
                | "native_history"
                | "authentication"
        )
        || p.launch_protocol != 1
        || !owner_alive(&p.owner)?
    {
        return Err(reject("personal_writer_request_invalid"));
    }
    let lease = PersonalWriterLease {
        lease_id: format!("writer_{}", uuid::Uuid::new_v4().simple()),
        profile_id: profile.into(),
        instance_fence: instance_fence.into(),
        kind: p.kind.clone(),
        resource_id: p.resource_id.clone(),
        resource_revision: p.resource_revision.clone(),
        owner: p.owner.clone(),
        launch_protocol: p.launch_protocol,
        process_mode: p.process_mode.clone(),
        processes: vec![],
        state: if p.process_mode == PersonalWriterMode::LocalProcess {
            PersonalWriterState::Prepared
        } else {
            PersonalWriterState::Active
        },
        local_transport_finished: false,
        remote_outcome_unconfirmed: false,
        created_at: now,
        updated_at: now,
    };
    let state = if lease.state == PersonalWriterState::Prepared {
        "prepared"
    } else {
        "active"
    };
    tx.execute("INSERT INTO personal_profile_writers(lease_id,profile_id,client_operation_id,request_fingerprint,record_json,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?7)",params![lease.lease_id,profile,p.client_operation_id,request,encoded(&lease)?,state,now]).map_err(database_error("prepare personal writer"))?;
    Ok(lease)
}

fn owned_current(lease: &PersonalWriterLease, instance: &str) -> Result<(), PersistenceError> {
    if lease.instance_fence != instance {
        return Err(reject("personal_writer_previous_instance"));
    }
    if lease.state == PersonalWriterState::Released {
        return Err(reject("personal_writer_released"));
    }
    Ok(())
}

/// # Errors
/// Gate 发许可前登记且核对进程指纹，已登记写者在维护期仍可完成停止回执。
pub fn register_writer_process(
    tx: &Transaction<'_>,
    p: &PersonalWriterRegisterParams,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<PersonalWriterLease, PersistenceError> {
    current_profile(tx, profile, instance_fence)?;
    let mut lease = read(tx, profile, &p.lease_id)?;
    owned_current(&lease, instance_fence)?;
    if lease.process_mode != PersonalWriterMode::LocalProcess
        || !owner_alive(&lease.owner)?
        || p.process.pid == 0
        || !text(&p.process.start_fingerprint)
        || fingerprint(p.process.pid)?.as_deref() != Some(p.process.start_fingerprint.as_str())
    {
        return Err(reject("personal_writer_process_unconfirmed"));
    }
    if lease
        .processes
        .iter()
        .any(|old| old.pid == p.process.pid && old.start_fingerprint != p.process.start_fingerprint)
    {
        return Err(reject("personal_writer_process_changed"));
    }
    if !lease.processes.contains(&p.process) {
        lease.processes.push(p.process.clone());
    }
    lease.state = PersonalWriterState::Active;
    lease.updated_at = now;
    store(tx, &lease)?;
    Ok(lease)
}

fn processes_exited(processes: &[ModelExecutionProcess]) -> Result<(), PersistenceError> {
    for process in processes {
        if fingerprint(process.pid)?.as_deref() == Some(process.start_fingerprint.as_str()) {
            return Err(reject("personal_writer_process_alive"));
        }
    }
    Ok(())
}

/// # Errors
/// 本地进程必须登记且已退出；远程只能由原拥有者确认本机传输结束。
pub fn release_writer(
    tx: &Transaction<'_>,
    p: &PersonalWriterLeaseParams,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<PersonalWriterLease, PersistenceError> {
    current_profile(tx, profile, instance_fence)?;
    let mut lease = read(tx, profile, &p.lease_id)?;
    if lease.kind == "namespace_action" {
        return Err(reject("personal_namespace_storage_typed_ack_required"));
    }
    if lease.state == PersonalWriterState::Released {
        return Ok(lease);
    }
    owned_current(&lease, instance_fence)?;
    match lease.process_mode {
        PersonalWriterMode::LocalProcess => {
            if lease.processes.is_empty() {
                return Err(reject("personal_writer_process_unconfirmed"));
            }
            processes_exited(&lease.processes)?;
        }
        PersonalWriterMode::RemoteRequest => {
            if p.owner.as_ref() != Some(&lease.owner)
                || !owner_alive(&lease.owner)?
                || !p.local_transport_finished
            {
                return Err(reject("personal_writer_transport_unconfirmed"));
            }
            lease.local_transport_finished = true;
            lease.remote_outcome_unconfirmed = p.remote_outcome_unconfirmed;
        }
    }
    lease.state = PersonalWriterState::Released;
    lease.updated_at = now;
    store(tx, &lease)?;
    Ok(lease)
}

/// # Errors
/// 仅当前拥有者、当前服务代次、尚未开始 spawn 的本地 Gate 准备可显式终止。
pub fn abort_writer_unlaunched(
    tx: &Transaction<'_>,
    p: &PersonalWriterAbortParams,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<PersonalWriterLease, PersistenceError> {
    current_profile(tx, profile, instance_fence)?;
    let mut lease = read(tx, profile, &p.lease_id)?;
    owned_current(&lease, instance_fence)?;
    if lease.owner != p.owner
        || !owner_alive(&lease.owner)?
        || lease.launch_protocol != 1
        || lease.process_mode != PersonalWriterMode::LocalProcess
        || lease.state != PersonalWriterState::Prepared
        || !lease.processes.is_empty()
    {
        return Err(reject("personal_writer_abort_not_safe"));
    }
    lease.state = PersonalWriterState::Released;
    lease.updated_at = now;
    store(tx, &lease)?;
    Ok(lease)
}

/// # Errors
/// 拒绝无法核对的条目；不枚举或操作登记范围以外的进程。
pub fn active_writers(
    tx: &Transaction<'_>,
    profile: &str,
) -> Result<Vec<PersonalWriterLease>, PersistenceError> {
    let mut statement=tx.prepare("SELECT lease_id FROM personal_profile_writers WHERE profile_id=?1 AND state<>'released' ORDER BY lease_id").map_err(database_error("list personal writers"))?;
    let ids = statement
        .query_map([profile], |r| r.get::<_, String>(0))
        .map_err(database_error("query personal writers"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read personal writer ids"))?;
    ids.into_iter().map(|id| read(tx, profile, &id)).collect()
}

/// # Errors
/// 未登记的本地进程继续标为未确认；只核对退出，不停止进程或重发请求。
pub fn recover_writer(
    tx: &Transaction<'_>,
    id: &str,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<PersonalWriterLease, PersistenceError> {
    current_profile(tx, profile, instance_fence)?;
    let mut lease = read(tx, profile, id)?;
    if lease.kind == "namespace_action" {
        return Err(reject("personal_namespace_storage_typed_ack_required"));
    }
    if lease.state == PersonalWriterState::Released {
        return Ok(lease);
    }
    if owner_alive(&lease.owner)? {
        return Err(reject("personal_writer_owner_alive"));
    }
    if lease.process_mode == PersonalWriterMode::LocalProcess {
        if lease.processes.is_empty() {
            return Err(reject("personal_writer_process_unconfirmed"));
        }
        processes_exited(&lease.processes)?;
    } else {
        lease.local_transport_finished = true;
        lease.remote_outcome_unconfirmed = true;
    }
    lease.state = PersonalWriterState::Released;
    lease.updated_at = now;
    store(tx, &lease)?;
    Ok(lease)
}

/// # Errors
/// 只在当前资料内按私有方法分派，普通桌面转发不得暴露此接口。
pub fn writer_rpc(
    tx: &Transaction<'_>,
    method: &str,
    params: Value,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, PersistenceError> {
        serde_json::from_value(v).map_err(|_| reject("personal_writer_request_invalid"))
    }
    fn value(v: impl serde::Serialize) -> Result<Value, PersistenceError> {
        serde_json::to_value(v).map_err(|_| reject("personal_writer_record_invalid"))
    }
    current_profile(tx, profile, instance_fence)?;
    match method {
        "profileWriter/prepare" => value(prepare_writer(
            tx,
            &parse(params)?,
            profile,
            instance_fence,
            now,
        )?),
        "profileWriter/registerProcess" => value(register_writer_process(
            tx,
            &parse(params)?,
            profile,
            instance_fence,
            now,
        )?),
        "profileWriter/release" => value(release_writer(
            tx,
            &parse(params)?,
            profile,
            instance_fence,
            now,
        )?),
        "profileWriter/abortUnlaunched" => value(abort_writer_unlaunched(
            tx,
            &parse(params)?,
            profile,
            instance_fence,
            now,
        )?),
        "profileWriter/read" => value(read(
            tx,
            profile,
            &parse::<PersonalWriterLeaseParams>(params)?.lease_id,
        )?),
        "profileWriter/list" => value(active_writers(tx, profile)?),
        "profileWriter/recover" => value(recover_writer(
            tx,
            &parse::<PersonalWriterLeaseParams>(params)?.lease_id,
            profile,
            instance_fence,
            now,
        )?),
        _ => Err(reject("personal_writer_method_unknown")),
    }
}

fn fence_record(
    tx: &Transaction<'_>,
    profile: &str,
    id: &str,
) -> Result<PersonalMaintenanceFence, PersistenceError> {
    let raw:Option<String>=tx.query_row("SELECT record_json FROM personal_maintenance_fences WHERE profile_id=?1 AND fence_id=?2",params![profile,id],|r|r.get(0)).optional().map_err(database_error("read personal maintenance"))?;
    let fence: PersonalMaintenanceFence =
        serde_json::from_str(&raw.ok_or_else(|| reject("personal_maintenance_not_found"))?)
            .map_err(|_| reject("personal_maintenance_record_invalid"))?;
    if fence.profile_id != profile || fence.fence_id != id {
        return Err(reject("personal_maintenance_record_invalid"));
    }
    Ok(fence)
}

/// # Errors
/// 先关闭所有新准入；存在旧拥有者、未确认栅栏或改变的预览时拒绝替换。
pub fn acquire_maintenance(
    tx: &Transaction<'_>,
    p: &PersonalMaintenanceAcquireParams,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<PersonalMaintenanceFence, PersistenceError> {
    current_profile(tx, profile, instance_fence)?;
    if !owner_alive(&p.owner)? {
        return Err(reject("personal_writer_owner_invalid"));
    }
    let matches:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_maintenance_operation_registry WHERE profile_id=?1 AND operation_id=?2 AND preview_digest=?3)",params![profile,p.operation_id,p.preview_digest],|r|r.get(0)).map_err(database_error("check maintenance preview"))?;
    if !matches {
        return Err(reject("personal_data_preview_changed"));
    }
    let existing:Option<String>=tx.query_row("SELECT fence_id FROM personal_maintenance_fences WHERE profile_id=?1 AND operation_id=?2 AND state='draining'",params![profile,p.operation_id],|r|r.get(0)).optional().map_err(database_error("read previous maintenance"))?;
    if let Some(id) = existing {
        let fence = fence_record(tx, profile, &id)?;
        if fence.owner != p.owner
            || fence.instance_fence != instance_fence
            || fence.state != PersonalMaintenanceState::Draining
        {
            return Err(reject("personal_maintenance_owner_changed"));
        }
        return Ok(fence);
    }
    require_writer_admission(tx, profile, instance_fence)?;
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_maintenance_operation_registry WHERE profile_id=?1 AND operation_id=?2 AND preview_digest=?3 AND state IN ('prepared','incomplete','ready_to_apply'))",params![profile,p.operation_id,p.preview_digest],|r|r.get(0)).map_err(database_error("check maintenance operation"))?;
    if !valid {
        return Err(reject("personal_data_preview_changed"));
    }
    let fence = PersonalMaintenanceFence {
        fence_id: format!("maintenance_{}", uuid::Uuid::new_v4().simple()),
        operation_id: p.operation_id.clone(),
        profile_id: profile.into(),
        instance_fence: instance_fence.into(),
        owner: p.owner.clone(),
        state: PersonalMaintenanceState::Draining,
        created_at: now,
        updated_at: now,
    };
    tx.execute("INSERT INTO personal_maintenance_fences(fence_id,operation_id,profile_id,record_json,state,created_at,updated_at) VALUES(?1,?2,?3,?4,'draining',?5,?5)",params![fence.fence_id,fence.operation_id,profile,encoded(&fence)?,now]).map_err(database_error("acquire personal maintenance"))?;
    Ok(fence)
}

/// # Errors
/// 只允许当前拥有者释放自己的栅栏；不释放任何写者或声明其业务动作成功。
pub fn release_maintenance(
    tx: &Transaction<'_>,
    p: &PersonalMaintenanceParams,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<PersonalMaintenanceFence, PersistenceError> {
    current_profile(tx, profile, instance_fence)?;
    let mut fence = fence_record(tx, profile, &p.fence_id)?;
    if fence.owner != p.owner || fence.instance_fence != instance_fence || !owner_alive(&p.owner)? {
        return Err(reject("personal_maintenance_owner_changed"));
    }
    if fence.state == PersonalMaintenanceState::Released {
        return Ok(fence);
    }
    let operation_state: String = tx
        .query_row(
            "SELECT state FROM personal_maintenance_operation_registry WHERE profile_id=?1 AND operation_id=?2",
            params![profile, fence.operation_id],
            |row| row.get(0),
        )
        .map_err(database_error("check maintenance release operation"))?;
    if matches!(operation_state.as_str(), "capturing" | "applying") {
        return Err(reject("personal_data_operation_unconfirmed"));
    }
    if active_writers(tx, profile)?
        .iter()
        .any(|writer| writer.kind == "namespace_action")
    {
        return Err(reject("personal_namespace_storage_action_unconfirmed"));
    }
    fence.state = PersonalMaintenanceState::Released;
    fence.updated_at = now;
    tx.execute("UPDATE personal_maintenance_fences SET state='released',record_json=?3,updated_at=?4 WHERE profile_id=?1 AND fence_id=?2",params![profile,p.fence_id,encoded(&fence)?,now]).map_err(database_error("release personal maintenance"))?;
    Ok(fence)
}

/// # Errors
/// 仅旧拥有者确证退出且操作不在应用/捕获中时释放失联栅栏；写者账本保持原状。
pub fn recover_maintenance(
    tx: &Transaction<'_>,
    p: &PersonalMaintenanceParams,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<PersonalMaintenanceFence, PersistenceError> {
    current_profile(tx, profile, instance_fence)?;
    if !owner_alive(&p.owner)? {
        return Err(reject("personal_writer_owner_invalid"));
    }
    let mut fence = fence_record(tx, profile, &p.fence_id)?;
    if fence.state == PersonalMaintenanceState::Released {
        return Ok(fence);
    }
    if owner_alive(&fence.owner)? {
        return Err(reject("personal_maintenance_owner_alive"));
    }
    let state: String = tx
        .query_row(
            "SELECT state FROM personal_maintenance_operation_registry WHERE operation_id=?1 AND profile_id=?2",
            params![fence.operation_id, profile],
            |r| r.get(0),
        )
        .map_err(database_error("check interrupted data operation"))?;
    if matches!(state.as_str(), "capturing" | "applying") {
        return Err(reject("personal_data_operation_unconfirmed"));
    }
    if active_writers(tx, profile)?
        .iter()
        .any(|writer| writer.kind == "namespace_action")
    {
        return Err(reject("personal_namespace_storage_action_unconfirmed"));
    }
    fence.state = PersonalMaintenanceState::Released;
    fence.updated_at = now;
    tx.execute("UPDATE personal_maintenance_fences SET state='released',record_json=?3,updated_at=?4 WHERE profile_id=?1 AND fence_id=?2",params![profile,p.fence_id,encoded(&fence)?,now]).map_err(database_error("recover personal maintenance"))?;
    Ok(fence)
}

/// # Errors
/// 当前实例栅栏与拥有者不一致时拒绝，不接受资料外部路径或 PID。
pub fn maintenance_rpc(
    tx: &Transaction<'_>,
    method: &str,
    params: Value,
    profile: &str,
    instance_fence: &str,
    now: i64,
) -> Result<Value, PersistenceError> {
    if method == "personalMaintenance/operationStatus" {
        let p: contexthub_protocol::PersonalDataParams = serde_json::from_value(params)
            .map_err(|_| reject("personal_maintenance_request_invalid"))?;
        current_profile(tx, profile, instance_fence)?;
        return serde_json::to_value(crate::maintenance_operations::read(
            tx,
            profile,
            &p.operation_id,
        )?)
        .map_err(|_| reject("personal_maintenance_record_invalid"));
    }
    if method == "personalMaintenance/list" {
        let _: contexthub_protocol::PersonalMaintenanceListParams = serde_json::from_value(params)
            .map_err(|_| reject("personal_maintenance_request_invalid"))?;
        current_profile(tx, profile, instance_fence)?;
        let mut statement=tx.prepare("SELECT fence_id FROM personal_maintenance_fences WHERE profile_id=?1 ORDER BY created_at DESC,fence_id").map_err(database_error("list personal maintenance fences"))?;
        let ids = statement
            .query_map([profile], |row| row.get::<_, String>(0))
            .map_err(database_error("query personal maintenance fences"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read personal maintenance fence ids"))?;
        let fences = ids
            .into_iter()
            .map(|id| fence_record(tx, profile, &id))
            .collect::<Result<Vec<_>, _>>()?;
        return serde_json::to_value(fences)
            .map_err(|_| reject("personal_maintenance_record_invalid"));
    }
    if method == "personalMaintenance/drainStatus" {
        let p: PersonalMaintenanceParams = serde_json::from_value(params)
            .map_err(|_| reject("personal_maintenance_request_invalid"))?;
        current_profile(tx, profile, instance_fence)?;
        let fence = fence_record(tx, profile, &p.fence_id)?;
        if fence.owner != p.owner
            || fence.instance_fence != instance_fence
            || fence.state != PersonalMaintenanceState::Draining
            || !owner_alive(&p.owner)?
        {
            return Err(reject("personal_maintenance_owner_changed"));
        }
        let writers = active_writers(tx, profile)?;
        let mut statement = tx.prepare("SELECT record_json FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:lease:*' ORDER BY record_id").map_err(database_error("read all native model leases"))?;
        let mut leases = Vec::new();
        for raw in statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error("query all native model leases"))?
        {
            let value: Value =
                serde_json::from_str(&raw.map_err(database_error("read native model lease"))?)
                    .map_err(|_| reject("personal_data_quiescence_unconfirmed"))?;
            let lease: contexthub_protocol::ModelExecutionLease =
                serde_json::from_value(value["value"].clone())
                    .map_err(|_| reject("personal_data_quiescence_unconfirmed"))?;
            if !lease.released {
                leases.push(lease);
            }
        }
        return serde_json::to_value(contexthub_protocol::PersonalMaintenanceDrainStatus {
            fence,
            active_model_leases: leases,
            writer_registry_digest: hash(&writers)?,
            active_writers: writers,
        })
        .map_err(|_| reject("personal_maintenance_record_invalid"));
    }
    let result = match method {
        "personalMaintenance/acquire" => acquire_maintenance(
            tx,
            &serde_json::from_value(params)
                .map_err(|_| reject("personal_maintenance_request_invalid"))?,
            profile,
            instance_fence,
            now,
        )?,
        "personalMaintenance/release" => release_maintenance(
            tx,
            &serde_json::from_value(params)
                .map_err(|_| reject("personal_maintenance_request_invalid"))?,
            profile,
            instance_fence,
            now,
        )?,
        "personalMaintenance/recover" => recover_maintenance(
            tx,
            &serde_json::from_value(params)
                .map_err(|_| reject("personal_maintenance_request_invalid"))?,
            profile,
            instance_fence,
            now,
        )?,
        "personalMaintenance/read" => {
            let p: PersonalMaintenanceParams = serde_json::from_value(params)
                .map_err(|_| reject("personal_maintenance_request_invalid"))?;
            current_profile(tx, profile, instance_fence)?;
            fence_record(tx, profile, &p.fence_id)?
        }
        _ => return Err(reject("personal_maintenance_method_unknown")),
    };
    serde_json::to_value(result).map_err(|_| reject("personal_maintenance_record_invalid"))
}
