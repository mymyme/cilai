use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "cilai-home-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("ai-runtime")).unwrap();
        Self(root)
    }
    fn ai(&self) -> PathBuf {
        self.0.join("ai-runtime")
    }
    fn home(&self, name: &str) -> PathBuf {
        self.ai().join(name)
    }
    fn job(&self, name: &str) -> PathBuf {
        self.ai().join("cilai-home-migrations-v1").join(name)
    }
    fn history(&self, name: &str) -> String {
        let home = self.home(name);
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::create_dir_all(home.join("tmp/arg0/previous")).unwrap();
        fs::write(home.join("tmp/arg0/previous/stale"), "old process alias").unwrap();
        let path = home.join("sessions/rollout.jsonl");
        let body = json!({"type":"response_item","payload":{"type":"message","content":[{"text":format!("正文引用旧位置 {}",home.display())}]}}).to_string()+"\n";
        let metadata = json!({"type":"session_meta","payload":{"id":"thread-one","cwd":home,"instructions":format!("保留原说明 {}",home.display())}}).to_string()+"\n";
        fs::write(&path, metadata + &body).unwrap();
        fs::write(
            home.join("config.toml"),
            format!(
                "[[skills.config]]\npath = {}\nenabled = false\n",
                serde_json::to_string(&home.join("skills/.system/skill-creator/SKILL.md")).unwrap()
            ),
        )
        .unwrap();
        let connection = Connection::open(home.join("state_5.sqlite")).unwrap();
        connection
            .set_db_config(
                rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
                true,
            )
            .unwrap();
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT, title TEXT); CREATE TABLE future_extension(value TEXT);").unwrap();
        connection
            .execute(
                "INSERT INTO threads VALUES('thread-one',?1,?2,?3)",
                params![path.to_str().unwrap(), home.to_str().unwrap(), body],
            )
            .unwrap();
        body
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn both_homes_preserve_native_identity_bodies_wal_and_original_backups() {
    let fixture = Fixture::new();
    let bodies = HOMES.map(|(old, _)| fixture.history(old));
    let originals =
        HOMES.map(|(old, _)| fs::read(fixture.home(old).join("sessions/rollout.jsonl")).unwrap());
    migrate(&fixture.0).unwrap();
    for (index, (old, new)) in HOMES.iter().enumerate() {
        assert!(!fixture.home(old).exists());
        let home = fixture.home(new);
        assert!(!home.join("tmp").exists());
        let connection = Connection::open(home.join("state_5.sqlite")).unwrap();
        let (id, path, cwd, body): (String, String, String, String) = connection
            .query_row("SELECT id,rollout_path,cwd,title FROM threads", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap();
        assert_eq!(id, "thread-one");
        assert_eq!(Path::new(&path), home.join("sessions/rollout.jsonl"));
        assert_eq!(Path::new(&cwd), home);
        assert_eq!(body, bodies[index]);
        let rollout = fs::read_to_string(home.join("sessions/rollout.jsonl")).unwrap();
        assert!(rollout.ends_with(&bodies[index]));
        let metadata: Value = serde_json::from_str(rollout.lines().next().unwrap()).unwrap();
        assert_eq!(metadata["payload"]["cwd"], json!(home));
        assert!(
            metadata["payload"]["instructions"]
                .as_str()
                .unwrap()
                .contains(old)
        );
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        let configured_path: String = serde_json::from_str(
            config
                .lines()
                .find_map(|line| line.strip_prefix("path = "))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            Path::new(&configured_path),
            home.join("skills/.system/skill-creator/SKILL.md")
        );
        assert_eq!(
            fs::read(fixture.job(new).join("source/sessions/rollout.jsonl")).unwrap(),
            originals[index]
        );
        assert!(
            fixture
                .job(new)
                .join("source/tmp/arg0/previous/stale")
                .is_file()
        );
    }
    let before = inventory(&fixture.home("cilai-harness")).unwrap();
    migrate(&fixture.0).unwrap();
    assert_eq!(inventory(&fixture.home("cilai-harness")).unwrap(), before);
}

#[test]
fn fresh_profile_does_not_create_legacy_or_migration_directories() {
    let fixture = Fixture::new();
    migrate(&fixture.0).unwrap();
    assert_eq!(entries(&fixture.ai()).unwrap().len(), 0);
}

#[test]
fn conflict_keeps_both_sources_unchanged() {
    let fixture = Fixture::new();
    fixture.history("codex-harness");
    fs::create_dir(fixture.home("cilai-harness")).unwrap();
    fs::write(
        fixture.home("cilai-harness").join("existing"),
        "independent",
    )
    .unwrap();
    let before = inventory(&fixture.home("codex-harness")).unwrap();
    assert!(
        migrate(&fixture.0)
            .unwrap_err()
            .contains("directory_conflict")
    );
    assert_eq!(before, inventory(&fixture.home("codex-harness")).unwrap());
    assert_eq!(
        fs::read_to_string(fixture.home("cilai-harness").join("existing")).unwrap(),
        "independent"
    );
}

#[test]
fn large_wal_database_migrates_with_the_services_minimal_environment() {
    const PROBE: &str = "CILAI_HOME_MIGRATION_TEST_PROFILE";
    if let Some(profile) = std::env::var_os(PROBE) {
        migrate(Path::new(&profile)).unwrap();
        return;
    }
    let fixture = Fixture::new();
    fixture.history("codex-harness");
    let path = fixture.home("codex-harness").join("large-log.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .unwrap();
    connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE entries(id INTEGER PRIMARY KEY, path TEXT, body TEXT);").unwrap();
    connection.execute(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<12000) INSERT INTO entries SELECT x, ?1, hex(zeroblob(256)) FROM n",
        [fixture.home("codex-harness").join("sessions/rollout.jsonl").to_string_lossy().as_ref()],
    ).unwrap();
    drop(connection);
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "harness_home_migration::tests::large_wal_database_migrates_with_the_services_minimal_environment", "--test-threads=1"])
        .env_clear().env(PROBE, &fixture.0);
    contexthub_platform_adapter::process::retain_required_system_environment(&mut command).unwrap();
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let connection =
        Connection::open(fixture.home("cilai-harness").join("large-log.sqlite")).unwrap();
    let (count, old_paths): (i64, i64) = connection
        .query_row(
            "SELECT COUNT(*), SUM(instr(path, 'codex-harness')>0) FROM entries",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((count, old_paths), (12000, 0));
    assert!(
        fixture
            .job("cilai-harness")
            .join("source/large-log.sqlite")
            .is_file()
    );
}

#[test]
fn inactive_coordination_locks_are_archived_without_reading_or_copying_locked_files() {
    let fixture = Fixture::new();
    fixture.history("codex-harness");
    let home = fixture.home("codex-harness");
    fs::create_dir_all(home.join("thread-writer-locks")).unwrap();
    for relative in ["memory-worker.lock", "thread-writer-locks/thread-one.lock"] {
        fs::write(home.join(relative), b"old coordination fixture").unwrap();
    }
    let history = fs::read(home.join("sessions/rollout.jsonl")).unwrap();
    migrate(&fixture.0).unwrap();
    for relative in ["memory-worker.lock", "thread-writer-locks/thread-one.lock"] {
        assert!(!fixture.home("cilai-harness").join(relative).exists());
        assert_eq!(
            fs::read(fixture.job("cilai-harness").join("source").join(relative)).unwrap(),
            b"old coordination fixture"
        );
    }
    assert_eq!(
        fs::read(
            fixture
                .job("cilai-harness")
                .join("source/sessions/rollout.jsonl")
        )
        .unwrap(),
        history
    );
    migrate(&fixture.0).unwrap();
}

#[test]
fn active_memory_or_thread_coordination_lock_still_blocks_migration() {
    for relative in ["memory-worker.lock", "thread-writer-locks/thread-one.lock"] {
        let fixture = Fixture::new();
        fixture.history("codex-harness");
        let path = fixture.home("codex-harness").join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lock = File::create(&path).unwrap();
        lock.try_lock().unwrap();
        assert!(
            migrate(&fixture.0)
                .unwrap_err()
                .contains("source_runtime_busy")
        );
        assert!(fixture.home("codex-harness").exists());
        assert!(!fixture.home("cilai-harness").exists());
        drop(lock);
        migrate(&fixture.0).unwrap();
    }
}

#[test]
fn running_native_alias_blocks_migration_before_copy() {
    let fixture = Fixture::new();
    fixture.history("codex-harness");
    let lock_path = fixture
        .home("codex-harness")
        .join("tmp/arg0/previous/.lock");
    let lock = File::create(lock_path).unwrap();
    lock.try_lock().unwrap();
    assert!(
        migrate(&fixture.0)
            .unwrap_err()
            .contains("source_runtime_busy")
    );
    assert!(fixture.home("codex-harness").exists());
    assert!(!fixture.home("cilai-harness").exists());
    drop(lock);
    migrate(&fixture.0).unwrap();
}

#[test]
fn corrupt_database_and_missing_indexed_history_never_activate_destination() {
    for corrupt in [false, true] {
        let fixture = Fixture::new();
        fixture.history("codex-harness");
        if corrupt {
            fs::write(
                fixture.home("codex-harness").join("broken.sqlite"),
                "not sqlite",
            )
            .unwrap();
        } else {
            fs::remove_file(fixture.home("codex-harness").join("sessions/rollout.jsonl")).unwrap();
        }
        assert!(migrate(&fixture.0).is_err());
        assert!(fixture.home("codex-harness").is_dir());
        assert!(!fixture.home("cilai-harness").exists());
    }
}

#[test]
fn interrupted_archive_and_activation_resume_without_reimporting() {
    for before_activation in [false, true] {
        let fixture = Fixture::new();
        fixture.history("codex-harness");
        migrate(&fixture.0).unwrap();
        let job = fixture.job("cilai-harness");
        fs::remove_file(job.join("complete.json")).unwrap();
        if before_activation {
            fs::rename(fixture.home("cilai-harness"), job.join("staging")).unwrap();
        }
        migrate(&fixture.0).unwrap();
        assert!(job.join("complete.json").is_file());
        assert!(!fixture.home("codex-harness").exists());
        assert!(
            fixture
                .home("cilai-harness")
                .join("sessions/rollout.jsonl")
                .is_file()
        );
    }
}

#[test]
fn tampered_unconfirmed_destination_is_preserved_and_rejected() {
    let fixture = Fixture::new();
    fixture.history("codex-harness");
    migrate(&fixture.0).unwrap();
    fs::remove_file(fixture.job("cilai-harness").join("complete.json")).unwrap();
    let path = fixture.home("cilai-harness").join("sessions/rollout.jsonl");
    fs::write(&path, "changed after interruption").unwrap();
    assert!(
        migrate(&fixture.0)
            .unwrap_err()
            .contains("unconfirmed_destination_changed")
    );
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "changed after interruption"
    );
    assert!(fixture.job("cilai-harness").join("source").is_dir());
}

#[test]
fn partial_copy_is_rebuilt_only_under_its_matching_plan() {
    let fixture = Fixture::new();
    fixture.history("codex-harness");
    let job = fixture.job("cilai-harness");
    security::ensure_restricted_directory(&job).unwrap();
    security::ensure_restricted_directory(&job.join("staging")).unwrap();
    write_json(
        &job.join("plan.json"),
        &Plan {
            version: 1,
            source: fixture.home("codex-harness").canonicalize().unwrap(),
            destination: fixture.ai().canonicalize().unwrap().join("cilai-harness"),
        },
    )
    .unwrap();
    fs::write(job.join("staging/partial"), "incomplete").unwrap();
    migrate(&fixture.0).unwrap();
    assert!(!fixture.home("cilai-harness").join("partial").exists());
    assert!(job.join("source").exists());
}

#[test]
fn metadata_rebasing_does_not_rewrite_similar_paths_or_user_text() {
    let mappings = [(
        PathBuf::from("/profile/codex-harness"),
        PathBuf::from("/profile/cilai-harness"),
    )];
    let mut record = json!({"cwd":"/profile/codex-harness","path":"/profile/codex-harness-other/a","instructions":"/profile/codex-harness","sandbox_policy":{"writable_roots":["/profile/codex-harness/memories","/workspace"]}});
    assert!(rebase_metadata(&mut record, &mappings));
    assert_eq!(record["cwd"], "/profile/cilai-harness");
    assert_eq!(record["path"], "/profile/codex-harness-other/a");
    assert_eq!(record["instructions"], "/profile/codex-harness");
    assert_eq!(
        record["sandbox_policy"]["writable_roots"][0],
        json!(Path::new("/profile/cilai-harness").join("memories"))
    );
}

#[test]
fn memory_path_migration_publishes_a_new_version_without_editing_the_previous_one() {
    let fixture = Fixture::new();
    fixture.history("codex-harness");
    let home = fixture.home("codex-harness");
    let revision = uuid::Uuid::new_v4().to_string();
    let publication = home.join("memory-publication");
    let version = publication.join("versions").join(&revision);
    fs::create_dir_all(version.join("memories")).unwrap();
    let original = format!(
        "事实不变\nrollout_path: {}/sessions/rollout.jsonl\n",
        home.display()
    );
    fs::write(version.join("memories/MEMORY.md"), &original).unwrap();
    fs::write(version.join("memories/memory_summary.md"), "原摘要").unwrap();
    fs::write(
        version.join("manifest.json"),
        json!({"revision":revision,"sourceCount":1,"publishedAt":123}).to_string(),
    )
    .unwrap();
    fs::write(publication.join("current"), &revision).unwrap();
    migrate(&fixture.0).unwrap();
    let publication = fixture.home("cilai-harness").join("memory-publication");
    assert_eq!(
        fs::read_to_string(
            publication
                .join("versions")
                .join(&revision)
                .join("memories/MEMORY.md")
        )
        .unwrap(),
        original
    );
    let current = fs::read_to_string(publication.join("current")).unwrap();
    assert_ne!(current, revision);
    uuid::Uuid::parse_str(&current).unwrap();
    let version = publication.join("versions").join(&current);
    assert_eq!(
        fs::read_to_string(version.join("memories/MEMORY.md")).unwrap(),
        original.replace(
            &home.to_string_lossy().to_string(),
            &fixture.home("cilai-harness").to_string_lossy()
        )
    );
    let manifest: Value = read_json(&version.join("manifest.json")).unwrap();
    assert_eq!(manifest["revision"], current);
    assert_eq!(manifest["sourceCount"], 1);
    assert_eq!(manifest["publishedAt"], 123);
}

#[test]
fn indexed_paths_cannot_escape_the_migrated_home() {
    let fixture = Fixture::new();
    fixture.history("codex-harness");
    let home = fixture.home("codex-harness");
    fs::write(fixture.ai().join("outside.jsonl"), "outside").unwrap();
    let connection = Connection::open(home.join("state_5.sqlite")).unwrap();
    connection
        .execute(
            "UPDATE threads SET rollout_path=?1",
            [home.join("../outside.jsonl").to_str().unwrap()],
        )
        .unwrap();
    drop(connection);
    assert!(
        migrate(&fixture.0)
            .unwrap_err()
            .contains("indexed_rollout_path_escape")
    );
    assert!(!fixture.home("cilai-harness").exists());
    assert_eq!(
        fs::read_to_string(fixture.ai().join("outside.jsonl")).unwrap(),
        "outside"
    );
}
