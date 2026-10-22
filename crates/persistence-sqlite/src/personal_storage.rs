//! 个人存储清理的不可变预览、引用核对和持久逐项回执。
use crate::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use contexthub_protocol::{
    PersonalMaintenanceOperationStatus, PersonalStorageOperation, PersonalStorageOperationState,
    PersonalStorageReference,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug)]
pub struct PersonalStorageRecord {
    pub operation: PersonalStorageOperation,
    pub candidates: Value,
    pub client_operation_id: String,
    pub request_fingerprint: String,
}
#[derive(Clone, Debug)]
pub struct PersonalStoragePathReference {
    pub relative_path: String,
    pub reference: PersonalStorageReference,
}
#[derive(Clone, Debug)]
pub struct PersonalStorageWorkspaceReference {
    pub workspace_id: String,
    pub display_name: String,
    pub path: String,
}
fn reject(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Conflict, reason, reason)
}
fn encoded(value: &impl serde::Serialize) -> Result<String, PersistenceError> {
    serde_json::to_string(value).map_err(|_| reject("personal_storage_record_invalid"))
}
fn state(value: &PersonalStorageOperationState) -> &'static str {
    match value {
        PersonalStorageOperationState::Prepared => "prepared",
        PersonalStorageOperationState::Applying => "applying",
        PersonalStorageOperationState::Completed => "completed",
        PersonalStorageOperationState::Incomplete => "incomplete",
        PersonalStorageOperationState::Cancelled => "cancelled",
    }
}
fn record(
    db: &Connection,
    profile: &str,
    id: &str,
) -> Result<Option<PersonalStorageRecord>, PersistenceError> {
    let row:Option<(String,String,String,String)>=db.query_row("SELECT record_json,candidate_json,client_operation_id,request_fingerprint FROM personal_storage_operations WHERE profile_id=?1 AND operation_id=?2",params![profile,id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(database_error("read personal storage operation"))?;
    row.map(|(op, candidates, client, fingerprint)| {
        Ok(PersonalStorageRecord {
            operation: serde_json::from_str(&op)
                .map_err(|_| reject("personal_storage_record_invalid"))?,
            candidates: serde_json::from_str(&candidates)
                .map_err(|_| reject("personal_storage_record_invalid"))?,
            client_operation_id: client,
            request_fingerprint: fingerprint,
        })
    })
    .transpose()
}
impl SqlitePersistence {
    /// # Errors
    /// 回执不可读取时明确失败。
    pub fn personal_storage_operation(
        &self,
        profile: &str,
        id: &str,
    ) -> Result<Option<PersonalStorageRecord>, PersistenceError> {
        record(&self.connection, profile, id)
    }
    /// # Errors
    /// 不用空列表代替回执读取错误。
    pub fn personal_storage_operations(
        &self,
        profile: &str,
    ) -> Result<Vec<PersonalStorageRecord>, PersistenceError> {
        let mut s=self.connection.prepare("SELECT operation_id FROM personal_storage_operations WHERE profile_id=?1 ORDER BY created_at DESC,operation_id").map_err(database_error("list personal storage operations"))?;
        let ids = s
            .query_map([profile], |row| row.get::<_, String>(0))
            .map_err(database_error("query personal storage operations"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read personal storage operation ids"))?;
        ids.into_iter()
            .map(|id| {
                record(&self.connection, profile, &id)?
                    .ok_or_else(|| reject("personal_storage_record_invalid"))
            })
            .collect()
    }
    /// # Errors
    /// 同一次准备不能修改选择或替换已冻结的文件范围。
    pub fn personal_storage_by_client(
        &self,
        profile: &str,
        client: &str,
        fingerprint: &str,
    ) -> Result<Option<PersonalStorageRecord>, PersistenceError> {
        let old:Option<(String,String)>=self.connection.query_row("SELECT operation_id,request_fingerprint FROM personal_storage_operations WHERE profile_id=?1 AND client_operation_id=?2",params![profile,client],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(database_error("read storage client operation"))?;
        match old {
            Some((id, expected)) if expected == fingerprint => {
                record(&self.connection, profile, &id)
            }
            Some(_) => Err(reject("operation_id_reused")),
            None => Ok(None),
        }
    }
    /// # Errors
    /// 资料未就绪或预览冲突时保留原回执。
    pub fn prepare_personal_storage(
        &mut self,
        record_value: &PersonalStorageRecord,
    ) -> Result<PersonalStorageRecord, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin storage preparation"))?;
        let op = &record_value.operation;
        let ready:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM personal_profile_compatibility WHERE profile_id=?1 AND state='ready')",[&op.profile_id],|row|row.get(0)).map_err(database_error("check storage profile readiness"))?;
        if !ready {
            return Err(reject("personal_storage_profile_not_ready"));
        }
        let old:Option<(String,String)>=tx.query_row("SELECT operation_id,request_fingerprint FROM personal_storage_operations WHERE profile_id=?1 AND client_operation_id=?2",params![op.profile_id,record_value.client_operation_id],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(database_error("read concurrent storage preparation"))?;
        if let Some((id, expected)) = old {
            if expected != record_value.request_fingerprint {
                return Err(reject("operation_id_reused"));
            }
            return record(&tx, &op.profile_id, &id)?
                .ok_or_else(|| reject("personal_storage_record_invalid"));
        }
        crate::maintenance_operations::register(
            &tx,
            &PersonalMaintenanceOperationStatus {
                operation_id: op.operation_id.clone(),
                profile_id: op.profile_id.clone(),
                kind: "storage_cleanup".into(),
                state: state(&op.state).into(),
                preview_digest: op.preview_digest.clone(),
            },
            op.updated_at,
        )?;
        tx.execute("INSERT INTO personal_storage_operations(operation_id,profile_id,client_operation_id,request_fingerprint,preview_digest,record_json,candidate_json,state,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![op.operation_id,op.profile_id,record_value.client_operation_id,record_value.request_fingerprint,op.preview_digest,encoded(op)?,encoded(&record_value.candidates)?,state(&op.state),op.created_at,op.updated_at]).map_err(database_error("save storage preparation"))?;
        tx.commit()
            .map_err(database_error("commit storage preparation"))?;
        Ok(record_value.clone())
    }
    /// # Errors
    /// 摘要或原状态改变时拒绝覆盖；中断后不自动继续删除。
    pub fn transition_personal_storage(
        &mut self,
        op: &PersonalStorageOperation,
        expected: &PersonalStorageOperationState,
    ) -> Result<PersonalStorageOperation, PersistenceError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(database_error("begin storage transition"))?;
        let old = record(&tx, &op.profile_id, &op.operation_id)?
            .ok_or_else(|| reject("personal_storage_operation_not_found"))?;
        if old.operation.preview_digest != op.preview_digest {
            return Err(reject("personal_storage_preview_changed"));
        }
        if old.operation.state != *expected {
            return Err(reject("personal_storage_operation_unconfirmed"));
        }
        tx.execute("UPDATE personal_storage_operations SET record_json=?3,state=?4,updated_at=?5 WHERE profile_id=?1 AND operation_id=?2",params![op.profile_id,op.operation_id,encoded(op)?,state(&op.state),op.updated_at]).map_err(database_error("save storage transition"))?;
        crate::maintenance_operations::register(
            &tx,
            &PersonalMaintenanceOperationStatus {
                operation_id: op.operation_id.clone(),
                profile_id: op.profile_id.clone(),
                kind: "storage_cleanup".into(),
                state: state(&op.state).into(),
                preview_digest: op.preview_digest.clone(),
            },
            op.updated_at,
        )?;
        tx.commit()
            .map_err(database_error("commit storage transition"))?;
        Ok(op.clone())
    }
    /// # Errors
    /// 引用不明确时不开放文件清理。
    pub fn personal_storage_references(
        &self,
        profile_root: &Path,
    ) -> Result<Vec<PersonalStoragePathReference>, PersistenceError> {
        let mut out = vec![];
        for (table, id, path, prefix, filter) in [
            (
                "user_skills",
                "skill_id",
                "managed_relative_path",
                "skills/",
                "1=1",
            ),
            (
                "capability_creation_projects",
                "creation_project_id",
                "managed_relative_path",
                "",
                "1=1",
            ),
            (
                "personal_capability_installations",
                "creation_project_id",
                "installed_relative_path",
                "",
                "1=1",
            ),
            (
                "personal_package_imports",
                "import_id",
                "candidate_relative_path",
                "",
                "state='prepared'",
            ),
        ] {
            let sql = format!("SELECT {id},{path} FROM {table} WHERE {filter}");
            let mut s = self
                .connection
                .prepare(&sql)
                .map_err(database_error("read storage resource references"))?;
            for row in s
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(database_error("query storage resource references"))?
            {
                let (id, path) = row.map_err(database_error("read storage resource reference"))?;
                out.push(PersonalStoragePathReference {
                    relative_path: format!("{prefix}{path}"),
                    reference: PersonalStorageReference {
                        kind: table.into(),
                        object_id: id,
                    },
                });
            }
        }
        let mut statement=self.connection.prepare("SELECT creation_project_id,package_digest,version FROM personal_capability_versions").map_err(database_error("read stored application versions"))?;
        for row in statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error("query stored application versions"))?
        {
            let (id, digest, version) =
                row.map_err(database_error("read stored application version"))?;
            out.push(PersonalStoragePathReference {
                relative_path: format!(
                    "personal-capabilities/{id}/{}",
                    digest.trim_start_matches("sha256:")
                ),
                reference: PersonalStorageReference {
                    kind: "application_version".into(),
                    object_id: format!("{id}@{version}"),
                },
            });
        }
        append_record_paths(&self.connection, profile_root, &mut out)?;
        Ok(out)
    }
    /// # Errors
    /// 项目原文件只返回已登记路径，不遍历文件系统。
    pub fn personal_storage_workspace_references(
        &self,
    ) -> Result<Vec<PersonalStorageWorkspaceReference>, PersistenceError> {
        let mut s=self.connection.prepare("SELECT workspace_id,display_name,local_root FROM workspaces WHERE workspace_kind='local'").map_err(database_error("read external workspace references"))?;
        s.query_map([], |row| {
            Ok(PersonalStorageWorkspaceReference {
                workspace_id: row.get(0)?,
                display_name: row.get(1)?,
                path: row.get(2)?,
            })
        })
        .map_err(database_error("query external workspace references"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error("read external workspace references"))
    }
    /// # Errors
    /// 无法解释的执行记录不能被解释为空闲。
    pub fn personal_storage_active_writer_count(
        &self,
        profile: &str,
    ) -> Result<u32, PersistenceError> {
        let mut count:u32=self.connection.query_row("SELECT COUNT(*) FROM personal_profile_writers WHERE profile_id=?1 AND state<>'released'",[profile],|row|row.get(0)).map_err(database_error("read storage writer count"))?;
        let mut s=self.connection.prepare("SELECT record_json FROM ai_runtime_entries WHERE record_id GLOB 'personalModel:lease:*'").map_err(database_error("read storage model leases"))?;
        for raw in s
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error("query storage model leases"))?
        {
            let value: Value =
                serde_json::from_str(&raw.map_err(database_error("read storage model lease"))?)
                    .map_err(|_| reject("personal_storage_record_invalid"))?;
            let lease: contexthub_protocol::ModelExecutionLease =
                serde_json::from_value(value["value"].clone())
                    .map_err(|_| reject("personal_storage_record_invalid"))?;
            if !lease.released {
                count = count
                    .checked_add(1)
                    .ok_or_else(|| reject("personal_storage_record_invalid"))?;
            }
        }
        Ok(count)
    }
}
fn collect_paths(value: &Value, key: Option<&str>, root: &Path, paths: &mut BTreeSet<String>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                collect_paths(value, Some(key), root, paths);
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|value| collect_paths(value, key, root, paths)),
        Value::String(value)
            if key.is_some_and(|key| {
                matches!(
                    key,
                    "localPath"
                        | "filePath"
                        | "path"
                        | "workspaceRoot"
                        | "workingDirectory"
                        | "program"
                        | "cwd"
                        | "readRoots"
                        | "writeRoots"
                )
            }) =>
        {
            if let Ok(relative) = Path::new(value).strip_prefix(root)
                && let Some(relative) = relative.to_str()
            {
                paths.insert(relative.replace('\\', "/"));
            }
        }
        _ => {}
    }
}

fn append_record_paths(
    db: &Connection,
    profile_root: &Path,
    out: &mut Vec<PersonalStoragePathReference>,
) -> Result<(), PersistenceError> {
    for (table, id, column) in [
        ("ai_runtime_entries", "record_id", "record_json"),
        ("ai_conversation_ledger", "record_id", "record_json"),
        ("ai_message_ledger", "record_id", "record_json"),
        ("ai_run_ledger", "record_id", "record_json"),
        ("personal_mcp_records", "record_id", "record_json"),
    ] {
        let sql = format!("SELECT {id},{column} FROM {table}");
        let mut s = db
            .prepare(&sql)
            .map_err(database_error("read storage path references"))?;
        for row in s
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error("query storage path references"))?
        {
            let (id, raw) = row.map_err(database_error("read storage path reference"))?;
            let value: Value = serde_json::from_str(&raw)
                .map_err(|_| reject("personal_storage_record_invalid"))?;
            let mut paths = BTreeSet::new();
            collect_paths(&value, None, profile_root, &mut paths);
            for relative_path in paths {
                out.push(PersonalStoragePathReference {
                    relative_path,
                    reference: PersonalStorageReference {
                        kind: table.into(),
                        object_id: id.clone(),
                    },
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MIGRATIONS, migration_checksum};
    #[test]
    fn schema_72_fence_and_data_receipt_survive_registry_upgrade() {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "cilai-storage-migration-{}",
            uuid::Uuid::new_v4().simple()
        ));
        contexthub_platform_adapter::security::ensure_restricted_directory(&root).unwrap();
        let path = root.join("contexthub.db");
        let connection = Connection::open(&path).unwrap();
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version <= 72)
        {
            if migration.rebuilds_foreign_key_parent {
                connection
                    .pragma_update(None, "foreign_keys", false)
                    .unwrap();
            }
            connection.execute_batch(migration.sql).unwrap();
            connection.execute("INSERT INTO schema_migrations(version,name,checksum,applied_at,app_version) VALUES(?1,?2,?3,1,'previous')",params![migration.version,migration.name,migration_checksum(migration.sql)]).unwrap();
            if migration.rebuilds_foreign_key_parent {
                connection
                    .pragma_update(None, "foreign_keys", true)
                    .unwrap();
            }
        }
        connection.execute("INSERT INTO personal_data_operations(operation_id,profile_id,operation_kind,client_operation_id,request_fingerprint,preview_digest,record_json,state,created_at,updated_at) VALUES('data-old','old-profile','backup','old-client','old-fingerprint','old-preview','{}','incomplete',1,1)",[]).unwrap();
        connection.execute("INSERT INTO personal_maintenance_fences(fence_id,operation_id,profile_id,record_json,state,created_at,updated_at) VALUES('fence-old','data-old','old-profile','{}','draining',1,1)",[]).unwrap();
        drop(connection);
        let db = SqlitePersistence::open(&path, "current").unwrap();
        assert_eq!(
            i64::from(crate::personal_data_schema_version()),
            MIGRATIONS.last().unwrap().version
        );
        let status =
            crate::maintenance_operations::read(&db.connection, "old-profile", "data-old").unwrap();
        assert_eq!(status.kind, "backup");
        assert_eq!(status.state, "incomplete");
        assert_eq!(status.preview_digest, "old-preview");
        let foreign_key: String = db
            .connection
            .query_row(
                "SELECT \"table\" FROM pragma_foreign_key_list('personal_maintenance_fences')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(foreign_key, "personal_maintenance_operation_registry");
        assert_eq!(
            db.connection
                .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get::<_, u32>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(db.connection.query_row("SELECT COUNT(*) FROM personal_maintenance_fences WHERE fence_id='fence-old' AND state='draining'",[],|row|row.get::<_,u32>(0)).unwrap(),1);
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn path_reference_collection_only_indexes_current_profile_without_reading_external_files() {
        let root = Path::new("/private/tmp/own-profile");
        let value = serde_json::json!({"readRoots":["/private/tmp/own-profile/ai-runtime/skill-snapshots/a","/outside/other"],"nested":{"localPath":"/private/tmp/own-profile/attachments/file"},"content":"/private/tmp/own-profile/should-not-be-a-reference"});
        let mut paths = BTreeSet::new();
        collect_paths(&value, None, root, &mut paths);
        assert_eq!(
            paths.into_iter().collect::<Vec<_>>(),
            vec!["ai-runtime/skill-snapshots/a", "attachments/file"]
        );
    }
}
