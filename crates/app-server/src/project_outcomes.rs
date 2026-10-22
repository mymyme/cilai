use contexthub_application::{
    AttachProjectWorkspaceCommand, CreateDeliverableCommand, CreateDeliverableVersionCommand,
    CreateProjectCommand, DeleteProjectCommand, DeleteWorkspaceCommand, DeliverableCarrierInput,
    DeliverableRecord, DeliverableVersionRecord, DetachProjectWorkspaceCommand,
    ProjectOutcomeRepository, ProjectRecord, ReviewDeliverableVersionCommand, UnitOfWork,
    UpdateProjectCommand, WorkspaceRepository,
};
use contexthub_domain::{AcceptanceDecision, EventId, WorkspaceId};
use contexthub_persistence_sqlite::{PersistenceError, PersistenceErrorKind, SqlitePersistence};
use contexthub_protocol::{
    DeliverableCarrierItem, DeliverableCreateParams, DeliverableCreateResult,
    DeliverableFileBindingItem, DeliverableItem, DeliverableListParams, DeliverableListResult,
    DeliverableVersionCreateParams, DeliverableVersionCreateResult, DeliverableVersionItem,
    DeliverableVersionReviewParams, DeliverableVersionReviewResult, ProjectAttachWorkspaceParams,
    ProjectAttachWorkspaceResult, ProjectCreateParams, ProjectCreateResult, ProjectDeleteParams,
    ProjectDeleteResult, ProjectDetachWorkspaceParams, ProjectDetachWorkspaceResult,
    ProjectGetParams, ProjectGetResult, ProjectListParams, ProjectListResult, ProjectSummary,
    ProjectUpdateParams, ProjectUpdateResult, WorkspaceDeleteParams, WorkspaceDeleteResult,
};

/// 读取工作区显式绑定的项目。
///
/// # Errors
///
/// 参数为空、数据库不可读或领域状态无效时返回错误。
pub fn list_projects(
    persistence: &mut SqlitePersistence,
    params: &ProjectListParams,
) -> Result<ProjectListResult, PersistenceError> {
    if params.workspace_id.is_empty() {
        return Err(configuration_error("project_workspace_required"));
    }
    let workspace_id = WorkspaceId::new(params.workspace_id.clone());
    persistence
        .transact(|repositories| repositories.projects_for_workspace(&workspace_id))
        .map(|projects| ProjectListResult {
            projects: projects.into_iter().map(project_summary).collect(),
        })
}

/// 创建本地项目，目录可以在创建后关联。
///
/// # Errors
///
/// 参数、工作区归属、幂等或数据库约束不满足时返回错误。
pub fn create_project(
    persistence: &mut SqlitePersistence,
    params: &ProjectCreateParams,
    now: i64,
) -> Result<ProjectCreateResult, PersistenceError> {
    if !params.team_space_ids.is_empty() {
        return Err(configuration_error("personal_cloud_source_unavailable"));
    }
    let workspace_ids =
        selected_workspace_ids(params.workspace_id.as_deref(), &params.workspace_ids)?;
    if params.display_name.trim().is_empty()
        || params.client_operation_id.is_empty()
        || params.request_fingerprint.is_empty()
    {
        return Err(configuration_error("invalid_project_create_params"));
    }
    let id = uuid::Uuid::new_v4();
    let command = CreateProjectCommand {
        project_id: format!("project-{id}"),
        display_name: params.display_name.trim().to_owned(),
        goal: params.goal.trim().to_owned(),
        primary_workspace_id: workspace_ids.first().cloned().map(WorkspaceId::new),
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("project-event-{id}")),
        correlation_id: format!("project-correlation-{id}"),
        occurred_at: now,
    };
    persistence.transact(|repositories| {
        for workspace_id in &workspace_ids {
            if repositories
                .workspace(&WorkspaceId::new(workspace_id))?
                .is_some_and(|workspace| workspace.workspace_kind.as_str() != "local")
            {
                return Err(configuration_error("personal_cloud_source_unavailable"));
            }
        }
        let mut project = contexthub_application::create_project(repositories, &command)?;
        if !project.replayed {
            for workspace_id in workspace_ids.iter().skip(1) {
                let attach = AttachProjectWorkspaceCommand {
                    project_id: project.project_id.clone(),
                    workspace_id: WorkspaceId::new(workspace_id.clone()),
                    client_operation_id: format!("{}:{workspace_id}", params.client_operation_id),
                    request_fingerprint: params.request_fingerprint.clone(),
                    event_id: EventId::new(format!(
                        "project-workspace-event-{}",
                        uuid::Uuid::new_v4()
                    )),
                    correlation_id: command.correlation_id.clone(),
                    occurred_at: now,
                };
                project = contexthub_application::attach_project_workspace(repositories, &attach)?;
            }
        }
        Ok(ProjectCreateResult {
            project: project_summary(project),
        })
    })
}

/// 修改项目的名称与目标。
///
/// # Errors
///
/// 参数为空、项目不存在或修订与调用方所见不一致时返回错误。
pub fn update_project(
    persistence: &mut SqlitePersistence,
    params: &ProjectUpdateParams,
    now: i64,
) -> Result<ProjectUpdateResult, PersistenceError> {
    if params.project_id.is_empty() || params.display_name.trim().is_empty() {
        return Err(configuration_error("invalid_project_update_params"));
    }
    let id = uuid::Uuid::new_v4();
    let command = UpdateProjectCommand {
        project_id: params.project_id.clone(),
        display_name: params.display_name.trim().to_owned(),
        goal: params.goal.trim().to_owned(),
        expected_revision: params.expected_revision,
        event_id: EventId::new(format!("project-event-{id}")),
        correlation_id: format!("project-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| contexthub_application::update_project(repositories, &command))
        .map(|project| ProjectUpdateResult {
            project: project_summary(project),
        })
}

/// 删除项目及其过程信息，释放其持有的工作区。磁盘上的文件不受影响。
///
/// # Errors
///
/// 参数为空、项目不存在或删除约束不满足时返回错误。
pub fn delete_project(
    persistence: &mut SqlitePersistence,
    params: &ProjectDeleteParams,
    now: i64,
) -> Result<ProjectDeleteResult, PersistenceError> {
    if params.project_id.is_empty()
        || params.client_operation_id.is_empty()
        || params.request_fingerprint.is_empty()
    {
        return Err(configuration_error("invalid_project_delete_params"));
    }
    let id = uuid::Uuid::new_v4();
    let command = DeleteProjectCommand {
        project_id: params.project_id.clone(),
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("project-delete-event-{id}")),
        correlation_id: format!("project-delete-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| contexthub_application::delete_project(repositories, &command))
        .map(|summary| ProjectDeleteResult {
            project_id: summary.project_id,
            released_workspace_ids: summary
                .released_workspace_ids
                .into_iter()
                .map(|workspace| workspace.as_str().to_owned())
                .collect(),
            removed_phases: summary.removed_phases,
            removed_tasks: summary.removed_tasks,
            removed_deliverables: summary.removed_deliverables,
            removed_attention_cases: summary.removed_attention_cases,
        })
}

/// 删除一个不属于任何项目的工作区，连同其文件索引与变更历史。磁盘文件不受影响。
///
/// # Errors
///
/// 参数为空、工作区不存在、仍归属项目或仍被交付物引用时返回错误。
pub fn delete_workspace(
    persistence: &mut SqlitePersistence,
    params: &WorkspaceDeleteParams,
    now: i64,
) -> Result<WorkspaceDeleteResult, PersistenceError> {
    if params.workspace_id.is_empty()
        || params.client_operation_id.is_empty()
        || params.request_fingerprint.is_empty()
    {
        return Err(configuration_error("invalid_workspace_delete_params"));
    }
    let command = DeleteWorkspaceCommand {
        workspace_id: WorkspaceId::new(params.workspace_id.clone()),
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::delete_workspace(
                repositories,
                &command,
                EventId::new(format!("workspace-delete-event-{}", uuid::Uuid::new_v4())),
            )
        })
        .map(|summary| WorkspaceDeleteResult {
            workspace_id: summary.workspace_id,
            removed_files: summary.removed_files,
            removed_events: summary.removed_events,
        })
}

/// 解除当前项目与文件夹的关联，保留其他项目的关联。
///
/// # Errors
///
/// 参数为空或项目不存在时返回错误。
pub fn detach_project_workspace(
    persistence: &mut SqlitePersistence,
    params: &ProjectDetachWorkspaceParams,
    now: i64,
) -> Result<ProjectDetachWorkspaceResult, PersistenceError> {
    if params.project_id.is_empty()
        || params.workspace_id.is_empty()
        || params.client_operation_id.is_empty()
        || params.request_fingerprint.is_empty()
    {
        return Err(configuration_error(
            "invalid_project_detach_workspace_params",
        ));
    }
    let id = uuid::Uuid::new_v4();
    let command = DetachProjectWorkspaceCommand {
        project_id: params.project_id.clone(),
        workspace_id: WorkspaceId::new(params.workspace_id.clone()),
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("project-workspace-detach-event-{id}")),
        correlation_id: format!("project-workspace-detach-correlation-{id}"),
        occurred_at: now,
    };
    persistence.transact(|repositories| {
        let project = contexthub_application::detach_project_workspace(repositories, &command)?;
        let workspace_ids = repositories
            .workspaces_for_project(&project.project_id)?
            .into_iter()
            .map(|workspace| workspace.as_str().to_owned())
            .collect();
        Ok(ProjectDetachWorkspaceResult {
            project: project_summary(project),
            workspace_ids,
        })
    })
}

/// 把一个尚未归属任何项目的工作区绑定到已有项目。
///
/// # Errors
///
/// 参数为空、项目或工作区不存在、该工作区已归属其他项目，或持久化失败时返回错误。
pub fn attach_project_workspace(
    persistence: &mut SqlitePersistence,
    params: &ProjectAttachWorkspaceParams,
    now: i64,
) -> Result<ProjectAttachWorkspaceResult, PersistenceError> {
    let selected = selected_workspace_ids(params.workspace_id.as_deref(), &params.workspace_ids)?;
    if params.project_id.is_empty()
        || selected.is_empty()
        || params.client_operation_id.is_empty()
        || params.request_fingerprint.is_empty()
    {
        return Err(configuration_error(
            "invalid_project_attach_workspace_params",
        ));
    }
    let correlation_id = format!("project-workspace-correlation-{}", uuid::Uuid::new_v4());
    persistence.transact(|repositories| {
        for workspace_id in &selected {
            if repositories
                .workspace(&WorkspaceId::new(workspace_id))?
                .is_some_and(|workspace| workspace.workspace_kind.as_str() != "local")
            {
                return Err(configuration_error("personal_cloud_source_unavailable"));
            }
        }
        let mut project = None;
        for workspace_id in &selected {
            let command = AttachProjectWorkspaceCommand {
                project_id: params.project_id.clone(),
                workspace_id: WorkspaceId::new(workspace_id.clone()),
                client_operation_id: format!("{}:{workspace_id}", params.client_operation_id),
                request_fingerprint: params.request_fingerprint.clone(),
                event_id: EventId::new(format!("project-workspace-event-{}", uuid::Uuid::new_v4())),
                correlation_id: correlation_id.clone(),
                occurred_at: now,
            };
            project = Some(contexthub_application::attach_project_workspace(
                repositories,
                &command,
            )?);
        }
        let project = project
            .ok_or_else(|| configuration_error("invalid_project_attach_workspace_params"))?;
        let workspace_ids = repositories
            .workspaces_for_project(&project.project_id)?
            .into_iter()
            .map(|workspace| workspace.as_str().to_owned())
            .collect();
        Ok(ProjectAttachWorkspaceResult {
            project: project_summary(project),
            workspace_ids,
        })
    })
}

fn selected_workspace_ids(
    single: Option<&str>,
    multiple: &[String],
) -> Result<Vec<String>, PersistenceError> {
    if single.is_some() && !multiple.is_empty() {
        return Err(configuration_error("ambiguous_project_workspaces"));
    }
    let selected = single.map_or_else(|| multiple.to_vec(), |id| vec![id.to_owned()]);
    let mut seen = std::collections::HashSet::new();
    if selected
        .iter()
        .any(|id| id.trim().is_empty() || !seen.insert(id))
    {
        return Err(configuration_error("invalid_project_workspaces"));
    }
    Ok(selected)
}

/// 按项目身份读取单个项目及其绑定工作区，不要求调用方先知道所属工作区。
///
/// # Errors
///
/// 项目参数为空、项目不存在或数据库事实无法读取时返回错误。
pub fn get_project(
    persistence: &mut SqlitePersistence,
    params: &ProjectGetParams,
) -> Result<ProjectGetResult, PersistenceError> {
    if params.project_id.is_empty() {
        return Err(configuration_error("project_get_project_required"));
    }
    persistence.transact(|repositories| {
        let project = repositories.project_by_id(&params.project_id)?;
        let workspace_ids = repositories
            .workspaces_for_project(&params.project_id)?
            .into_iter()
            .map(|workspace_id| workspace_id.as_str().to_owned())
            .collect();
        Ok(ProjectGetResult {
            project: project_summary(project),
            workspace_ids,
        })
    })
}

/// 读取项目的交付物、版本和文件绑定。
///
/// # Errors
///
/// 项目参数为空或数据库事实无法读取时返回错误。
pub fn list_deliverables(
    persistence: &mut SqlitePersistence,
    params: &DeliverableListParams,
) -> Result<DeliverableListResult, PersistenceError> {
    if params.project_id.is_empty() {
        return Err(configuration_error("deliverable_project_required"));
    }
    persistence.transact(|repositories| {
        let deliverables = repositories.deliverables_for_project(&params.project_id)?;
        let mut items = Vec::with_capacity(deliverables.len());
        for deliverable in deliverables {
            let versions = repositories.versions_for_deliverable(&deliverable.deliverable_id)?;
            items.push(deliverable_item(deliverable, versions));
        }
        Ok(DeliverableListResult {
            project_id: params.project_id.clone(),
            deliverables: items,
        })
    })
}

/// 创建需要独立验收的项目交付物。
///
/// # Errors
///
/// 必填字段、项目状态、幂等或数据库约束不满足时返回错误。
pub fn create_deliverable(
    persistence: &mut SqlitePersistence,
    params: &DeliverableCreateParams,
    now: i64,
) -> Result<DeliverableCreateResult, PersistenceError> {
    if params.project_id.is_empty()
        || params.title.trim().is_empty()
        || params.deliverable_type.trim().is_empty()
        || params.owner_label.trim().is_empty()
        || params.acceptance_criteria.trim().is_empty()
        || params.client_operation_id.is_empty()
        || params.request_fingerprint.is_empty()
    {
        return Err(configuration_error("invalid_deliverable_create_params"));
    }
    let id = uuid::Uuid::new_v4();
    let command = CreateDeliverableCommand {
        deliverable_id: format!("deliverable-{id}"),
        project_id: params.project_id.clone(),
        title: params.title.trim().to_owned(),
        deliverable_type: params.deliverable_type.trim().to_owned(),
        owner_label: params.owner_label.trim().to_owned(),
        acceptance_criteria: params.acceptance_criteria.trim().to_owned(),
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("deliverable-event-{id}")),
        correlation_id: format!("deliverable-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| contexthub_application::create_deliverable(repositories, &command))
        .map(|deliverable| DeliverableCreateResult {
            deliverable: deliverable_item(deliverable, Vec::new()),
        })
}

/// 从已扫描真实文件冻结交付物候选版本。
///
/// # Errors
///
/// 文件不可绑定、参数、幂等或数据库约束不满足时返回错误。
pub fn create_deliverable_version(
    persistence: &mut SqlitePersistence,
    params: &DeliverableVersionCreateParams,
    now: i64,
) -> Result<DeliverableVersionCreateResult, PersistenceError> {
    if params.deliverable_id.is_empty()
        || params.local_file_ids.is_empty() && params.carriers.is_empty()
        || params.client_operation_id.is_empty()
        || params.request_fingerprint.is_empty()
    {
        return Err(configuration_error(
            "invalid_deliverable_version_create_params",
        ));
    }
    let id = uuid::Uuid::new_v4();
    let command = CreateDeliverableVersionCommand {
        version_id: format!("deliverable-version-{id}"),
        deliverable_id: params.deliverable_id.clone(),
        summary: params.summary.trim().to_owned(),
        local_file_ids: params.local_file_ids.clone(),
        carriers: params
            .carriers
            .iter()
            .enumerate()
            .map(|(index, carrier)| DeliverableCarrierInput {
                carrier_id: format!("deliverable-carrier-{id}-{index}"),
                carrier_type: carrier.carrier_type.clone(),
                carrier_role: carrier.carrier_role.clone(),
                label: carrier.label.trim().to_owned(),
                reference: carrier.reference.trim().to_owned(),
                version_label: carrier.version_label.trim().to_owned(),
                source_version_id: carrier.source_version_id.clone(),
            })
            .collect(),
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("deliverable-version-event-{id}")),
        correlation_id: format!("deliverable-version-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::create_deliverable_version(repositories, &command)
        })
        .map(|version| DeliverableVersionCreateResult {
            version: deliverable_version_item(version),
        })
}

/// 通过或退回候选版本。
///
/// # Errors
///
/// 决策、候选状态、乐观修订、幂等或数据库约束不满足时返回错误。
pub fn review_deliverable_version(
    persistence: &mut SqlitePersistence,
    params: &DeliverableVersionReviewParams,
    now: i64,
) -> Result<DeliverableVersionReviewResult, PersistenceError> {
    let decision = AcceptanceDecision::try_from(params.decision.as_str())
        .map_err(|()| configuration_error("invalid_deliverable_review_decision"))?;
    if params.deliverable_id.is_empty()
        || params.version_id.is_empty()
        || params.reviewer_label.trim().is_empty()
        || params.expected_deliverable_revision < 0
        || params.client_operation_id.is_empty()
        || params.request_fingerprint.is_empty()
    {
        return Err(configuration_error(
            "invalid_deliverable_version_review_params",
        ));
    }
    let id = uuid::Uuid::new_v4();
    let command = ReviewDeliverableVersionCommand {
        acceptance_id: format!("deliverable-acceptance-{id}"),
        deliverable_id: params.deliverable_id.clone(),
        version_id: params.version_id.clone(),
        decision,
        reviewer_label: params.reviewer_label.trim().to_owned(),
        comment: params.comment.trim().to_owned(),
        expected_deliverable_revision: params.expected_deliverable_revision,
        client_operation_id: params.client_operation_id.clone(),
        request_fingerprint: params.request_fingerprint.clone(),
        event_id: EventId::new(format!("deliverable-review-event-{id}")),
        correlation_id: format!("deliverable-review-correlation-{id}"),
        occurred_at: now,
    };
    persistence
        .transact(|repositories| {
            contexthub_application::review_deliverable_version(repositories, &command)
        })
        .map(|review| DeliverableVersionReviewResult {
            deliverable: deliverable_item(review.deliverable, vec![review.version.clone()]),
            version: deliverable_version_item(review.version),
            decision: review.decision.as_str().to_owned(),
        })
}

fn project_summary(project: ProjectRecord) -> ProjectSummary {
    ProjectSummary {
        project_id: project.project_id,
        display_name: project.display_name,
        goal: project.goal,
        status: project.status.as_str().to_owned(),
        primary_workspace_id: project
            .primary_workspace_id
            .map(|workspace| workspace.as_str().to_owned()),
        state_revision: project.state_revision,
        updated_at: project.updated_at,
    }
}

fn deliverable_item(
    deliverable: DeliverableRecord,
    versions: Vec<DeliverableVersionRecord>,
) -> DeliverableItem {
    DeliverableItem {
        deliverable_id: deliverable.deliverable_id,
        project_id: deliverable.project_id,
        title: deliverable.title,
        deliverable_type: deliverable.deliverable_type,
        owner_label: deliverable.owner_label,
        acceptance_criteria: deliverable.acceptance_criteria,
        status: deliverable.status.as_str().to_owned(),
        current_accepted_version_id: deliverable.current_accepted_version_id,
        state_revision: deliverable.state_revision,
        updated_at: deliverable.updated_at,
        versions: versions.into_iter().map(deliverable_version_item).collect(),
    }
}

fn deliverable_version_item(version: DeliverableVersionRecord) -> DeliverableVersionItem {
    DeliverableVersionItem {
        version_id: version.version_id,
        version_no: version.version_no,
        status: version.status.as_str().to_owned(),
        summary: version.summary,
        created_at: version.created_at,
        accepted_at: version.accepted_at,
        rejected_at: version.rejected_at,
        bindings: version
            .bindings
            .into_iter()
            .map(|binding| DeliverableFileBindingItem {
                local_file_id: binding.local_file_id,
                workspace_id: binding.workspace_id.as_str().to_owned(),
                relative_path: binding.relative_path,
                file_revision: binding.file_revision,
                checksum_algorithm: binding.checksum_algorithm,
                checksum_value: binding.checksum_value,
                binding_role: binding.binding_role,
            })
            .collect(),
        carriers: version
            .carriers
            .into_iter()
            .map(|carrier| DeliverableCarrierItem {
                carrier_id: carrier.carrier_id,
                carrier_type: carrier.carrier_type,
                carrier_role: carrier.carrier_role,
                label: carrier.label,
                reference: carrier.reference,
                version_label: carrier.version_label,
                source_project_id: carrier.source_project_id,
                source_deliverable_id: carrier.source_deliverable_id,
                source_version_id: carrier.source_version_id,
            })
            .collect(),
    }
}

fn configuration_error(reason: &'static str) -> PersistenceError {
    PersistenceError::with_reason(PersistenceErrorKind::Configuration, reason, reason)
}

#[cfg(test)]
mod folder_list_tests {
    use super::*;
    use crate::workspace::{create_preflighted_local_workspace, local_workspace_command};
    use std::path::PathBuf;

    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("cilai-project-folders-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("data")).unwrap();
            Self { root }
        }
        fn database(&self) -> SqlitePersistence {
            let mut db = SqlitePersistence::open(&self.root.join("data/test.db"), "0.1.0").unwrap();
            for id in ["one", "two", "three"] {
                let folder = self.root.join(id);
                std::fs::create_dir_all(&folder).unwrap();
                std::fs::write(folder.join("keep.txt"), id).unwrap();
                create_preflighted_local_workspace(
                    &mut db,
                    &folder,
                    &[self.root.join("data")],
                    local_workspace_command(
                        id,
                        id,
                        &format!("scan-{id}"),
                        &format!("event-{id}"),
                        &format!("corr-{id}"),
                        "desktop",
                        1,
                    ),
                )
                .unwrap();
            }
            db
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn create(ids: &[&str], operation: &str) -> ProjectCreateParams {
        ProjectCreateParams {
            workspace_id: None,
            team_space_ids: Vec::new(),
            workspace_ids: ids.iter().map(|id| (*id).to_owned()).collect(),
            display_name: "项目".to_owned(),
            goal: "验收".to_owned(),
            client_operation_id: operation.to_owned(),
            request_fingerprint: format!("{operation}-{ids:?}"),
        }
    }
    fn owners(db: &mut SqlitePersistence, id: &str) -> Vec<ProjectRecord> {
        db.transact(|repositories| repositories.projects_for_workspace(&WorkspaceId::new(id)))
            .unwrap()
    }
    fn folders(db: &mut SqlitePersistence, id: &str) -> Vec<WorkspaceId> {
        db.transact(|repositories| repositories.workspaces_for_project(id))
            .unwrap()
    }

    #[test]
    fn cloud_selection_is_refused_without_creating_a_project() {
        let fixture = Fixture::new();
        let mut db = fixture.database();
        let mut params = create(&["one"], "cloud-denied");
        params.team_space_ids = vec!["team".to_owned()];
        assert_eq!(
            create_project(&mut db, &params, 2).unwrap_err().reason(),
            Some("personal_cloud_source_unavailable")
        );
        assert!(owners(&mut db, "one").is_empty());
    }

    #[test]
    fn shared_local_workspaces_survive_detach_and_project_deletion() {
        let fixture = Fixture::new();
        let mut db = fixture.database();
        let first = create_project(&mut db, &create(&["one", "two"], "first"), 2)
            .unwrap()
            .project;
        let second = create_project(&mut db, &create(&["one", "two"], "second"), 3)
            .unwrap()
            .project;
        assert_eq!(
            folders(&mut db, &first.project_id),
            folders(&mut db, &second.project_id)
        );
        detach_project_workspace(
            &mut db,
            &ProjectDetachWorkspaceParams {
                project_id: first.project_id.clone(),
                workspace_id: "one".into(),
                client_operation_id: "detach".into(),
                request_fingerprint: "detach".into(),
            },
            4,
        )
        .unwrap();
        assert_eq!(owners(&mut db, "one").len(), 1);
        delete_project(
            &mut db,
            &ProjectDeleteParams {
                project_id: first.project_id,
                client_operation_id: "delete".into(),
                request_fingerprint: "delete".into(),
            },
            5,
        )
        .unwrap();
        assert_eq!(
            folders(&mut db, &second.project_id),
            vec![WorkspaceId::new("one"), WorkspaceId::new("two")]
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("one/keep.txt")).unwrap(),
            "one"
        );
    }

    #[test]
    fn creates_all_selected_folders_and_replays_without_duplicates() {
        let fixture = Fixture::new();
        let mut db = fixture.database();
        let params = create(&["one", "two"], "multiple");
        let result = create_project(&mut db, &params, 2).unwrap();
        assert_eq!(result.project.primary_workspace_id.as_deref(), Some("one"));
        let replay = create_project(&mut db, &params, 3).unwrap();
        assert_eq!(result.project.project_id, replay.project.project_id);
        assert_eq!(folders(&mut db, &result.project.project_id).len(), 2);
        assert!(owners(&mut db, "three").is_empty());
        for id in ["one", "two", "three"] {
            assert_eq!(
                std::fs::read_to_string(fixture.root.join(id).join("keep.txt")).unwrap(),
                id
            );
        }
    }

    #[test]
    fn missing_folder_rolls_back_creation_and_batch_attachment() {
        let fixture = Fixture::new();
        let mut db = fixture.database();
        create_project(&mut db, &create(&["two"], "owner"), 2).unwrap();
        assert!(create_project(&mut db, &create(&["one", "missing"], "conflict"), 3).is_err());
        assert!(owners(&mut db, "one").is_empty());
        let project = create_project(&mut db, &create(&["two"], "attachment-target"), 4)
            .unwrap()
            .project;
        let mut attach = ProjectAttachWorkspaceParams {
            project_id: project.project_id.clone(),
            workspace_id: None,
            workspace_ids: vec!["one".to_owned(), "missing".to_owned()],
            client_operation_id: "attach".to_owned(),
            request_fingerprint: "attach-one-two".to_owned(),
        };
        assert!(attach_project_workspace(&mut db, &attach, 5).is_err());
        assert_eq!(
            folders(&mut db, &project.project_id),
            vec![WorkspaceId::new("two")]
        );
        assert!(owners(&mut db, "one").is_empty());
        attach.workspace_ids = vec!["one".to_owned(), "three".to_owned()];
        let result = attach_project_workspace(&mut db, &attach, 6).unwrap();
        assert_eq!(result.workspace_ids.len(), 3);
        assert_eq!(result.project.primary_workspace_id.as_deref(), Some("two"));
    }

    #[test]
    fn legacy_single_folder_requests_still_work() {
        let fixture = Fixture::new();
        let mut db = fixture.database();
        let mut params = create(&[], "legacy");
        params.workspace_id = Some("one".to_owned());
        let result = create_project(&mut db, &params, 2).unwrap();
        let attach: ProjectAttachWorkspaceParams = serde_json::from_value(serde_json::json!({
            "projectId": result.project.project_id, "workspaceId": "two", "clientOperationId": "legacy-attach", "requestFingerprint": "legacy-attach"
        })).unwrap();
        assert_eq!(
            attach_project_workspace(&mut db, &attach, 3)
                .unwrap()
                .workspace_ids
                .len(),
            2
        );
    }

    #[test]
    fn project_without_directory_can_be_created_replayed_and_attached_later() {
        let fixture = Fixture::new();
        let mut db = fixture.database();
        let params = create(&[], "empty");
        let created = create_project(&mut db, &params, 2).unwrap();
        assert!(created.project.primary_workspace_id.is_none());
        assert!(folders(&mut db, &created.project.project_id).is_empty());
        let replayed = create_project(&mut db, &params, 3).unwrap();
        assert_eq!(created.project.project_id, replayed.project.project_id);
        let attached = attach_project_workspace(
            &mut db,
            &ProjectAttachWorkspaceParams {
                project_id: created.project.project_id.clone(),
                workspace_id: Some("one".into()),
                workspace_ids: Vec::new(),
                client_operation_id: "later".into(),
                request_fingerprint: "later".into(),
            },
            4,
        )
        .unwrap();
        assert_eq!(
            attached.project.primary_workspace_id.as_deref(),
            Some("one")
        );
        assert_eq!(folders(&mut db, &created.project.project_id).len(), 1);
    }

    #[test]
    fn invalid_folder_lists_are_rejected_before_any_binding() {
        let fixture = Fixture::new();
        let mut db = fixture.database();
        for ids in [
            &["one", "one"][..],
            &["one", ""][..],
            &["one", "missing"][..],
        ] {
            assert!(create_project(&mut db, &create(ids, "invalid"), 2).is_err());
            assert!(owners(&mut db, "one").is_empty());
        }
        let mut mixed = create(&["two"], "mixed");
        mixed.workspace_id = Some("one".to_owned());
        assert!(create_project(&mut db, &mixed, 2).is_err());
    }
}
