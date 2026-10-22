use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{
    PersonalPageAuditEvent, PersonalPageAuditFinishParams, PersonalPageAuditListParams,
    PersonalPageAuditListResult, PersonalPageAuditPrepareParams, PersonalPageAuditRecord,
    PersonalPageBindingSnapshot,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

fn error(code: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, code, code)
}
fn opaque(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 160
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|v| {
        v.len() == 64
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn operation(id: &str) -> bool {
    uuid::Uuid::parse_str(id)
        .is_ok_and(|value| value.get_version_num() == 4 && value.simple().to_string() == id)
}
fn reason(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 100
        && code
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn validate(p: &PersonalPageAuditPrepareParams) -> Result<(), PersistenceError> {
    if !operation(&p.operation_id)
        || !opaque(&p.document_id)
        || !opaque(&p.request_id)
        || p.session_id.as_ref().is_some_and(|s| !opaque(s))
        || p.run_id.as_ref().is_some_and(|s| !opaque(s))
        || !digest(&p.arguments_digest)
        || !digest(&p.permission_digest)
        || !digest(&p.binding.expected_package_digest)
        || !p
            .binding
            .creation_project_id
            .strip_prefix("capability_")
            .is_some_and(|s| {
                s.len() == 32
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
    {
        return Err(error("personal_page_audit_request_invalid"));
    }
    Ok(())
}
fn event_id(id: &str, finished: bool) -> String {
    format!(
        "personal-page-audit:{id}:{}",
        if finished { "finished" } else { "prepared" }
    )
}
fn decoded(raw: &str) -> Result<PersonalPageAuditRecord, PersistenceError> {
    let record: PersonalPageAuditRecord =
        serde_json::from_str(raw).map_err(|_| error("personal_page_audit_record_invalid"))?;
    validate(&record.request)?;
    if record.profile_id.len() != 32
        || !record
            .profile_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || record.read_only_after_restore
        || record.prepared_at < 0
        || record.finished_at.is_some_and(|time| time < 0)
        || record
            .reason_code
            .as_ref()
            .is_some_and(|code| !reason(code))
        || record.outcome.is_some() != record.finished_at.is_some()
        || (record.outcome.is_none() && record.reason_code.is_some())
    {
        return Err(error("personal_page_audit_record_invalid"));
    }
    Ok(record)
}
fn read(
    db: &Connection,
    id: &str,
    finished: bool,
) -> Result<Option<PersonalPageAuditRecord>, PersistenceError> {
    let raw: Option<(String,String)> = db.query_row("SELECT object_id,payload_json FROM durable_events WHERE event_id=?1 AND object_type='personal_page_audit'", [event_id(id, finished)], |row| Ok((row.get(0)?,row.get(1)?))).optional().map_err(database_error("read personal page audit"))?;
    raw.map(|(project, raw)| {
        let record = decoded(&raw)?;
        if record.request.operation_id != id
            || record.request.binding.creation_project_id != project
            || record.outcome.is_some() != finished
        {
            return Err(error("personal_page_audit_record_invalid"));
        }
        Ok(record)
    })
    .transpose()
}
fn append(
    tx: &Transaction<'_>,
    record: &PersonalPageAuditRecord,
    now: i64,
) -> Result<(), PersistenceError> {
    let payload =
        serde_json::to_string(record).map_err(|_| error("personal_page_audit_record_invalid"))?;
    let finished = record.outcome.is_some();
    tx.execute("INSERT INTO durable_events(event_id,event_type,object_type,object_id,payload_json,correlation_id,occurred_at,created_at) VALUES(?1,?2,'personal_page_audit',?3,?4,?5,?6,?6)",params![event_id(&record.request.operation_id,finished),if finished {"personal_page.audit_finished"}else{"personal_page.audit_prepared"},record.request.binding.creation_project_id,payload,record.request.operation_id,now]).map_err(database_error("append personal page audit"))?;
    Ok(())
}
fn admission(tx: &Transaction<'_>, profile: &str) -> Result<(), PersistenceError> {
    let instance: String = tx.query_row("SELECT instance_fence FROM personal_profile_compatibility WHERE profile_id=?1 AND state='ready'",[profile],|r|r.get(0)).map_err(database_error("read page audit profile fence"))?;
    crate::profile_writers::require_writer_admission(tx, profile, &instance)
}
fn validate_binding(
    tx: &Transaction<'_>,
    b: &PersonalPageBindingSnapshot,
) -> Result<(), PersistenceError> {
    let facts: (i64,String,String,i64,String,bool,String,String) = tx.query_row(
        "SELECT p.state_revision,p.requested_permissions_json,p.package_digest,i.state_revision,i.package_digest,i.enabled,p.status,d.slot_id FROM capability_creation_projects p JOIN personal_capability_installations i USING(creation_project_id) JOIN personal_application_data_pointers d ON d.creation_project_id=p.creation_project_id WHERE p.creation_project_id=?1 AND d.profile_id=?2",
        params![b.creation_project_id,b.profile_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))
    ).map_err(database_error("check page audit installation"))?;
    let permissions: Vec<String> =
        serde_json::from_str(&facts.1).map_err(|_| error("personal_page_audit_record_invalid"))?;
    if facts.0 != b.project_revision
        || permissions != b.permissions
        || facts.2 != b.package_digest
        || facts.3 != b.installation_revision
        || facts.4 != b.package_digest
        || !facts.5
        || facts.6 != "installed"
        || facts.7 != b.active_data_slot_id
    {
        return Err(error("personal_page_binding_changed"));
    }
    Ok(())
}

impl SqlitePersistence {
    /// # Errors
    /// 资料维护、绑定变化、标识非法或幂等冲突时拒绝新的页面操作。
    pub fn prepare_personal_page_audit(
        &mut self,
        profile: &str,
        p: &PersonalPageAuditPrepareParams,
        binding: &PersonalPageBindingSnapshot,
        now: i64,
    ) -> Result<PersonalPageAuditRecord, PersistenceError> {
        validate(p)?;
        if binding.profile_id != profile
            || binding.creation_project_id != p.binding.creation_project_id
            || binding.package_digest != p.binding.expected_package_digest
            || binding.permission_digest != p.permission_digest
        {
            return Err(error("personal_page_binding_changed"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin personal page audit"))?;
        admission(&tx, profile)?;
        if let Some(record) =
            read(&tx, &p.operation_id, true)?.or(read(&tx, &p.operation_id, false)?)
        {
            if record.profile_id != profile {
                return Err(error("personal_page_audit_restored_read_only"));
            }
            if record.request != *p {
                return Err(error("personal_page_audit_operation_reused"));
            }
            return Ok(record);
        }
        validate_binding(&tx, binding)?;
        let record = PersonalPageAuditRecord {
            profile_id: profile.into(),
            request: p.clone(),
            prepared_at: now,
            finished_at: None,
            outcome: None,
            reason_code: None,
            read_only_after_restore: false,
        };
        append(&tx, &record, now)?;
        tx.commit()
            .map_err(database_error("commit personal page audit"))?;
        Ok(record)
    }

    /// # Errors
    /// 缺少真实开始记录、资料来源变化或终态不一致时拒绝伪造或重写回执。
    pub fn finish_personal_page_audit(
        &mut self,
        profile: &str,
        p: &PersonalPageAuditFinishParams,
        now: i64,
    ) -> Result<PersonalPageAuditRecord, PersistenceError> {
        if !operation(&p.operation_id)
            || p.reason_code.as_ref().is_some_and(|s| !reason(s))
            || now < 0
        {
            return Err(error("personal_page_audit_request_invalid"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("finish personal page audit"))?;
        let mut record = read(&tx, &p.operation_id, false)?
            .ok_or_else(|| error("personal_page_audit_not_prepared"))?;
        if record.profile_id != profile {
            return Err(error("personal_page_audit_restored_read_only"));
        }
        if let Some(done) = read(&tx, &p.operation_id, true)? {
            if done.request != record.request
                || done.outcome.as_ref() != Some(&p.outcome)
                || done.reason_code != p.reason_code
            {
                return Err(error("personal_page_audit_already_finished"));
            }
            return Ok(done);
        }
        record.finished_at = Some(now);
        record.outcome = Some(p.outcome.clone());
        record.reason_code.clone_from(&p.reason_code);
        append(&tx, &record, now)?;
        tx.commit()
            .map_err(database_error("commit page audit result"))?;
        Ok(record)
    }

    /// # Errors
    /// 分页参数或历史记录损坏时拒绝返回不完整的虚构审计结果。
    pub fn personal_page_audits(
        &self,
        profile: &str,
        p: &PersonalPageAuditListParams,
    ) -> Result<PersonalPageAuditListResult, PersistenceError> {
        if !(1..=200).contains(&p.limit) || p.before_sequence.is_some_and(|s| s <= 0) {
            return Err(error("personal_page_audit_request_invalid"));
        }
        let mut statement = self.connection.prepare("SELECT global_sequence,payload_json FROM durable_events WHERE object_type='personal_page_audit' AND object_id=?1 AND (?2 IS NULL OR global_sequence<?2) ORDER BY global_sequence DESC LIMIT ?3").map_err(database_error("list personal page audit"))?;
        let rows = statement
            .query_map(
                params![
                    p.creation_project_id,
                    p.before_sequence,
                    i64::from(p.limit) + 1
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(database_error("query personal page audit"))?;
        let mut items = rows
            .map(|row| {
                let (sequence, raw) = row.map_err(database_error("read page audit entry"))?;
                let mut record = decoded(&raw)?;
                if record.request.binding.creation_project_id != p.creation_project_id {
                    return Err(error("personal_page_audit_record_invalid"));
                }
                record.read_only_after_restore = record.profile_id != profile;
                Ok(PersonalPageAuditEvent { sequence, record })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let more = items.len() > p.limit as usize;
        items.truncate(p.limit as usize);
        let next_before_sequence = more.then(|| items.last().map(|i| i.sequence)).flatten();
        Ok(PersonalPageAuditListResult {
            items,
            next_before_sequence,
        })
    }
}
