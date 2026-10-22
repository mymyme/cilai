use contexthub_local_transport::{Endpoint, artifact_exists};
use contexthub_persistence_sqlite::SqlitePersistence;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn changed_migration_history_blocks_server_readiness_with_exit_code_13() {
    let root = unique_root("checksum");
    let data_dir = root.join("test");
    let runtime_dir = data_dir.join("run");
    fs::create_dir_all(&runtime_dir).unwrap();
    let database_path = data_dir.join("contexthub.db");
    drop(SqlitePersistence::open(&database_path, "0.1.0").unwrap());
    let connection = Connection::open(&database_path).unwrap();
    connection
        .execute(
            "UPDATE schema_migrations SET checksum = ?1 WHERE version = 1",
            ["0".repeat(64)],
        )
        .unwrap();
    drop(connection);

    let output = server_command(&data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("App Server starts");
    assert_eq!(output.status.code(), Some(13));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("does not match embedded name and checksum")
    );
    assert!(!runtime_dir.join("instance.json").exists());
    // 启动在迁移校验阶段就失败，本地端点不应被发布：两端都通过统一的端点残留判定核对，
    // 而不是直接假设文件系统上存在套接字文件（Windows 命名管道不落地为文件）。
    let endpoint = Endpoint::resolve(&runtime_dir).unwrap();
    assert!(!artifact_exists(&endpoint).unwrap());
    assert!(runtime_dir.join("instance.lock").is_file());
    assert!(database_path.is_file());

    fs::remove_file(database_path).unwrap();
    fs::remove_file(runtime_dir.join("instance.lock")).unwrap();
    fs::remove_dir(runtime_dir).unwrap();
    fs::remove_dir(data_dir).unwrap();
    fs::remove_dir(root).unwrap();
}

fn server_command(data_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_contexthub-app-server"));
    command
        .args([
            "serve",
            "--transport",
            "stdio",
            "--enable-local-ipc",
            "--environment",
            "test",
            "--data-dir",
        ])
        .arg(data_dir)
        .arg("--runtime-dir")
        .arg(data_dir.join("run"));
    command
}

fn unique_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from("/tmp").join(format!(
        "ch-database-startup-{label}-{}-{nanos}",
        std::process::id()
    ))
}
