use contexthub_platform_adapter::{backup_link, security};
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const HOMES: [(&str, &str); 2] = [
    ("codex-harness", "cilai-harness"),
    ("application-harness", "cilai-application-harness"),
];

mod official_runtime;
type Tree = BTreeMap<PathBuf, String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Plan {
    version: u32,
    source: PathBuf,
    destination: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Prepared {
    source: Tree,
    destination: Tree,
}

/// Migrate only this Profile's application-owned harness homes before services start.
/// # Errors
/// Rejects conflicting, busy, damaged or unverified data without activating a partial home.
pub fn migrate(profile: &Path) -> Result<(), String> {
    migrate_inner(profile).map_err(|error| format!("cilai_home_migration_failed:{error}"))
}

fn migrate_inner(profile: &Path) -> Result<(), String> {
    let ai = profile.join("ai-runtime");
    if !ai.try_exists().map_err(err)? {
        return Ok(());
    }
    directory(&ai).map_err(|error| format!("inspect_runtime_root:{error}"))?;
    let logical_ai = ai.clone();
    let ai = ai
        .canonicalize()
        .map_err(|error| format!("resolve_runtime_root:{error}"))?;
    let journal = ai.join("cilai-home-migrations-v1");
    let mut legacy_present = false;
    for (old, _) in HOMES {
        match fs::symlink_metadata(ai.join(old)) {
            Ok(_) => legacy_present = true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(err(error)),
        }
    }
    if !journal.try_exists().map_err(err)? && !legacy_present {
        return Ok(());
    }
    security::ensure_restricted_directory(&journal)
        .map_err(|error| format!("prepare_migration_journal:{error}"))?;
    let lock_path = journal.join("migration.lock");
    if !lock_path.try_exists().map_err(err)? {
        security::create_new_restricted_file(&lock_path).map_err(err)?;
    }
    regular_file(&lock_path)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("open_migration_lock:{error}"))?;
    lock.try_lock().map_err(|_| "migration_busy".to_owned())?;
    for (old, new) in HOMES {
        let source = ai.join(old);
        let destination = ai.join(new);
        if source.try_exists().map_err(err)? && destination.try_exists().map_err(err)? {
            return Err(format!("directory_conflict:{old}:{new}"));
        }
        for path in [&source, &destination] {
            if fs::symlink_metadata(path).is_ok() {
                directory(path).map_err(|error| format!("inspect_home_directory:{error}"))?;
            }
        }
    }
    let mut mappings = Vec::new();
    for root in [&logical_ai, &ai] {
        for (old, new) in HOMES {
            let pair = (root.join(old), root.join(new));
            if !mappings.contains(&pair) {
                mappings.push(pair);
            }
        }
    }
    for (old, new) in HOMES {
        migrate_one(&ai, &journal, old, new, &mappings)
            .map_err(|error| format!("migrate_home:{new}:{error}"))?;
        official_runtime::migrate(&ai, &journal, old, new, &mappings)
            .map_err(|error| format!("relocate_official_runtime:{new}:{error}"))?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "归档、激活及中断恢复按同一状态顺序审查"
)]
fn migrate_one(
    ai: &Path,
    journal: &Path,
    old: &str,
    new: &str,
    mappings: &[(PathBuf, PathBuf)],
) -> Result<(), String> {
    let source = ai.join(old);
    let destination = ai.join(new);
    let job = journal.join(new);
    if !source.try_exists().map_err(err)? && !job.try_exists().map_err(err)? {
        return Ok(());
    }
    security::ensure_restricted_directory(&job)
        .map_err(|error| format!("prepare_migration_job:{error}"))?;
    let plan = Plan {
        version: 1,
        source: source.clone(),
        destination: destination.clone(),
    };
    let plan_path = job.join("plan.json");
    if plan_path.try_exists().map_err(err)? {
        if read_json::<Plan>(&plan_path).map_err(|error| format!("read_migration_plan:{error}"))?
            != plan
        {
            return Err("migration_plan_mismatch".into());
        }
    } else {
        write_json(&plan_path, &plan).map_err(|error| format!("write_migration_plan:{error}"))?;
    }
    let backup = job.join("source");
    let stage = job.join("staging");
    let prepared_path = job.join("prepared.json");
    let complete = job.join("complete.json");
    if complete.try_exists().map_err(err)? {
        if read_json::<Plan>(&complete)? != plan || source.exists() || !destination.is_dir() {
            return Err("completed_migration_conflict".into());
        }
        return Ok(());
    }
    if !prepared_path.try_exists().map_err(err)? {
        if !source.is_dir() || backup.exists() || destination.exists() {
            return Err("unprepared_migration_conflict".into());
        }
        let _runtime_locks =
            runtime_locks(&source).map_err(|error| format!("lock_source_runtime:{error}"))?;
        let _databases =
            database_locks(&source).map_err(|error| format!("lock_source_databases:{error}"))?;
        let original = inventory(&source)?;
        if stage.try_exists().map_err(err)? {
            directory(&stage)?;
            fs::remove_dir_all(&stage).map_err(err)?;
        }
        copy_tree(&source, &stage, &source).map_err(|error| format!("copy_source_home:{error}"))?;
        if inventory(&stage)? != original || inventory(&source)? != original {
            return Err("source_changed_during_copy".into());
        }
        rewrite_home(&stage, mappings)
            .map_err(|error| format!("prepare_destination_home:{error}"))?;
        let transformed = inventory(&stage)?;
        if inventory(&source)? != original {
            return Err("source_changed_during_prepare".into());
        }
        write_json(
            &prepared_path,
            &Prepared {
                source: original,
                destination: transformed,
            },
        )?;
    }
    let prepared: Prepared =
        read_json(&prepared_path).map_err(|error| format!("read_prepared_migration:{error}"))?;
    if source.try_exists().map_err(err)? {
        if backup.exists() || destination.exists() || inventory(&source)? != prepared.source {
            return Err("source_changed_before_archive".into());
        }
        let runtime_locks = runtime_locks(&source)?;
        let databases = database_locks(&source)?;
        if inventory(&source)? != prepared.source || inventory(&stage)? != prepared.destination {
            return Err("prepared_migration_changed".into());
        }
        drop(databases);
        drop(runtime_locks);
        fs::rename(&source, &backup).map_err(|error| format!("archive_source_home:{error}"))?;
    }
    directory(&backup).map_err(|error| format!("inspect_source_backup:{error}"))?;
    if inventory(&backup).map_err(|error| format!("verify_source_backup:{error}"))?
        != prepared.source
    {
        return Err("migration_backup_changed".into());
    }
    if destination.try_exists().map_err(err)? {
        if stage.exists()
            || inventory(&destination)
                .map_err(|error| format!("verify_existing_destination:{error}"))?
                != prepared.destination
        {
            return Err("unconfirmed_destination_changed".into());
        }
    } else {
        if inventory(&stage).map_err(|error| format!("verify_destination_stage:{error}"))?
            != prepared.destination
        {
            return Err("migration_stage_changed".into());
        }
        fs::rename(&stage, &destination)
            .map_err(|error| format!("activate_destination_home:{error}"))?;
    }
    security::ensure_owner_writable_runtime_tree(&destination)
        .map_err(|error| format!("prepare_destination_access:{error}"))?;
    write_json(&complete, &plan).map_err(|error| format!("complete_migration:{error}"))?;
    Ok(())
}

fn directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(err)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("directory_must_not_be_link".into());
    }
    Ok(())
}

fn regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(err)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("file_must_not_be_link".into());
    }
    Ok(())
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    regular_file(path)?;
    serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let pending = path.with_extension("pending");
    if pending.try_exists().map_err(err)? {
        regular_file(&pending)?;
        fs::remove_file(&pending).map_err(err)?;
    }
    let mut file = security::create_new_restricted_file(&pending).map_err(err)?;
    file.write_all(&serde_json::to_vec(value).map_err(err)?)
        .map_err(err)?;
    file.sync_all().map_err(err)?;
    drop(file);
    if path.try_exists().map_err(err)? {
        return Err("journal_already_exists".into());
    }
    fs::rename(pending, path).map_err(err)
}

fn entries(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut entries = fs::read_dir(path)
        .map_err(err)?
        .map(|entry| entry.map(|entry| entry.path()).map_err(err))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    Ok(entries)
}

fn transient(relative: &Path) -> bool {
    // 协调锁仍随来源目录归档；迁移期间持有锁时，Windows 不允许另一句柄读取它们。
    // 新目录由新进程创建自己的锁，不能把旧进程的协调文件当作历史内容复制。
    relative.starts_with("tmp")
        || relative.to_string_lossy().ends_with(".sqlite-shm")
        || relative == Path::new("memory-worker.lock")
        || (relative.starts_with("thread-writer-locks")
            && relative.to_string_lossy().ends_with(".lock"))
}

fn inventory(root: &Path) -> Result<Tree, String> {
    directory(root)?;
    let mut result = Tree::new();
    inventory_at(root, root, &mut result)?;
    Ok(result)
}

fn inventory_at(root: &Path, directory: &Path, result: &mut Tree) -> Result<(), String> {
    for path in entries(directory)? {
        let relative = path.strip_prefix(root).map_err(err)?;
        if transient(relative) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(err)?;
        let value = if metadata.file_type().is_symlink() {
            format!("link:{}", fs::read_link(&path).map_err(err)?.display())
        } else if metadata.is_dir() {
            inventory_at(root, &path, result)?;
            "directory".into()
        } else if metadata.is_file() {
            let mut file = File::open(&path).map_err(err)?;
            let mut digest = Sha256::new();
            let mut buffer = vec![0_u8; 65_536];
            loop {
                let size = file.read(&mut buffer).map_err(err)?;
                if size == 0 {
                    break;
                }
                digest.update(&buffer[..size]);
            }
            format!("file:{:x}", digest.finalize())
        } else {
            return Err("unsupported_source_entry".into());
        };
        result.insert(relative.to_path_buf(), value);
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path, root: &Path) -> Result<(), String> {
    security::ensure_restricted_directory(destination).map_err(err)?;
    for path in entries(source)? {
        if transient(path.strip_prefix(root).map_err(err)?) {
            continue;
        }
        let target = destination.join(path.file_name().ok_or("invalid_source_name")?);
        let metadata = fs::symlink_metadata(&path).map_err(err)?;
        if metadata.file_type().is_symlink() {
            backup_link::copy_symbolic_link(&path, &target).map_err(err)?;
        } else if metadata.is_dir() {
            copy_tree(&path, &target, root)?;
        } else if metadata.is_file() {
            fs::copy(&path, &target).map_err(err)?;
        } else {
            return Err("unsupported_source_entry".into());
        }
    }
    Ok(())
}

fn runtime_locks(root: &Path) -> Result<Vec<File>, String> {
    let mut paths = vec![root.join("memory-worker.lock")];
    for directory in [root.join("thread-writer-locks"), root.join("tmp/arg0")] {
        if directory.try_exists().map_err(err)? {
            collect_locks(&directory, &mut paths)?;
        }
    }
    let mut locks = Vec::new();
    for path in paths {
        if !path.try_exists().map_err(err)? {
            continue;
        }
        regular_file(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(err)?;
        file.try_lock()
            .map_err(|_| "source_runtime_busy".to_owned())?;
        locks.push(file);
    }
    Ok(locks)
}

fn collect_locks(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
    self::directory(directory)?;
    for path in entries(directory)? {
        let kind = fs::symlink_metadata(&path).map_err(err)?;
        if kind.file_type().is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_locks(&path, paths)?;
        } else if path.to_string_lossy().ends_with(".lock") {
            paths.push(path);
        }
    }
    Ok(())
}

fn databases(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut result = Vec::new();
    for path in entries(root)? {
        if path
            .extension()
            .is_some_and(|extension| extension == "sqlite")
        {
            regular_file(&path)?;
            for suffix in ["-wal", "-shm", "-journal"] {
                let sidecar = path.with_file_name(format!(
                    "{}{suffix}",
                    path.file_name()
                        .ok_or("invalid_database_name")?
                        .to_string_lossy()
                ));
                match fs::symlink_metadata(&sidecar) {
                    Ok(_) => regular_file(&sidecar)?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => return Err(err(error)),
                }
            }
            result.push(path);
        }
    }
    Ok(result)
}

fn database_locks(root: &Path) -> Result<Vec<Connection>, String> {
    let mut paths = databases(root)?;
    let official = root.join("official-runtime");
    if official.try_exists().map_err(err)? {
        directory(&official)?;
        paths.extend(databases(&official)?);
    }
    paths
        .into_iter()
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
                .execute_batch("PRAGMA busy_timeout=0; BEGIN EXCLUSIVE;")
                .map_err(|_| "source_database_busy_or_invalid".to_owned())?;
            Ok(connection)
        })
        .collect()
}

fn rebase(value: &str, mappings: &[(PathBuf, PathBuf)]) -> Option<String> {
    let value = contexthub_platform_adapter::filesystem::path_for_comparison(Path::new(value));
    mappings.iter().find_map(|(old, new)| {
        let old = contexthub_platform_adapter::filesystem::path_for_comparison(old);
        value.strip_prefix(old).ok().map(|relative| {
            if relative.as_os_str().is_empty() {
                new.clone()
            } else {
                new.join(relative)
            }
            .to_string_lossy()
            .into_owned()
        })
    })
}

fn rewrite_home(stage: &Path, mappings: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    for database in databases(stage)? {
        rewrite_database(&database, mappings, stage).map_err(|error| {
            format!(
                "rewrite_database:{}:{error}",
                database.file_name().unwrap_or_default().to_string_lossy()
            )
        })?;
    }
    let config = stage.join("config.toml");
    if config.try_exists().map_err(err)? {
        regular_file(&config)?;
        let original = fs::read_to_string(&config).map_err(err)?;
        let mut text = original.clone();
        for (old, new) in mappings {
            for escaped in [false, true] {
                let (old, new) = if escaped {
                    (
                        serde_json::to_string(&old.to_string_lossy()).map_err(err)?,
                        serde_json::to_string(&new.to_string_lossy()).map_err(err)?,
                    )
                } else {
                    (
                        format!("\"{}\"", old.display()),
                        format!("\"{}\"", new.display()),
                    )
                };
                for suffix in ["/", "\\", "\\\\", "\"", "'"] {
                    let from = format!("{}{suffix}", old.trim_matches('"'));
                    let to = format!("{}{suffix}", new.trim_matches('"'));
                    text = text.replace(&from, &to);
                }
            }
        }
        if text != original {
            fs::write(config, text).map_err(err)?;
        }
    }
    for relative in inventory(stage)?.keys() {
        let path = stage.join(relative);
        let metadata = fs::symlink_metadata(&path).map_err(err)?;
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path).map_err(err)?;
            if let Some(target) = rebase(&target.to_string_lossy(), mappings) {
                backup_link::replace_symbolic_link_target(&path, Path::new(&target))
                    .map_err(err)?;
            }
        } else if metadata.is_file()
            && relative
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            && (relative.starts_with("sessions") || relative.starts_with("archived_sessions"))
        {
            rewrite_rollout(&path, mappings)?;
        }
    }
    rewrite_memory_paths(stage, mappings)?;
    Ok(())
}

fn rewrite_memory_paths(stage: &Path, mappings: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    let working = stage.join("memories");
    if working.try_exists().map_err(err)? {
        rewrite_memory_files(&working, mappings)?;
    }
    let publication = stage.join("memory-publication");
    let current = publication.join("current");
    if !current.try_exists().map_err(err)? {
        return Ok(());
    }
    regular_file(&current)?;
    let previous = fs::read_to_string(&current).map_err(err)?;
    if previous.is_empty() {
        return Ok(());
    }
    uuid::Uuid::parse_str(&previous).map_err(err)?;
    let original = publication.join("versions").join(&previous);
    directory(&original)?;
    let revision = uuid::Uuid::new_v4().to_string();
    let derived = publication.join("versions").join(&revision);
    copy_tree(&original, &derived, &original)?;
    if !rewrite_memory_files(&derived.join("memories"), mappings)? {
        fs::remove_dir_all(&derived).map_err(err)?;
        return Ok(());
    }
    let manifest_path = derived.join("manifest.json");
    let mut manifest: Value = read_json(&manifest_path)?;
    if manifest["revision"] != previous || !manifest.is_object() {
        return Err("memory_publication_invalid".into());
    }
    manifest["revision"] = Value::String(revision.clone());
    fs::write(&manifest_path, serde_json::to_vec(&manifest).map_err(err)?).map_err(err)?;
    fs::write(current, revision).map_err(err)?;
    Ok(())
}

fn rewrite_memory_files(root: &Path, mappings: &[(PathBuf, PathBuf)]) -> Result<bool, String> {
    let mut changed = false;
    for relative in inventory(root)?
        .keys()
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
    {
        let path = root.join(relative);
        regular_file(&path)?;
        let original = fs::read_to_string(&path).map_err(err)?;
        let mut text = original.clone();
        for (old, new) in mappings {
            for suffix in ["/", "\\", " ", "\n", ")", "]", "`", "\""] {
                text = text.replace(
                    &format!("{}{suffix}", old.display()),
                    &format!("{}{suffix}", new.display()),
                );
            }
        }
        if text != original {
            fs::write(path, text).map_err(err)?;
            changed = true;
        }
    }
    Ok(changed)
}

fn rewrite_database(
    path: &Path,
    mappings: &[(PathBuf, PathBuf)],
    stage: &Path,
) -> Result<(), String> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|error| format!("open:{error}"))?;
    // 与工作台主资料库一致，迁移连接的临时数据也留在内存中，
    // 不依赖服务精简环境中未提供的 TEMP/TMP 或系统目录写权限。
    connection
        .pragma_update(None, "temp_store", "MEMORY")
        .map_err(|error| format!("configure_temporary_storage:{error}"))?;
    let transaction = connection.transaction().map_err(err)?;
    let tables = transaction
        .prepare("SELECT name FROM sqlite_schema WHERE type='table'")
        .map_err(err)?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    for table in tables {
        let quoted_table = quote(&table);
        let columns = transaction
            .prepare(&format!("PRAGMA table_info({quoted_table})"))
            .map_err(err)?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?;
        for column in columns.into_iter().filter(|column| {
            matches!(
                column.as_str(),
                "path"
                    | "cwd"
                    | "rollout_path"
                    | "agent_path"
                    | "input_csv_path"
                    | "output_csv_path"
            )
        }) {
            let quoted_column = quote(&column);
            for (old, new) in mappings {
                let old = old.to_string_lossy();
                let new = new.to_string_lossy();
                transaction.execute(&format!("UPDATE {quoted_table} SET {quoted_column}=?2 || substr({quoted_column},length(?1)+1) WHERE typeof({quoted_column})='text' AND ({quoted_column}=?1 OR substr({quoted_column},1,length(?1)+1)=?1 || '/' OR substr({quoted_column},1,length(?1)+1)=?1 || char(92))"), params![old.as_ref(), new.as_ref()]).map_err(err)?;
            }
            if column == "rollout_path" {
                let paths = transaction.prepare(&format!("SELECT {quoted_column} FROM {quoted_table} WHERE typeof({quoted_column})='text'")).map_err(err)?
                    .query_map([], |row| row.get::<_, String>(0)).map_err(err)?.collect::<Result<Vec<_>, _>>().map_err(err)?;
                for value in paths {
                    if let Some((_, new)) = mappings.iter().find(|(_, new)| {
                        stage.file_name() == Some(std::ffi::OsStr::new("staging"))
                            && Path::new(&value).starts_with(new)
                    }) {
                        // Only the current home's own rollout paths are verified against this stage.
                        if new.file_name()
                            == path
                                .parent()
                                .and_then(Path::parent)
                                .and_then(Path::file_name)
                        {
                            let relative = Path::new(&value).strip_prefix(new).map_err(err)?;
                            if relative
                                .components()
                                .any(|part| !matches!(part, std::path::Component::Normal(_)))
                            {
                                return Err("indexed_rollout_path_escape".into());
                            }
                            let record = stage.join(relative);
                            if !record.is_file() {
                                return Err("indexed_rollout_missing".into());
                            }
                            regular_file(&record)?;
                            if !record
                                .canonicalize()
                                .map_err(err)?
                                .starts_with(stage.canonicalize().map_err(err)?)
                            {
                                return Err("indexed_rollout_path_escape".into());
                            }
                        }
                    }
                }
            }
        }
    }
    let integrity: String = transaction
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| format!("check_integrity:{error}"))?;
    if integrity != "ok" {
        return Err("native_database_integrity_failed".into());
    }
    transaction
        .commit()
        .map_err(|error| format!("commit:{error}"))?;
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|error| format!("checkpoint:{error}"))?;
    Ok(())
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn rewrite_rollout(path: &Path, mappings: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    let original = fs::read_to_string(path).map_err(err)?;
    let mut changed = false;
    let mut output = String::new();
    for line in original.split_inclusive('\n') {
        let mut record: Value = serde_json::from_str(line).map_err(err)?;
        if matches!(
            record["type"].as_str(),
            Some("session_meta" | "turn_context")
        ) && rebase_metadata(&mut record["payload"], mappings)
        {
            output.push_str(&serde_json::to_string(&record).map_err(err)?);
            if line.ends_with('\n') {
                output.push('\n');
            }
            changed = true;
        } else {
            output.push_str(line);
        }
    }
    if changed {
        fs::write(path, output).map_err(err)?;
    }
    Ok(())
}

fn rebase_metadata(value: &mut Value, mappings: &[(PathBuf, PathBuf)]) -> bool {
    let mut changed = false;
    if let Value::Object(object) = value {
        for (key, value) in object {
            if matches!(
                key.as_str(),
                "cwd" | "path" | "rollout_path" | "harness_home" | "harnessHome"
            ) {
                if let Some(new) = value.as_str().and_then(|value| rebase(value, mappings)) {
                    *value = Value::String(new);
                    changed = true;
                }
            } else if key == "writable_roots" {
                if let Some(paths) = value.as_array_mut() {
                    for path in paths {
                        if let Some(new) = path.as_str().and_then(|value| rebase(value, mappings)) {
                            *path = Value::String(new);
                            changed = true;
                        }
                    }
                }
            } else if matches!(key.as_str(), "sandbox_policy" | "sandboxPolicy") {
                changed |= rebase_metadata(value, mappings);
            }
        }
    }
    changed
}

#[cfg(test)]
#[path = "harness_home_migration_tests.rs"]
mod tests;
