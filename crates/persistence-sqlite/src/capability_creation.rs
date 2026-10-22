use super::{PersistenceError, PersistenceErrorKind, SqlitePersistence, database_error};
use rusqlite::{OptionalExtension, Row, Transaction, params};
use serde_json::{Value, json};

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityCreationProjectRecord {
    pub creation_project_id: String,
    pub title: String,
    pub business_goal: String,
    pub target_type: String,
    pub status: String,
    pub conversation_id: Option<String>,
    pub managed_relative_path: String,
    pub requested_permissions_json: String,
    pub acceptance_examples_json: String,
    pub source_digest: Option<String>,
    pub source_file_count: i64,
    pub state_revision: i64,
    pub package_digest: Option<String>,
    pub error_reason: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityCreationOperationRecord {
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub operation_kind: String,
    pub creation_project_id: String,
    pub result_revision: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityCreationCreateCommand {
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub creation_project_id: String,
    pub title: String,
    pub business_goal: String,
    pub target_type: String,
    pub conversation_id: Option<String>,
    pub managed_relative_path: String,
    pub requested_permissions_json: String,
    pub acceptance_examples_json: String,
    pub initial_status: String,
    pub source_digest: Option<String>,
    pub source_file_count: i64,
    pub package_digest: Option<String>,
    pub build: Option<CapabilityCreationBuildRecord>,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CapabilityCreationBuildRecord {
    pub build_id: String,
    pub creation_project_id: String,
    pub source_digest: String,
    pub package_digest: String,
    pub package_relative_path: String,
    pub file_count: i64,
    pub total_bytes: i64,
    pub created_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PersonalCapabilityInstallationRecord {
    pub creation_project_id: String,
    pub package_digest: String,
    pub installed_relative_path: String,
    pub enabled: bool,
    pub state_revision: i64,
    pub installed_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityCreationInstallCommand {
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub creation_project_id: String,
    pub expected_revision: i64,
    pub package_digest: String,
    pub installed_relative_path: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonalCapabilityManageCommand {
    pub client_operation_id: String,
    pub request_fingerprint: String,
    pub creation_project_id: String,
    pub expected_revision: i64,
    pub package_digest: String,
    pub action: String,
    pub occurred_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityCreationDeliveryRecord {
    pub run_id: String,
    pub status: String,
    pub source_json: String,
    pub result_json: Option<String>,
}

pub(super) fn enqueue(transaction: &Transaction<'_>) -> Result<(), PersistenceError> {
    let pending = {
        let mut statement = transaction
            .prepare(
                "SELECT run.run_id,run.run_json FROM ai_business_runs run
                 WHERE run.status='completed'
                   AND NOT EXISTS (
                       SELECT 1 FROM ai_capability_project_deliveries known
                       WHERE known.run_id=run.run_id
                   )
                 ORDER BY run.position,run.run_id",
            )
            .map_err(database_error(
                "prepare capability project delivery candidates",
            ))?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(database_error(
                "query capability project delivery candidates",
            ))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error(
                "read capability project delivery candidates",
            ))?
    };
    for (id, run) in pending {
        let run: Value = serde_json::from_str(&run).map_err(|_| invalid_ai_state())?;
        let message: Option<String> = transaction
            .query_row(
                "SELECT message_json FROM ai_business_messages WHERE run_id=?1 AND role='assistant'
                 ORDER BY position DESC,message_id DESC LIMIT 1",
                [&id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error("read capability project delivery reply"))?;
        let Some(message) = message else {
            continue;
        };
        let message: Value = serde_json::from_str(&message).map_err(|_| invalid_ai_state())?;
        let message = super::ai_state::hydrate_message(transaction, message)?;
        let source = json!({
            "runId": id,
            "conversationId": run["conversationId"],
            "superseded": run["superseded"],
            "skillIds": run["skillIds"],
            "message": message,
        });
        transaction
            .execute(
                "INSERT INTO ai_capability_project_deliveries(run_id,status,source_json)
                 VALUES (?1,'pending',?2)",
                params![id, source.to_string()],
            )
            .map_err(database_error("enqueue capability project delivery"))?;
    }
    Ok(())
}

fn invalid_ai_state() -> PersistenceError {
    PersistenceError::with_reason(
        PersistenceErrorKind::Configuration,
        "ai_state_invalid",
        "Invalid AI state",
    )
}

impl SqlitePersistence {
    /// 读取对话交付处理事实。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn capability_creation_delivery(
        &self,
        run_id: &str,
    ) -> Result<Option<CapabilityCreationDeliveryRecord>, PersistenceError> {
        self.connection
            .query_row(
                "SELECT run_id,status,source_json,result_json
                 FROM ai_capability_project_deliveries WHERE run_id=?1",
                [run_id],
                |row| {
                    Ok(CapabilityCreationDeliveryRecord {
                        run_id: row.get(0)?,
                        status: row.get(1)?,
                        source_json: row.get(2)?,
                        result_json: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(database_error("read capability project delivery"))
    }

    /// 读取等待处理的能力项目交付。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn pending_capability_creation_deliveries(&self) -> Result<Vec<String>, PersistenceError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT run_id FROM ai_capability_project_deliveries
                 WHERE status='pending' ORDER BY rowid LIMIT 8",
            )
            .map_err(database_error(
                "prepare pending capability project deliveries",
            ))?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(database_error(
                "query pending capability project deliveries",
            ))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read pending capability project deliveries"))
    }

    /// 保存能力项目交付结果。
    ///
    /// # Errors
    /// 数据库不可写时返回错误。
    pub fn finish_capability_creation_delivery(
        &self,
        run_id: &str,
        status: &str,
        result: &Value,
    ) -> Result<(), PersistenceError> {
        self.connection
            .execute(
                "UPDATE ai_capability_project_deliveries
                 SET status=?2,result_json=?3 WHERE run_id=?1",
                params![run_id, status, result.to_string()],
            )
            .map_err(database_error("finish capability project delivery"))?;
        Ok(())
    }

    /// 核对来源运行仍可用于创建项目。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn capability_creation_source_run_available(
        &self,
        run_id: &str,
    ) -> Result<bool, PersistenceError> {
        self.connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM ai_business_runs
                    WHERE run_id=?1 AND status='completed'
                      AND COALESCE(json_extract(run_json,'$.superseded'),0)=0
                 )",
                [run_id],
                |row| row.get(0),
            )
            .map_err(database_error("check capability project source run"))
    }

    /// 读取能力创建项目，按最近更新时间排序。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn list_capability_creation_projects(
        &self,
    ) -> Result<Vec<CapabilityCreationProjectRecord>, PersistenceError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT creation_project_id,title,business_goal,target_type,status,conversation_id,managed_relative_path,requested_permissions_json,acceptance_examples_json,source_digest,source_file_count,state_revision,package_digest,error_reason,created_at,updated_at
                 FROM capability_creation_projects ORDER BY updated_at DESC,creation_project_id ASC",
            )
            .map_err(database_error("prepare capability creation project list"))?;
        statement
            .query_map([], row_to_project)
            .map_err(database_error("query capability creation project list"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error("read capability creation project list"))
    }

    /// 读取单个能力创建项目。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn capability_creation_project(
        &self,
        creation_project_id: &str,
    ) -> Result<Option<CapabilityCreationProjectRecord>, PersistenceError> {
        self.connection
            .query_row(
                "SELECT creation_project_id,title,business_goal,target_type,status,conversation_id,managed_relative_path,requested_permissions_json,acceptance_examples_json,source_digest,source_file_count,state_revision,package_digest,error_reason,created_at,updated_at
                 FROM capability_creation_projects WHERE creation_project_id=?1",
                [creation_project_id],
                row_to_project,
            )
            .optional()
            .map_err(database_error("read capability creation project"))
    }

    /// 读取创建操作的幂等事实。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn capability_creation_operation(
        &self,
        client_operation_id: &str,
    ) -> Result<Option<CapabilityCreationOperationRecord>, PersistenceError> {
        self.connection
            .query_row(
                "SELECT client_operation_id,request_fingerprint,operation_kind,creation_project_id,result_revision
                 FROM capability_creation_project_operations WHERE client_operation_id=?1",
                [client_operation_id],
                |row| {
                    Ok(CapabilityCreationOperationRecord {
                        client_operation_id: row.get(0)?,
                        request_fingerprint: row.get(1)?,
                        operation_kind: row.get(2)?,
                        creation_project_id: row.get(3)?,
                        result_revision: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(database_error("read capability creation operation"))
    }

    fn replayed_capability_creation_project(
        &self,
        client_operation_id: &str,
        request_fingerprint: &str,
        operation_kind: &str,
        creation_project_id: &str,
    ) -> Result<Option<CapabilityCreationProjectRecord>, PersistenceError> {
        let Some(operation) = self.capability_creation_operation(client_operation_id)? else {
            return Ok(None);
        };
        if operation.request_fingerprint != request_fingerprint
            || operation.operation_kind != operation_kind
            || operation.creation_project_id != creation_project_id
        {
            return Err(PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "operation_id_reused",
                "capability operation id was reused with different parameters",
            ));
        }
        self.capability_creation_project(&operation.creation_project_id)?
            .ok_or_else(|| {
                PersistenceError::with_reason(
                    PersistenceErrorKind::Conflict,
                    "capability_creation_project_not_found",
                    "capability project for the completed operation no longer exists",
                )
            })
            .map(Some)
    }

    /// 幂等创建能力项目。
    ///
    /// # Errors
    /// 操作号复用、目标记录冲突或数据库不可写时返回错误。
    pub fn create_capability_creation_project(
        &mut self,
        command: &CapabilityCreationCreateCommand,
    ) -> Result<CapabilityCreationProjectRecord, PersistenceError> {
        if let Some(project) = self.replayed_capability_creation_project(
            &command.client_operation_id,
            &command.request_fingerprint,
            "create",
            &command.creation_project_id,
        )? {
            return Ok(project);
        }

        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin capability creation project create"))?;
        transaction
            .execute(
                "INSERT INTO capability_creation_projects(
                    creation_project_id,title,business_goal,target_type,status,conversation_id,
                    managed_relative_path,requested_permissions_json,acceptance_examples_json,
                    source_digest,source_file_count,state_revision,package_digest,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,1,?12,?13,?13)",
                params![
                    command.creation_project_id,
                    command.title,
                    command.business_goal,
                    command.target_type,
                    command.initial_status,
                    command.conversation_id,
                    command.managed_relative_path,
                    command.requested_permissions_json,
                    command.acceptance_examples_json,
                    command.source_digest,
                    command.source_file_count,
                    command.package_digest,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert capability creation project"))?;
        if let Some(build) = command.build.as_ref() {
            if build.creation_project_id != command.creation_project_id
                || Some(build.source_digest.as_str()) != command.source_digest.as_deref()
                || Some(build.package_digest.as_str()) != command.package_digest.as_deref()
                || build.file_count != command.source_file_count
            {
                return Err(PersistenceError::with_reason(
                    PersistenceErrorKind::Configuration,
                    "capability_creation_build_invalid",
                    "capability creation build does not match the project command",
                ));
            }
            transaction
                .execute(
                    "INSERT INTO capability_creation_builds(
                        build_id,creation_project_id,source_digest,package_digest,
                        package_relative_path,file_count,total_bytes,created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        build.build_id,
                        build.creation_project_id,
                        build.source_digest,
                        build.package_digest,
                        build.package_relative_path,
                        build.file_count,
                        build.total_bytes,
                        build.created_at,
                    ],
                )
                .map_err(database_error("insert capability creation build"))?;
        }
        transaction
            .execute(
                "INSERT INTO capability_creation_project_operations(
                    client_operation_id,request_fingerprint,operation_kind,creation_project_id,
                    result_revision,created_at)
                 VALUES (?1,?2,'create',?3,1,?4)",
                params![
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.creation_project_id,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("record capability creation operation"))?;
        transaction
            .commit()
            .map_err(database_error("commit capability creation project create"))?;
        self.capability_creation_project(&command.creation_project_id)?
            .ok_or_else(|| {
                PersistenceError::with_reason(
                    PersistenceErrorKind::Configuration,
                    "capability_creation_project_not_found",
                    "created capability project is missing",
                )
            })
    }

    /// 保存项目首次不可变构建事实；与项目创建在同一事务外调用时，仅允许目标项目与摘要完全匹配。
    ///
    /// # Errors
    /// 项目不存在、摘要冲突或数据库不可写时返回错误。
    pub fn record_capability_creation_build(
        &self,
        record: &CapabilityCreationBuildRecord,
    ) -> Result<(), PersistenceError> {
        let changed = self
            .connection
            .execute(
                "INSERT INTO capability_creation_builds(
                    build_id,creation_project_id,source_digest,package_digest,
                    package_relative_path,file_count,total_bytes,created_at)
                 SELECT ?1,?2,?3,?4,?5,?6,?7,?8
                 WHERE EXISTS(
                    SELECT 1 FROM capability_creation_projects
                    WHERE creation_project_id=?2 AND source_digest=?3 AND package_digest=?4
                 )
                 ON CONFLICT(creation_project_id,source_digest) DO NOTHING",
                params![
                    record.build_id,
                    record.creation_project_id,
                    record.source_digest,
                    record.package_digest,
                    record.package_relative_path,
                    record.file_count,
                    record.total_bytes,
                    record.created_at,
                ],
            )
            .map_err(database_error("record capability creation build"))?;
        if changed == 1 {
            return Ok(());
        }
        let matches: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM capability_creation_builds
                    WHERE creation_project_id=?1 AND source_digest=?2
                      AND package_digest=?3 AND package_relative_path=?4
                 )",
                params![
                    record.creation_project_id,
                    record.source_digest,
                    record.package_digest,
                    record.package_relative_path,
                ],
                |row| row.get(0),
            )
            .map_err(database_error("verify capability creation build"))?;
        if matches {
            Ok(())
        } else {
            Err(PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "capability_creation_build_conflict",
                "capability creation build does not match the project",
            ))
        }
    }

    /// 读取指定项目和包摘要对应的不可变构建。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn capability_creation_build(
        &self,
        creation_project_id: &str,
        package_digest: &str,
    ) -> Result<Option<CapabilityCreationBuildRecord>, PersistenceError> {
        if let Some(version) = self
            .personal_capability_versions(creation_project_id)?
            .into_iter()
            .find(|version| version.build.package_digest == package_digest)
        {
            return Ok(Some(version.build));
        }
        self.connection
            .query_row(
                "SELECT build_id,creation_project_id,source_digest,package_digest,
                        package_relative_path,file_count,total_bytes,created_at
                 FROM capability_creation_builds
                 WHERE creation_project_id=?1 AND package_digest=?2",
                params![creation_project_id, package_digest],
                |row| {
                    Ok(CapabilityCreationBuildRecord {
                        build_id: row.get(0)?,
                        creation_project_id: row.get(1)?,
                        source_digest: row.get(2)?,
                        package_digest: row.get(3)?,
                        package_relative_path: row.get(4)?,
                        file_count: row.get(5)?,
                        total_bytes: row.get(6)?,
                        created_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(database_error("read capability creation build"))
    }

    /// 读取当前 Profile 的个人能力安装事实。
    ///
    /// # Errors
    /// 数据库不可读时返回错误。
    pub fn personal_capability_installation(
        &self,
        creation_project_id: &str,
    ) -> Result<Option<PersonalCapabilityInstallationRecord>, PersistenceError> {
        self.connection
            .query_row(
                "SELECT creation_project_id,package_digest,installed_relative_path,enabled,
                        state_revision,installed_at,updated_at
                 FROM personal_capability_installations WHERE creation_project_id=?1",
                [creation_project_id],
                |row| {
                    Ok(PersonalCapabilityInstallationRecord {
                        creation_project_id: row.get(0)?,
                        package_digest: row.get(1)?,
                        installed_relative_path: row.get(2)?,
                        enabled: row.get(3)?,
                        state_revision: row.get(4)?,
                        installed_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(database_error("read personal capability installation"))
    }

    /// 修改个人安装状态，保留创建项目、构建产物和用户数据。
    ///
    /// # Errors
    /// 操作号、版本或安装状态冲突，以及数据库写入失败时返回错误。
    pub fn manage_personal_capability(
        &mut self,
        command: &PersonalCapabilityManageCommand,
    ) -> Result<bool, PersistenceError> {
        let previous: Option<(String, String, String)> = self.connection.query_row(
            "SELECT request_fingerprint,creation_project_id,action FROM personal_capability_management_operations WHERE client_operation_id=?1",
            [&command.client_operation_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional().map_err(database_error("read personal management operation"))?;
        if let Some((fingerprint, project_id, action)) = previous {
            if fingerprint != command.request_fingerprint
                || project_id != command.creation_project_id
                || action != command.action
            {
                return Err(PersistenceError::with_reason(
                    PersistenceErrorKind::Conflict,
                    "operation_id_reused",
                    "operation request changed",
                ));
            }
            return Ok(true);
        }
        let conflict = || {
            PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "personal_capability_state_conflict",
                "personal installation changed",
            )
        };
        if !matches!(command.action.as_str(), "enable" | "disable" | "uninstall") {
            return Err(conflict());
        }
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin personal management"))?;
        let next_status = if command.action == "uninstall" {
            "packaged"
        } else {
            "installed"
        };
        let changed = transaction.execute(
            "UPDATE capability_creation_projects SET status=?1,state_revision=state_revision+1,updated_at=?2 WHERE creation_project_id=?3 AND status='installed' AND state_revision=?4 AND package_digest=?5",
            params![next_status,command.occurred_at,command.creation_project_id,command.expected_revision,command.package_digest],
        ).map_err(database_error("advance personal capability state"))?;
        if changed != 1 {
            return Err(conflict());
        }
        let changed = if command.action == "uninstall" {
            transaction.execute("DELETE FROM personal_capability_installations WHERE creation_project_id=?1 AND package_digest=?2",params![command.creation_project_id,command.package_digest])
        } else {
            transaction.execute("UPDATE personal_capability_installations SET enabled=?1,state_revision=state_revision+1,updated_at=?2 WHERE creation_project_id=?3 AND package_digest=?4", params![command.action == "enable",command.occurred_at,command.creation_project_id,command.package_digest])
        }.map_err(database_error("update personal installation"))?;
        if changed != 1 {
            return Err(conflict());
        }
        transaction.execute(
            "INSERT INTO personal_capability_management_operations(client_operation_id,request_fingerprint,creation_project_id,action,result_revision,created_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![command.client_operation_id,command.request_fingerprint,command.creation_project_id,command.action,command.expected_revision+1,command.occurred_at],
        ).map_err(database_error("record personal management operation"))?;
        transaction
            .commit()
            .map_err(database_error("commit personal management"))?;
        Ok(false)
    }

    /// 幂等安装个人能力，并推进项目状态。
    ///
    /// # Errors
    /// 操作号冲突、项目状态或修订不匹配、数据库不可写时返回错误。
    pub fn install_capability_creation_project(
        &mut self,
        command: &CapabilityCreationInstallCommand,
    ) -> Result<(CapabilityCreationProjectRecord, bool), PersistenceError> {
        if let Some(project) = self.replayed_capability_creation_project(
            &command.client_operation_id,
            &command.request_fingerprint,
            "install",
            &command.creation_project_id,
        )? {
            return Ok((project, true));
        }
        let project = self
            .capability_creation_project(&command.creation_project_id)?
            .ok_or_else(|| {
                PersistenceError::with_reason(
                    PersistenceErrorKind::Conflict,
                    "capability_creation_project_not_found",
                    "capability project does not exist",
                )
            })?;
        if !matches!(
            project.target_type.as_str(),
            "web_application" | "local_mcp" | "web_mcp_application"
        ) || project.status != "packaged"
            || project.state_revision != command.expected_revision
            || project.package_digest.as_deref() != Some(command.package_digest.as_str())
        {
            return Err(PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "capability_creation_install_state_conflict",
                "capability project is not installable",
            ));
        }
        let next_revision = project.state_revision + 1;
        let transaction = self
            .connection
            .transaction()
            .map_err(database_error("begin personal capability install"))?;
        transaction
            .execute(
                "INSERT INTO personal_capability_installations(
                    creation_project_id,package_digest,installed_relative_path,enabled,
                    state_revision,installed_at,updated_at)
                 VALUES (?1,?2,?3,1,1,?4,?4)",
                params![
                    command.creation_project_id,
                    command.package_digest,
                    command.installed_relative_path,
                    command.occurred_at,
                ],
            )
            .map_err(database_error("insert personal capability installation"))?;
        let changed = transaction
            .execute(
                "UPDATE capability_creation_projects
                 SET status='installed',state_revision=?2,updated_at=?3,error_reason=NULL
                 WHERE creation_project_id=?1 AND state_revision=?4 AND status='packaged'",
                params![
                    command.creation_project_id,
                    next_revision,
                    command.occurred_at,
                    command.expected_revision,
                ],
            )
            .map_err(database_error("advance installed capability project"))?;
        if changed != 1 {
            return Err(PersistenceError::with_reason(
                PersistenceErrorKind::Conflict,
                "capability_creation_install_state_conflict",
                "capability project changed during installation",
            ));
        }
        transaction
            .execute(
                "INSERT INTO capability_creation_project_operations(
                    client_operation_id,request_fingerprint,operation_kind,creation_project_id,
                    result_revision,created_at)
                 VALUES (?1,?2,'install',?3,?4,?5)",
                params![
                    command.client_operation_id,
                    command.request_fingerprint,
                    command.creation_project_id,
                    next_revision,
                    command.occurred_at,
                ],
            )
            .map_err(database_error(
                "record personal capability install operation",
            ))?;
        transaction
            .commit()
            .map_err(database_error("commit personal capability install"))?;
        let project = self
            .capability_creation_project(&command.creation_project_id)?
            .ok_or_else(|| {
                PersistenceError::with_reason(
                    PersistenceErrorKind::Configuration,
                    "capability_creation_project_not_found",
                    "installed capability project is missing",
                )
            })?;
        Ok((project, false))
    }
}

fn row_to_project(row: &Row<'_>) -> rusqlite::Result<CapabilityCreationProjectRecord> {
    Ok(CapabilityCreationProjectRecord {
        creation_project_id: row.get(0)?,
        title: row.get(1)?,
        business_goal: row.get(2)?,
        target_type: row.get(3)?,
        status: row.get(4)?,
        conversation_id: row.get(5)?,
        managed_relative_path: row.get(6)?,
        requested_permissions_json: row.get(7)?,
        acceptance_examples_json: row.get(8)?,
        source_digest: row.get(9)?,
        source_file_count: row.get(10)?,
        state_revision: row.get(11)?,
        package_digest: row.get(12)?,
        error_reason: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_is_idempotent_and_operation_reuse_is_rejected() {
        let root = std::env::temp_dir().join(format!(
            "cilai-capability-creation-persistence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let mut persistence = SqlitePersistence::open(&root.join("contexthub.db"), "test").unwrap();
        let command = CapabilityCreationCreateCommand {
            client_operation_id: "create-1".to_owned(),
            request_fingerprint: "a".repeat(64),
            creation_project_id: "capability_1".to_owned(),
            title: "文件检查".to_owned(),
            business_goal: "检查个人目录中的文件".to_owned(),
            target_type: "local_mcp".to_owned(),
            conversation_id: Some("conversation-1".to_owned()),
            managed_relative_path: "capability-projects/capability_1".to_owned(),
            requested_permissions_json: "[\"personal_file.read\"]".to_owned(),
            acceptance_examples_json: "[{\"input\":\"目录\",\"expected\":\"返回问题清单\"}]"
                .to_owned(),
            initial_status: "planned".to_owned(),
            source_digest: None,
            source_file_count: 0,
            package_digest: None,
            build: None,
            occurred_at: 10,
        };
        let created = persistence
            .create_capability_creation_project(&command)
            .unwrap();
        let replayed = persistence
            .create_capability_creation_project(&command)
            .unwrap();
        assert_eq!(created, replayed);
        assert_eq!(
            persistence
                .list_capability_creation_projects()
                .unwrap()
                .len(),
            1
        );

        let mut conflicting = command;
        conflicting.request_fingerprint = "b".repeat(64);
        assert_eq!(
            persistence
                .create_capability_creation_project(&conflicting)
                .unwrap_err()
                .reason(),
            Some("operation_id_reused")
        );
        drop(persistence);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_marks_existing_runs_historical() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ai_runs(run_id TEXT,status TEXT);
                 INSERT INTO ai_runs VALUES ('old','completed'),('active','running');",
            )
            .unwrap();
        connection
            .execute_batch(include_str!(
                "../../../migrations/0046_add_capability_creation_projects.sql"
            ))
            .unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM ai_capability_project_deliveries WHERE run_id='old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "historical");
        let pending: i64 = connection
            .query_row(
                "SELECT count(*) FROM ai_capability_project_deliveries WHERE status='pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending, 0);
    }
}
