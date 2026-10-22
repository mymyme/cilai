use super::*;
use contexthub_persistence_sqlite::SqlitePersistence;
use std::process::{Command, Stdio};

const PROFILE: &str = "11111111111141118111111111111111";

struct Fixture {
    base: PathBuf,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("cilai-personal-import-{}", Uuid::new_v4()));
        let root = base
            .join("cn.cilai.personal.workbench/business/profiles")
            .join(PROFILE);
        fs::create_dir_all(&root).unwrap();
        let mut db = SqlitePersistence::open(&root.join("contexthub.db"), "import-test").unwrap();
        db.begin_personal_profile_upgrade(PROFILE, "import-test-fence")
            .unwrap();
        db.finish_personal_profile_upgrade(PROFILE, "import-test-fence", "manifest-digest")
            .unwrap();
        Self { base, root }
    }
    fn open(&self) -> AppServerServices {
        open(&self.root)
    }
    fn sql(&self, sql: &str) {
        rusqlite::Connection::open(self.root.join("contexthub.db"))
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }
    fn archive(
        &self,
        services: &mut AppServerServices,
        mcp: bool,
    ) -> (CapabilityCreationProject, PathBuf) {
        let mut request = CapabilityCreationCreateParams {
            title: "来源应用".into(),
            business_goal: "完整业务目标。".repeat(50),
            target_type: if mcp {
                "web_mcp_application"
            } else {
                "web_application"
            }
            .into(),
            conversation_id: Some("original-conversation".into()),
            requested_permissions: if mcp {
                vec![
                    "workbench_artifact.create".into(),
                    "personal_file.read".into(),
                ]
            } else {
                vec![]
            },
            acceptance_examples: vec![CapabilityCreationAcceptanceExample {
                input: "打开".into(),
                expected: "显示原有内容".into(),
            }],
            source_files: vec![CapabilityCreationSourceFile {
                path: "web/index.html".into(),
                content: "<h1>原有页面</h1>".into(),
            }],
            client_operation_id: Uuid::new_v4().to_string(),
            request_fingerprint: String::new(),
        };
        if mcp {
            request.source_files.push(CapabilityCreationSourceFile {
                path: "mcp/server.mjs".into(),
                content: "export const name = 'personal';".into(),
            });
        }
        request.request_fingerprint = capability_creation_request_fingerprint(&request);
        let project = services
            .capability_creation_create(&request, 10)
            .unwrap()
            .project;
        let exported = services
            .personal_package_export(
                &PersonalApplicationOpenParams {
                    creation_project_id: project.creation_project_id.clone(),
                },
                11,
            )
            .unwrap();
        let bytes: Vec<u8> = serde_json::from_value(exported["bytes"].clone()).unwrap();
        let path = self.base.join(format!("{}.zip", Uuid::new_v4()));
        fs::write(&path, bytes).unwrap();
        (project, path)
    }
    fn prepare(
        &self,
        services: &mut AppServerServices,
        archive: &Path,
    ) -> PersonalPackageImportPreview {
        assert!(archive.starts_with(&self.base));
        services
            .personal_package_import_prepare(&prepare(archive, "prepare-import"), 20)
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.base).unwrap();
    }
}

fn open(root: &Path) -> AppServerServices {
    let db = SqlitePersistence::open(&root.join("contexthub.db"), "import-test").unwrap();
    AppServerServices::with_data_dir(Some(db), Some(root.to_path_buf()))
}
fn prepare(path: &Path, operation: &str) -> PersonalPackageImportPrepareParams {
    PersonalPackageImportPrepareParams {
        archive_path: path.to_str().unwrap().into(),
        target: PersonalPackageImportTarget::New {},
        client_operation_id: operation.into(),
    }
}
fn commit(p: &PersonalPackageImportPreview) -> PersonalPackageImportCommitParams {
    PersonalPackageImportCommitParams {
        data_change: None,
        import_id: p.import_id.clone(),
        preview_digest: p.preview_digest.clone(),
        confirmed_permissions: p.requested_permissions.clone(),
        client_operation_id: "commit-import".into(),
    }
}
fn cancel(p: &PersonalPackageImportPreview) -> PersonalPackageImportCancelParams {
    PersonalPackageImportCancelParams {
        import_id: p.import_id.clone(),
        preview_digest: p.preview_digest.clone(),
    }
}
fn project(services: &AppServerServices, id: &str) -> CapabilityCreationProjectRecord {
    services
        .persistence()
        .unwrap()
        .capability_creation_project(id)
        .unwrap()
        .unwrap()
}
fn install(services: &mut AppServerServices, p: &CapabilityCreationProject) {
    let params = json!({"creationProjectId":p.creation_project_id,"expectedRevision":p.state_revision,"packageDigest":p.package_digest,"confirmedPermissions":p.requested_permissions,"clientOperationId":Uuid::new_v4().to_string()});
    super::super::data::test_support::upgrade(services, params, "import-test-fence", 15).unwrap();
}
fn version_count(services: &AppServerServices, id: &str) -> usize {
    services
        .persistence()
        .unwrap()
        .personal_capability_versions(id)
        .unwrap()
        .len()
}
fn installation(
    services: &AppServerServices,
    id: &str,
) -> contexthub_persistence_sqlite::PersonalCapabilityInstallationRecord {
    services
        .persistence()
        .unwrap()
        .personal_capability_installation(id)
        .unwrap()
        .unwrap()
}

#[test]
fn import_creates_new_local_identity_without_claiming_source_ownership_or_acceptance() {
    let files = Fixture::new();
    let mut services = files.open();
    let (source, archive) = files.archive(&mut services, false);
    let preview = files.prepare(&mut services, &archive);
    assert_ne!(
        preview.local_creation_project_id,
        source.creation_project_id
    );
    assert!(!preview.source.authenticated);
    assert!(
        !preview.source.full_business_goal_present && !preview.source.acceptance_examples_present
    );
    assert_eq!(preview.source.summary.chars().count(), 200);
    assert_eq!(preview.source.version, "0.1.0");
    assert_eq!(preview.local_version, "0.1.0");
    assert_ne!(preview.local_package_digest, preview.source.archive_digest);
    assert_eq!(preview.source.source_digest, source.source_digest.unwrap());
    assert!(
        services
            .persistence()
            .unwrap()
            .capability_creation_project(&preview.local_creation_project_id)
            .unwrap()
            .is_none()
    );
    let installed = super::super::data::test_support::import_commit(
        &mut services,
        &commit(&preview),
        "import-test-fence",
        30,
    )
    .unwrap();
    assert_eq!(
        installed.project.creation_project_id,
        preview.local_creation_project_id
    );
    assert_eq!(installed.project.business_goal, preview.source.summary);
    assert!(installed.project.acceptance_examples.is_empty());
    assert!(installed.project.conversation_id.is_none());
    assert_eq!(installed.project.status, "installed");
    assert_eq!(
        services
            .personal_application_open(
                &PersonalApplicationOpenParams {
                    creation_project_id: installed.project.creation_project_id.clone()
                },
                31
            )
            .unwrap()
            .html,
        "<h1>原有页面</h1>"
    );
    assert_eq!(
        project(&services, &source.creation_project_id).status,
        "packaged"
    );
    let exported = services
        .personal_package_export(
            &PersonalApplicationOpenParams {
                creation_project_id: installed.project.creation_project_id,
            },
            31,
        )
        .unwrap();
    assert_eq!(exported["packageDigest"], preview.local_package_digest);
}

#[test]
fn preparation_and_receipt_replays_use_locked_candidates_after_the_original_archive_changes() {
    let files = Fixture::new();
    let mut services = files.open();
    let (_, archive) = files.archive(&mut services, true);
    let request = prepare(&archive, "prepare-import");
    let preview = services
        .personal_package_import_prepare(&request, 20)
        .unwrap();
    fs::write(&archive, b"archive changed after explicit preview").unwrap();
    assert_eq!(
        services
            .personal_package_import_prepare(&request, 21)
            .unwrap(),
        preview
    );
    let mut changed = request;
    changed.archive_path.push_str(".changed");
    assert_eq!(
        services
            .personal_package_import_prepare(&changed, 21)
            .unwrap_err()
            .reason,
        "operation_id_reused"
    );
    drop(services);
    let mut reopened = files.open();
    assert_eq!(
        reopened
            .personal_package_import_list(&PersonalPackageImportListParams {}, 21)
            .unwrap(),
        vec![preview.clone()]
    );
    let installed = super::super::data::test_support::import_commit(
        &mut reopened,
        &commit(&preview),
        "import-test-fence",
        30,
    )
    .unwrap();
    assert!(!installed.replayed);
    fs::remove_dir_all(reopened.package_import_root(&preview.import_id).unwrap()).unwrap();
    let replayed = super::super::data::test_support::import_commit(
        &mut reopened,
        &commit(&preview),
        "import-test-fence",
        31,
    )
    .unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.project, installed.project);
    assert_eq!(
        reopened
            .personal_package_import_get(
                &PersonalPackageImportParams {
                    import_id: preview.import_id.clone()
                },
                31
            )
            .unwrap()
            .preview_digest,
        preview.preview_digest
    );
    assert_eq!(
        reopened
            .personal_package_import_cancel(&cancel(&preview), 32)
            .unwrap_err()
            .reason,
        "personal_package_import_already_committed"
    );
    let mut changed_commit = commit(&preview);
    changed_commit.client_operation_id = "different-commit".into();
    assert_eq!(
        reopened
            .personal_package_import_commit(&changed_commit, 32)
            .unwrap_err()
            .reason,
        "personal_package_import_already_committed"
    );
}

#[test]
fn confirmation_is_bound_to_the_exact_preview_and_permission_set_and_cancel_is_terminal() {
    let files = Fixture::new();
    let mut services = files.open();
    let (_, archive) = files.archive(&mut services, true);
    let preview = files.prepare(&mut services, &archive);
    assert_eq!(preview.added_permissions, preview.requested_permissions);
    for permissions in [
        vec![],
        vec!["personal_file.read".into()],
        vec!["personal_file.read".into(), "personal_file.read".into()],
        vec!["shell.execute".into()],
    ] {
        let mut invalid = commit(&preview);
        invalid.confirmed_permissions = permissions;
        assert_eq!(
            services
                .personal_package_import_commit(&invalid, 30)
                .unwrap_err()
                .reason,
            "personal_package_import_permissions_unconfirmed"
        );
    }
    let mut changed = commit(&preview);
    changed.preview_digest = "unconfirmed-preview".into();
    assert_eq!(
        services
            .personal_package_import_commit(&changed, 30)
            .unwrap_err()
            .reason,
        "personal_package_import_preview_changed"
    );
    let cancelled = services
        .personal_package_import_cancel(&cancel(&preview), 30)
        .unwrap();
    assert_eq!(cancelled.state, PersonalPackageImportState::Cancelled);
    assert_eq!(cancelled.preview_digest, preview.preview_digest);
    assert_eq!(
        services
            .personal_package_import_cancel(&cancel(&preview), 31)
            .unwrap()
            .state,
        PersonalPackageImportState::Cancelled
    );
    assert_eq!(
        services
            .personal_package_import_commit(&commit(&preview), 32)
            .unwrap_err()
            .reason,
        "personal_package_import_cancelled"
    );
    assert!(
        services
            .persistence()
            .unwrap()
            .capability_creation_project(&preview.local_creation_project_id)
            .unwrap()
            .is_none()
    );
}

fn modify_zip(archive: &Path, change: impl FnOnce(&mut BTreeMap<String, Vec<u8>>)) {
    let mut contents = decode_archive(&fs::read(archive).unwrap()).unwrap();
    change(&mut contents);
    fs::write(archive, encode_package(&contents).unwrap()).unwrap();
}

#[test]
fn untrusted_manifests_and_unknown_data_migrations_are_refused_before_any_local_registration() {
    let files = Fixture::new();
    let mut services = files.open();
    let (_, original) = files.archive(&mut services, true);
    for (index, field) in [
        "tenantId",
        "enterpriseAccount",
        "migrations",
        "dataMigration",
    ]
    .iter()
    .enumerate()
    {
        let archive = files.base.join(format!("invalid-{index}.zip"));
        fs::copy(&original, &archive).unwrap();
        modify_zip(&archive, |contents| {
            let path = "generated/plugin-manifest.json";
            let mut plugin: Value = serde_json::from_slice(&contents[path]).unwrap();
            plugin[field] = json!("untrusted");
            contents.insert(path.into(), encode_json(&plugin).unwrap());
        });
        let error = services
            .personal_package_import_prepare(&prepare(&archive, &format!("invalid-{index}")), 20)
            .unwrap_err();
        assert_eq!(
            error.reason,
            if index > 1 {
                "personal_package_import_data_migration_unsupported"
            } else {
                "personal_package_import_manifest_invalid"
            }
        );
    }
    assert!(
        services
            .personal_package_import_list(&PersonalPackageImportListParams {}, 21)
            .unwrap()
            .is_empty()
    );
}

fn zip_entries(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, content) in entries {
        writer
            .start_file(*path, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn corrupt_archives_paths_duplicates_and_oversized_entries_never_escape_managed_storage() {
    let files = Fixture::new();
    let mut services = files.open();
    let bad = [
        b"not a zip".to_vec(),
        zip_entries(&[("../../outside.txt", b"escape".to_vec())]),
        zip_entries(&[("/outside.txt", b"escape".to_vec())]),
        zip_entries(&[("source\\web\\index.html", b"escape".to_vec())]),
        zip_entries(&[
            ("source/web/index.html", b"a".to_vec()),
            ("source/Web/index.html", b"b".to_vec()),
        ]),
        zip_entries(&[(
            "source/web/index.html",
            vec![b'a'; MAX_SOURCE_FILE_BYTES + 1],
        )]),
        vec![0; MAX_ARCHIVE_BYTES + 1],
    ];
    for (index, bytes) in bad.iter().enumerate() {
        let path = files.base.join(format!("corrupt-{index}.zip"));
        fs::write(&path, bytes).unwrap();
        assert!(
            services
                .personal_package_import_prepare(&prepare(&path, &format!("corrupt-{index}")), 20)
                .is_err()
        );
    }
    let mut request = prepare(Path::new("relative.zip"), "relative");
    assert_eq!(
        services
            .personal_package_import_prepare(&request, 20)
            .unwrap_err()
            .reason,
        "personal_package_import_archive_path_invalid"
    );
    request.archive_path = files.base.to_str().unwrap().into();
    assert_eq!(
        services
            .personal_package_import_prepare(&request, 20)
            .unwrap_err()
            .reason,
        "personal_package_import_archive_unavailable"
    );
    assert!(!files.base.join("outside.txt").exists());
    assert!(
        services
            .personal_package_import_list(&PersonalPackageImportListParams {}, 21)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tampered_locked_archive_or_local_candidate_is_refused_without_installing() {
    for locked_archive in [true, false] {
        let files = Fixture::new();
        let mut services = files.open();
        let (_, archive) = files.archive(&mut services, false);
        let preview = files.prepare(&mut services, &archive);
        let root = services.package_import_root(&preview.import_id).unwrap();
        let path = if locked_archive {
            root.join("archive.zip")
        } else {
            root.join("candidate/artifacts").join(format!(
                "{}.zip",
                preview.local_package_digest.trim_start_matches("sha256:")
            ))
        };
        fs::write(path, b"tampered candidate").unwrap();
        assert!(
            services
                .personal_package_import_commit(&commit(&preview), 30)
                .is_err()
        );
        assert!(
            services
                .persistence()
                .unwrap()
                .personal_capability_installation(&preview.local_creation_project_id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            services
                .import_record(&preview.import_id)
                .unwrap()
                .preview
                .state,
            PersonalPackageImportState::Prepared
        );
    }
}

#[cfg(unix)]
#[test]
fn symlink_archives_and_candidate_destinations_are_refused_without_touching_the_target() {
    use std::os::unix::fs::symlink;
    let files = Fixture::new();
    let mut services = files.open();
    let (_, archive) = files.archive(&mut services, false);
    let link = files.base.join("link.zip");
    symlink(&archive, &link).unwrap();
    assert!(
        services
            .personal_package_import_prepare(&prepare(&link, "link"), 20)
            .is_err()
    );
    let preview = files.prepare(&mut services, &archive);
    let outside = files.base.join("unrelated");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("original.txt"), "unchanged").unwrap();
    symlink(
        &outside,
        files
            .root
            .join("capability-projects")
            .join(&preview.local_creation_project_id),
    )
    .unwrap();
    assert!(
        services
            .personal_package_import_commit(&commit(&preview), 30)
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(outside.join("original.txt")).unwrap(),
        "unchanged"
    );
    assert_eq!(fs::read_dir(outside).unwrap().count(), 1);
}

fn upgrade_preview(
    files: &Fixture,
    services: &mut AppServerServices,
    target: &CapabilityCreationProject,
    archive: &Path,
) -> PersonalPackageImportPreview {
    services
        .personal_package_import_prepare(
            &PersonalPackageImportPrepareParams {
                archive_path: archive.to_str().unwrap().into(),
                target: PersonalPackageImportTarget::Upgrade {
                    creation_project_id: target.creation_project_id.clone(),
                    expected_revision: project(services, &target.creation_project_id)
                        .state_revision,
                },
                client_operation_id: "prepare-upgrade".into(),
            },
            20,
        )
        .unwrap_or_else(|e| panic!("{} at {}", e.reason, files.root.display()))
}

#[test]
fn explicit_upgrade_preserves_original_goal_acceptance_conversation_data_and_disabled_state() {
    let files = Fixture::new();
    let mut services = files.open();
    let (target, original_archive) = files.archive(&mut services, true);
    install(&mut services, &target);
    let (source, update) = files.archive(&mut services, true);
    modify_zip(&update, |contents| {
        let id = source.creation_project_id.clone();
        let source = BTreeMap::from([
            ("web/index.html".into(), b"<h1>updated</h1>".to_vec()),
            (
                "mcp/server.mjs".into(),
                b"export const name='new';".to_vec(),
            ),
        ]);
        *contents = generate_personal_manifests(
            &id,
            "更新来源",
            "来源只有简短摘要",
            "web_mcp_application",
            &["personal_file.write_confirmed".into()],
            &format!("sha256:{}", digest_files(&source)),
            "3.2.1",
            None,
        )
        .unwrap();
        contents.extend(source.into_iter().map(|(p, c)| (format!("source/{p}"), c)));
    });
    files.sql(&format!(
        "UPDATE personal_capability_installations SET enabled=0 WHERE creation_project_id='{}';",
        target.creation_project_id
    ));
    let old = project(&services, &target.creation_project_id);
    let old_install = installation(&services, &target.creation_project_id);
    let data = services
        .personal_application_data_runtime_root(
            &target.creation_project_id,
            &installation(&services, &target.creation_project_id).package_digest,
            16,
        )
        .unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("saved.txt"), "用户旧数据").unwrap();
    let preview = upgrade_preview(&files, &mut services, &target, &update);
    assert_eq!(preview.source.version, "3.2.1");
    assert_eq!(preview.local_version, "0.1.1");
    assert_eq!(
        preview.added_permissions,
        vec!["personal_file.write_confirmed"]
    );
    assert_eq!(preview.installed_digest, old.package_digest);
    let installed = super::super::data::test_support::import_commit(
        &mut services,
        &commit(&preview),
        "import-test-fence",
        30,
    )
    .unwrap();
    assert_eq!(installed.project.business_goal, old.business_goal);
    assert_eq!(installed.project.conversation_id, old.conversation_id);
    assert_eq!(
        serde_json::to_string(&installed.project.acceptance_examples).unwrap(),
        old.acceptance_examples_json
    );
    assert_eq!(installed.project.created_at, old.created_at);
    assert!(!installation(&services, &target.creation_project_id).enabled);
    assert_eq!(
        fs::read_to_string(data.join("saved.txt")).unwrap(),
        "用户旧数据"
    );
    assert!(
        files
            .root
            .join(old_install.installed_relative_path)
            .is_dir()
    );
    assert!(original_archive.is_file());
    let exported = services
        .personal_package_export(
            &PersonalApplicationOpenParams {
                creation_project_id: target.creation_project_id.clone(),
            },
            31,
        )
        .unwrap();
    let bytes: Vec<u8> = serde_json::from_value(exported["bytes"].clone()).unwrap();
    assert_eq!(
        inspect(&bytes).unwrap().source.summary,
        old.business_goal.chars().take(200).collect::<String>()
    );
}

#[test]
fn receipt_failure_preserves_old_data_and_requires_a_new_explicit_operation() {
    let files = Fixture::new();
    let mut services = files.open();
    let (target, _) = files.archive(&mut services, false);
    install(&mut services, &target);
    let (_, archive) = files.archive(&mut services, false);
    let before = project(&services, &target.creation_project_id);
    let before_install = installation(&services, &target.creation_project_id);
    let data = services
        .personal_application_data_runtime_root(
            &target.creation_project_id,
            &installation(&services, &target.creation_project_id).package_digest,
            16,
        )
        .unwrap();
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("saved.txt"), "原有可用数据").unwrap();
    let preview = upgrade_preview(&files, &mut services, &target, &archive);
    files.sql("CREATE TRIGGER fail_import_receipt BEFORE UPDATE OF state ON personal_package_imports WHEN NEW.state='committed' BEGIN SELECT RAISE(ABORT,'controlled receipt failure'); END;");
    assert!(
        super::super::data::test_support::import_commit(
            &mut services,
            &commit(&preview),
            "import-test-fence",
            30
        )
        .is_err()
    );
    assert_eq!(
        project_from_record(project(&services, &target.creation_project_id)).unwrap(),
        project_from_record(before).unwrap()
    );
    assert_eq!(
        installation(&services, &target.creation_project_id).package_digest,
        before_install.package_digest
    );
    assert_eq!(version_count(&services, &target.creation_project_id), 0);
    assert_eq!(
        fs::read_to_string(data.join("saved.txt")).unwrap(),
        "原有可用数据"
    );
    assert_eq!(
        services
            .personal_application_open(
                &PersonalApplicationOpenParams {
                    creation_project_id: target.creation_project_id.clone()
                },
                30
            )
            .unwrap()
            .html,
        "<h1>原有页面</h1>"
    );
    assert_eq!(
        services
            .import_record(&preview.import_id)
            .unwrap()
            .preview
            .state,
        PersonalPackageImportState::Prepared
    );
    drop(services);
    files.sql("DROP TRIGGER fail_import_receipt;");
    let mut reopened = files.open();
    assert_eq!(
        super::super::data::test_support::import_commit(
            &mut reopened,
            &commit(&preview),
            "import-test-fence",
            31
        )
        .unwrap_err()
        .reason,
        "personal_application_data_operation_unconfirmed"
    );
    let mut next = commit(&preview);
    next.client_operation_id = "explicit-new-import-commit".into();
    assert!(
        !super::super::data::test_support::import_commit(
            &mut reopened,
            &next,
            "import-test-fence",
            31
        )
        .unwrap()
        .replayed
    );
    assert_eq!(
        reopened
            .persistence()
            .unwrap()
            .personal_capability_versions(&target.creation_project_id)
            .unwrap()
            .len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn relocated_candidate_ancestors_are_rejected_even_when_the_artifact_bytes_are_identical() {
    use std::os::unix::fs::symlink;
    let files = Fixture::new();
    let mut services = files.open();
    let (_, archive) = files.archive(&mut services, false);
    let preview = files.prepare(&mut services, &archive);
    let candidate = services
        .package_import_root(&preview.import_id)
        .unwrap()
        .join("candidate");
    let outside = files.base.join("relocated-candidate");
    fs::rename(&candidate, &outside).unwrap();
    symlink(&outside, &candidate).unwrap();
    assert_eq!(
        services
            .personal_package_import_commit(&commit(&preview), 30)
            .unwrap_err()
            .reason,
        "capability_creation_project_path_invalid"
    );
    assert!(
        services
            .persistence()
            .unwrap()
            .personal_capability_installation(&preview.local_creation_project_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn upgraded_packages_revoke_old_version_file_grants_and_remain_editable_and_exportable() {
    let files = Fixture::new();
    let mut services = files.open();
    let (target, archive) = files.archive(&mut services, true);
    install(&mut services, &target);
    let selected = files.base.join("selected.txt");
    fs::write(&selected, "用户明确选择的内容").unwrap();
    let selection = serde_json::from_value(json!({"creationProjectId":target.creation_project_id,"packageDigest":target.package_digest,"path":selected})).unwrap();
    let grant = services.personal_file_select(&selection, 16).unwrap();
    let preview = upgrade_preview(&files, &mut services, &target, &archive);
    super::super::data::test_support::import_commit(
        &mut services,
        &commit(&preview),
        "import-test-fence",
        30,
    )
    .unwrap();
    let access = serde_json::from_value(json!({"creationProjectId":target.creation_project_id,"packageDigest":target.package_digest,"grantId":grant["grantId"]})).unwrap();
    assert_eq!(
        services
            .personal_file_consume(&access, 31)
            .unwrap_err()
            .reason,
        "personal_capability_state_conflict"
    );
    let read = PersonalApplicationOpenParams {
        creation_project_id: target.creation_project_id.clone(),
    };
    let editor = services.personal_capability_editor(&read, 32).unwrap();
    assert_eq!(editor["candidateVersion"], "0.1.1");
    assert_eq!(editor["sourceFiles"].as_array().unwrap().len(), 2);
    let request = serde_json::from_value(json!({"creationProjectId":target.creation_project_id,"expectedRevision":project(&services, &target.creation_project_id).state_revision,"clientOperationId":"edit-imported","title":"本地修改标题","businessGoal":"用户本地修改的目标","requestedPermissions":["personal_file.read"],"sourceFiles":editor["sourceFiles"]})).unwrap();
    let edited = services.personal_capability_build(&request, 33).unwrap();
    assert_eq!(edited["candidateVersion"], "0.1.2");
    assert_eq!(edited["installedDigest"], preview.local_package_digest);
    assert_eq!(fs::read_to_string(selected).unwrap(), "用户明确选择的内容");
}

#[test]
fn stale_upgrade_and_arbitrary_source_identity_cannot_replace_an_existing_installation() {
    let files = Fixture::new();
    let mut services = files.open();
    let (target, archive) = files.archive(&mut services, false);
    install(&mut services, &target);
    let preview = upgrade_preview(&files, &mut services, &target, &archive);
    files.sql(&format!("UPDATE capability_creation_projects SET state_revision=state_revision+1 WHERE creation_project_id='{}';", target.creation_project_id));
    assert_eq!(
        services
            .personal_package_import_commit(&commit(&preview), 30)
            .unwrap_err()
            .reason,
        "personal_package_import_target_changed"
    );
    assert_eq!(
        installation(&services, &target.creation_project_id).package_digest,
        target.package_digest.unwrap()
    );
    assert!(
        services
            .personal_package_import_get(
                &PersonalPackageImportParams {
                    import_id: "../../escape".into()
                },
                30
            )
            .is_err()
    );
}

fn guarded_import_candidate(
    services: &mut AppServerServices,
    preview: &PersonalPackageImportPreview,
) -> contexthub_protocol::PersonalApplicationDataExecuteParams {
    let data = super::super::data::test_support::guarded(
        services,
        &contexthub_protocol::PersonalApplicationDataPrepareParams {
            creation_project_id: preview.local_creation_project_id.clone(),
            expected_revision: 1,
            package_digest: preview.local_package_digest.clone(),
            client_operation_id: "racing-commit".into(),
            action: contexthub_protocol::PersonalApplicationDataAction::ImportUpgrade,
            target_operation_id: None,
        },
        "import-test-fence",
    );
    services
        .prepare_application_data_copy(
            &data,
            &super::super::data::Activation {
                client: "racing-commit",
                project: &preview.local_creation_project_id,
                digest: &preview.local_package_digest,
                revision: 1,
                action: contexthub_protocol::PersonalApplicationDataAction::ImportUpgrade,
            },
            30,
        )
        .unwrap();
    data
}

#[test]
fn two_sqlite_writers_serialize_commit_and_cancel_to_one_consistent_terminal_state() {
    let files = Fixture::new();
    let mut services = files.open();
    let (_, archive) = files.archive(&mut services, false);
    let preview = files.prepare(&mut services, &archive);
    let record = services.import_record(&preview.import_id).unwrap();
    let (version, _) = services.verified_import_candidate(&record, 30).unwrap();
    let data = guarded_import_candidate(&mut services, &preview);
    let result = json!({"importId":preview.import_id,"localPackageDigest":preview.local_package_digest,"project":project_from_record(version.project.clone()).unwrap(),"replayed":false});
    let path = files.root.join("contexthub.db");
    let mut first = SqlitePersistence::open(&path, "import-test").unwrap();
    let mut second = SqlitePersistence::open(&path, "import-test").unwrap();
    let barrier = std::sync::Barrier::new(2);
    let installed_path = format!(
        "personal-capabilities/{}/{}",
        preview.local_creation_project_id,
        preview.local_package_digest.trim_start_matches("sha256:")
    );
    let (committed, cancelled) = std::thread::scope(|scope| {
        let barrier = &barrier;
        let a = scope.spawn(|| {
            barrier.wait();
            first.commit_personal_package_import(&PersonalPackageImportCommand {
                data_operation_id: &data.operation_id,
                profile_id: PROFILE,
                import_id: &preview.import_id,
                preview_digest: &preview.preview_digest,
                client_operation_id: "racing-commit",
                fingerprint: "racing-fingerprint",
                version: &version,
                installed_relative_path: &installed_path,
                result: &result,
                occurred_at: 30,
            })
        });
        let b = scope.spawn(|| {
            barrier.wait();
            second.cancel_personal_package_import(
                PROFILE,
                &preview.import_id,
                &preview.preview_digest,
                30,
            )
        });
        (a.join().unwrap(), b.join().unwrap())
    });
    if committed.is_err() {
        services
            .fail_application_data(
                &data.operation_id,
                "personal_application_data_activation_unconfirmed",
                31,
            )
            .unwrap();
    }
    super::super::data::test_support::release(&mut services, &data, "import-test-fence");
    let current = services.import_record(&preview.import_id).unwrap();
    match current.preview.state {
        PersonalPackageImportState::Committed => {
            assert!(committed.is_ok());
            assert_eq!(
                cancelled.unwrap_err().reason(),
                Some("personal_package_import_already_committed")
            );
            assert_eq!(
                installation(&services, &preview.local_creation_project_id).package_digest,
                preview.local_package_digest
            );
            assert_eq!(current.result.unwrap(), result);
        }
        PersonalPackageImportState::Cancelled => {
            assert_eq!(
                committed.unwrap_err().reason(),
                Some("personal_package_import_cancelled")
            );
            assert!(cancelled.is_ok());
            assert!(
                services
                    .persistence()
                    .unwrap()
                    .capability_creation_project(&preview.local_creation_project_id)
                    .unwrap()
                    .is_none()
            );
            assert!(current.result.is_none());
        }
        PersonalPackageImportState::Prepared => panic!("race must reach a terminal state"),
    }
}

#[test]
fn concurrent_preparation_with_the_same_operation_keeps_one_frozen_identity() {
    let files = Fixture::new();
    let mut services = files.open();
    let (_, archive) = files.archive(&mut services, false);
    let preview = files.prepare(&mut services, &archive);
    let original = services.import_record(&preview.import_id).unwrap();
    files.sql("DELETE FROM personal_package_imports;");
    let mut other = original.clone();
    other.preview.import_id = format!("import_{}", Uuid::new_v4().simple());
    other.preview.local_creation_project_id = format!("capability_{}", Uuid::new_v4().simple());
    other.preview.preview_digest = preview_digest(&other.preview).unwrap();
    other.candidate_relative_path = format!(
        "personal-package-imports/{}/candidate",
        other.preview.import_id
    );
    let path = files.root.join("contexthub.db");
    let mut first = SqlitePersistence::open(&path, "import-test").unwrap();
    let mut second = SqlitePersistence::open(&path, "import-test").unwrap();
    let barrier = std::sync::Barrier::new(2);
    let (a, b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            first.prepare_personal_package_import(&original).unwrap()
        });
        let b = scope.spawn(|| {
            barrier.wait();
            second.prepare_personal_package_import(&other).unwrap()
        });
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(a.preview, b.preview);
    assert_eq!(
        services
            .personal_package_import_list(&PersonalPackageImportListParams {}, 30)
            .unwrap(),
        vec![a.preview]
    );
    other.prepare_fingerprint = "different-request".into();
    assert_eq!(
        first
            .prepare_personal_package_import(&other)
            .unwrap_err()
            .reason(),
        Some("operation_id_reused")
    );
}

#[test]
fn profile_and_install_target_changes_are_rechecked_in_the_commit_transaction() {
    let files = Fixture::new();
    let mut services = files.open();
    let (_, archive) = files.archive(&mut services, false);
    let preview = files.prepare(&mut services, &archive);
    let record = services.import_record(&preview.import_id).unwrap();
    let mut db = SqlitePersistence::open(&files.root.join("contexthub.db"), "import-test").unwrap();
    assert!(
        db.personal_package_import("22222222222242228222222222222222", &preview.import_id)
            .is_err()
    );
    files.sql("UPDATE personal_profile_compatibility SET state='pending';");
    assert_eq!(
        db.prepare_personal_package_import(&record)
            .unwrap_err()
            .reason(),
        Some("personal_package_import_profile_not_ready")
    );
    assert_eq!(
        services
            .personal_package_import_commit(&commit(&preview), 30)
            .unwrap_err()
            .reason,
        "personal_package_import_profile_not_ready"
    );
    assert!(
        services
            .persistence()
            .unwrap()
            .personal_capability_installation(&preview.local_creation_project_id)
            .unwrap()
            .is_none()
    );
}

fn child(files: &Fixture, mode: &str, preview: &PersonalPackageImportPreview) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.env_clear();
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    let output = command
        .args([
            "--ignored",
            "--exact",
            "capability_creation::imports::tests::interrupted_package_import_probe",
        ])
        .env("CILAI_PERSONAL_IMPORT_TEST_ROOT", &files.root)
        .env("CILAI_PERSONAL_IMPORT_TEST_MODE", mode)
        .env("CILAI_PERSONAL_IMPORT_TEST_ID", &preview.import_id)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(86),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn actual_process_exit_before_and_after_database_commit_recovers_without_replaying_installation() {
    let files = Fixture::new();
    let mut services = files.open();
    let (_, archive) = files.archive(&mut services, false);
    let preview = files.prepare(&mut services, &archive);
    drop(services);
    child(&files, "before_commit", &preview);
    let services = files.open();
    assert!(
        services
            .persistence()
            .unwrap()
            .personal_capability_installation(&preview.local_creation_project_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        services
            .import_record(&preview.import_id)
            .unwrap()
            .preview
            .state,
        PersonalPackageImportState::Prepared
    );
    drop(services);
    child(&files, "after_commit", &preview);
    let mut services = files.open();
    let installed = super::super::data::test_support::import_commit(
        &mut services,
        &commit(&preview),
        "import-test-fence",
        32,
    )
    .unwrap();
    assert!(installed.replayed);
    assert_eq!(installed.project.state_revision, 2);
    assert_eq!(
        services
            .import_record(&preview.import_id)
            .unwrap()
            .preview
            .state,
        PersonalPackageImportState::Committed
    );
}

#[test]
#[ignore = "由进程中断测试在独立个人临时资料中调用"]
fn interrupted_package_import_probe() {
    let root = PathBuf::from(std::env::var_os("CILAI_PERSONAL_IMPORT_TEST_ROOT").unwrap());
    let mode = std::env::var("CILAI_PERSONAL_IMPORT_TEST_MODE").unwrap();
    let id = std::env::var("CILAI_PERSONAL_IMPORT_TEST_ID").unwrap();
    let mut services = open(&root);
    let record = services.import_record(&id).unwrap();
    if mode == "before_commit" {
        let (version, bytes) = services.verified_import_candidate(&record, 30).unwrap();
        services
            .publish_import_candidate(&record, &version, &bytes)
            .unwrap();
    } else {
        super::super::data::test_support::import_commit(
            &mut services,
            &commit(&record.preview),
            "import-test-fence",
            30,
        )
        .unwrap();
    }
    std::process::exit(86);
}
