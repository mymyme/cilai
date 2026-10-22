use contexthub_persistence_sqlite::SqlitePersistence;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn startup_recovers_known_running_tasks_before_publishing_service() {
    let fixture = Fixture::new("known");
    initialize_database(&fixture.database_path);
    let connection = Connection::open(&fixture.database_path).unwrap();
    insert_running_task(&connection, "scan-1", "scan");
    insert_running_task(&connection, "sync-1", "sync");
    connection
        .execute(
            "INSERT INTO tasks(task_id, task_type, state, progress_completed, cancellable, payload_json, correlation_id, created_by_client, created_at) VALUES ('queued-1', 'scan', 'queued', 0, 1, '{}', 'correlation-queued', 'desktop', 1)",
            [],
        )
        .unwrap();
    drop(connection);

    let output = run_transient_server(&fixture);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let connection = Connection::open(&fixture.database_path).unwrap();
    assert_eq!(task_state(&connection, "scan-1"), "interrupted");
    assert_eq!(task_state(&connection, "sync-1"), "outcome_unknown");
    assert_eq!(task_state(&connection, "queued-1"), "queued");
    let event_count: i64 = connection
        .query_row(
            "SELECT count(*) FROM durable_events WHERE event_type='task.startupRecovered'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(event_count, 2);
}

#[test]
fn unknown_running_task_blocks_startup_without_partial_recovery() {
    let fixture = Fixture::new("unknown");
    initialize_database(&fixture.database_path);
    let connection = Connection::open(&fixture.database_path).unwrap();
    insert_running_task(&connection, "scan-1", "scan");
    insert_running_task(&connection, "custom-1", "custom");
    drop(connection);

    let output = run_transient_server(&fixture);
    assert_eq!(output.status.code(), Some(20));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no startup recovery policy"));
    let connection = Connection::open(&fixture.database_path).unwrap();
    assert_eq!(task_state(&connection, "scan-1"), "running");
    assert_eq!(task_state(&connection, "custom-1"), "running");
    let event_count: i64 = connection
        .query_row("SELECT count(*) FROM durable_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(event_count, 0);
}

fn initialize_database(database_path: &Path) {
    drop(SqlitePersistence::open(database_path, "0.1.0").unwrap());
}

fn insert_running_task(connection: &Connection, task_id: &str, task_type: &str) {
    connection
        .execute(
            "INSERT INTO tasks(task_id, task_type, state, progress_completed, cancellable, payload_json, correlation_id, created_by_client, created_at, started_at, heartbeat_at) VALUES (?1, ?2, 'running', 0, 1, '{}', ?3, 'desktop', 1, 2, 2)",
            rusqlite::params![task_id, task_type, format!("correlation-{task_id}")],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO task_attempts(attempt_id, task_id, attempt_no, server_instance_id, state, started_at) VALUES (?1, ?2, 1, 'server-previous', 'running', 2)",
            rusqlite::params![format!("attempt-{task_id}"), task_id],
        )
        .unwrap();
}

fn task_state(connection: &Connection, task_id: &str) -> String {
    connection
        .query_row(
            "SELECT state FROM tasks WHERE task_id=?1",
            [task_id],
            |row| row.get(0),
        )
        .unwrap()
}

fn run_transient_server(fixture: &Fixture) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_contexthub-app-server"))
        .args([
            "serve",
            "--transport",
            "local-ipc",
            "--transient",
            "--idle-timeout-ms",
            "100",
            "--environment",
            "test",
            "--data-dir",
        ])
        .arg(&fixture.data_dir)
        .arg("--runtime-dir")
        .arg(&fixture.runtime_dir)
        .output()
        .unwrap()
}

struct Fixture {
    root: PathBuf,
    data_dir: PathBuf,
    runtime_dir: PathBuf,
    database_path: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from("/tmp").join(format!(
            "ch-task-recovery-{label}-{}-{nanos}",
            std::process::id()
        ));
        let data_dir = root.join("test");
        fs::create_dir_all(&data_dir).unwrap();
        Self {
            runtime_dir: data_dir.join("run"),
            database_path: data_dir.join("contexthub.db"),
            data_dir,
            root,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
