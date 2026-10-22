//! 主动下载的临时文件清理日志。只在持有 profile 实例锁且尚未启动下载时重试。
//! 下载方须先创建空临时文件并持有该文件的独占锁，再持久化日志，最后写入正文；
//! 持锁直到网络写入结束，关闭句柄后移除临时文件，成功后才调用 `complete`。

use contexthub_platform_adapter::filesystem::{
    FileMetadataSnapshot, open_regular_file_no_follow, opened_file_metadata_with_stable_identity,
    symlink_metadata_with_stable_identity, sync_directory,
};
use contexthub_platform_adapter::security::{
    create_new_restricted_file, ensure_restricted_directory,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const JOURNAL_DIRECTORY: &str = "download-cleanup";
const TEMPORARY_PREFIX: &str = ".cilai-download-";
const MAX_JOURNAL_BYTES: u64 = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    id: String,
    root: PathBuf,
    root_identity: String,
    temporary: PathBuf,
    file_identity: String,
}

#[derive(Debug)]
pub struct CleanupRecord {
    journal_path: PathBuf,
    journal_identity: String,
    record: Journal,
}

#[derive(Debug)]
pub struct PendingCleanup {
    pub journal: PathBuf,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct CleanupReport {
    pub removed: usize,
    pub pending: Vec<PendingCleanup>,
}

/// `temporary` 必须是 `root` 的直属 `.cilai-download-UUID` 普通文件。
/// 调用前必须已通过临时文件的写入句柄取得 `File::try_lock` 独占锁。
///
/// # Errors
///
/// 路径、身份不合法，或日志不能完整写入并持久化时返回错误，调用方不得开始写入正文。
pub fn create(profile: &Path, temporary: &Path, root: &Path) -> io::Result<CleanupRecord> {
    let root_snapshot = directory_snapshot(root)?;
    let root = root.canonicalize()?;
    let name = temporary
        .file_name()
        .ok_or_else(|| invalid("临时文件缺少文件名"))?;
    let id = temporary_id(
        name.to_str()
            .ok_or_else(|| invalid("临时文件名不是 UTF-8"))?,
    )?;
    let parent = temporary
        .parent()
        .ok_or_else(|| invalid("临时文件缺少父目录"))?;
    if parent.canonicalize()? != root {
        return Err(invalid("临时文件必须直接位于登记的下载目录"));
    }
    let temporary = root.join(name);
    let file_snapshot = regular_snapshot(&temporary)?;
    verify_identity(
        &directory_snapshot(&root)?,
        &root_snapshot.stable_identity,
        "下载目录",
    )?;
    let directory = profile.join(JOURNAL_DIRECTORY);
    ensure_restricted_directory(&directory)?;
    let directory = directory.canonicalize()?;
    let journal_path = directory.join(format!("{id}.json"));
    let record = Journal {
        schema_version: 1,
        id,
        root,
        root_identity: root_snapshot.stable_identity,
        temporary,
        file_identity: file_snapshot.stable_identity,
    };
    let bytes = serde_json::to_vec(&record).map_err(invalid)?;
    let mut journal_file = create_new_restricted_file(&journal_path)?;
    let journal_identity =
        opened_file_metadata_with_stable_identity(&journal_file)?.stable_identity;
    journal_file.write_all(&bytes)?;
    journal_file.sync_all()?;
    sync_directory(&record.root)?;
    sync_directory(&directory)?;
    Ok(CleanupRecord {
        journal_path,
        journal_identity,
        record,
    })
}

impl CleanupRecord {
    /// 下载写入结束并关闭写句柄后，核验身份并清除本任务的临时文件及日志。
    ///
    /// # Errors
    ///
    /// 文件仍被锁定、文件或目录身份改变，或清理失败时返回错误并保留日志。
    pub fn cleanup(&self) -> io::Result<()> {
        retry(self)
    }

    /// 只在临时文件已移除后删除日志；失败时保留日志供启动时重试。
    ///
    /// # Errors
    ///
    /// 临时文件仍存在、目录或日志被替换，或删除与持久化失败时返回错误。
    pub fn complete(&self) -> io::Result<()> {
        verify_root(&self.record)?;
        match fs::symlink_metadata(&self.record.temporary) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
            Ok(_) => return Err(pending("临时文件仍存在，清理日志必须保留")),
        }
        sync_directory(&self.record.root)?;
        remove_journal(&self.journal_path, &self.journal_identity)
    }
}

/// 调用者须已取得 profile 实例锁，并保证此时没有新下载或活跃下载写入。
/// 此函数不扫描下载目录，不应在服务处理下载请求期间定时调用。
///
/// # Errors
///
/// 日志目录无法安全访问时返回错误；单条记录的失败保留为报告中的 `pending`。
pub fn cleanup(profile: &Path) -> io::Result<CleanupReport> {
    let directory = profile.join(JOURNAL_DIRECTORY);
    match directory_snapshot(&directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(CleanupReport::default());
        }
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let mut report = CleanupReport::default();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        let result = read_record(&path).and_then(|record| retry(&record));
        match result {
            Ok(()) => report.removed += 1,
            Err(error) => report.pending.push(PendingCleanup {
                journal: path,
                reason: error.to_string(),
            }),
        }
    }
    Ok(report)
}

fn read_record(path: &Path) -> io::Result<CleanupRecord> {
    let id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|_| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .ok_or_else(|| invalid("不识别的下载清理日志文件名"))?;
    validate_id(id)?;
    let snapshot = regular_snapshot(path)?;
    if snapshot.metadata.len() > MAX_JOURNAL_BYTES {
        return Err(invalid("下载清理日志超过大小限制"));
    }
    let file = open_regular_file_no_follow(path, false)?;
    verify_identity(
        &opened_file_metadata_with_stable_identity(&file)?,
        &snapshot.stable_identity,
        "清理日志",
    )?;
    verify_identity(
        &regular_snapshot(path)?,
        &snapshot.stable_identity,
        "清理日志",
    )?;
    let mut bytes = Vec::new();
    file.take(MAX_JOURNAL_BYTES + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_err(invalid)? > MAX_JOURNAL_BYTES {
        return Err(invalid("下载清理日志超过大小限制"));
    }
    let record: Journal = serde_json::from_slice(&bytes).map_err(invalid)?;
    if record.schema_version != 1
        || record.id != id
        || !record.root.is_absolute()
        || record.temporary != record.root.join(format!("{TEMPORARY_PREFIX}{id}"))
    {
        return Err(invalid("下载清理日志的路径或版本不合法"));
    }
    Ok(CleanupRecord {
        journal_path: path.to_path_buf(),
        journal_identity: snapshot.stable_identity,
        record,
    })
}

fn retry(record: &CleanupRecord) -> io::Result<()> {
    verify_root(&record.record)?;
    let snapshot = match regular_snapshot(&record.record.temporary) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return record.complete(),
        other => other?,
    };
    verify_identity(&snapshot, &record.record.file_identity, "临时文件")?;
    let file = open_regular_file_no_follow(&record.record.temporary, true)?;
    verify_identity(
        &opened_file_metadata_with_stable_identity(&file)?,
        &record.record.file_identity,
        "临时文件",
    )?;
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => pending("临时文件仍被活动下载锁定"),
        TryLockError::Error(error) => error,
    })?;
    verify_root(&record.record)?;
    verify_identity(
        &regular_snapshot(&record.record.temporary)?,
        &record.record.file_identity,
        "临时文件",
    )?;
    // Windows 删除前需关闭句柄；profile 实例锁和启动阶段约束防止此处发起新下载。
    drop(file);
    verify_root(&record.record)?;
    verify_identity(
        &regular_snapshot(&record.record.temporary)?,
        &record.record.file_identity,
        "临时文件",
    )?;
    fs::remove_file(&record.record.temporary)?;
    record.complete()
}

fn verify_root(record: &Journal) -> io::Result<()> {
    verify_identity(
        &directory_snapshot(&record.root)?,
        &record.root_identity,
        "下载目录",
    )?;
    if record.root.canonicalize()? != record.root {
        return Err(pending("下载目录路径已变为链接或发生替换"));
    }
    Ok(())
}

fn remove_journal(path: &Path, identity: &str) -> io::Result<()> {
    verify_identity(&regular_snapshot(path)?, identity, "清理日志")?;
    fs::remove_file(path)?;
    sync_directory(path.parent().ok_or_else(|| invalid("清理日志缺少父目录"))?)
}

fn directory_snapshot(path: &Path) -> io::Result<FileMetadataSnapshot> {
    let snapshot = symlink_metadata_with_stable_identity(path)?;
    if !snapshot.metadata.is_dir() || snapshot.metadata.file_type().is_symlink() {
        return Err(pending("下载目录不是可验证的真实目录"));
    }
    Ok(snapshot)
}

fn regular_snapshot(path: &Path) -> io::Result<FileMetadataSnapshot> {
    let snapshot = symlink_metadata_with_stable_identity(path)?;
    if !snapshot.metadata.is_file() || snapshot.metadata.file_type().is_symlink() {
        return Err(pending("清理目标不是可验证的普通文件"));
    }
    Ok(snapshot)
}

fn verify_identity(snapshot: &FileMetadataSnapshot, expected: &str, label: &str) -> io::Result<()> {
    if snapshot.stable_identity != expected {
        return Err(pending(format!("{label}身份已改变，保留文件及清理日志")));
    }
    Ok(())
}

fn temporary_id(name: &str) -> io::Result<String> {
    let id = name
        .strip_prefix(TEMPORARY_PREFIX)
        .ok_or_else(|| invalid("不是受控下载临时文件"))?;
    validate_id(id)?;
    Ok(id.to_owned())
}

fn validate_id(id: &str) -> io::Result<()> {
    let parsed = Uuid::parse_str(id).map_err(invalid)?;
    if parsed.hyphenated().to_string() != id {
        return Err(invalid("下载清理编号不是规范 UUID"));
    }
    Ok(())
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn pending(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    struct Fixture {
        base: PathBuf,
        profile: PathBuf,
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let base =
                std::env::temp_dir().join(format!("cilai-download-cleanup-{}", Uuid::new_v4()));
            let profile = base.join("profile");
            let root = base.join("downloads");
            fs::create_dir_all(&profile).unwrap();
            fs::create_dir(&root).unwrap();
            Self {
                base,
                profile,
                root,
            }
        }
        fn temporary(&self) -> PathBuf {
            let path = self
                .root
                .join(format!("{TEMPORARY_PREFIX}{}", Uuid::new_v4()));
            fs::write(&path, b"partial download").unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    fn startup_cleans_recorded_temporary_and_preserves_user_files() {
        let fixture = Fixture::new();
        let temporary = fixture.temporary();
        let formal = fixture.root.join("用户已下载的文件.docx");
        fs::write(&formal, b"user content").unwrap();
        let unrecorded = fixture.temporary();
        create(&fixture.profile, &temporary, &fixture.root).unwrap();
        let result = cleanup(&fixture.profile).unwrap();
        assert_eq!(result.removed, 1);
        assert!(result.pending.is_empty());
        assert!(!temporary.exists());
        assert!(unrecorded.exists());
        assert_eq!(fs::read(formal).unwrap(), b"user content");
    }

    #[test]
    fn replaced_file_identity_is_preserved_and_reported_pending() {
        let fixture = Fixture::new();
        let temporary = fixture.temporary();
        let record = create(&fixture.profile, &temporary, &fixture.root).unwrap();
        fs::rename(&temporary, fixture.root.join("original-kept")).unwrap();
        fs::write(&temporary, b"replacement user file").unwrap();
        let result = cleanup(&fixture.profile).unwrap();
        assert_eq!(result.removed, 0);
        assert_eq!(result.pending.len(), 1);
        assert!(result.pending[0].reason.contains("身份已改变"));
        assert_eq!(
            result.pending[0].journal.canonicalize().unwrap(),
            record.journal_path
        );
        assert!(record.journal_path.exists());
        assert_eq!(fs::read(temporary).unwrap(), b"replacement user file");
    }

    #[test]
    fn locked_temporary_is_pending_until_its_writer_finishes() {
        let fixture = Fixture::new();
        let temporary = fixture.temporary();
        let file = File::options()
            .read(true)
            .write(true)
            .open(&temporary)
            .unwrap();
        file.try_lock().unwrap();
        create(&fixture.profile, &temporary, &fixture.root).unwrap();
        let pending = cleanup(&fixture.profile).unwrap();
        assert_eq!(pending.removed, 0);
        assert_eq!(pending.pending.len(), 1);
        assert!(temporary.exists());
        drop(file);
        assert_eq!(cleanup(&fixture.profile).unwrap().removed, 1);
    }

    #[test]
    fn replaced_root_is_not_followed() {
        let fixture = Fixture::new();
        let temporary = fixture.temporary();
        let record = create(&fixture.profile, &temporary, &fixture.root).unwrap();
        fs::rename(&fixture.root, fixture.base.join("old-root")).unwrap();
        fs::create_dir(&fixture.root).unwrap();
        fs::write(&temporary, b"new root file").unwrap();
        let report = cleanup(&fixture.profile).unwrap();
        assert_eq!(report.pending.len(), 1);
        assert!(record.journal_path.exists());
        assert_eq!(fs::read(temporary).unwrap(), b"new root file");
    }

    #[test]
    fn complete_requires_temporary_removal_and_keeps_final_download() {
        let fixture = Fixture::new();
        let temporary = fixture.temporary();
        let record = create(&fixture.profile, &temporary, &fixture.root).unwrap();
        assert!(record.complete().is_err());
        let formal = fixture.root.join("最终下载.pptx");
        fs::rename(temporary, &formal).unwrap();
        record.complete().unwrap();
        assert!(!record.journal_path.exists());
        assert!(formal.exists());
    }

    #[test]
    fn unlinked_temporary_removes_only_its_persisted_journal() {
        let fixture = Fixture::new();
        let temporary = fixture.temporary();
        let record = create(&fixture.profile, &temporary, &fixture.root).unwrap();
        fs::remove_file(temporary).unwrap();
        record.cleanup().unwrap();
        assert_eq!(cleanup(&fixture.profile).unwrap().removed, 0);
        assert!(!record.journal_path.exists());
    }

    #[test]
    fn malformed_journal_and_non_temporary_names_are_preserved() {
        let fixture = Fixture::new();
        let formal = fixture.root.join("important.xlsx");
        fs::write(&formal, b"user content").unwrap();
        assert!(create(&fixture.profile, &formal, &fixture.root).is_err());
        let directory = fixture.profile.join(JOURNAL_DIRECTORY);
        fs::create_dir(&directory).unwrap();
        let malformed = directory.join(format!("{}.json", Uuid::new_v4()));
        fs::write(&malformed, b"{unfinished").unwrap();
        let report = cleanup(&fixture.profile).unwrap();
        assert_eq!(report.pending.len(), 1);
        assert!(malformed.exists());
        assert!(formal.exists());
    }
}
