//! 个人应用数据代次及升级回执。文件物化由 `AppServer` 完成，指针与安装在同一事务激活。
use crate::{
    CapabilityCreationProjectRecord, PersistenceError, PersistenceErrorKind,
    PersonalCapabilityInstallationRecord, SqlitePersistence, database_error,
};
use contexthub_protocol::{
    PersonalApplicationDataAction as Action, PersonalApplicationDataOperation as Operation,
    PersonalApplicationDataPrepareParams as Prepare, PersonalApplicationDataState as State,
    PersonalMaintenanceOperationStatus,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalApplicationDataSlot {
    pub profile_id: String,
    pub creation_project_id: String,
    pub slot_id: String,
    pub package_digest: String,
    pub relative_path: String,
    pub created_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalApplicationDataRecord {
    pub operation: Operation,
    pub request: Prepare,
    pub fingerprint: String,
    pub source_slot: Option<PersonalApplicationDataSlot>,
    pub target_slot: PersonalApplicationDataSlot,
    pub previous_project: Option<CapabilityCreationProjectRecord>,
    pub previous_installation: Option<PersonalCapabilityInstallationRecord>,
    pub rollback_project: Option<CapabilityCreationProjectRecord>,
    pub rollback_installation: Option<PersonalCapabilityInstallationRecord>,
    pub expected_retained_digest: Option<String>,
}
fn rejected(code: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, code, code)
}
fn encoded(value: &impl Serialize) -> Result<String, PersistenceError> {
    serde_json::to_string(value).map_err(|_| rejected("personal_application_data_record_invalid"))
}
fn decoded<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T, PersistenceError> {
    serde_json::from_str(raw).map_err(|_| rejected("personal_application_data_record_invalid"))
}
fn hash(value: &impl Serialize) -> Result<String, PersistenceError> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(encoded(value)?.as_bytes())
    ))
}
fn slot(
    db: &Connection,
    profile: &str,
    id: &str,
) -> Result<PersonalApplicationDataSlot, PersistenceError> {
    let raw:Option<String>=db.query_row("SELECT record_json FROM personal_application_data_slots WHERE profile_id=?1 AND slot_id=?2",params![profile,id],|r|r.get(0)).optional().map_err(database_error("read application data slot"))?;
    let record: PersonalApplicationDataSlot =
        decoded(&raw.ok_or_else(|| rejected("personal_application_data_slot_missing"))?)?;
    validate_slot(&record, profile, &record.creation_project_id)?;
    if record.slot_id != id {
        return Err(rejected("personal_application_data_record_invalid"));
    }
    Ok(record)
}
pub(crate) fn validate_slot(
    record: &PersonalApplicationDataSlot,
    profile: &str,
    project: &str,
) -> Result<(), PersistenceError> {
    let legacy = format!("personal-capability-data/{project}");
    let versioned = format!("personal-capability-slots/{project}/{}", record.slot_id);
    let valid_project = project.strip_prefix("capability_").is_some_and(|id| {
        id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    });
    if record.profile_id != profile
        || record.creation_project_id != project
        || !valid_project
        || !valid_uuid(&record.slot_id)
        || ![legacy, versioned].contains(&record.relative_path)
    {
        return Err(rejected("personal_application_data_record_invalid"));
    }
    Ok(())
}
fn valid_uuid(id: &str) -> bool {
    uuid::Uuid::parse_str(id)
        .is_ok_and(|v| v.get_version_num() == 4 && v.simple().to_string() == id)
}
fn active(
    db: &Connection,
    profile: &str,
    project: &str,
) -> Result<Option<PersonalApplicationDataSlot>, PersistenceError> {
    let id:Option<String>=db.query_row("SELECT slot_id FROM personal_application_data_pointers WHERE profile_id=?1 AND creation_project_id=?2",params![profile,project],|r|r.get(0)).optional().map_err(database_error("read active application data"))?;
    id.map(|id| {
        let record = slot(db, profile, &id)?;
        validate_slot(&record, profile, project)?;
        Ok(record)
    })
    .transpose()
}
fn admission(tx: &Transaction<'_>, profile: &str) -> Result<(), PersistenceError> {
    let instance: String = tx.query_row("SELECT instance_fence FROM personal_profile_compatibility WHERE profile_id=?1 AND state='ready'", [profile], |r|r.get(0)).map_err(database_error("read application data instance"))?;
    crate::profile_writers::require_writer_admission(tx, profile, &instance)
}
fn save_slot(db: &Connection, slot: &PersonalApplicationDataSlot) -> Result<(), PersistenceError> {
    db.execute("INSERT INTO personal_application_data_slots(profile_id,slot_id,creation_project_id,record_json,created_at) VALUES(?1,?2,?3,?4,?5)",params![slot.profile_id,slot.slot_id,slot.creation_project_id,encoded(slot)?,slot.created_at]).map_err(database_error("save immutable application data slot"))?;
    Ok(())
}
fn status(state: &State) -> &'static str {
    match state {
        State::Prepared => "prepared",
        State::Applying => "applying",
        State::Completed => "completed",
        State::Incomplete => "incomplete",
        State::Cancelled => "cancelled",
    }
}
fn save_operation(
    db: &Connection,
    record: &PersonalApplicationDataRecord,
) -> Result<(), PersistenceError> {
    let op = &record.operation;
    crate::maintenance_operations::register(
        db,
        &PersonalMaintenanceOperationStatus {
            operation_id: op.operation_id.clone(),
            profile_id: op.profile_id.clone(),
            kind: "personal_capability_data_change".into(),
            state: status(&op.state).into(),
            preview_digest: op.preview_digest.clone(),
        },
        op.updated_at,
    )?;
    db.execute("INSERT INTO personal_application_data_operations(operation_id,profile_id,creation_project_id,client_operation_id,fingerprint,record_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(operation_id) DO UPDATE SET record_json=excluded.record_json,updated_at=excluded.updated_at WHERE personal_application_data_operations.profile_id=excluded.profile_id AND personal_application_data_operations.fingerprint=excluded.fingerprint",params![op.operation_id,op.profile_id,op.creation_project_id,op.client_operation_id,record.fingerprint,encoded(record)?,op.created_at,op.updated_at]).map_err(database_error("save application data operation"))?;
    Ok(())
}
pub(crate) fn read(
    db: &Connection,
    profile: &str,
    id: &str,
) -> Result<PersonalApplicationDataRecord, PersistenceError> {
    let raw:Option<String>=db.query_row("SELECT record_json FROM personal_application_data_operations WHERE profile_id=?1 AND operation_id=?2",params![profile,id],|r|r.get(0)).optional().map_err(database_error("read application data operation"))?;
    let record: PersonalApplicationDataRecord =
        decoded(&raw.ok_or_else(|| rejected("personal_application_data_operation_missing"))?)?;
    validate_slot(
        &record.target_slot,
        profile,
        &record.operation.creation_project_id,
    )?;
    if let Some(source) = &record.source_slot {
        validate_slot(source, profile, &record.operation.creation_project_id)?;
    }
    if record.operation.profile_id != profile
        || record.operation.operation_id != id
        || record.operation.target_slot_id != record.target_slot.slot_id
        || record.operation.source_slot_id != record.source_slot.as_ref().map(|s| s.slot_id.clone())
        || record.operation.package_digest != record.target_slot.package_digest
        || record.operation.creation_project_id != record.request.creation_project_id
        || record.operation.client_operation_id != record.request.client_operation_id
        || record.operation.expected_revision != record.request.expected_revision
        || record.operation.package_digest != record.request.package_digest
        || record.operation.action != record.request.action
    {
        return Err(rejected("personal_application_data_record_invalid"));
    }
    Ok(record)
}
type Previous = (
    Option<CapabilityCreationProjectRecord>,
    Option<PersonalCapabilityInstallationRecord>,
    Option<PersonalApplicationDataSlot>,
);
fn record_for(
    profile: &str,
    p: &Prepare,
    previous: Previous,
    retained: Option<PersonalApplicationDataRecord>,
    fingerprint: String,
    now: i64,
) -> Result<PersonalApplicationDataRecord, PersistenceError> {
    let (project, installation, source) = previous;
    let target = if let Some(r) = &retained {
        r.source_slot
            .clone()
            .ok_or_else(|| rejected("personal_application_data_rollback_source_invalid"))?
    } else {
        let id = uuid::Uuid::new_v4().simple().to_string();
        PersonalApplicationDataSlot {
            profile_id: profile.into(),
            creation_project_id: p.creation_project_id.clone(),
            relative_path: format!("personal-capability-slots/{}/{id}", p.creation_project_id),
            slot_id: id,
            package_digest: p.package_digest.clone(),
            created_at: now,
        }
    };
    let mut operation = Operation {
        read_only_after_restore: false,
        source_profile_id: None,
        operation_id: format!("application_data_{}", uuid::Uuid::new_v4().simple()),
        profile_id: profile.into(),
        creation_project_id: p.creation_project_id.clone(),
        client_operation_id: p.client_operation_id.clone(),
        expected_revision: p.expected_revision,
        package_digest: p.package_digest.clone(),
        source_package_digest: installation.as_ref().map(|i| i.package_digest.clone()),
        source_slot_id: source.as_ref().map(|s| s.slot_id.clone()),
        target_slot_id: target.slot_id.clone(),
        action: p.action.clone(),
        state: State::Prepared,
        preview_digest: String::new(),
        snapshot_digest: None,
        reason: None,
        activated_revision: None,
        created_at: now,
        updated_at: now,
    };
    operation.preview_digest = hash(&(&operation, &project, &installation, &source, &target))?;
    Ok(PersonalApplicationDataRecord {
        operation,
        request: p.clone(),
        fingerprint,
        source_slot: source,
        target_slot: target,
        previous_project: project,
        previous_installation: installation,
        rollback_project: retained.as_ref().and_then(|r| r.previous_project.clone()),
        rollback_installation: retained
            .as_ref()
            .and_then(|r| r.previous_installation.clone()),
        expected_retained_digest: retained.and_then(|r| r.operation.snapshot_digest),
    })
}
impl SqlitePersistence {
    /// # Errors
    /// 维护期间禁止新建运行资料或准备升级，不能绕过全局写者准入。
    pub fn check_personal_application_data_admission(
        &mut self,
        profile: &str,
    ) -> Result<(), PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("check application data admission"))?;
        admission(&tx, profile)?;
        tx.commit()
            .map_err(database_error("finish application data admission"))
    }
    /// # Errors
    /// 元数据缺失、归属改变或未知数据代次时拒绝猜测数据目录。
    pub fn personal_application_data_slot(
        &self,
        profile: &str,
        project: &str,
    ) -> Result<Option<PersonalApplicationDataSlot>, PersistenceError> {
        active(&self.connection, profile, project)
    }
    /// # Errors
    /// 只读取持久回执，不重做文件复制或安装。
    pub fn personal_application_data_operation(
        &self,
        profile: &str,
        id: &str,
    ) -> Result<PersonalApplicationDataRecord, PersistenceError> {
        read(&self.connection, profile, id)
    }
    /// # Errors
    /// 当前资料或操作记录不可读时返回错误。
    pub fn personal_application_data_operations(
        &self,
        profile: &str,
    ) -> Result<Vec<PersonalApplicationDataRecord>, PersistenceError> {
        let mut statement=self.connection.prepare("SELECT operation_id FROM personal_application_data_operations WHERE profile_id=?1 ORDER BY created_at DESC,operation_id").map_err(database_error("list application data operations"))?;
        let ids = statement
            .query_map([profile], |row| row.get::<_, String>(0))
            .map_err(database_error("query application data operations"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read application data operation ids"))?;
        ids.iter()
            .map(|id| read(&self.connection, profile, id))
            .collect()
    }
    /// # Errors
    /// 旧个人版本的既有数据仅登记原路径，不移动或改写任何文件。
    pub fn register_personal_application_legacy_data(
        &mut self,
        profile: &str,
        project: &str,
        digest: &str,
        now: i64,
    ) -> Result<PersonalApplicationDataSlot, PersistenceError> {
        if let Some(current) = active(&self.connection, profile, project)? {
            return Ok(current);
        }
        let record = PersonalApplicationDataSlot {
            profile_id: profile.into(),
            creation_project_id: project.into(),
            slot_id: uuid::Uuid::new_v4().simple().to_string(),
            package_digest: digest.into(),
            relative_path: format!("personal-capability-data/{project}"),
            created_at: now,
        };
        let tx = self
            .connection
            .transaction()
            .map_err(database_error("register legacy application data"))?;
        admission(&tx, profile)?;
        save_slot(&tx, &record)?;
        tx.execute("INSERT INTO personal_application_data_pointers(profile_id,creation_project_id,slot_id) VALUES(?1,?2,?3)",params![profile,project,record.slot_id]).map_err(database_error("bind legacy application data"))?;
        tx.commit().map_err(database_error(
            "commit legacy application data registration",
        ))?;
        Ok(record)
    }
    /// # Errors
    /// 请求、当前修订或回退来源改变时拒绝；相同准备仅返回已有操作。
    pub fn prepare_personal_application_data(
        &mut self,
        profile: &str,
        p: &Prepare,
        now: i64,
    ) -> Result<PersonalApplicationDataRecord, PersistenceError> {
        let fingerprint = hash(p)?;
        let old:Option<String>=self.connection.query_row("SELECT operation_id FROM personal_application_data_operations WHERE profile_id=?1 AND client_operation_id=?2",params![profile,p.client_operation_id],|r|r.get(0)).optional().map_err(database_error("read application data preparation"))?;
        if let Some(id) = old {
            let r = read(&self.connection, profile, &id)?;
            return if r.fingerprint == fingerprint {
                Ok(r)
            } else {
                Err(rejected("operation_id_reused"))
            };
        }
        let project = self.capability_creation_project(&p.creation_project_id)?;
        let installation = self.personal_capability_installation(&p.creation_project_id)?;
        if project
            .as_ref()
            .is_some_and(|v| v.state_revision != p.expected_revision)
            || project.is_none() && p.action != Action::ImportUpgrade
        {
            return Err(rejected("personal_capability_state_conflict"));
        }
        let source = active(&self.connection, profile, &p.creation_project_id)?;
        if installation.as_ref().is_some_and(|i| {
            source
                .as_ref()
                .is_none_or(|s| s.package_digest != i.package_digest)
        }) {
            return Err(rejected("personal_application_data_slot_missing"));
        }
        let retained = if p.action == Action::Rollback {
            let r = read(
                &self.connection,
                profile,
                p.target_operation_id.as_deref().ok_or_else(|| {
                    rejected("personal_application_data_rollback_source_required")
                })?,
            )?;
            if r.operation.read_only_after_restore
                || r.operation.state != State::Completed
                || r.operation.creation_project_id != p.creation_project_id
                || r.previous_installation
                    .as_ref()
                    .is_none_or(|i| i.package_digest != p.package_digest)
            {
                return Err(rejected(
                    "personal_application_data_rollback_source_invalid",
                ));
            }
            Some(r)
        } else {
            if p.target_operation_id.is_some() {
                return Err(rejected("personal_application_data_request_invalid"));
            }
            None
        };
        let record = record_for(
            profile,
            p,
            (project, installation, source),
            retained,
            fingerprint,
            now,
        )?;
        let tx = self
            .connection
            .transaction()
            .map_err(database_error("begin application data preparation"))?;
        admission(&tx, profile)?;
        let revision: Option<i64> = tx.query_row("SELECT state_revision FROM capability_creation_projects WHERE creation_project_id=?1", [&p.creation_project_id], |r|r.get(0)).optional().map_err(database_error("recheck application data target"))?;
        if revision != record.previous_project.as_ref().map(|v| v.state_revision)
            || active(&tx, profile, &p.creation_project_id)? != record.source_slot
        {
            return Err(rejected("personal_capability_state_conflict"));
        }
        save_operation(&tx, &record)?;
        tx.commit()
            .map_err(database_error("commit application data preparation"))?;
        Ok(record)
    }
    /// # Errors
    /// 只允许明确状态推进，未知中断不自动回到 prepared。
    pub fn transition_personal_application_data(
        &mut self,
        record: &PersonalApplicationDataRecord,
        expected: &State,
    ) -> Result<(), PersistenceError> {
        let tx = self
            .connection
            .transaction()
            .map_err(database_error("begin application data transition"))?;
        let old = read(
            &tx,
            &record.operation.profile_id,
            &record.operation.operation_id,
        )?;
        if old.operation.read_only_after_restore
            || old.operation.state != *expected
            || old.fingerprint != record.fingerprint
            || old.operation.preview_digest != record.operation.preview_digest
        {
            return Err(rejected("personal_application_data_state_conflict"));
        }
        if !matches!(
            (expected, &record.operation.state),
            (
                State::Prepared,
                State::Applying | State::Cancelled | State::Incomplete
            ) | (State::Applying, State::Applying | State::Incomplete)
                | (State::Incomplete, State::Cancelled)
        ) {
            return Err(rejected("personal_application_data_state_conflict"));
        }
        save_operation(&tx, record)?;
        tx.commit()
            .map_err(database_error("commit application data transition"))
    }
    /// # Errors
    /// 只在原安装和指针仍一致且旧拥有者已退出时结束中断状态，不重做任何文件操作。
    pub fn resolve_interrupted_application_data(
        &mut self,
        profile: &str,
        id: &str,
        preview: &str,
        now: i64,
    ) -> Result<PersonalApplicationDataRecord, PersistenceError> {
        let mut r = read(&self.connection, profile, id)?;
        if r.operation.read_only_after_restore {
            return Err(rejected("personal_application_data_restored_read_only"));
        }
        if r.operation.preview_digest != preview {
            return Err(rejected("personal_application_data_preview_changed"));
        }
        if r.operation.state != State::Applying {
            return Ok(r);
        }
        self.verify_interrupted_personal_data(profile, id)?;
        let project = self.capability_creation_project(&r.operation.creation_project_id)?;
        let installation =
            self.personal_capability_installation(&r.operation.creation_project_id)?;
        if project != r.previous_project
            || installation != r.previous_installation
            || active(&self.connection, profile, &r.operation.creation_project_id)? != r.source_slot
        {
            return Err(rejected("personal_application_data_binding_unconfirmed"));
        }
        r.operation.state = State::Incomplete;
        r.operation.reason = Some("personal_application_data_interrupted_original_retained".into());
        r.operation.updated_at = now;
        self.transition_personal_application_data(&r, &State::Applying)?;
        Ok(r)
    }
    /// # Errors
    /// 回退仍须已有受控复制证明和未改变的当前安装；同一事务切换包与数据指针。
    pub fn rollback_personal_application_data(
        &mut self,
        profile: &str,
        id: &str,
        now: i64,
    ) -> Result<(), PersistenceError> {
        let r = read(&self.connection, profile, id)?;
        if r.operation.read_only_after_restore
            || r.operation.action != Action::Rollback
            || r.operation.state != State::Applying
        {
            return Err(rejected("personal_application_data_activation_invalid"));
        }
        let project = r
            .rollback_project
            .as_ref()
            .ok_or_else(|| rejected("personal_application_data_rollback_source_invalid"))?;
        let install = r
            .rollback_installation
            .as_ref()
            .ok_or_else(|| rejected("personal_application_data_rollback_source_invalid"))?;
        let tx = self
            .connection
            .transaction()
            .map_err(database_error("begin application rollback"))?;
        let changed = tx.execute("UPDATE capability_creation_projects SET title=?3,business_goal=?4,target_type=?5,status=?6,conversation_id=?7,managed_relative_path=?8,requested_permissions_json=?9,acceptance_examples_json=?10,source_digest=?11,source_file_count=?12,package_digest=?13,error_reason=?14,state_revision=state_revision+1,updated_at=?15 WHERE creation_project_id=?1 AND state_revision=?2", params![r.operation.creation_project_id,r.operation.expected_revision,project.title,project.business_goal,project.target_type,project.status,project.conversation_id,project.managed_relative_path,project.requested_permissions_json,project.acceptance_examples_json,project.source_digest,project.source_file_count,project.package_digest,project.error_reason,now]).map_err(database_error("restore previous application record"))?;
        if changed != 1 {
            return Err(rejected("personal_capability_state_conflict"));
        }
        let changed = tx.execute("UPDATE personal_capability_installations SET package_digest=?2,installed_relative_path=?3,enabled=?4,state_revision=state_revision+1,updated_at=?5 WHERE creation_project_id=?1", params![r.operation.creation_project_id,install.package_digest,install.installed_relative_path,install.enabled,now]).map_err(database_error("restore previous application installation"))?;
        if changed != 1 {
            return Err(rejected("personal_capability_state_conflict"));
        }
        activate(
            &tx,
            profile,
            id,
            &r.operation.creation_project_id,
            &r.operation.package_digest,
            r.operation.expected_revision + 1,
            now,
        )?;
        tx.commit()
            .map_err(database_error("commit application rollback"))
    }
    /// # Errors
    /// 读取当前受信服务代次，拒绝外部传入的维护身份。
    pub fn verify_application_data_quiescence(
        &mut self,
        profile: &str,
        operation: &str,
        proof: &contexthub_protocol::PersonalDataQuiescence,
    ) -> Result<(), PersistenceError> {
        let instance:String=self.connection.query_row("SELECT instance_fence FROM personal_profile_compatibility WHERE profile_id=?1 AND state='ready'",[profile],|r|r.get(0)).map_err(database_error("read application data service fence"))?;
        self.verify_personal_data_quiescence(profile, operation, &instance, proof)
    }
}

/// 安装/包导入事务内部调用；指针切换不能在安装事务之外先行提交。
pub(crate) fn activate(
    tx: &Transaction<'_>,
    profile: &str,
    operation: &str,
    project: &str,
    digest: &str,
    revision: i64,
    now: i64,
) -> Result<(), PersistenceError> {
    let mut r = read(tx, profile, operation)?;
    if r.operation.read_only_after_restore
        || r.operation.state != State::Applying
        || r.operation.creation_project_id != project
        || r.operation.package_digest != digest
        || r.operation.expected_revision + 1 != revision
        || r.operation.snapshot_digest.is_none()
    {
        return Err(rejected("personal_application_data_activation_invalid"));
    }
    let current = active(tx, profile, project)?;
    if current.as_ref().map(|s| &s.slot_id) != r.source_slot.as_ref().map(|s| &s.slot_id) {
        return Err(rejected("personal_application_data_source_changed"));
    }
    if r.operation.action != Action::Rollback {
        save_slot(tx, &r.target_slot)?;
    }
    tx.execute("INSERT INTO personal_application_data_pointers(profile_id,creation_project_id,slot_id) VALUES(?1,?2,?3) ON CONFLICT(profile_id,creation_project_id) DO UPDATE SET slot_id=excluded.slot_id",params![profile,project,r.target_slot.slot_id]).map_err(database_error("activate application data pointer"))?;
    r.operation.state = State::Completed;
    r.operation.activated_revision = Some(revision);
    r.operation.updated_at = now;
    save_operation(tx, &r)
}
