//! 备份/恢复回执和一致业务快照；仅由本地服务协调文件静止后调用。
use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{
    PersonalDataOperation, PersonalDataOperationState, PersonalDataQuiescence,
    PersonalMaintenanceFence, PersonalMaintenanceState, PersonalWriterState,
};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

mod workspaces;

fn reject(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}
fn encode(value: &impl serde::Serialize) -> Result<String, PersistenceError> {
    serde_json::to_string(value).map_err(|_| reject("personal_data_record_invalid"))
}
fn read(
    db: &rusqlite::Connection,
    profile: &str,
    id: &str,
) -> Result<Option<PersonalDataOperation>, PersistenceError> {
    let raw:Option<String>=db.query_row("SELECT record_json FROM personal_data_operations WHERE profile_id=?1 AND operation_id=?2",params![profile,id],|r|r.get(0)).optional().map_err(database_error("read personal data operation"))?;
    raw.map(|v| serde_json::from_str(&v).map_err(|_| reject("personal_data_record_invalid")))
        .transpose()
}
fn state(op: &PersonalDataOperation) -> &'static str {
    match op.state {
        PersonalDataOperationState::Prepared => "prepared",
        PersonalDataOperationState::Capturing => "capturing",
        PersonalDataOperationState::Incomplete => "incomplete",
        PersonalDataOperationState::ReadyToApply => "ready_to_apply",
        PersonalDataOperationState::Applying => "applying",
        PersonalDataOperationState::Completed => "completed",
        PersonalDataOperationState::Cancelled => "cancelled",
    }
}
fn verify_restore_candidate(database_path: &Path) -> Result<(), PersistenceError> {
    let directory = database_path
        .parent()
        .ok_or_else(|| reject("personal_data_restore_candidate_required"))?;
    let candidate = directory
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("restore-candidate-"))
        .ok_or_else(|| reject("personal_data_restore_candidate_required"))?;
    if candidate.len() != 32
        || !candidate
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || database_path.file_name().and_then(|name| name.to_str()) != Some("contexthub.db")
        || directory
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("profiles")
        || directory
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("business")
        || directory
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("cn.cilai.personal.workbench")
    {
        return Err(reject("personal_data_restore_candidate_required"));
    }
    for path in directory.ancestors() {
        let meta = std::fs::symlink_metadata(path)
            .map_err(|_| reject("personal_data_restore_candidate_required"))?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(reject("personal_data_restore_candidate_required"));
        }
    }
    contexthub_platform_adapter::security::verify_restricted_path(directory, true)
        .map_err(|_| reject("personal_data_restore_candidate_required"))?;
    Ok(())
}

impl SqlitePersistence {
    /// # Errors
    /// 离线恢复只读核对原维护归属，不改变服务代次。
    pub fn personal_data_maintenance_fence(
        &self,
        profile: &str,
        id: &str,
    ) -> Result<PersonalMaintenanceFence, PersistenceError> {
        let raw:String=self.connection.query_row("SELECT record_json FROM personal_maintenance_fences WHERE profile_id=?1 AND fence_id=?2",params![profile,id],|r|r.get(0)).map_err(database_error("read offline maintenance fence"))?;
        serde_json::from_str(&raw).map_err(|_| reject("personal_data_quiescence_unconfirmed"))
    }
    /// # Errors
    /// 数据库或资料回执损坏时明确失败。
    pub fn personal_data_operation(
        &self,
        profile: &str,
        id: &str,
    ) -> Result<Option<PersonalDataOperation>, PersistenceError> {
        read(&self.connection, profile, id)
    }
    /// # Errors
    /// 数据库无法读取时失败，不使用空列表代替。
    pub fn personal_data_operations(
        &self,
        profile: &str,
    ) -> Result<Vec<PersonalDataOperation>, PersistenceError> {
        let mut s=self.connection.prepare("SELECT record_json FROM personal_data_operations WHERE profile_id=?1 ORDER BY created_at DESC,operation_id").map_err(database_error("list personal data operations"))?;
        s.query_map([profile], |r| r.get::<_, String>(0))
            .map_err(database_error("query personal data operations"))?
            .map(|v| {
                serde_json::from_str(&v.map_err(database_error("read personal data operation"))?)
                    .map_err(|_| reject("personal_data_record_invalid"))
            })
            .collect()
    }
    /// # Errors
    /// 同次操作的输入改变或回执无法读取时拒绝。
    pub fn personal_data_by_client_operation(
        &self,
        profile: &str,
        id: &str,
        fingerprint: &str,
    ) -> Result<Option<PersonalDataOperation>, PersistenceError> {
        let old:Option<(String,String)>=self.connection.query_row("SELECT operation_id,request_fingerprint FROM personal_data_operations WHERE profile_id=?1 AND client_operation_id=?2",params![profile,id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(database_error("read personal data client operation"))?;
        match old {
            Some((op, f)) if f == fingerprint => read(&self.connection, profile, &op),
            Some(_) => Err(reject("operation_id_reused")),
            None => Ok(None),
        }
    }
    /// # Errors
    /// 准备回执只新增，不覆盖旧操作或其候选。
    pub fn prepare_personal_data(
        &mut self,
        op: &PersonalDataOperation,
        client: &str,
        fingerprint: &str,
    ) -> Result<PersonalDataOperation, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin personal data preparation"))?;
        let old:Option<(String,String)>=tx.query_row("SELECT operation_id,request_fingerprint FROM personal_data_operations WHERE profile_id=?1 AND client_operation_id=?2",params![op.profile_id,client],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(database_error("read concurrent personal data preparation"))?;
        if let Some((id, f)) = old {
            if f != fingerprint {
                return Err(reject("operation_id_reused"));
            }
            return read(&tx, &op.profile_id, &id)?
                .ok_or_else(|| reject("personal_data_record_invalid"));
        }
        let kind = match op.kind {
            contexthub_protocol::PersonalDataOperationKind::Backup => "backup",
            contexthub_protocol::PersonalDataOperationKind::Restore => "restore",
        };
        tx.execute("INSERT INTO personal_data_operations(operation_id,profile_id,operation_kind,client_operation_id,request_fingerprint,preview_digest,record_json,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![op.operation_id,op.profile_id,kind,client,fingerprint,op.preview_digest,encode(op)?,state(op),op.created_at,op.updated_at]).map_err(database_error("save personal data preparation"))?;
        crate::maintenance_operations::register(
            &tx,
            &contexthub_protocol::PersonalMaintenanceOperationStatus {
                operation_id: op.operation_id.clone(),
                profile_id: op.profile_id.clone(),
                kind: kind.into(),
                state: state(op).into(),
                preview_digest: op.preview_digest.clone(),
            },
            op.updated_at,
        )?;
        tx.commit()
            .map_err(database_error("commit personal data preparation"))?;
        Ok(op.clone())
    }
    /// # Errors
    /// 预览改变或状态不是预期时，不能覆盖并发终态。
    pub fn transition_personal_data(
        &mut self,
        op: &PersonalDataOperation,
        expected: &PersonalDataOperationState,
    ) -> Result<PersonalDataOperation, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin personal data transition"))?;
        let old = read(&tx, &op.profile_id, &op.operation_id)?
            .ok_or_else(|| reject("personal_data_operation_not_found"))?;
        if old.preview_digest != op.preview_digest {
            return Err(reject("personal_data_preview_changed"));
        }
        if &old.state != expected {
            return Err(reject("personal_data_operation_unconfirmed"));
        }
        tx.execute("UPDATE personal_data_operations SET record_json=?3,state=?4,updated_at=?5 WHERE profile_id=?1 AND operation_id=?2",params![op.profile_id,op.operation_id,encode(op)?,state(op),op.updated_at]).map_err(database_error("save personal data transition"))?;
        crate::maintenance_operations::register(
            &tx,
            &contexthub_protocol::PersonalMaintenanceOperationStatus {
                operation_id: op.operation_id.clone(),
                profile_id: op.profile_id.clone(),
                kind: match op.kind {
                    contexthub_protocol::PersonalDataOperationKind::Backup => "backup",
                    contexthub_protocol::PersonalDataOperationKind::Restore => "restore",
                }
                .into(),
                state: state(op).into(),
                preview_digest: op.preview_digest.clone(),
            },
            op.updated_at,
        )?;
        tx.commit()
            .map_err(database_error("commit personal data transition"))?;
        Ok(op.clone())
    }
    /// # Errors
    /// 无活动维护栅栏、拥有者已退出、写者仍活动或无法核对时拒绝。
    pub fn verify_personal_data_quiescence(
        &mut self,
        profile: &str,
        operation: &str,
        instance: &str,
        proof: &PersonalDataQuiescence,
    ) -> Result<(), PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("verify personal data quiescence"))?;
        let raw:Option<String>=tx.query_row("SELECT record_json FROM personal_maintenance_fences WHERE profile_id=?1 AND operation_id=?2 AND fence_id=?3 AND state='draining'",params![profile,operation,proof.token],|r|r.get(0)).optional().map_err(database_error("read personal data maintenance proof"))?;
        let fence: PersonalMaintenanceFence = serde_json::from_str(
            &raw.ok_or_else(|| reject("personal_data_quiescence_unconfirmed"))?,
        )
        .map_err(|_| reject("personal_data_quiescence_unconfirmed"))?;
        if proof.protocol_version != 1
            || proof.profile_id != profile
            || fence.owner != proof.owner
            || fence.instance_fence != instance
            || fence.state != PersonalMaintenanceState::Draining
            || contexthub_platform_adapter::instance::process_start_fingerprint(proof.owner.pid)
                .map_err(|_| reject("personal_data_quiescence_unconfirmed"))?
                .as_deref()
                != Some(proof.owner.start_fingerprint.as_str())
        {
            return Err(reject("personal_data_quiescence_unconfirmed"));
        }
        let expected = [
            "foreground",
            "automation",
            "review",
            "memory",
            "page_ai",
            "authentication",
            "personal_package_mcp",
            "user_mcp",
            "attachments",
            "local_operations",
            "ledger",
            "native_history",
            "runtime_initialization",
        ];
        let covered: std::collections::BTreeSet<_> =
            proof.covered_writers.iter().map(String::as_str).collect();
        if covered.len() != proof.covered_writers.len() || covered != expected.into_iter().collect()
        {
            return Err(reject("personal_data_writer_coverage_unconfirmed"));
        }
        let writers = crate::profile_writers::active_writers(&tx, profile)?;
        if writers
            .iter()
            .any(|w| w.state != PersonalWriterState::Released)
        {
            return Err(reject("personal_data_writer_active"));
        }
        let digest = format!("sha256:{:x}", Sha256::digest(encode(&writers)?.as_bytes()));
        if digest != proof.writer_registry_digest {
            return Err(reject("personal_data_writer_registry_changed"));
        }
        let mut s=tx.prepare("SELECT record_json FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:lease:*'").map_err(database_error("check model execution writers"))?;
        for raw in s
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(database_error("query model execution writers"))?
        {
            let raw = raw.map_err(database_error("read model execution writer"))?;
            let value: Value = serde_json::from_str(&raw)
                .map_err(|_| reject("personal_data_quiescence_unconfirmed"))?;
            if value
                .get("value")
                .and_then(|v| v.get("released"))
                .and_then(Value::as_bool)
                != Some(true)
            {
                return Err(reject("personal_data_writer_active"));
            }
        }
        Ok(())
    }
    /// # Errors
    /// 活动维护栅栏或数据库无法读取时返回明确状态。
    pub fn personal_maintenance_active(&self, profile: &str) -> Result<bool, PersistenceError> {
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM personal_maintenance_fences WHERE profile_id=?1 AND state='draining')",[profile],|r|r.get(0)).map_err(database_error("read personal maintenance state"))
    }
    /// 由持有本地服务主锁且已独立核对静止的调用者创建一致 `SQLite` 快照。
    /// # Errors
    /// 不覆盖已有文件；WAL中的已提交数据包含于VACUUM INTO结果。
    pub fn snapshot_personal_data_database(
        &self,
        destination: &Path,
    ) -> Result<u32, PersistenceError> {
        if destination.exists() {
            return Err(reject("personal_data_candidate_exists"));
        }
        self.connection
            .execute(
                "VACUUM INTO ?1",
                [destination
                    .to_str()
                    .ok_or_else(|| reject("personal_data_path_invalid"))?],
            )
            .map_err(database_error("snapshot personal data database"))?;
        contexthub_platform_adapter::security::restrict_existing_path(destination)
            .map_err(|_| reject("personal_data_write_failed"))?;
        let db = rusqlite::Connection::open(destination)
            .map_err(database_error("open personal data snapshot"))?;
        sanitize_snapshot(&db)?;
        db.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .map_err(database_error("read snapshot migration version"))
    }
}

fn preserve_application_data_history(db: &rusqlite::Connection) -> Result<(), PersistenceError> {
    let mut statement = db
        .prepare("SELECT operation_id,profile_id FROM personal_application_data_operations")
        .map_err(database_error("read backup application data history"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error("query backup application data history"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read backup application history records"))?;
    for (id, profile) in rows {
        let mut record = crate::personal_application_data::read(db, &profile, &id)
            .map_err(|_| reject("personal_data_record_invalid"))?;
        let registered:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM personal_maintenance_operation_registry WHERE operation_id=?1 AND profile_id=?2 AND preview_digest=?3 AND operation_kind='personal_capability_data_change')",params![id,record.operation.profile_id,record.operation.preview_digest],|row|row.get(0)).map_err(database_error("verify backup application history authority"))?;
        if !registered
            || record.operation.operation_id != id
            || record.operation.profile_id != profile
        {
            return Err(reject("personal_data_record_invalid"));
        }
        record.operation.read_only_after_restore = true;
        if record.operation.source_profile_id.is_none() {
            record.operation.source_profile_id = Some(record.operation.profile_id.clone());
        }
        db.execute(
            "UPDATE personal_application_data_operations SET record_json=?2 WHERE operation_id=?1",
            params![id, encode(&record)?],
        )
        .map_err(database_error("preserve read only application history"))?;
    }
    Ok(())
}
fn sanitize_runtime_entries(db: &rusqlite::Connection) -> Result<(), PersistenceError> {
    let entries = {
        let mut s = db
            .prepare("SELECT record_id,record_json,position FROM ai_runtime_entries")
            .map_err(database_error("read backup runtime settings"))?;
        s.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(database_error("query backup runtime settings"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read backup runtime settings"))?
    };
    for (id, raw, position) in entries {
        if id == "memoryEnableConfirmation"
            || (id.starts_with("personalModel:")
                && !id.starts_with("personalModel:connection:")
                && !id.starts_with("personalModel:descriptor:"))
        {
            db.execute("DELETE FROM ai_runtime_entries WHERE record_id=?1", [&id])
                .map_err(database_error("remove backup authorization state"))?;
            continue;
        }
        let mut value: Value =
            serde_json::from_str(&raw).map_err(|_| reject("personal_data_record_invalid"))?;
        sanitize_settings(&mut value);
        let raw = encode(&value)?;
        let digest = crate::ai_business_ledger::digest(
            &serde_json::json!({"position":position,"value":value}),
        );
        db.execute(
            "UPDATE ai_runtime_entries SET record_json=?2,digest=?3 WHERE record_id=?1",
            params![id, raw, digest],
        )
        .map_err(database_error("save sanitized runtime settings"))?;
    }
    Ok(())
}

fn sanitize_snapshot(db: &rusqlite::Connection) -> Result<(), PersistenceError> {
    db.execute_batch("PRAGMA foreign_keys=OFF; BEGIN IMMEDIATE; DELETE FROM personal_maintenance_fences; DELETE FROM personal_profile_writers; DELETE FROM personal_data_operations; DELETE FROM personal_storage_operations; DELETE FROM personal_namespace_storage_operations; DELETE FROM personal_memory_reset_operations; DELETE FROM personal_maintenance_operation_registry WHERE operation_id NOT IN (SELECT operation_id FROM personal_application_data_operations); UPDATE personal_maintenance_operation_registry SET state='cancelled' WHERE operation_id IN (SELECT operation_id FROM personal_application_data_operations); DELETE FROM personal_mcp_records WHERE kind<>'connection'; DELETE FROM personal_file_grants; UPDATE personal_capability_installations SET enabled=0; UPDATE accounts SET credential_ref=NULL; DELETE FROM employee_service_credentials; DELETE FROM ai_ledger_commands;").map_err(database_error("sanitize personal data snapshot"))?;
    preserve_application_data_history(db)?;
    sanitize_runtime_entries(db)?;
    let mcp = {
        let mut s = db
            .prepare(
                "SELECT record_id,record_json FROM personal_mcp_records WHERE kind='connection'",
            )
            .map_err(database_error("read backup MCP connections"))?;
        s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(database_error("query backup MCP connections"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read backup MCP connections"))?
    };
    for (id, raw) in mcp {
        let mut value: Value =
            serde_json::from_str(&raw).map_err(|_| reject("personal_data_record_invalid"))?;
        sanitize_settings(&mut value);
        db.execute("UPDATE personal_mcp_records SET record_json=?2 WHERE kind='connection' AND record_id=?1",params![id,encode(&value)?]).map_err(database_error("save backup MCP metadata"))?;
    }
    let retired_update:Option<String>=db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='retired_ai_runtime_state_update'",[],|row|row.get(0)).optional().map_err(database_error("read retired snapshot configuration guard"))?;
    if let Some(sql) = retired_update.as_deref() {
        let expected = include_str!("ai_native_business_views.sql")
            .lines()
            .find(|line| line.starts_with("CREATE TRIGGER retired_ai_runtime_state_update "))
            .ok_or_else(|| reject("personal_data_schema_invalid"))?;
        if sql.trim_end_matches(';') != expected.trim_end_matches(';') {
            return Err(reject("personal_data_schema_invalid"));
        }
        db.execute_batch("DROP TRIGGER retired_ai_runtime_state_update;")
            .map_err(database_error(
                "prepare snapshot configuration sanitization",
            ))?;
    }
    for (table, key, column) in [
        ("ai_runtime_state", "state_key", "state_json"),
        ("ai_memory_state", "state_key", "value_json"),
    ] {
        let sql = format!("SELECT {key},{column} FROM {table}");
        let rows = {
            let mut s = db
                .prepare(&sql)
                .map_err(database_error("read backup configuration"))?;
            s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(database_error("query backup configuration"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error("read backup configuration rows"))?
        };
        for (id, raw) in rows {
            let mut value: Value =
                serde_json::from_str(&raw).map_err(|_| reject("personal_data_record_invalid"))?;
            sanitize_settings(&mut value);
            let sql = format!("UPDATE {table} SET {column}=?2 WHERE {key}=?1");
            db.execute(&sql, params![id, encode(&value)?])
                .map_err(database_error("sanitize backup configuration"))?;
        }
    }
    if let Some(sql) = retired_update {
        db.execute_batch(&sql)
            .map_err(database_error("restore snapshot configuration guard"))?;
    }
    db.execute_batch("COMMIT; VACUUM; PRAGMA foreign_keys=ON;")
        .map_err(database_error("finalize sanitized database"))?;
    verify_snapshot_integrity(db)
}
fn verify_snapshot_integrity(db: &rusqlite::Connection) -> Result<(), PersistenceError> {
    let integrity: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(database_error("verify snapshot integrity"))?;
    let broken_references: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
            [],
            |row| row.get(0),
        )
        .map_err(database_error("verify snapshot ownership references"))?;
    if integrity != "ok" || broken_references {
        return Err(reject("personal_data_database_invalid"));
    }
    Ok(())
}
fn rebind_application_data(
    db: &rusqlite::Connection,
    source: &str,
    target: &str,
) -> Result<(), PersistenceError> {
    for table in [
        "personal_application_data_slots",
        "personal_application_data_pointers",
        "personal_application_data_operations",
    ] {
        let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE profile_id<>?1)");
        let foreign: bool = db
            .query_row(&sql, [source], |row| row.get(0))
            .map_err(database_error("verify restored application ownership"))?;
        if foreign {
            return Err(reject("personal_data_restore_profile_reference_unknown"));
        }
    }
    let mut statement = db
        .prepare(
            "SELECT slot_id,creation_project_id,record_json FROM personal_application_data_slots WHERE profile_id=?1",
        )
        .map_err(database_error("read restored application slots"))?;
    let rows = statement
        .query_map([source], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error("query restored application slots"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read restored application slot records"))?;
    for (id, project, raw) in rows {
        let mut slot: crate::PersonalApplicationDataSlot =
            serde_json::from_str(&raw).map_err(|_| reject("personal_data_record_invalid"))?;
        crate::personal_application_data::validate_slot(&slot, source, &project)
            .map_err(|_| reject("personal_data_restore_profile_reference_unknown"))?;
        if slot.slot_id != id {
            return Err(reject("personal_data_restore_profile_reference_unknown"));
        }
        slot.profile_id = target.into();
        db.execute("UPDATE personal_application_data_slots SET profile_id=?1,record_json=?4 WHERE profile_id=?2 AND slot_id=?3",params![target,source,id,encode(&slot)?]).map_err(database_error("rebind restored application slot"))?;
    }
    db.execute(
        "UPDATE personal_application_data_pointers SET profile_id=?1 WHERE profile_id=?2",
        params![target, source],
    )
    .map_err(database_error("rebind restored application data pointers"))?;
    let mut statement=db.prepare("SELECT operation_id,creation_project_id FROM personal_application_data_operations WHERE profile_id=?1").map_err(database_error("read restored application history"))?;
    let rows = statement
        .query_map([source], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(database_error("query restored application history"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read restored application history records"))?;
    for (id, project) in rows {
        let mut record = crate::personal_application_data::read(db, source, &id)
            .map_err(|_| reject("personal_data_restore_profile_reference_unknown"))?;
        if record.operation.creation_project_id != project {
            return Err(reject("personal_data_restore_profile_reference_unknown"));
        }
        record.operation.profile_id = target.into();
        record.operation.read_only_after_restore = true;
        if record.operation.source_profile_id.is_none() {
            record.operation.source_profile_id = Some(source.into());
        }
        for slot in record
            .source_slot
            .iter_mut()
            .chain(std::iter::once(&mut record.target_slot))
        {
            if slot.profile_id != source {
                return Err(reject("personal_data_restore_profile_reference_unknown"));
            }
            slot.profile_id = target.into();
        }
        db.execute("UPDATE personal_application_data_operations SET profile_id=?1,record_json=?4 WHERE profile_id=?2 AND operation_id=?3",params![target,source,id,encode(&record)?]).map_err(database_error("rebind restored read only application history"))?;
    }
    db.execute("UPDATE personal_maintenance_operation_registry SET profile_id=?1 WHERE profile_id=?2 AND operation_kind='personal_capability_data_change'",params![target,source]).map_err(database_error("rebind restored application history authority"))?;
    Ok(())
}
fn sanitize_settings(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                match key.as_str() {
                    "secretRef"
                    | "proxySecretRef"
                    | "networkCredentialRef"
                    | "credentialRef"
                    | "credential"
                    | "apiKey"
                    | "accessToken"
                    | "refreshToken"
                    | "password"
                    | "token" => *value = Value::Null,
                    "env" | "environment" | "headers" => *value = serde_json::json!({}),
                    "enabled" | "autoApprove" | "memoryEnabled" | "verified" => {
                        *value = Value::Bool(false);
                    }
                    "approvedToolNames" | "purposes" => {
                        *value = serde_json::json!([]);
                    }
                    "validation"
                    | "approvedCatalogDigest"
                    | "approvalReviewModel"
                    | "memoryEnableConfirmation" => {
                        *value = Value::Null;
                    }
                    "authScopes" => *value = serde_json::json!([]),
                    "authGeneration" => *value = serde_json::json!(0),
                    _ => sanitize_settings(value),
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(sanitize_settings),
        _ => {}
    }
}

impl SqlitePersistence {
    /// 重绑当前恢复目标的本地配置归属，历史执行快照和原生数据库保持来源身份。
    /// # Errors
    /// 非预期来源、未知资料表或数据库校验失败时拒绝激活。
    #[expect(
        clippy::too_many_lines,
        reason = "单一恢复事务显式核对资料表与配置归属白名单"
    )]
    pub fn prepare_restored_personal_profile(
        &mut self,
        source: &str,
        target: &str,
    ) -> Result<(), PersistenceError> {
        verify_restore_candidate(&self.database_path)?;
        sanitize_snapshot(&self.connection)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("prepare restored personal profile"))?;
        let current: Option<String> = tx
            .query_row(
                "SELECT profile_id FROM personal_profile_compatibility WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(database_error("read restored profile identity"))?;
        if current.as_deref() != Some(source) {
            return Err(reject("personal_data_restore_source_mismatch"));
        }
        let tables = {
            let mut s=tx.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'").map_err(database_error("list restored profile tables"))?;
            s.query_map([], |r| r.get::<_, String>(0))
                .map_err(database_error("query restored profile tables"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(database_error("read restored profile table names"))?
        };
        let known = [
            "personal_profile_compatibility",
            "personal_mcp_records",
            "personal_data_operations",
            "personal_profile_writers",
            "personal_maintenance_fences",
            "personal_package_imports",
            "personal_maintenance_operation_registry",
            "personal_storage_operations",
            "personal_namespace_storage_operations",
            "personal_memory_reset_operations",
            "personal_application_data_slots",
            "personal_application_data_pointers",
            "personal_application_data_operations",
        ];
        for table in tables {
            let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\""));
            let mut s = tx
                .prepare(&sql)
                .map_err(database_error("inspect restored profile table"))?;
            for column in s
                .query_map([], |r| r.get::<_, String>(1))
                .map_err(database_error("query restored profile columns"))?
            {
                if column.map_err(database_error("read restored profile column"))? == "profile_id"
                    && !known.contains(&table.as_str())
                {
                    return Err(reject("personal_data_restore_profile_reference_unknown"));
                }
            }
        }
        tx.pragma_update(None, "defer_foreign_keys", true)
            .map_err(database_error(
                "defer restored application ownership references",
            ))?;
        rebind_application_data(&tx, source, target)?;
        tx.execute(
            "DELETE FROM ai_runtime_entries WHERE record_id='personalNotifications'",
            [],
        )
        .map_err(database_error(
            "reset restored notification authorization and receipts",
        ))?;
        tx.execute("UPDATE personal_profile_compatibility SET profile_id=?1,state='pending',instance_fence=NULL,manifest_digest=NULL WHERE singleton=1",[target]).map_err(database_error("bind restored personal profile"))?;
        tx.execute("DELETE FROM personal_package_imports", [])
            .map_err(database_error("retire restored import candidates"))?;
        tx.execute(
            "UPDATE personal_mcp_records SET profile_id=?1 WHERE profile_id=?2",
            params![target, source],
        )
        .map_err(database_error("bind restored MCP metadata"))?;
        for (table, id_column) in [
            ("ai_runtime_entries", "record_id"),
            ("personal_mcp_records", "record_id"),
        ] {
            let select = if table == "ai_runtime_entries" {
                "SELECT record_id,record_json FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:connection:*'"
            } else {
                "SELECT record_id,record_json FROM personal_mcp_records"
            };
            let rows = {
                let mut s = tx
                    .prepare(select)
                    .map_err(database_error("read restored local metadata"))?;
                s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(database_error("query restored local metadata"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(database_error("read restored local metadata rows"))?
            };
            for (id, raw) in rows {
                let mut value: Value = serde_json::from_str(&raw)
                    .map_err(|_| reject("personal_data_record_invalid"))?;
                let body = if table == "ai_runtime_entries" {
                    value
                        .get_mut("value")
                        .ok_or_else(|| reject("personal_data_record_invalid"))?
                } else {
                    &mut value
                };
                if let Some(object) = body.as_object_mut()
                    && object.get("profileId").and_then(Value::as_str) == Some(source)
                {
                    object.insert("profileId".into(), Value::String(target.into()));
                }
                let raw = encode(&value)?;
                let sql = format!("UPDATE {table} SET record_json=?2 WHERE {id_column}=?1");
                tx.execute(&sql, params![id, raw])
                    .map_err(database_error("save restored local metadata"))?;
                if table == "ai_runtime_entries" {
                    let position: i64 = tx
                        .query_row(
                            "SELECT position FROM ai_runtime_entries WHERE record_id=?1",
                            [&id],
                            |row| row.get(0),
                        )
                        .map_err(database_error("read restored metadata position"))?;
                    tx.execute(
                        "UPDATE ai_runtime_entries SET digest=?2 WHERE record_id=?1",
                        params![
                            id,
                            crate::ai_business_ledger::digest(
                                &serde_json::json!({"position":position,"value":value})
                            )
                        ],
                    )
                    .map_err(database_error("save restored metadata digest"))?;
                }
            }
        }
        tx.commit()
            .map_err(database_error("commit restored profile identity"))?;
        Ok(())
    }
    /// # Errors
    /// 恢复结果仅插入一次；未知或冲突的结果不能覆盖。
    pub fn record_personal_data_restore_receipt(
        &mut self,
        op: &PersonalDataOperation,
    ) -> Result<(), PersistenceError> {
        if let Some(old) = read(&self.connection, &op.profile_id, &op.operation_id)? {
            if old == *op {
                return Ok(());
            }
            return Err(reject("personal_data_restore_receipt_conflict"));
        }
        self.prepare_personal_data(
            op,
            &format!("restore-receipt-{}", op.operation_id),
            &op.preview_digest,
        )?;
        Ok(())
    }
}

/// 对已静止并复制到候选内的原生 `SQLite` 执行受支持的文件快照，不更改原生表或运行库。
/// # Errors
/// WAL、完整性或数据库内容无法读回时拒绝，不忽略日志文件。
pub fn materialize_native_database(
    copied: &Path,
    destination: &Path,
) -> Result<(), PersistenceError> {
    if destination.exists() {
        return Err(reject("personal_data_candidate_exists"));
    }
    let db = rusqlite::Connection::open(copied)
        .map_err(database_error("open copied native database"))?;
    let check: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(database_error("check copied native database"))?;
    if check != "ok" {
        return Err(reject("personal_data_native_database_invalid"));
    }
    db.execute(
        "VACUUM INTO ?1",
        [destination
            .to_str()
            .ok_or_else(|| reject("personal_data_path_invalid"))?],
    )
    .map_err(database_error("materialize copied native WAL"))?;
    contexthub_platform_adapter::security::restrict_existing_path(destination)
        .map_err(|_| reject("personal_data_write_failed"))?;
    Ok(())
}

impl SqlitePersistence {
    /// # Errors
    /// 中断确认必须证明旧维护拥有者已退出，且无未释放的模型或本机写者。
    pub fn verify_interrupted_personal_data(
        &mut self,
        profile: &str,
        operation: &str,
    ) -> Result<(), PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("verify interrupted personal data"))?;
        let raw:Option<String>=tx.query_row("SELECT record_json FROM personal_maintenance_fences WHERE profile_id=?1 AND operation_id=?2 AND state='draining'",params![profile,operation],|row|row.get(0)).optional().map_err(database_error("read interrupted personal maintenance"))?;
        let fence: PersonalMaintenanceFence = serde_json::from_str(
            &raw.ok_or_else(|| reject("personal_data_quiescence_unconfirmed"))?,
        )
        .map_err(|_| reject("personal_data_quiescence_unconfirmed"))?;
        if contexthub_platform_adapter::instance::process_start_fingerprint(fence.owner.pid)
            .map_err(|_| reject("personal_data_quiescence_unconfirmed"))?
            .as_deref()
            == Some(fence.owner.start_fingerprint.as_str())
        {
            return Err(reject("personal_maintenance_owner_alive"));
        }
        if !crate::profile_writers::active_writers(&tx, profile)?.is_empty() {
            return Err(reject("personal_data_writer_active"));
        }
        let mut statement=tx.prepare("SELECT record_json FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:lease:*'").map_err(database_error("read interrupted model leases"))?;
        for raw in statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error("query interrupted model leases"))?
        {
            let value: Value =
                serde_json::from_str(&raw.map_err(database_error("read interrupted model lease"))?)
                    .map_err(|_| reject("personal_data_record_invalid"))?;
            let lease: contexthub_protocol::ModelExecutionLease =
                serde_json::from_value(value["value"].clone())
                    .map_err(|_| reject("personal_data_record_invalid"))?;
            if !lease.released {
                return Err(reject("personal_data_writer_active"));
            }
        }
        Ok(())
    }
    /// # Errors
    /// 包内数据库结构与当前版本不符、完整性失败或包含未知触发器时拒绝。
    pub fn verify_personal_data_snapshot_schema(
        &self,
        path: &Path,
    ) -> Result<(), PersistenceError> {
        let other = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(database_error("open backup database for validation"))?;
        let schema=|db:&rusqlite::Connection|->Result<Vec<(String,String,Option<String>)>,PersistenceError>{let mut s=db.prepare("SELECT type,name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name").map_err(database_error("read backup database schema"))?;s.query_map([],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(database_error("query backup database schema"))?.collect::<Result<Vec<_>,_>>().map_err(database_error("read backup schema records"))};
        if normalized_backup_schema(schema(&self.connection)?)?
            != normalized_backup_schema(schema(&other)?)?
        {
            return Err(reject("personal_data_schema_invalid"));
        }
        let integrity: String = other
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(database_error("validate backup database integrity"))?;
        if integrity != "ok" {
            return Err(reject("personal_data_database_invalid"));
        }
        Ok(())
    }
}

impl SqlitePersistence {
    /// # Errors
    /// 会话附件引用无法读取时不能声明附件完整。
    pub fn personal_data_attachment_paths(&self) -> Result<Vec<String>, PersistenceError> {
        fn collect(value: &Value, paths: &mut std::collections::BTreeSet<String>) {
            match value {
                Value::Object(fields) => {
                    if fields.contains_key("attachmentId")
                        && let Some(path) = fields.get("localPath").and_then(Value::as_str)
                    {
                        paths.insert(path.into());
                    }
                    for value in fields.values() {
                        collect(value, paths);
                    }
                }
                Value::Array(values) => values.iter().for_each(|v| collect(v, paths)),
                _ => {}
            }
        }
        let mut paths = std::collections::BTreeSet::new();
        for (table, column) in [
            ("ai_message_ledger", "record_json"),
            ("ai_messages", "message_json"),
            ("ai_application_messages", "attachments_json"),
            ("ai_application_attachment_refs", "metadata_json"),
        ] {
            let sql = format!("SELECT {column} FROM {table}");
            let mut statement = self
                .connection
                .prepare(&sql)
                .map_err(database_error("read backup attachment references"))?;
            for raw in statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(database_error("query backup attachment references"))?
            {
                let value: Value = serde_json::from_str(
                    &raw.map_err(database_error("read backup attachment reference"))?,
                )
                .map_err(|_| reject("personal_data_record_invalid"))?;
                collect(&value, &mut paths);
            }
        }
        Ok(paths.into_iter().collect())
    }
}

fn normalized_backup_schema(
    rows: Vec<(String, String, Option<String>)>,
) -> Result<Vec<(String, String, Option<String>)>, PersistenceError> {
    let native = include_str!("ai_native_business_views.sql");
    let mut output = vec![];
    let mut views = 0;
    let mut retired = 0;
    let mut native_views = 0;
    for (kind, name, sql) in rows {
        if kind == "view"
            && [
                "ai_business_conversations",
                "ai_business_messages",
                "ai_business_runs",
            ]
            .contains(&name.as_str())
        {
            let legacy_table = match name.as_str() {
                "ai_business_conversations" => "ai_conversations",
                "ai_business_messages" => "ai_messages",
                _ => "ai_runs",
            };
            let legacy = format!("CREATE VIEW {name} AS SELECT * FROM {legacy_table}");
            let value = sql
                .as_deref()
                .ok_or_else(|| reject("personal_data_schema_invalid"))?;
            if value != legacy
                && !native.split(';').map(str::trim).any(|part| {
                    part.starts_with(&format!("CREATE VIEW {name} AS"))
                        && part == value.trim_end_matches(';')
                })
            {
                return Err(reject("personal_data_schema_invalid"));
            }
            if value != legacy {
                native_views += 1;
            }
            views += 1;
            continue;
        }
        if kind == "trigger" && name.starts_with("retired_ai_") {
            let value = sql
                .as_deref()
                .ok_or_else(|| reject("personal_data_schema_invalid"))?;
            if !native.lines().any(|line| {
                line.starts_with(&format!("CREATE TRIGGER {name} "))
                    && line.trim_end_matches(';') == value.trim_end_matches(';')
            }) {
                return Err(reject("personal_data_schema_invalid"));
            }
            retired += 1;
            continue;
        }
        output.push((kind, name, sql));
    }
    if views != 3 || !matches!((retired, native_views), (0, 0) | (15, 3)) {
        return Err(reject("personal_data_schema_invalid"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("personal_data_application_tests.rs");
    #[test]
    fn restore_sanitization_refuses_current_profile_database_before_any_write() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "cilai-data-current-protection-{}",
            uuid::Uuid::new_v4().simple()
        ));
        contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        db.connection
            .execute(
                "INSERT INTO app_metadata(key,value,updated_at) VALUES('protected','unchanged',1)",
                [],
            )
            .unwrap();
        let rows_before: i64 = db
            .connection
            .query_row("SELECT COUNT(*) FROM app_metadata", [], |row| row.get(0))
            .unwrap();
        let result = db.prepare_restored_personal_profile("source", "target");
        assert_eq!(
            result.unwrap_err().reason(),
            Some("personal_data_restore_candidate_required")
        );
        assert_eq!(
            db.connection
                .query_row("SELECT COUNT(*) FROM app_metadata", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            rows_before
        );
        assert_eq!(
            db.connection
                .query_row(
                    "SELECT value FROM app_metadata WHERE key='protected'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "unchanged"
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn snapshot_sanitizes_runtime_scalar_entries_and_purges_secret_pages() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "cilai-data-snapshot-{}",
            uuid::Uuid::new_v4().simple()
        ));
        contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
        let db = SqlitePersistence::open(&root.join("source.sqlite"), "test").unwrap();
        for (id, value) in [
            (
                "approvalReviewModel",
                serde_json::json!({"entryId":"approvalReviewModel","field":"approvalReviewModel","shape":"scalar","value":null}),
            ),
            (
                "memoryPreferences",
                serde_json::json!({"entryId":"memoryPreferences","field":"memoryPreferences","shape":"scalar","value":{"enabled":true}}),
            ),
            (
                "memoryEnableConfirmation",
                serde_json::json!({"entryId":"memoryEnableConfirmation","field":"memoryEnableConfirmation","shape":"scalar","value":{"consentDigest":"sha256:previous-consent","fromToolChats":true,"confirmedAt":1}}),
            ),
        ] {
            db.connection.execute("INSERT INTO ai_runtime_entries(record_id,position,record_json,digest,updated_at) VALUES(?1,0,?2,'test',1)",params![id,value.to_string()]).unwrap();
        }
        db.connection.execute("INSERT INTO employee_service_credentials(tenant_id,membership_id,capability_type,api_key,key_prefix,gateway_url,issued_at,updated_at) VALUES('test','test','test','synthetic-sensitive-value','test','https://example.invalid',1,1)",[]).unwrap();
        db.connection
            .execute_batch(include_str!("ai_native_business_views.sql"))
            .unwrap();
        let snapshot = root.join("snapshot.sqlite");
        db.snapshot_personal_data_database(&snapshot).unwrap();
        let data = std::fs::read(&snapshot).unwrap();
        assert!(
            !data
                .windows(b"synthetic-sensitive-value".len())
                .any(|v| v == b"synthetic-sensitive-value")
        );
        let checked = rusqlite::Connection::open(&snapshot).unwrap();
        let raw: String = checked
            .query_row(
                "SELECT record_json FROM ai_runtime_entries WHERE record_id='memoryPreferences'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&raw).unwrap()["value"]["enabled"],
            false
        );
        assert_eq!(checked.query_row("SELECT COUNT(*) FROM ai_runtime_entries WHERE record_id='memoryEnableConfirmation'",[],|r|r.get::<_,u32>(0)).unwrap(),0);
        assert_eq!(db.connection.query_row("SELECT COUNT(*) FROM ai_runtime_entries WHERE record_id='memoryEnableConfirmation'",[],|r|r.get::<_,u32>(0)).unwrap(),1);
        let mut network = serde_json::json!({"proxySecretRef":{"service":"synthetic-network-key"},"networkCredentialRef":{"account":"synthetic-network-account"},"network":{"headerNames":["x-api-key"],"proxyAuth":"basic","caCertificatePem":"PUBLIC_CA"}});
        sanitize_settings(&mut network);
        assert!(network["proxySecretRef"].is_null());
        assert!(network["networkCredentialRef"].is_null());
        assert_eq!(
            network["network"]["headerNames"],
            serde_json::json!(["x-api-key"])
        );
        assert_eq!(network["network"]["caCertificatePem"], "PUBLIC_CA");
        let mut old_state = serde_json::json!({"memoryEnableConfirmation":{"consentDigest":"sha256:previous-consent","fromToolChats":true,"confirmedAt":1},"memoryPreferences":{"enabled":true}});
        sanitize_settings(&mut old_state);
        assert!(old_state["memoryEnableConfirmation"].is_null());
        assert_eq!(old_state["memoryPreferences"]["enabled"], false);
        drop(checked);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
}
