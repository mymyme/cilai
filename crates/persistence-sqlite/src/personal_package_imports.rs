use super::{
    CapabilityCreationProjectRecord, PersistenceError, PersistenceErrorKind,
    PersonalCapabilityVersion, SqlitePersistence, database_error,
};
use contexthub_protocol::{
    PersonalPackageImportPreview, PersonalPackageImportState, PersonalPackageImportTarget,
};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct PersonalPackageImportRecord {
    pub preview: PersonalPackageImportPreview,
    pub prepare_operation_id: String,
    pub prepare_fingerprint: String,
    pub candidate_relative_path: String,
    pub commit_operation_id: Option<String>,
    pub commit_fingerprint: Option<String>,
    pub result: Option<Value>,
}

pub struct PersonalPackageImportCommand<'a> {
    pub data_operation_id: &'a str,
    pub profile_id: &'a str,
    pub import_id: &'a str,
    pub preview_digest: &'a str,
    pub client_operation_id: &'a str,
    pub fingerprint: &'a str,
    pub version: &'a PersonalCapabilityVersion,
    pub installed_relative_path: &'a str,
    pub result: &'a Value,
    pub occurred_at: i64,
}

fn rejected(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}

fn row_record(row: &Row<'_>) -> rusqlite::Result<PersonalPackageImportRecord> {
    let raw: String = row.get(0)?;
    let mut preview: PersonalPackageImportPreview =
        serde_json::from_str(&raw).map_err(|_| rusqlite::Error::InvalidQuery)?;
    preview.state = match row.get::<_, String>(1)?.as_str() {
        "prepared" => PersonalPackageImportState::Prepared,
        "committed" => PersonalPackageImportState::Committed,
        "cancelled" => PersonalPackageImportState::Cancelled,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    preview.updated_at = row.get(2)?;
    let result: Option<String> = row.get(8)?;
    Ok(PersonalPackageImportRecord {
        preview,
        prepare_operation_id: row.get(3)?,
        prepare_fingerprint: row.get(4)?,
        candidate_relative_path: row.get(5)?,
        commit_operation_id: row.get(6)?,
        commit_fingerprint: row.get(7)?,
        result: result
            .map(|raw| serde_json::from_str(&raw))
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
    })
}

fn record(
    db: &Connection,
    profile: &str,
    id: &str,
) -> Result<Option<PersonalPackageImportRecord>, PersistenceError> {
    db.query_row("SELECT preview_json,state,updated_at,prepare_operation_id,prepare_fingerprint,candidate_relative_path,commit_operation_id,commit_fingerprint,result_json FROM personal_package_imports WHERE profile_id=?1 AND import_id=?2",params![profile,id],row_record).optional().map_err(database_error("read personal package import"))
}

fn operation_record(
    db: &Connection,
    profile: &str,
    operation: &str,
) -> Result<Option<PersonalPackageImportRecord>, PersistenceError> {
    db.query_row("SELECT preview_json,state,updated_at,prepare_operation_id,prepare_fingerprint,candidate_relative_path,commit_operation_id,commit_fingerprint,result_json FROM personal_package_imports WHERE profile_id=?1 AND prepare_operation_id=?2",params![profile,operation],row_record).optional().map_err(database_error("read package prepare operation"))
}

fn ready(db: &Connection, profile: &str) -> Result<(), PersistenceError> {
    let ready: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM personal_profile_compatibility WHERE profile_id=?1 AND state='ready' AND data_version=2)",[profile],|r|r.get(0)).map_err(database_error("read import profile readiness"))?;
    if ready {
        Ok(())
    } else {
        Err(rejected("personal_package_import_profile_not_ready"))
    }
}

fn check_target(
    db: &Connection,
    preview: &PersonalPackageImportPreview,
) -> Result<(), PersistenceError> {
    match &preview.target {
        PersonalPackageImportTarget::New {} => {
            let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM capability_creation_projects WHERE creation_project_id=?1)",[&preview.local_creation_project_id],|r|r.get(0)).map_err(database_error("check new imported identity"))?;
            if exists {
                return Err(rejected("personal_package_import_target_changed"));
            }
        }
        PersonalPackageImportTarget::Upgrade {
            creation_project_id,
            expected_revision,
        } => {
            let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM capability_creation_projects p JOIN personal_capability_installations i ON i.creation_project_id=p.creation_project_id WHERE p.creation_project_id=?1 AND p.state_revision=?2 AND p.status='installed' AND p.target_type=?3 AND i.package_digest=?4 AND NOT EXISTS(SELECT 1 FROM enterprise_capability_bindings e WHERE e.creation_project_id=p.creation_project_id))",params![creation_project_id,expected_revision,preview.source.target_type,preview.installed_digest],|r|r.get(0)).map_err(database_error("check imported upgrade target"))?;
            if !valid || creation_project_id != &preview.local_creation_project_id {
                return Err(rejected("personal_package_import_target_changed"));
            }
        }
    }
    Ok(())
}

impl SqlitePersistence {
    /// # Errors
    /// Returns an error if the current profile is not ready or the ledger cannot be read.
    pub fn personal_package_import(
        &self,
        profile: &str,
        id: &str,
    ) -> Result<Option<PersonalPackageImportRecord>, PersistenceError> {
        ready(&self.connection, profile)?;
        record(&self.connection, profile, id)
    }

    /// # Errors
    /// Returns an error when a preparation operation cannot be read.
    pub fn personal_package_import_by_operation(
        &self,
        profile: &str,
        operation: &str,
    ) -> Result<Option<PersonalPackageImportRecord>, PersistenceError> {
        ready(&self.connection, profile)?;
        operation_record(&self.connection, profile, operation)
    }

    /// # Errors
    /// Returns an error when pending and completed import records cannot be read.
    pub fn personal_package_imports(
        &self,
        profile: &str,
    ) -> Result<Vec<PersonalPackageImportRecord>, PersistenceError> {
        ready(&self.connection, profile)?;
        let mut statement=self.connection.prepare("SELECT preview_json,state,updated_at,prepare_operation_id,prepare_fingerprint,candidate_relative_path,commit_operation_id,commit_fingerprint,result_json FROM personal_package_imports WHERE profile_id=?1 ORDER BY prepared_at DESC,import_id").map_err(database_error("list package imports"))?;
        statement
            .query_map([profile], row_record)
            .map_err(database_error("query package imports"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read package imports"))
    }

    /// # Errors
    /// Refuses a changed operation shape or target and never changes an existing installation.
    pub fn prepare_personal_package_import(
        &mut self,
        record: &PersonalPackageImportRecord,
    ) -> Result<PersonalPackageImportRecord, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("begin package import preparation"))?;
        ready(&tx, &record.preview.profile_id)?;
        if let Some(old) = operation_record(
            &tx,
            &record.preview.profile_id,
            &record.prepare_operation_id,
        )? {
            if old.prepare_fingerprint != record.prepare_fingerprint {
                return Err(rejected("operation_id_reused"));
            }
            return Ok(old);
        }
        check_target(&tx, &record.preview)?;
        tx.execute("INSERT INTO personal_package_imports(import_id,profile_id,prepare_operation_id,prepare_fingerprint,preview_digest,preview_json,candidate_relative_path,state,prepared_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'prepared',?8,?8)",params![record.preview.import_id,record.preview.profile_id,record.prepare_operation_id,record.prepare_fingerprint,record.preview.preview_digest,serde_json::to_string(&record.preview).map_err(|_|rejected("personal_package_import_record_invalid"))?,record.candidate_relative_path,record.preview.prepared_at]).map_err(database_error("save package import preparation"))?;
        tx.commit()
            .map_err(database_error("commit package import preparation"))?;
        Ok(record.clone())
    }

    /// # Errors
    /// Refuses a committed import, a different preview, or an unavailable profile.
    pub fn cancel_personal_package_import(
        &mut self,
        profile: &str,
        id: &str,
        digest: &str,
        now: i64,
    ) -> Result<PersonalPackageImportRecord, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("begin package import cancellation"))?;
        ready(&tx, profile)?;
        let record = record(&tx, profile, id)?
            .ok_or_else(|| rejected("personal_package_import_not_found"))?;
        if record.preview.preview_digest != digest {
            return Err(rejected("personal_package_import_preview_changed"));
        }
        if record.preview.state == PersonalPackageImportState::Committed {
            return Err(rejected("personal_package_import_already_committed"));
        }
        tx.execute("UPDATE personal_package_imports SET state='cancelled',updated_at=?3 WHERE profile_id=?1 AND import_id=?2 AND state='prepared'",params![profile,id,now]).map_err(database_error("cancel package import"))?;
        tx.commit()
            .map_err(database_error("commit package import cancellation"))?;
        self.personal_package_import(profile, id)?
            .ok_or_else(|| rejected("personal_package_import_not_found"))
    }

    /// # Errors
    /// Refuses stale targets, cancelled operations, or conflicting commit receipts atomically.
    pub fn commit_personal_package_import(
        &mut self,
        command: &PersonalPackageImportCommand<'_>,
    ) -> Result<bool, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error("begin package import commit"))?;
        ready(&tx, command.profile_id)?;
        let prepared = record(&tx, command.profile_id, command.import_id)?
            .ok_or_else(|| rejected("personal_package_import_not_found"))?;
        if prepared.preview.preview_digest != command.preview_digest {
            return Err(rejected("personal_package_import_preview_changed"));
        }
        match prepared.preview.state {
            PersonalPackageImportState::Cancelled => {
                return Err(rejected("personal_package_import_cancelled"));
            }
            PersonalPackageImportState::Committed => {
                return if prepared.commit_operation_id.as_deref()
                    == Some(command.client_operation_id)
                    && prepared.commit_fingerprint.as_deref() == Some(command.fingerprint)
                {
                    Ok(true)
                } else {
                    Err(rejected("personal_package_import_already_committed"))
                };
            }
            PersonalPackageImportState::Prepared => (),
        }
        check_target(&tx, &prepared.preview)?;
        check_candidate(&prepared.preview, command)?;
        let project = &command.version.project;
        let expected = prepared.preview.expected_revision.unwrap_or(1);
        if matches!(prepared.preview.target, PersonalPackageImportTarget::New {}) {
            insert_new_project(&tx, project)?;
            insert_initial_build(&tx, &command.version.build)?;
        } else {
            tx.execute("INSERT INTO personal_capability_versions(creation_project_id,package_digest,version,project_json,build_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![project.creation_project_id,prepared.preview.local_package_digest,command.version.version,serde_json::to_string(project).map_err(|_|rejected("personal_package_import_record_invalid"))?,serde_json::to_string(&command.version.build).map_err(|_|rejected("personal_package_import_record_invalid"))?,command.occurred_at]).map_err(database_error("save imported version"))?;
        }
        activate_import(&tx, command, expected)?;
        crate::personal_application_data::activate(
            &tx,
            command.profile_id,
            command.data_operation_id,
            &project.creation_project_id,
            &command.version.build.package_digest,
            expected + 1,
            command.occurred_at,
        )?;
        tx.execute("UPDATE personal_package_imports SET state='committed',commit_operation_id=?3,commit_fingerprint=?4,result_json=?5,updated_at=?6 WHERE profile_id=?1 AND import_id=?2 AND state='prepared'",params![command.profile_id,command.import_id,command.client_operation_id,command.fingerprint,command.result.to_string(),command.occurred_at]).map_err(database_error("record package import commit"))?;
        tx.commit()
            .map_err(database_error("commit package import installation"))?;
        Ok(false)
    }
}

fn check_candidate(
    preview: &PersonalPackageImportPreview,
    c: &PersonalPackageImportCommand<'_>,
) -> Result<(), PersistenceError> {
    let p = &c.version.project;
    let permissions: Vec<String> = serde_json::from_str(&p.requested_permissions_json)
        .map_err(|_| rejected("personal_package_import_candidate_changed"))?;
    if p.creation_project_id != preview.local_creation_project_id
        || p.package_digest.as_deref() != Some(preview.local_package_digest.as_str())
        || p.source_digest.as_deref() != Some(preview.source.source_digest.as_str())
        || p.target_type != preview.source.target_type
        || permissions != preview.requested_permissions
        || c.version.version != preview.local_version
        || c.version.build.package_digest != preview.local_package_digest
        || c.version.build.creation_project_id != preview.local_creation_project_id
        || c.version.build.source_digest != preview.source.source_digest
        || p.state_revision != preview.expected_revision.unwrap_or(1) + 1
    {
        return Err(rejected("personal_package_import_candidate_changed"));
    }
    Ok(())
}

fn insert_new_project(
    tx: &Connection,
    p: &CapabilityCreationProjectRecord,
) -> Result<(), PersistenceError> {
    tx.execute("INSERT INTO capability_creation_projects(creation_project_id,title,business_goal,target_type,status,conversation_id,managed_relative_path,requested_permissions_json,acceptance_examples_json,source_digest,source_file_count,state_revision,package_digest,created_at,updated_at) VALUES(?1,?2,?3,?4,'packaged',NULL,?5,?6,'[]',?7,?8,1,?9,?10,?10)",params![p.creation_project_id,p.title,p.business_goal,p.target_type,p.managed_relative_path,p.requested_permissions_json,p.source_digest,p.source_file_count,p.package_digest,p.created_at]).map_err(database_error("register imported personal project"))?;
    Ok(())
}

fn insert_initial_build(
    tx: &Connection,
    b: &super::CapabilityCreationBuildRecord,
) -> Result<(), PersistenceError> {
    tx.execute("INSERT INTO capability_creation_builds(build_id,creation_project_id,source_digest,package_digest,package_relative_path,file_count,total_bytes,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![b.build_id,b.creation_project_id,b.source_digest,b.package_digest,b.package_relative_path,b.file_count,b.total_bytes,b.created_at]).map_err(database_error("register imported initial build"))?;
    Ok(())
}

fn activate_import(
    tx: &Connection,
    c: &PersonalPackageImportCommand<'_>,
    expected: i64,
) -> Result<(), PersistenceError> {
    let p = &c.version.project;
    let changed=tx.execute("UPDATE capability_creation_projects SET title=?3,business_goal=?4,status='installed',requested_permissions_json=?5,source_digest=?6,source_file_count=?7,package_digest=?8,state_revision=state_revision+1,updated_at=?9,error_reason=NULL WHERE creation_project_id=?1 AND state_revision=?2",params![p.creation_project_id,expected,p.title,p.business_goal,p.requested_permissions_json,p.source_digest,p.source_file_count,p.package_digest,c.occurred_at]).map_err(database_error("activate imported project"))?;
    if changed != 1 {
        return Err(rejected("personal_package_import_target_changed"));
    }
    tx.execute("INSERT INTO personal_capability_installations(creation_project_id,package_digest,installed_relative_path,enabled,state_revision,installed_at,updated_at) VALUES(?1,?2,?3,1,1,?4,?4) ON CONFLICT(creation_project_id) DO UPDATE SET package_digest=excluded.package_digest,installed_relative_path=excluded.installed_relative_path,state_revision=personal_capability_installations.state_revision+1,updated_at=excluded.updated_at",params![p.creation_project_id,p.package_digest,c.installed_relative_path,c.occurred_at]).map_err(database_error("activate imported installation"))?;
    Ok(())
}
