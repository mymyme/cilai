use super::*;
use std::process::{Command, Stdio};

struct TestFiles {
    root: PathBuf,
}

impl TestFiles {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("cilai-personal-skill-storage-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn open(&self) -> AppServerServices {
        let db = contexthub_persistence_sqlite::SqlitePersistence::open(
            &self.root.join("contexthub.db"),
            "personal-skill-test",
        )
        .unwrap();
        AppServerServices::with_data_dir(Some(db), Some(self.root.clone()))
    }

    fn path(&self, services: &AppServerServices, skill_id: &str) -> PathBuf {
        let record = services
            .persistence()
            .unwrap()
            .user_skill(skill_id)
            .unwrap()
            .unwrap();
        self.root.join("skills").join(record.managed_relative_path)
    }

    fn sql(&self, sql: &str) {
        rusqlite::Connection::open(self.root.join("contexthub.db"))
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }
}

impl Drop for TestFiles {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn save_request(
    skill: Option<&SkillDetail>,
    operation: &str,
    instructions: &str,
) -> SkillSaveParams {
    SkillSaveParams {
        skill_id: skill.map(|s| s.skill_id.clone()),
        name: "durable-skill".into(),
        description: "Personal skill package".into(),
        instructions: instructions.into(),
        files: skill.is_none().then(|| {
            vec![SkillPackageFileInput {
                path: "references/nested/guide.md".into(),
                content: "原有参考内容".into(),
            }]
        }),
        enabled: skill.is_some_and(|s| s.enabled),
        automatic: skill.is_some_and(|s| s.automatic),
        expected_revision: skill.map(|s| s.state_revision),
        client_operation_id: operation.into(),
        request_fingerprint: sha256_hex(operation.as_bytes()),
    }
}

fn detail(services: &mut AppServerServices, skill: &SkillDetail) -> SkillDetail {
    services
        .skill_get(&SkillGetParams {
            skill_id: skill.skill_id.clone(),
        })
        .unwrap()
        .skill
}

#[test]
fn sqlite_failure_after_pointer_update_preserves_the_original_package_and_state() {
    let files = TestFiles::new();
    let mut services = files.open();
    let original = services
        .skill_save(&save_request(None, "original", "原有技能正文"), 1)
        .unwrap()
        .skill;
    let path = files.path(&services, &original.skill_id);
    let bytes = fs::read(&path).unwrap();
    files.sql("CREATE TRIGGER fail_skill_receipt BEFORE INSERT ON user_skill_operations WHEN NEW.client_operation_id='failed-upgrade' BEGIN SELECT RAISE(ABORT,'controlled receipt failure'); END;");
    let error = services
        .skill_save(
            &save_request(Some(&original), "failed-upgrade", "未提交的新正文"),
            2,
        )
        .unwrap_err();
    assert_eq!(error.reason, "skill_persistence_failed");
    assert_eq!(files.path(&services, &original.skill_id), path);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(detail(&mut services, &original), original);
    drop(services);
    let mut reopened = files.open();
    assert_eq!(detail(&mut reopened, &original), original);
    assert!(
        !reopened
            .skill_runtime_catalog(&SkillRuntimeCatalogParams {})
            .unwrap()
            .skills
            .iter()
            .any(|s| s.skill_id == original.skill_id)
    );
    files.sql("DROP TRIGGER fail_skill_receipt;");
    let saved = reopened
        .skill_save(
            &save_request(Some(&original), "explicit-upgrade", "明确保存的新正文"),
            3,
        )
        .unwrap()
        .skill;
    assert_ne!(files.path(&reopened, &saved.skill_id), path);
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert_eq!(saved.files, original.files);
    assert!(!saved.enabled && !saved.automatic);
}

#[test]
fn stale_updates_never_replace_the_current_package_or_an_earlier_package() {
    let files = TestFiles::new();
    let mut services = files.open();
    let mut first_request = save_request(None, "original", "第一版");
    first_request.enabled = true;
    let original = services.skill_save(&first_request, 1).unwrap().skill;
    let first = files.path(&services, &original.skill_id);
    let first_bytes = fs::read(&first).unwrap();
    let current = services
        .skill_save(&save_request(Some(&original), "next", "第二版"), 2)
        .unwrap()
        .skill;
    let second = files.path(&services, &current.skill_id);
    let second_bytes = fs::read(&second).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        services
            .skill_save(&save_request(Some(&original), "stale", "冲突版本"), 3)
            .unwrap_err()
            .reason,
        "skill_revision_conflict"
    );
    assert_eq!(files.path(&services, &current.skill_id), second);
    assert_eq!(detail(&mut services, &current), current);
    assert_eq!(fs::read(first).unwrap(), first_bytes);
    assert_eq!(fs::read(&second).unwrap(), second_bytes);
    let runtime = services
        .skill_runtime_catalog(&SkillRuntimeCatalogParams {})
        .unwrap();
    let active: Vec<_> = runtime
        .skills
        .iter()
        .filter(|s| s.source == "user")
        .collect();
    assert_eq!(active.len(), 1);
    assert_eq!(Path::new(&active[0].managed_path), second);
}

#[test]
fn legacy_package_upgrades_preserve_the_legacy_directory_and_resource_content() {
    let files = TestFiles::new();
    let mut services = files.open();
    let id = format!("skill_{}", Uuid::new_v4().simple());
    let relative = format!("user/{id}/SKILL.md");
    let path = files.root.join("skills").join(&relative);
    let content =
        render_skill_file("durable-skill", "Personal skill package", "旧目录中的正文").unwrap();
    atomic_write(&path, content.as_bytes()).unwrap();
    atomic_write(
        &path.parent().unwrap().join("assets/guide.md"),
        b"unchanged asset",
    )
    .unwrap();
    services
        .persistence_mut()
        .unwrap()
        .save_user_skill(&UserSkillSaveCommand {
            client_operation_id: "legacy".into(),
            request_fingerprint: "a".repeat(64),
            skill_id: id.clone(),
            name: "durable-skill".into(),
            normalized_name: "durable-skill".into(),
            description: "Personal skill package".into(),
            managed_relative_path: relative,
            enabled: false,
            automatic: false,
            expected_revision: None,
            content_digest: package_digest(path.parent().unwrap()).unwrap(),
            occurred_at: 1,
        })
        .unwrap();
    let original = services
        .skill_get(&SkillGetParams { skill_id: id })
        .unwrap()
        .skill;
    let blocked_parent = files.root.join("skills/user-packages");
    fs::write(&blocked_parent, b"not a directory").unwrap();
    assert_eq!(
        services
            .skill_save(
                &save_request(Some(&original), "blocked-upgrade", "无法写入的新正文"),
                2
            )
            .unwrap_err()
            .reason,
        "skill_file_write_failed"
    );
    assert_eq!(detail(&mut services, &original), original);
    assert_eq!(fs::read_to_string(&path).unwrap(), content);
    fs::remove_file(blocked_parent).unwrap();
    let upgraded = services
        .skill_save(
            &save_request(Some(&original), "upgraded", "新目录中的正文"),
            2,
        )
        .unwrap()
        .skill;
    assert_eq!(fs::read_to_string(&path).unwrap(), content);
    assert_eq!(
        fs::read(path.parent().unwrap().join("assets/guide.md")).unwrap(),
        b"unchanged asset"
    );
    assert_ne!(files.path(&services, &upgraded.skill_id), path);
    assert_eq!(upgraded.files, original.files);
    assert_eq!(detail(&mut services, &upgraded), upgraded);
}

#[test]
fn failed_import_never_registers_a_partial_package_or_changes_an_existing_skill() {
    let files = TestFiles::new();
    let mut services = files.open();
    let original = services
        .skill_save(&save_request(None, "original", "原技能"), 1)
        .unwrap()
        .skill;
    let archive = files.root.join("import.zip");
    super::tests::write_skill_zip(
        &archive,
        &[
            (
                "SKILL.md",
                "---\nname: new-import\ndescription: Import test\n---\n\nImported content.\n",
            ),
            ("assets/guide.md", "Imported asset"),
        ],
    );
    files.sql("CREATE TRIGGER fail_import BEFORE INSERT ON user_skills WHEN NEW.normalized_name='new-import' BEGIN SELECT RAISE(ABORT,'controlled import failure'); END;");
    let params = SkillImportParams {
        archive_path: archive.to_string_lossy().into_owned(),
        client_operation_id: "import".into(),
        request_fingerprint: "b".repeat(64),
    };
    assert_eq!(
        services.skill_import(&params, 2).unwrap_err().reason,
        "skill_persistence_failed"
    );
    assert_eq!(
        services
            .persistence()
            .unwrap()
            .list_user_skills()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(detail(&mut services, &original), original);
    assert!(archive.is_file());
    files.sql("DROP TRIGGER fail_import;");
    let imported = services.skill_import(&params, 3).unwrap().skill;
    fs::remove_file(archive).unwrap();
    assert_eq!(services.skill_import(&params, 4).unwrap().skill, imported);
    assert_eq!(detail(&mut services, &original), original);
}

#[test]
fn deletion_failure_preserves_the_package_and_success_retires_only_the_database_reference() {
    let files = TestFiles::new();
    let mut services = files.open();
    let original = services
        .skill_save(&save_request(None, "original", "可用技能"), 1)
        .unwrap()
        .skill;
    let path = files.path(&services, &original.skill_id);
    let bytes = fs::read(&path).unwrap();
    let params = SkillDeleteParams {
        skill_id: original.skill_id.clone(),
        expected_revision: original.state_revision,
        client_operation_id: "delete".into(),
        request_fingerprint: "c".repeat(64),
    };
    files.sql("CREATE TRIGGER fail_delete BEFORE INSERT ON user_skill_operations WHEN NEW.operation_kind='delete' BEGIN SELECT RAISE(ABORT,'controlled delete failure'); END;");
    assert_eq!(
        services.skill_delete(&params, 2).unwrap_err().reason,
        "skill_persistence_failed"
    );
    assert_eq!(detail(&mut services, &original), original);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    files.sql("DROP TRIGGER fail_delete;");
    assert!(services.skill_delete(&params, 3).unwrap().deleted);
    assert_eq!(
        services
            .skill_get(&SkillGetParams {
                skill_id: original.skill_id.clone()
            })
            .unwrap_err()
            .reason,
        "skill_not_found"
    );
    assert_eq!(fs::read(path).unwrap(), bytes);
    assert!(services.skill_delete(&params, 4).unwrap().deleted);
}

#[test]
fn process_exit_before_or_after_commit_recovers_the_database_selected_complete_package() {
    let files = TestFiles::new();
    let mut services = files.open();
    let original = services
        .skill_save(&save_request(None, "original", "中断前正文"), 1)
        .unwrap()
        .skill;
    let path = files.path(&services, &original.skill_id);
    let bytes = fs::read(&path).unwrap();
    drop(services);
    for mode in ["before_commit", "after_commit"] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.env_clear();
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            child.env("SystemRoot", system_root);
        }
        let output = child
            .args([
                "--ignored",
                "--exact",
                "skills::storage_tests::interrupted_skill_writer_probe",
            ])
            .env("CILAI_PERSONAL_SKILL_TEST_ROOT", &files.root)
            .env("CILAI_PERSONAL_SKILL_TEST_ID", &original.skill_id)
            .env("CILAI_PERSONAL_SKILL_TEST_MODE", mode)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(86),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut reopened = files.open();
        let current = detail(&mut reopened, &original);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        if mode == "before_commit" {
            assert_eq!(current, original);
            assert_eq!(files.path(&reopened, &original.skill_id), path);
        } else {
            assert_eq!(current.instructions, "提交后的完整正文");
            assert_eq!(current.files, original.files);
            assert_ne!(files.path(&reopened, &original.skill_id), path);
            assert_eq!(
                reopened
                    .skill_save(
                        &save_request(Some(&original), "crash-save", "提交后的完整正文"),
                        4
                    )
                    .unwrap()
                    .skill,
                current
            );
        }
    }
}

#[test]
#[ignore = "由进程中断测试以独立临时资料启动"]
fn interrupted_skill_writer_probe() {
    let root = PathBuf::from(std::env::var_os("CILAI_PERSONAL_SKILL_TEST_ROOT").unwrap());
    let id = std::env::var("CILAI_PERSONAL_SKILL_TEST_ID").unwrap();
    let mode = std::env::var("CILAI_PERSONAL_SKILL_TEST_MODE").unwrap();
    let db = contexthub_persistence_sqlite::SqlitePersistence::open(
        &root.join("contexthub.db"),
        "personal-skill-test",
    )
    .unwrap();
    let mut services = AppServerServices::with_data_dir(Some(db), Some(root));
    let original = services
        .skill_get(&SkillGetParams {
            skill_id: id.clone(),
        })
        .unwrap()
        .skill;
    if mode == "before_commit" {
        create_skill_package_candidate(
            &services.skills_root().unwrap(),
            &id,
            render_skill_file(
                "durable-skill",
                "Personal skill package",
                "尚未提交的候选正文",
            )
            .unwrap()
            .as_bytes(),
            &[],
        )
        .unwrap();
    } else {
        assert_eq!(mode, "after_commit");
        services
            .skill_save(
                &save_request(Some(&original), "crash-save", "提交后的完整正文"),
                3,
            )
            .unwrap();
    }
    std::process::exit(86);
}
