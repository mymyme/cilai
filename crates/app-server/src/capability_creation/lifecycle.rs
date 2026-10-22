use super::{
    AppServerServices, CapabilityCreationBuildRecord, CapabilityCreationInstallCommand,
    CapabilityCreationInstallParams, CapabilityCreationListParams, CapabilityCreationProjectRecord,
    CapabilityCreationSourceFile, Digest, Path, PathBuf, PersonalApplicationOpenParams,
    ServiceError, Sha256, Uuid, fs, json, materialize_personal_web_install,
    materialize_source_package, pages, persistence_error, project_from_record, reject_symlink,
    validate_installed_source, validate_project_id, validate_source_files, validate_text,
};
use contexthub_persistence_sqlite::PersonalCapabilityVersion;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersonalBuildParams {
    creation_project_id: String,
    expected_revision: i64,
    client_operation_id: String,
    title: String,
    business_goal: String,
    requested_permissions: Vec<String>,
    source_files: Vec<CapabilityCreationSourceFile>,
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use contexthub_persistence_sqlite::SqlitePersistence;

    #[test]
    #[allow(clippy::too_many_lines)]
    fn candidate_build_keeps_active_version_and_upgrade_preserves_data_and_disabled_state() {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("cilai-lifecycle-{}", Uuid::new_v4()));
        let profile = Uuid::new_v4().simple().to_string();
        let root = base
            .join("cn.cilai.personal.workbench/business/profiles")
            .join(&profile);
        fs::create_dir_all(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("test.db"), "test").unwrap();
        db.begin_personal_profile_upgrade(&profile, "lifecycle-test")
            .unwrap();
        db.finish_personal_profile_upgrade(&profile, "lifecycle-test", "manifest")
            .unwrap();
        let mut services = AppServerServices::with_data_dir(Some(db), Some(root.clone()));
        let mut create = CapabilityCreationCreateParams {
            title: "测试页面".into(),
            business_goal: "展示测试内容".into(),
            target_type: "web_application".into(),
            conversation_id: None,
            requested_permissions: vec![],
            acceptance_examples: vec![CapabilityCreationAcceptanceExample {
                input: "打开".into(),
                expected: "显示内容".into(),
            }],
            source_files: vec![CapabilityCreationSourceFile {
                path: "web/index.html".into(),
                content: "<h1>第一版</h1>".into(),
            }],
            client_operation_id: "create".into(),
            request_fingerprint: String::new(),
        };
        create.request_fingerprint = capability_creation_request_fingerprint(&create);
        let project = services
            .capability_creation_create(&create, 1)
            .unwrap()
            .project;
        let read = PersonalApplicationOpenParams {
            creation_project_id: project.creation_project_id.clone(),
        };
        let mut install = CapabilityCreationInstallParams {
            creation_project_id: read.creation_project_id.clone(),
            expected_revision: 1,
            package_digest: project.package_digest.clone().unwrap(),
            client_operation_id: "install".into(),
            request_fingerprint: String::new(),
        };
        install.request_fingerprint = capability_install_request_fingerprint(&install);
        let initial = services.personal_capability_editor(&read, 1).unwrap();
        assert_eq!(initial["candidateVersion"], "0.1.0");
        let preview = services
            .personal_capability_preview_runtime(
                &PersonalVersionParams {
                    creation_project_id: read.creation_project_id.clone(),
                    package_digest: install.package_digest.clone(),
                },
                1,
            )
            .unwrap();
        assert_eq!(preview["html"], "<h1>第一版</h1>");
        super::super::data::test_support::upgrade(&mut services,
            json!({"creationProjectId":read.creation_project_id,"expectedRevision":1,"clientOperationId":"install-initial","packageDigest":install.package_digest,"confirmedPermissions":[]}),
            "lifecycle-test",2).unwrap();
        let data = services
            .personal_application_data_runtime_root(
                &read.creation_project_id,
                &install.package_digest,
                2,
            )
            .unwrap();
        fs::write(data.join("saved.txt"), "个人数据").unwrap();
        let mut build = PersonalBuildParams {
            creation_project_id: read.creation_project_id.clone(),
            expected_revision: 2,
            client_operation_id: "build".into(),
            title: "第二版".into(),
            business_goal: create.business_goal.clone(),
            requested_permissions: vec![],
            source_files: vec![CapabilityCreationSourceFile {
                path: "web/index.html".into(),
                content: "<h1>第二版</h1>".into(),
            }],
        };
        let candidate = services.personal_capability_build(&build, 3).unwrap();
        assert_eq!(candidate["candidateVersion"], "0.1.1");
        assert_ne!(candidate["candidateDigest"], candidate["installedDigest"]);
        assert_eq!(
            services.personal_application_open(&read, 4).unwrap().html,
            "<h1>第一版</h1>"
        );
        assert_eq!(
            services.personal_capability_build(&build, 4).unwrap()["project"]["stateRevision"],
            3
        );
        build.title = "不同请求".into();
        assert!(services.personal_capability_build(&build, 4).is_err());
        let mut upgrade = PersonalUpgradeParams {
            data_change: None,
            creation_project_id: read.creation_project_id.clone(),
            expected_revision: 2,
            client_operation_id: "upgrade".into(),
            package_digest: candidate["candidateDigest"].as_str().unwrap().into(),
            confirmed_permissions: vec![],
        };
        assert!(services.personal_capability_upgrade(&upgrade, 4).is_err());
        services
            .personal_capability_manage(
                &PersonalCapabilityManageParams {
                    creation_project_id: read.creation_project_id.clone(),
                    expected_revision: 3,
                    package_digest: install.package_digest.clone(),
                    client_operation_id: "disable".into(),
                    action: "disable".into(),
                },
                5,
            )
            .unwrap();
        upgrade.expected_revision = 4;
        let installed = super::super::data::test_support::upgrade(
            &mut services,
            serde_json::to_value(&upgrade).unwrap(),
            "lifecycle-test",
            6,
        )
        .unwrap();
        assert_eq!(installed["installedDigest"], candidate["candidateDigest"]);
        assert!(services.personal_application_open(&read, 7).is_err());
        assert_eq!(
            fs::read_to_string(data.join("saved.txt")).unwrap(),
            "个人数据"
        );
        assert_eq!(
            services.personal_capability_upgrade(&upgrade, 7).unwrap()["project"]["stateRevision"],
            5
        );
        services
            .personal_capability_manage(
                &PersonalCapabilityManageParams {
                    creation_project_id: read.creation_project_id.clone(),
                    expected_revision: 5,
                    package_digest: upgrade.package_digest.clone(),
                    client_operation_id: "enable".into(),
                    action: "enable".into(),
                },
                8,
            )
            .unwrap();
        assert_eq!(
            services.personal_application_open(&read, 9).unwrap().html,
            "<h1>第二版</h1>"
        );
        let preview = services
            .capability_creation_preview(
                &CapabilityCreationPreviewParams {
                    creation_project_id: read.creation_project_id.clone(),
                },
                9,
            )
            .unwrap();
        assert_eq!(preview.html, "<h1>第二版</h1>");
        drop(services);
        fs::remove_dir_all(base).unwrap();
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersonalUpgradeParams {
    #[serde(default)]
    data_change: Option<contexthub_protocol::PersonalApplicationDataExecuteParams>,
    creation_project_id: String,
    expected_revision: i64,
    client_operation_id: String,
    package_digest: String,
    confirmed_permissions: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersonalVersionParams {
    creation_project_id: String,
    package_digest: String,
}

fn fingerprint(kind: &str, params: &impl Serialize) -> Result<String, ServiceError> {
    let mut request = serde_json::to_value(params)
        .map_err(|_| ServiceError::new("personal_capability_request_invalid"))?;
    if let Some(object) = request.as_object_mut() {
        object.remove("dataChange");
    }
    let bytes = serde_json::to_vec(&json!({"kind":kind,"request":request}))
        .map_err(|_| ServiceError::new("personal_capability_request_invalid"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

impl AppServerServices {
    fn current_personal_candidate(
        &self,
        id: &str,
        digest: &str,
    ) -> Result<PersonalCapabilityVersion, ServiceError> {
        let versions = self
            .persistence()?
            .personal_capability_versions(id)
            .map_err(persistence_error)?;
        if let Some(version) = versions.into_iter().next() {
            if version.build.package_digest != digest {
                return Err(ServiceError::new("personal_capability_candidate_changed"));
            }
            return Ok(version);
        }
        let project = self
            .persistence()?
            .capability_creation_project(id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        if project.package_digest.as_deref() != Some(digest) {
            return Err(ServiceError::new("personal_capability_candidate_changed"));
        }
        let build = self
            .persistence()?
            .capability_creation_build(id, digest)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_build_not_found"))?;
        Ok(PersonalCapabilityVersion {
            version: "0.1.0".into(),
            project,
            build,
        })
    }
    pub(crate) fn personal_capability_catalog(
        &mut self,
        _: &CapabilityCreationListParams,
        _: i64,
    ) -> Result<Value, ServiceError> {
        let projects = self
            .persistence()?
            .list_capability_creation_projects()
            .map_err(persistence_error)?;
        let mut items = Vec::new();
        for project in projects {
            if self
                .persistence()?
                .enterprise_capability_binding(&project.creation_project_id)
                .map_err(persistence_error)?
                .is_some()
            {
                continue;
            }
            if project.status != "installed" || project.target_type == "web_application" {
                continue;
            }
            let installation = self
                .persistence()?
                .personal_capability_installation(&project.creation_project_id)
                .map_err(persistence_error)?;
            if installation
                .is_some_and(|i| i.enabled && Some(i.package_digest) == project.package_digest)
            {
                items.push(json!({"creationProjectId":project.creation_project_id,"name":project.title,"description":project.business_goal,"packageDigest":project.package_digest}));
            }
        }
        Ok(json!({"items":items}))
    }
    pub(super) fn personal_source_root(
        &self,
        project: &CapabilityCreationProjectRecord,
    ) -> Result<PathBuf, ServiceError> {
        let root = self
            .capability_projects_root()?
            .join(&project.creation_project_id);
        reject_symlink(&root)?;
        let Some(digest) = &project.package_digest else {
            return Ok(root);
        };
        let build = self
            .persistence()?
            .capability_creation_build(&project.creation_project_id, digest)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_build_not_found"))?;
        let relative = Path::new(&build.package_relative_path)
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| ServiceError::new("capability_creation_package_invalid"))?;
        let directory = root.join(relative);
        reject_symlink(&directory)?;
        if !directory
            .canonicalize()
            .map_err(|_| ServiceError::new("capability_creation_source_unavailable"))?
            .starts_with(
                root.canonicalize()
                    .map_err(|_| ServiceError::new("capability_creation_source_unavailable"))?,
            )
        {
            return Err(ServiceError::new(
                "capability_creation_project_path_invalid",
            ));
        }
        Ok(directory)
    }

    pub(crate) fn personal_capability_editor(
        &mut self,
        params: &PersonalApplicationOpenParams,
        _: i64,
    ) -> Result<Value, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        if self
            .persistence()?
            .enterprise_capability_binding(&params.creation_project_id)
            .map_err(persistence_error)?
            .is_some()
        {
            return Err(ServiceError::new("enterprise_capability_not_personal"));
        }
        let project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        let versions = self
            .persistence()?
            .personal_capability_versions(&params.creation_project_id)
            .map_err(persistence_error)?;
        let draft = versions
            .first()
            .map_or(&project, |version| &version.project);
        let files = if draft.source_digest.is_some() {
            validate_installed_source(
                &self.personal_source_root(draft)?,
                &draft.target_type,
                draft.source_digest.as_deref().unwrap_or_default(),
            )?
            .into_iter()
            .map(|(path, content)| {
                Ok(json!({"path":path,"content":String::from_utf8(content)
                    .map_err(|_| ServiceError::new("capability_creation_source_invalid"))?}))
            })
            .collect::<Result<Vec<_>, ServiceError>>()?
        } else {
            Vec::new()
        };
        let installed = self
            .persistence()?
            .personal_capability_installation(&params.creation_project_id)
            .map_err(persistence_error)?;
        let permissions: Vec<String> = serde_json::from_str(&draft.requested_permissions_json)
            .map_err(|_| ServiceError::new("capability_creation_permissions_invalid"))?;
        let active_permissions: Vec<String> = if installed.is_some() {
            serde_json::from_str(&project.requested_permissions_json)
                .map_err(|_| ServiceError::new("capability_creation_permissions_invalid"))?
        } else {
            Vec::new()
        };
        let page_ai = self.personal_page_manifest(draft)?;
        let installed_page_ai = if installed.is_some() {
            self.personal_page_manifest(&project)?
        } else {
            None
        };
        Ok(
            json!({"project":project_from_record(project.clone())?,"sourceFiles":files,
            "pageAi":page_ai,"installedPageAi":installed_page_ai,
            "title":draft.title,"businessGoal":draft.business_goal,"requestedPermissions":permissions,
            "addedPermissions":permissions.iter().filter(|p| !active_permissions.contains(p)).collect::<Vec<_>>(),
            "installedDigest":installed.as_ref().map(|value| &value.package_digest),
            "candidateDigest":draft.package_digest,"candidateVersion":draft.package_digest.as_ref().map(|_|versions.first().map_or("0.1.0",|v|v.version.as_str())),
            "versions":versions.iter().map(|v| json!({"version":v.version,"packageDigest":v.build.package_digest,"createdAt":v.build.created_at})).collect::<Vec<_>>() }),
        )
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn personal_capability_build(
        &mut self,
        params: &PersonalBuildParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        if self
            .persistence()?
            .enterprise_capability_binding(&params.creation_project_id)
            .map_err(persistence_error)?
            .is_some()
        {
            return Err(ServiceError::new("enterprise_capability_not_personal"));
        }
        validate_text(
            &params.client_operation_id,
            160,
            "client_operation_id_invalid",
        )?;
        let fingerprint = fingerprint("build", params)?;
        let read = PersonalApplicationOpenParams {
            creation_project_id: params.creation_project_id.clone(),
        };
        if self
            .persistence()?
            .personal_lifecycle_operation(&params.client_operation_id, &fingerprint)
            .map_err(persistence_error)?
            .is_some()
        {
            return self.personal_capability_editor(&read, now);
        }
        let mut project = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        if project.state_revision != params.expected_revision {
            return Err(ServiceError::new("personal_capability_state_conflict"));
        }
        validate_text(&params.title, 160, "capability_creation_title_invalid")?;
        validate_text(
            &params.business_goal,
            2000,
            "capability_creation_business_goal_invalid",
        )?;
        pages::validate_permissions(&project.target_type, &params.requested_permissions)?;
        validate_source_files(&project.target_type, &params.source_files)?;
        pages::from_files(
            &project.target_type,
            &params.requested_permissions,
            &params.source_files,
        )?;
        if params.source_files.is_empty() {
            return Err(ServiceError::new(
                "capability_creation_source_entry_missing",
            ));
        }
        let versions = self
            .persistence()?
            .personal_capability_versions(&params.creation_project_id)
            .map_err(persistence_error)?;
        let version = format!("0.1.{}", versions.len() + 1);
        let root = self
            .capability_projects_root()?
            .join(&params.creation_project_id)
            .join("versions");
        reject_symlink(&root)?;
        fs::create_dir_all(&root)
            .map_err(|_| ServiceError::new("capability_creation_project_path_failed"))?;
        let staging = root.join(format!(".{}.staging", Uuid::new_v4().simple()));
        fs::create_dir(&staging)
            .map_err(|_| ServiceError::new("capability_creation_project_path_failed"))?;
        let destination = root.join(&version);
        let result = (|| {
            let built = materialize_source_package(
                &staging,
                &params.creation_project_id,
                &params.title,
                &params.business_goal,
                &project.target_type,
                &params.requested_permissions,
                &params.source_files,
                &version,
            )?;
            if destination.exists() {
                return Err(ServiceError::new("personal_capability_state_conflict"));
            }
            fs::rename(&staging, &destination)
                .map_err(|_| ServiceError::new("capability_creation_package_write_failed"))?;
            project.status = "packaged".into();
            project.title.clone_from(&params.title);
            project.business_goal.clone_from(&params.business_goal);
            project.requested_permissions_json =
                serde_json::to_string(&params.requested_permissions)
                    .map_err(|_| ServiceError::new("capability_creation_permissions_invalid"))?;
            project.package_digest = Some(built.package_digest.clone());
            project.source_digest = Some(built.source_digest.clone());
            project.source_file_count = built.source_file_count;
            let candidate = PersonalCapabilityVersion {
                version: version.clone(),
                project,
                build: CapabilityCreationBuildRecord {
                    build_id: format!("build_{}", Uuid::new_v4().simple()),
                    creation_project_id: params.creation_project_id.clone(),
                    source_digest: built.source_digest,
                    package_digest: built.package_digest,
                    package_relative_path: format!(
                        "versions/{version}/{}",
                        built.package_relative_path
                    ),
                    file_count: built.source_file_count,
                    total_bytes: built.total_bytes,
                    created_at: now,
                },
            };
            if let Err(error) = self.persistence_mut()?.save_personal_capability_version(
                &candidate,
                params.expected_revision,
                &params.client_operation_id,
                &fingerprint,
                now,
            ) {
                let _ = fs::remove_dir_all(&destination);
                return Err(persistence_error(error));
            }
            self.personal_capability_editor(&read, now)
        })();
        if staging.exists() {
            let _ = fs::remove_dir_all(staging);
        }
        result
    }

    fn materialize_upgrade(
        &self,
        params: &PersonalUpgradeParams,
        candidate: &PersonalCapabilityVersion,
        fingerprint: &str,
        now: i64,
    ) -> Result<super::MaterializedPersonalInstall, ServiceError> {
        let bytes = fs::read(
            self.capability_projects_root()?
                .join(&params.creation_project_id)
                .join(&candidate.build.package_relative_path),
        )
        .map_err(|_| ServiceError::new("capability_creation_package_unavailable"))?;
        if format!("sha256:{:x}", Sha256::digest(&bytes)) != params.package_digest {
            return Err(ServiceError::new(
                "capability_creation_package_digest_mismatch",
            ));
        }
        let install_params = CapabilityCreationInstallParams {
            creation_project_id: params.creation_project_id.clone(),
            package_digest: params.package_digest.clone(),
            expected_revision: params.expected_revision,
            client_operation_id: params.client_operation_id.clone(),
            request_fingerprint: fingerprint.into(),
        };
        materialize_personal_web_install(
            &self.personal_capabilities_root()?,
            &install_params,
            &candidate.project,
            &bytes,
            now,
        )
    }

    pub(crate) fn personal_capability_upgrade(
        &mut self,
        params: &PersonalUpgradeParams,
        now: i64,
    ) -> Result<Value, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        if self
            .persistence()?
            .enterprise_capability_binding(&params.creation_project_id)
            .map_err(persistence_error)?
            .is_some()
        {
            return Err(ServiceError::new("enterprise_capability_not_personal"));
        }
        validate_text(
            &params.client_operation_id,
            160,
            "client_operation_id_invalid",
        )?;
        let fingerprint = fingerprint("upgrade", params)?;
        let read = PersonalApplicationOpenParams {
            creation_project_id: params.creation_project_id.clone(),
        };
        if self
            .persistence()?
            .personal_lifecycle_operation(&params.client_operation_id, &fingerprint)
            .map_err(persistence_error)?
            .is_some()
        {
            return self.personal_capability_editor(&read, now);
        }
        let current = self
            .persistence()?
            .capability_creation_project(&params.creation_project_id)
            .map_err(persistence_error)?
            .ok_or_else(|| ServiceError::new("capability_creation_project_not_found"))?;
        let candidate =
            self.current_personal_candidate(&params.creation_project_id, &params.package_digest)?;
        let permissions: Vec<String> =
            serde_json::from_str(&candidate.project.requested_permissions_json)
                .map_err(|_| ServiceError::new("capability_creation_permissions_invalid"))?;
        if current.state_revision != params.expected_revision
            || permissions != params.confirmed_permissions
        {
            return Err(ServiceError::new("personal_capability_state_conflict"));
        }
        let install = self.materialize_upgrade(params, &candidate, &fingerprint, now)?;
        let data_params = params
            .data_change
            .as_ref()
            .ok_or_else(|| ServiceError::new("personal_application_data_operation_required"))?;
        let data = self.prepare_application_data_copy(
            data_params,
            &super::data::Activation {
                client: &params.client_operation_id,
                project: &params.creation_project_id,
                digest: &params.package_digest,
                revision: params.expected_revision,
                action: contexthub_protocol::PersonalApplicationDataAction::Upgrade,
            },
            now,
        )?;
        let result = self.persistence_mut()?.upgrade_personal_capability(
            &candidate,
            &CapabilityCreationInstallCommand {
                creation_project_id: params.creation_project_id.clone(),
                package_digest: params.package_digest.clone(),
                expected_revision: params.expected_revision,
                client_operation_id: params.client_operation_id.clone(),
                request_fingerprint: fingerprint,
                installed_relative_path: install.relative_location,
                occurred_at: now,
            },
            &data.operation.profile_id,
            &data.operation.operation_id,
        );
        if let Err(error) = result {
            self.fail_application_data(
                &data.operation.operation_id,
                "personal_application_data_activation_unconfirmed",
                now,
            )?;
            return Err(persistence_error(error));
        }
        self.personal_capability_editor(&read, now)
    }

    pub(crate) fn personal_capability_preview_runtime(
        &mut self,
        params: &PersonalVersionParams,
        _: i64,
    ) -> Result<Value, ServiceError> {
        validate_project_id(&params.creation_project_id)?;
        if self
            .persistence()?
            .enterprise_capability_binding(&params.creation_project_id)
            .map_err(persistence_error)?
            .is_some()
        {
            return Err(ServiceError::new("enterprise_capability_not_personal"));
        }
        let version =
            self.current_personal_candidate(&params.creation_project_id, &params.package_digest)?;
        let root = self.personal_source_root(&version.project)?;
        let files = validate_installed_source(
            &root,
            &version.project.target_type,
            &version.build.source_digest,
        )?;
        let package = fs::read(root.join("artifacts").join(format!(
            "{}.zip",
            params.package_digest.trim_start_matches("sha256:")
        )))
        .map_err(|_| ServiceError::new("capability_creation_package_unavailable"))?;
        if format!("sha256:{:x}", Sha256::digest(package)) != params.package_digest {
            return Err(ServiceError::new(
                "capability_creation_package_digest_mismatch",
            ));
        }
        let data = self
            .data_dir
            .as_ref()
            .ok_or_else(|| ServiceError::new("data_dir_unavailable"))?
            .join("personal-capability-preview-data");
        reject_symlink(&data)?;
        fs::create_dir_all(&data)
            .map_err(|_| ServiceError::new("personal_capability_data_unavailable"))?;
        let data = data
            .join(&params.creation_project_id)
            .join(params.package_digest.trim_start_matches("sha256:"));
        reject_symlink(&data)?;
        fs::create_dir_all(&data)
            .map_err(|_| ServiceError::new("personal_capability_data_unavailable"))?;
        Ok(
            json!({"packageRoot":root,"dataRoot":data,"packageDigest":params.package_digest,
            "html":files.get("web/index.html").map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
            "targetType":version.project.target_type,"requestedPermissions":serde_json::from_str::<Value>(&version.project.requested_permissions_json).map_err(|_| ServiceError::new("capability_creation_permissions_invalid"))?}),
        )
    }
}
