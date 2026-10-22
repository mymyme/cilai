//! Forward repair for homes already copied by migration v1. Only the native
//! rollout locator columns and the path-bound Workbench receipt change. Native
//! session bytes and paginated turn databases remain opaque and unchanged.
use super::{
    Plan, Prepared, Tree, copy_tree, databases, directory, err, inventory, read_json, rebase,
    regular_file, write_json,
};
use contexthub_platform_adapter::security;
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params, types::ValueRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

const RECEIPT: &str = "cilai-public-history-migration.json";
const DATABASE: &str = "state_5.sqlite";

#[derive(Serialize, Deserialize)]
struct Repair {
    plan: Plan,
    original: Tree,
    receipt: Value,
    database_digest: Option<String>,
}

fn digest(value: &Value) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(err)?)
    ))
}

fn home_digest(path: &Path) -> Result<String, String> {
    digest(&serde_json::json!(path))
}

fn stable_tree(mut tree: Tree) -> Tree {
    const EMPTY: &str = "file:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let empty_wals: Vec<_> = tree
        .iter()
        .filter_map(|(path, hash)| {
            let name = path.file_name()?.to_str()?;
            if *hash != EMPTY || !name.ends_with(".sqlite-wal") {
                return None;
            }
            let database = path.with_file_name(name.strip_suffix("-wal")?);
            tree.get(&database)
                .filter(|hash| hash.starts_with("file:"))?;
            Some(path.clone())
        })
        .collect();
    for path in empty_wals {
        tree.remove(&path);
    }
    tree
}

fn official_inventory(root: &Path) -> Result<Tree, String> {
    // Even a read-only SQLite opener may create or remove an empty WAL. It has
    // no frames; ignore only a zero-byte WAL paired with an existing database.
    // Orphans and every nonempty WAL remain part of the strict content check.
    Ok(stable_tree(inventory(root)?))
}

fn subtree(tree: &Tree) -> Tree {
    stable_tree(
        tree.iter()
            .filter_map(|(path, hash)| {
                path.strip_prefix("official-runtime")
                    .ok()
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .filter(|relative| !super::transient(relative))
                    .map(|relative| (relative.to_path_buf(), hash.clone()))
            })
            .collect(),
    )
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Hash every schema and row value, normalizing only the two locator columns.
/// This permits an interrupted locator transaction to resume while rejecting
/// changes to names, archive flags, model metadata and all other native state.
fn database_digest(
    connection: &Connection,
    mappings: &[(PathBuf, PathBuf)],
) -> Result<String, String> {
    let schema = connection
        .prepare("SELECT name, sql FROM sqlite_master WHERE type='table' ORDER BY name")
        .map_err(err)?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    let mut digest = Sha256::new();
    for (table, sql) in schema {
        digest.update(serde_json::to_vec(&(&table, sql)).map_err(err)?);
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {}", quote(&table)))
            .map_err(err)?;
        let columns: Vec<String> = statement
            .column_names()
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        digest.update(serde_json::to_vec(&columns).map_err(err)?);
        let mut rows = statement.query([]).map_err(err)?;
        let mut hashes = Vec::new();
        while let Some(row) = rows.next().map_err(err)? {
            let mut hash = Sha256::new();
            for (index, column) in columns.iter().enumerate() {
                let value = row.get_ref(index).map_err(err)?;
                let bytes = match value {
                    ValueRef::Null => vec![0],
                    ValueRef::Integer(value) => [vec![1], value.to_le_bytes().to_vec()].concat(),
                    ValueRef::Real(value) => {
                        [vec![2], value.to_bits().to_le_bytes().to_vec()].concat()
                    }
                    ValueRef::Text(value)
                        if column == "rollout_path"
                            && matches!(
                                table.as_str(),
                                "threads" | "rollout_migration_skipped_rollouts"
                            ) =>
                    {
                        let text = std::str::from_utf8(value).map_err(err)?;
                        [
                            vec![3],
                            rebase(text, mappings)
                                .unwrap_or_else(|| text.to_owned())
                                .into_bytes(),
                        ]
                        .concat()
                    }
                    ValueRef::Text(value) => [vec![3], value.to_vec()].concat(),
                    ValueRef::Blob(value) => [vec![4], value.to_vec()].concat(),
                };
                hash.update((bytes.len() as u64).to_le_bytes());
                hash.update(bytes);
            }
            hashes.push(format!("{:x}", hash.finalize()));
        }
        hashes.sort();
        digest.update(serde_json::to_vec(&hashes).map_err(err)?);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn connection(home: &Path) -> Result<Connection, String> {
    let path = home.join(DATABASE);
    regular_file(&path)?;
    let connection =
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(err)?;
    connection
        .set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .map_err(err)?;
    connection
        .execute_batch("PRAGMA busy_timeout=0; PRAGMA temp_store=MEMORY;")
        .map_err(err)?;
    let check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(err)?;
    if check != "ok" {
        return Err("native_database_invalid".into());
    }
    Ok(connection)
}

fn lock_other_databases(home: &Path) -> Result<Vec<Connection>, String> {
    databases(home)?
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name != DATABASE))
        .map(|path| {
            let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
                .map_err(err)?;
            connection
                .set_db_config(
                    rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
                    true,
                )
                .map_err(err)?;
            connection
                .execute_batch("PRAGMA busy_timeout=0; BEGIN EXCLUSIVE")
                .map_err(|_| "official_runtime_busy")?;
            Ok(connection)
        })
        .collect()
}

fn verify_empty_home(home: &Path, receipt: &Value) -> Result<(), String> {
    let export: Value = read_json(&home.join("cilai-legacy-public-export.json"))?;
    let tree = inventory(home)?;
    let has_session_files = tree.iter().any(|(path, kind)| {
        (path.starts_with("sessions") || path.starts_with("archived_sessions"))
            && kind != "directory"
    });
    if receipt["threads"].as_u64() != Some(0)
        || receipt["turns"].as_u64() != Some(0)
        || export["threads"]
            .as_array()
            .is_none_or(|threads| !threads.is_empty())
        || receipt["sourceDigest"] != digest(&export)?
        || !databases(home)?.is_empty()
        || tree.keys().any(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().contains(".sqlite-"))
        })
        || has_session_files
    {
        return Err("official_empty_home_unverified".into());
    }
    Ok(())
}

fn verify_unchanged(home: &Path, original: &Tree) -> Result<(), String> {
    let retain = |tree: Tree| -> Tree {
        tree.into_iter()
            .filter(|(path, _)| {
                path != Path::new(RECEIPT)
                    && ![
                        DATABASE,
                        "state_5.sqlite-wal",
                        "state_5.sqlite-shm",
                        "state_5.sqlite-journal",
                    ]
                    .iter()
                    .any(|name| path == Path::new(name))
            })
            .collect()
    };
    if retain(official_inventory(home)?) != retain(original.clone()) {
        return Err("native_payload_changed".into());
    }
    Ok(())
}

fn repair_locators(
    connection: &Connection,
    home: &Path,
    mappings: &[(PathBuf, PathBuf)],
) -> Result<(), String> {
    let tables: Vec<String> = connection
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .map_err(err)?
        .query_map([], |row| row.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    if !tables.iter().any(|name| name == "threads") {
        return Err("native_threads_table_missing".into());
    }
    for table in ["threads", "rollout_migration_skipped_rollouts"] {
        if !tables.iter().any(|name| name == table) {
            continue;
        }
        let values: Vec<String> = connection
            .prepare(&format!(
                "SELECT DISTINCT rollout_path FROM {}",
                quote(table)
            ))
            .map_err(err)?
            .query_map([], |row| row.get(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        for before in values {
            let after = rebase(&before, mappings).unwrap_or_else(|| before.clone());
            let comparable_after =
                contexthub_platform_adapter::filesystem::path_for_comparison(Path::new(&after));
            let comparable_home =
                contexthub_platform_adapter::filesystem::path_for_comparison(home);
            let relative = comparable_after
                .strip_prefix(comparable_home)
                .map_err(|_| "native_rollout_outside_home")?;
            if !relative.starts_with("sessions") && !relative.starts_with("archived_sessions") {
                return Err("native_rollout_outside_sessions".into());
            }
            // A skipped malformed rollout may be absent; a live thread must resolve.
            if table == "threads" {
                regular_file(Path::new(&after))?;
                let resolved = Path::new(&after).canonicalize().map_err(err)?;
                if !resolved.starts_with(home) {
                    return Err("native_rollout_link_outside_home".into());
                }
            }
            if before != after {
                connection
                    .execute(
                        &format!(
                            "UPDATE {} SET rollout_path=?1 WHERE rollout_path=?2",
                            quote(table)
                        ),
                        params![after, before],
                    )
                    .map_err(err)?;
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "备份、定位事务及回执激活按同一恢复顺序审查"
)]
pub(super) fn migrate(
    ai: &Path,
    journal: &Path,
    old: &str,
    new: &str,
    mappings: &[(PathBuf, PathBuf)],
) -> Result<(), String> {
    let home = ai.join(new).join("official-runtime");
    let job = journal.join(new);
    let receipt_path = home.join(RECEIPT);
    if !job.join("complete.json").is_file() || !receipt_path.is_file() {
        return Ok(());
    }
    directory(&home)?;
    let plan = Plan {
        version: 1,
        source: ai.join(old),
        destination: ai.join(new),
    };
    if read_json::<Plan>(&job.join("complete.json"))? != plan {
        return Err("home_plan_mismatch".into());
    }
    let receipt: Value = read_json(&receipt_path)?;
    let expected_home = home_digest(&home)?;
    let repair_job = job.join("official-runtime-relocation-v1");
    let complete = repair_job.join("complete.json");
    if receipt["homeDigest"] == expected_home && !repair_job.exists() {
        return Ok(());
    }
    if receipt["status"] != "verified" || receipt["targetVersion"] != "0.163.0" {
        return Err("official_upgrade_receipt_invalid".into());
    }
    security::ensure_restricted_directory(&repair_job).map_err(err)?;
    let prepared_path = repair_job.join("prepared.json");
    let backup = repair_job.join("source");
    if !prepared_path.exists() {
        if receipt["homeDigest"] != home_digest(&plan.source.join("official-runtime"))? {
            return Err("official_upgrade_receipt_profile_mismatch".into());
        }
        let prepared: Prepared = read_json(&job.join("prepared.json"))?;
        let source_home = job.join("source/official-runtime");
        if official_inventory(&source_home)? != subtree(&prepared.source)
            || official_inventory(&home)? != subtree(&prepared.destination)
            || read_json::<Value>(&source_home.join(RECEIPT))? != receipt
        {
            return Err("official_source_changed".into());
        }
        let _other_databases = lock_other_databases(&home)?;
        let connection = if home.join(DATABASE).exists() {
            let connection = connection(&home)?;
            connection
                .execute_batch("BEGIN EXCLUSIVE")
                .map_err(|_| "official_runtime_busy")?;
            Some(connection)
        } else {
            verify_empty_home(&home, &receipt)?;
            None
        };
        let original = official_inventory(&home)?;
        let expected = connection
            .as_ref()
            .map(|connection| database_digest(connection, mappings))
            .transpose()?;
        if backup.exists() {
            if official_inventory(&backup)? != original {
                return Err("official_backup_conflict".into());
            }
        } else {
            copy_tree(&home, &backup, &home)?;
            if official_inventory(&backup)? != original {
                return Err("official_backup_changed".into());
            }
        }
        write_json(
            &prepared_path,
            &Repair {
                plan: plan.clone(),
                original,
                receipt: receipt.clone(),
                database_digest: expected,
            },
        )?;
        if let Some(connection) = connection {
            connection.execute_batch("ROLLBACK").map_err(err)?;
        }
    }
    let prepared: Repair = read_json(&prepared_path)?;
    if prepared.plan != plan {
        return Err("official_plan_changed".into());
    }
    let mut after_receipt = prepared.receipt.clone();
    after_receipt["homeDigest"] = Value::String(expected_home);
    if receipt != prepared.receipt && receipt != after_receipt {
        return Err("official_receipt_changed".into());
    }
    if complete.exists() {
        if read_json::<Value>(&complete)? != after_receipt || receipt != after_receipt {
            return Err("official_completion_changed".into());
        }
        return Ok(());
    }
    // A validated completion and path-bound receipt are sufficient on normal
    // startup. Re-read the full immutable backup only while completing a repair.
    if official_inventory(&backup)? != prepared.original {
        return Err("official_backup_changed".into());
    }
    let _other_databases = lock_other_databases(&home)?;
    verify_unchanged(&home, &prepared.original)?;
    if let Some(expected) = &prepared.database_digest {
        let mut connection = connection(&home)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Exclusive)
            .map_err(|_| "official_runtime_busy")?;
        if database_digest(&transaction, mappings)? != *expected {
            return Err("official_state_changed".into());
        }
        repair_locators(&transaction, &home, mappings)?;
        if database_digest(&transaction, mappings)? != *expected {
            return Err("official_state_changed".into());
        }
        transaction.commit().map_err(err)?;
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .map_err(err)?;
    } else {
        verify_empty_home(&home, &prepared.receipt)?;
    }
    verify_unchanged(&home, &prepared.original)?;
    let pending = repair_job.join("receipt.pending.json");
    if pending.exists() {
        if read_json::<Value>(&pending)? != after_receipt {
            return Err("official_pending_receipt_changed".into());
        }
    } else {
        write_json(&pending, &after_receipt)?;
    }
    fs::rename(&pending, &receipt_path).map_err(err)?;
    write_json(&complete, &after_receipt)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        old_home: PathBuf,
        new_home: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "cilai-official-relocation-{}",
                uuid::Uuid::new_v4()
            ));
            let old_home = root.join("ai-runtime/codex-harness/official-runtime");
            security::ensure_restricted_directory(&old_home).unwrap();
            let old_home = old_home.canonicalize().unwrap();
            fs::create_dir_all(old_home.join("sessions/2026/10/19")).unwrap();
            fs::create_dir_all(old_home.join("archived_sessions")).unwrap();
            let db = Connection::open(old_home.join(DATABASE)).unwrap();
            db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, rollout_path TEXT NOT NULL, name TEXT, archived INTEGER, opaque BLOB); CREATE TABLE rollout_migration_skipped_rollouts(rollout_path TEXT PRIMARY KEY); CREATE TABLE native_state(key TEXT, value BLOB);").unwrap();
            for (id, directory, archived) in [
                ("active", "sessions/2026/10/19", 0),
                ("archived", "archived_sessions", 1),
            ] {
                let path = old_home.join(directory).join(format!("rollout-{id}.jsonl"));
                fs::write(&path, b"opaque original session bytes\r\n").unwrap();
                db.execute(
                    "INSERT INTO threads VALUES (?1, ?2, '原有标题', ?3, x'010203')",
                    params![id, path.to_string_lossy(), archived],
                )
                .unwrap();
            }
            db.execute(
                "INSERT INTO native_state VALUES ('preserve', x'102030')",
                [],
            )
            .unwrap();
            drop(db);
            let history = Connection::open(old_home.join("thread_history_1.sqlite")).unwrap();
            history.execute_batch("CREATE TABLE turns(id TEXT, payload BLOB); INSERT INTO turns VALUES ('turn', x'112233');").unwrap();
            drop(history);
            let receipt = serde_json::json!({"status":"verified", "targetVersion":"0.163.0", "homeDigest":home_digest(&old_home).unwrap(), "sourceDigest":"unchanged", "providerAliases":["historical"]});
            write_json(&old_home.join(RECEIPT), &receipt).unwrap();
            let new_home = root.join("ai-runtime/cilai-harness/official-runtime");
            Self {
                root,
                old_home,
                new_home,
            }
        }
        fn job(&self) -> PathBuf {
            self.root.join(
                "ai-runtime/cilai-home-migrations-v1/cilai-harness/official-runtime-relocation-v1",
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn empty_fixture() -> Fixture {
        let fixture = Fixture::new();
        for name in [
            DATABASE,
            "thread_history_1.sqlite",
            "sessions/2026/10/19/rollout-active.jsonl",
            "archived_sessions/rollout-archived.jsonl",
        ] {
            fs::remove_file(fixture.old_home.join(name)).unwrap();
        }
        let export =
            serde_json::json!({"schemaVersion":1,"source":"legacy-public-app-server","threads":[]});
        write_json(
            &fixture.old_home.join("cilai-legacy-public-export.json"),
            &export,
        )
        .unwrap();
        let mut receipt: Value = read_json(&fixture.old_home.join(RECEIPT)).unwrap();
        receipt["threads"] = Value::from(0);
        receipt["turns"] = Value::from(0);
        receipt["sourceDigest"] = Value::String(digest(&export).unwrap());
        fs::write(
            fixture.old_home.join(RECEIPT),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        fixture
    }

    #[test]
    fn read_only_empty_wal_sidecars_do_not_change_the_verified_database_snapshot() {
        for bytes in [b"".as_slice(), b"unexpected nonempty WAL".as_slice()] {
            let fixture = Fixture::new();
            let ai = fixture.root.join("ai-runtime").canonicalize().unwrap();
            let journal = ai.join("cilai-home-migrations-v1");
            security::ensure_restricted_directory(&journal).unwrap();
            let mappings = vec![(ai.join("codex-harness"), ai.join("cilai-harness"))];
            super::super::migrate_one(&ai, &journal, "codex-harness", "cilai-harness", &mappings)
                .unwrap();
            fs::write(fixture.new_home.join("state_5.sqlite-wal"), bytes).unwrap();
            let result = super::super::migrate(&fixture.root);
            if bytes.is_empty() {
                result.unwrap();
                assert!(fixture.job().join("complete.json").is_file());
            } else {
                assert!(result.unwrap_err().contains("official_source_changed"));
                assert_eq!(
                    fs::read(fixture.new_home.join("state_5.sqlite-wal")).unwrap(),
                    bytes
                );
            }
        }
    }

    #[test]
    fn native_path_spelling_and_canonical_plan_refer_to_the_same_home() {
        let fixture = Fixture::new();
        let path = fixture
            .old_home
            .join("sessions/2026/10/19/rollout-active.jsonl");
        let plain = contexthub_platform_adapter::filesystem::path_for_comparison(&path);
        let db = connection(&fixture.old_home).unwrap();
        db.execute(
            "UPDATE threads SET rollout_path=?1 WHERE id='active'",
            [plain.to_string_lossy().as_ref()],
        )
        .unwrap();
        drop(db);
        super::super::migrate(&fixture.root).unwrap();
        let home = fixture.new_home.canonicalize().unwrap();
        let db = connection(&home).unwrap();
        let after: String = db
            .query_row(
                "SELECT rollout_path FROM threads WHERE id='active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(Path::new(&after).starts_with(&home));
        assert!(Path::new(&after).is_file());
    }

    #[test]
    fn old_inventory_uses_the_same_transient_scope_after_entering_the_subdirectory() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.old_home.join("tmp/arg0-windows/native-alias")).unwrap();
        fs::create_dir_all(fixture.old_home.join("thread-writer-locks")).unwrap();
        fs::write(
            fixture
                .old_home
                .join("tmp/arg0-windows/native-alias/apply_patch.bat"),
            b"temporary alias",
        )
        .unwrap();
        fs::write(
            fixture
                .old_home
                .join("thread-writer-locks/.coordination.lock"),
            [],
        )
        .unwrap();
        super::super::migrate(&fixture.root).unwrap();
        let archive = fixture
            .root
            .join("ai-runtime/cilai-home-migrations-v1/cilai-harness/source/official-runtime");
        assert_eq!(
            fs::read(archive.join("tmp/arg0-windows/native-alias/apply_patch.bat")).unwrap(),
            b"temporary alias"
        );
        assert!(
            archive
                .join("thread-writer-locks/.coordination.lock")
                .is_file()
        );
        assert!(fixture.job().join("complete.json").is_file());
    }

    #[test]
    fn verified_unused_home_rebinds_receipt_without_creating_a_database() {
        let fixture = empty_fixture();
        super::super::migrate(&fixture.root).unwrap();
        assert!(!fixture.new_home.join(DATABASE).exists());
        let receipt: Value = read_json(&fixture.new_home.join(RECEIPT)).unwrap();
        assert_eq!(
            receipt["homeDigest"],
            home_digest(&fixture.new_home.canonicalize().unwrap()).unwrap()
        );
        assert!(fixture.job().join("complete.json").is_file());
        super::super::migrate(&fixture.root).unwrap();
    }

    #[test]
    fn missing_database_with_remaining_sessions_is_not_treated_as_unused() {
        let fixture = empty_fixture();
        let path = fixture.old_home.join("sessions/2026/10/19/unindexed.jsonl");
        fs::write(&path, b"preserve unindexed history").unwrap();
        assert!(
            super::super::migrate(&fixture.root)
                .unwrap_err()
                .contains("empty_home_unverified")
        );
        assert_eq!(
            fs::read(fixture.new_home.join("sessions/2026/10/19/unindexed.jsonl")).unwrap(),
            b"preserve unindexed history"
        );
    }

    #[test]
    fn missing_database_with_orphan_wal_is_not_treated_as_unused() {
        let fixture = empty_fixture();
        fs::write(
            fixture.old_home.join("state_5.sqlite-wal"),
            b"preserve orphan WAL",
        )
        .unwrap();
        assert!(
            super::super::migrate(&fixture.root)
                .unwrap_err()
                .contains("empty_home_unverified")
        );
        assert_eq!(
            fs::read(fixture.new_home.join("state_5.sqlite-wal")).unwrap(),
            b"preserve orphan WAL"
        );
    }

    #[test]
    fn nested_paginated_history_relocates_without_changing_payload_or_native_metadata() {
        let fixture = Fixture::new();
        let original = inventory(&fixture.old_home).unwrap();
        super::super::migrate(&fixture.root).unwrap();
        let home = fixture.new_home.canonicalize().unwrap();
        let db = connection(&home).unwrap();
        for id in ["active", "archived"] {
            let (path, name, archived, opaque): (String, String, i64, Vec<u8>) = db
                .query_row(
                    "SELECT rollout_path, name, archived, opaque FROM threads WHERE id=?1",
                    [id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .unwrap();
            assert!(Path::new(&path).starts_with(&home));
            assert!(Path::new(&path).is_file());
            assert_eq!(name, "原有标题");
            assert_eq!(archived, i64::from(id == "archived"));
            assert_eq!(opaque, [1, 2, 3]);
        }
        drop(db);
        verify_unchanged(&home, &original).unwrap();
        assert_eq!(inventory(&fixture.job().join("source")).unwrap(), original);
        let receipt: Value = read_json(&home.join(RECEIPT)).unwrap();
        assert_eq!(receipt["homeDigest"], home_digest(&home).unwrap());
        super::super::migrate(&fixture.root).unwrap();
    }

    #[test]
    fn committed_locator_update_recovers_before_receipt_activation() {
        let fixture = Fixture::new();
        super::super::migrate(&fixture.root).unwrap();
        let prepared: Repair = read_json(&fixture.job().join("prepared.json")).unwrap();
        fs::remove_file(fixture.job().join("complete.json")).unwrap();
        fs::write(
            fixture.new_home.join(RECEIPT),
            serde_json::to_vec(&prepared.receipt).unwrap(),
        )
        .unwrap();
        super::super::migrate(&fixture.root).unwrap();
        assert!(fixture.job().join("complete.json").is_file());
    }

    #[test]
    fn invalid_profile_binding_is_rejected_with_original_history_preserved() {
        let fixture = Fixture::new();
        let mut receipt: Value = read_json(&fixture.old_home.join(RECEIPT)).unwrap();
        receipt["homeDigest"] = Value::String("different-profile".into());
        fs::write(
            fixture.old_home.join(RECEIPT),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        let original = inventory(&fixture.old_home).unwrap();
        assert!(
            super::super::migrate(&fixture.root)
                .unwrap_err()
                .contains("profile_mismatch")
        );
        assert_eq!(inventory(&fixture.new_home).unwrap(), original);
        assert!(!fixture.job().join("complete.json").exists());
    }

    #[test]
    fn native_writer_busy_blocks_outer_copy() {
        let fixture = Fixture::new();
        let db = connection(&fixture.old_home).unwrap();
        db.execute_batch("BEGIN EXCLUSIVE").unwrap();
        assert!(
            super::super::migrate(&fixture.root)
                .unwrap_err()
                .contains("source_database_busy")
        );
        assert!(fixture.old_home.is_dir());
        assert!(!fixture.new_home.exists());
    }

    #[test]
    fn retry_rejects_changed_paginated_payload() {
        let fixture = Fixture::new();
        super::super::migrate(&fixture.root).unwrap();
        fs::remove_file(fixture.job().join("complete.json")).unwrap();
        let history = Connection::open(fixture.new_home.join("thread_history_1.sqlite")).unwrap();
        history
            .execute("UPDATE turns SET payload=x'00'", [])
            .unwrap();
        drop(history);
        assert!(
            super::super::migrate(&fixture.root)
                .unwrap_err()
                .contains("native_payload_changed")
        );
    }
}
