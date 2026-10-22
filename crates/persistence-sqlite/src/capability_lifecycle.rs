use super::{
    CapabilityCreationBuildRecord, CapabilityCreationProjectRecord, PersistenceError,
    PersistenceErrorKind, SqlitePersistence, database_error,
};
use rusqlite::{OptionalExtension, params};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalCapabilityVersion {
    pub version: String,
    pub project: CapabilityCreationProjectRecord,
    pub build: CapabilityCreationBuildRecord,
}

fn conflict() -> PersistenceError {
    PersistenceError::with_reason(
        PersistenceErrorKind::Conflict,
        "personal_capability_state_conflict",
        "personal capability version changed",
    )
}

impl SqlitePersistence {
    /// # Errors
    /// Returns an error when the operation id belongs to a different request.
    pub fn personal_lifecycle_operation(
        &self,
        operation: &str,
        fingerprint: &str,
    ) -> Result<Option<String>, PersistenceError> {
        let previous: Option<(String, String)> = self.connection.query_row(
            "SELECT fingerprint,package_digest FROM personal_capability_lifecycle_operations WHERE operation_id=?1",
            [operation], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(database_error("read personal lifecycle operation"))?;
        match previous {
            Some((expected, digest)) if expected == fingerprint => Ok(Some(digest)),
            Some(_) => Err(PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "operation_id_reused",
                "personal lifecycle request changed",
            )),
            None => Ok(None),
        }
    }

    /// # Errors
    /// Returns an error when stored version metadata cannot be read.
    pub fn personal_capability_versions(
        &self,
        project_id: &str,
    ) -> Result<Vec<PersonalCapabilityVersion>, PersistenceError> {
        let mut statement = self.connection.prepare(
            "SELECT version,project_json,build_json FROM personal_capability_versions WHERE creation_project_id=?1 ORDER BY rowid DESC"
        ).map_err(database_error("prepare personal versions"))?;
        let rows = statement
            .query_map([project_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error("query personal versions"))?;
        rows.map(|row| {
            let (version, project, build) = row.map_err(database_error("read personal version"))?;
            Ok(PersonalCapabilityVersion {
                version,
                project: serde_json::from_str(&project).map_err(|_| conflict())?,
                build: serde_json::from_str(&build).map_err(|_| conflict())?,
            })
        })
        .collect()
    }

    /// # Errors
    /// Returns an error when the project changed or the candidate cannot be saved.
    pub fn save_personal_capability_version(
        &mut self,
        version: &PersonalCapabilityVersion,
        expected_revision: i64,
        operation: &str,
        fingerprint: &str,
        now: i64,
    ) -> Result<(), PersistenceError> {
        let project = &version.project;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin personal build"))?;
        if transaction.execute(
            "UPDATE capability_creation_projects SET state_revision=state_revision+1,updated_at=?3 WHERE creation_project_id=?1 AND state_revision=?2",
            params![project.creation_project_id, expected_revision, now],
        ).map_err(database_error("advance personal build revision"))? != 1 { return Err(conflict()); }
        transaction.execute(
            "INSERT INTO personal_capability_versions(creation_project_id,package_digest,version,project_json,build_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![project.creation_project_id, version.build.package_digest, version.version,
                serde_json::to_string(project).map_err(|_| conflict())?,
                serde_json::to_string(&version.build).map_err(|_| conflict())?, now],
        ).map_err(database_error("save personal candidate"))?;
        transaction.execute(
            "INSERT INTO personal_capability_lifecycle_operations(operation_id,fingerprint,creation_project_id,kind,package_digest,created_at) VALUES(?1,?2,?3,'build',?4,?5)",
            params![operation, fingerprint, project.creation_project_id, version.build.package_digest, now],
        ).map_err(database_error("record personal build operation"))?;
        transaction
            .commit()
            .map_err(database_error("commit personal build"))
    }

    /// # Errors
    /// Returns an error if the expected revision no longer identifies the active installation.
    pub fn upgrade_personal_capability(
        &mut self,
        version: &PersonalCapabilityVersion,
        command: &super::CapabilityCreationInstallCommand,
        data_profile: &str,
        data_operation: &str,
    ) -> Result<(), PersistenceError> {
        let project = &version.project;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin personal upgrade"))?;
        if transaction.execute(
            "UPDATE capability_creation_projects SET title=?3,business_goal=?4,status='installed',requested_permissions_json=?5,source_digest=?6,source_file_count=?7,package_digest=?8,state_revision=state_revision+1,updated_at=?9,error_reason=NULL WHERE creation_project_id=?1 AND state_revision=?2",
            params![command.creation_project_id, command.expected_revision, project.title,
                project.business_goal, project.requested_permissions_json, project.source_digest,
                project.source_file_count, command.package_digest, command.occurred_at],
        ).map_err(database_error("activate personal version"))? != 1 { return Err(conflict()); }
        transaction.execute(
            "INSERT INTO personal_capability_installations(creation_project_id,package_digest,installed_relative_path,enabled,state_revision,installed_at,updated_at) VALUES(?1,?2,?3,1,1,?4,?4) ON CONFLICT(creation_project_id) DO UPDATE SET package_digest=excluded.package_digest,installed_relative_path=excluded.installed_relative_path,state_revision=personal_capability_installations.state_revision+1,updated_at=excluded.updated_at",
            params![command.creation_project_id, command.package_digest, command.installed_relative_path, command.occurred_at],
        ).map_err(database_error("switch personal installation"))?;
        transaction.execute(
            "INSERT INTO personal_capability_lifecycle_operations(operation_id,fingerprint,creation_project_id,kind,package_digest,created_at) VALUES(?1,?2,?3,'upgrade',?4,?5)",
            params![command.client_operation_id,command.request_fingerprint,command.creation_project_id,command.package_digest,command.occurred_at],
        ).map_err(database_error("record personal upgrade"))?;
        crate::personal_application_data::activate(
            &transaction,
            data_profile,
            data_operation,
            &command.creation_project_id,
            &command.package_digest,
            command.expected_revision + 1,
            command.occurred_at,
        )?;
        transaction
            .commit()
            .map_err(database_error("commit personal upgrade"))
    }
}
