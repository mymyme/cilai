//! 工作区文件打开、稳定身份与云端占位文件识别等跨平台文件系统语义。

use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::path::Path;

/// 以只读方式打开工作区文件，并应用当前平台避免云端占位文件读取阻塞所需的打开参数。
///
/// # Errors
///
/// 目标不可读或系统拒绝打开时返回原始 I/O 错误。
pub fn open_workspace_file_for_read(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    platform::configure_workspace_read(&mut options);
    options.open(path)
}

/// 以只读方式打开需要稳定内容快照的工作区文件。Windows 端会拒绝并发写共享，
/// 避免读取期间接受修改时间尚未落盘的等长原位改写；普通读取的共享语义不变。
///
/// # Errors
///
/// 目标不可读、已被写句柄共享，或系统拒绝打开时返回原始 I/O 错误。
pub fn open_stable_workspace_file_for_read(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    platform::configure_stable_workspace_read(&mut options);
    options.open(path)
}

/// 打开已有普通文件，禁止跟随末级符号链接或 Windows 重解析点；不创建或截断文件。
/// `write` 仅控制是否额外授予写权限。父目录范围和前后稳定身份仍由调用方核验。
///
/// # Errors
///
/// 目标不是普通文件、为链接或重解析点，或系统拒绝打开时返回 I/O 错误。
pub fn open_regular_file_no_follow(path: &Path, write: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(write);
    platform::configure_no_follow(&mut options);
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || platform::is_reparse_point(&metadata)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular file without a reparse point",
        ));
    }
    Ok(file)
}

#[must_use]
pub fn is_link_or_reparse_point(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink() || platform::is_reparse_point(metadata)
}

/// 同一文件系统快照中的元数据与稳定身份。两项始终来自同一次路径查询或同一打开句柄，
/// 调用方不能把旧元数据与重开路径后的新身份拼接成伪快照。
#[derive(Debug)]
pub struct FileMetadataSnapshot {
    pub metadata: Metadata,
    pub stable_identity: String,
}

/// 查询路径当前指向对象的元数据与稳定身份，并跟随符号链接。
///
/// # Errors
///
/// 路径不可访问或稳定身份查询失败时返回错误。
pub fn metadata_with_stable_identity(path: &Path) -> io::Result<FileMetadataSnapshot> {
    platform::path_metadata_snapshot(path, true)
}

/// 查询路径本身的元数据与稳定身份，不跟随符号链接。
///
/// # Errors
///
/// 路径不可访问或稳定身份查询失败时返回错误。
pub fn symlink_metadata_with_stable_identity(path: &Path) -> io::Result<FileMetadataSnapshot> {
    platform::path_metadata_snapshot(path, false)
}

/// 从同一个已打开文件句柄取得元数据与稳定身份。
///
/// # Errors
///
/// 句柄元数据或稳定身份查询失败时返回错误。
pub fn opened_file_metadata_with_stable_identity(file: &File) -> io::Result<FileMetadataSnapshot> {
    platform::opened_file_metadata_snapshot(file)
}

/// 该文件是否为尚未下载正文的云端占位文件（如 iCloud Drive「按需下载」文件、
/// OneDrive/Dropbox 等基于 Cloud Files API 的按需文件）。调用方据此在打开正文前给出
/// 明确的“尚未落地”错误，而不是让读取阻塞等待远端下载。
#[must_use]
pub fn is_cloud_placeholder_file(metadata: &Metadata) -> bool {
    platform::is_cloud_placeholder_file(metadata)
}

/// 将目录变更刷新到文件系统，供原子替换记录和清单后使用。
///
/// # Errors
///
/// 目录不能打开或刷新失败时返回原始错误，不把未落盘状态当作成功。
pub fn sync_directory(path: &Path) -> io::Result<()> {
    platform::sync_directory(path)
}

/// 用于范围比较的平台路径拼写。Windows 仅归一普通盘符和 UNC 的扩展前缀，
/// 不解析链接、不改变大小写或接受设备路径别名；调用方仍需核验实际文件身份。
#[must_use]
pub fn path_for_comparison(path: &Path) -> std::path::PathBuf {
    platform::path_for_comparison(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "contexthub-filesystem-test-{}-{name}",
            std::process::id()
        ))
    }

    #[cfg(windows)]
    #[test]
    fn comparison_spelling_preserves_identity_and_rejects_device_alias_conversion() {
        let directory = fixture("comparison-spelling");
        std::fs::create_dir_all(&directory).unwrap();
        let canonical = directory.canonicalize().unwrap();
        let plain = path_for_comparison(&canonical);
        assert!(!plain.as_os_str().to_string_lossy().starts_with(r"\\?\"));
        assert_eq!(
            metadata_with_stable_identity(&plain)
                .unwrap()
                .stable_identity,
            metadata_with_stable_identity(&canonical)
                .unwrap()
                .stable_identity
        );
        assert_eq!(
            path_for_comparison(Path::new(r"\\?\UNC\server\share\资料\file")),
            Path::new(r"\\server\share\资料\file")
        );
        for special in [
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\file",
            r"\\.\pipe\private",
            "relative/file",
        ] {
            assert_eq!(path_for_comparison(Path::new(special)), Path::new(special));
        }
        std::fs::remove_dir(&directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn comparison_spelling_keeps_non_unicode_path_units_distinct() {
        use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
        let mut original: Vec<u16> = r"\\?\C:\private\".encode_utf16().collect();
        original.push(0xd800);
        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&original));
        let actual: Vec<u16> = path_for_comparison(&path)
            .as_os_str()
            .encode_wide()
            .collect();
        assert_eq!(actual, original[4..]);
    }

    #[test]
    fn no_follow_open_preserves_existing_content_and_does_not_create_files() {
        use std::io::{Seek as _, SeekFrom, Write as _};

        let directory = fixture("no-follow-regular");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("existing.txt");
        std::fs::write(&path, b"original").unwrap();
        let mut reader = open_regular_file_no_follow(&path, false).unwrap();
        let mut content = String::new();
        reader.read_to_string(&mut content).unwrap();
        assert_eq!(content, "original");
        assert!(reader.write_all(b"forbidden").is_err());
        drop(reader);

        let mut writer = open_regular_file_no_follow(&path, true).unwrap();
        assert_eq!(writer.metadata().unwrap().len(), 8);
        writer.seek(SeekFrom::Start(0)).unwrap();
        writer.write_all(b"updated!").unwrap();
        drop(writer);
        assert_eq!(std::fs::read(&path).unwrap(), b"updated!");

        for write in [false, true] {
            assert!(open_regular_file_no_follow(&directory, write).is_err());
            let missing = directory.join("missing.txt");
            assert_eq!(
                open_regular_file_no_follow(&missing, write)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::NotFound
            );
            assert!(!missing.exists());
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn no_follow_open_rejects_live_and_broken_symlinks_without_touching_the_target() {
        use std::os::unix::fs::symlink;

        let directory = fixture("no-follow-symlink");
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("target.txt");
        std::fs::write(&target, b"keep").unwrap();
        let link = directory.join("link.txt");
        let broken = directory.join("broken.txt");
        symlink(&target, &link).unwrap();
        symlink(directory.join("missing.txt"), &broken).unwrap();
        for write in [false, true] {
            assert!(open_regular_file_no_follow(&link, write).is_err());
            assert!(open_regular_file_no_follow(&broken, write).is_err());
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        assert!(!directory.join("missing.txt").exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    #[allow(unsafe_code)]
    fn no_follow_open_refuses_a_fifo_without_waiting_for_a_peer() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt as _;

        let directory = fixture("no-follow-fifo");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("fifo");
        let encoded = CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: `encoded` 在调用期间保持有效且以 NUL 结尾；只创建本测试目录内的 FIFO。
        assert_eq!(unsafe { libc::mkfifo(encoded.as_ptr(), 0o600) }, 0);
        for write in [false, true] {
            assert!(open_regular_file_no_follow(&path, write).is_err());
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn directory_sync_succeeds_and_reports_a_missing_directory() {
        let directory = fixture("directory-sync");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("record.json"), b"{}").unwrap();
        sync_directory(&directory).unwrap();
        assert_eq!(
            sync_directory(&directory.join("missing"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn ordinary_local_file_is_not_a_cloud_placeholder() {
        let directory = fixture("placeholder");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ordinary.txt");
        std::fs::write(&path, b"local content").unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(!is_cloud_placeholder_file(&metadata));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn workspace_read_and_stable_identity_are_platform_neutral() {
        let directory = fixture("identity");
        std::fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first.txt");
        let second = directory.join("second.txt");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();

        let first_snapshot = metadata_with_stable_identity(&first).unwrap();
        let first_identity = first_snapshot.stable_identity;
        assert_eq!(
            metadata_with_stable_identity(&first)
                .unwrap()
                .stable_identity,
            first_identity
        );
        assert_ne!(
            metadata_with_stable_identity(&second)
                .unwrap()
                .stable_identity,
            first_identity
        );

        let mut contents = String::new();
        let mut file = open_workspace_file_for_read(&first).unwrap();
        assert_eq!(
            opened_file_metadata_with_stable_identity(&file)
                .unwrap()
                .stable_identity,
            first_identity
        );
        file.read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "first");

        contents.clear();
        let mut stable_file = open_stable_workspace_file_for_read(&first).unwrap();
        stable_file.read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "first");

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn path_snapshot_keeps_replacement_metadata_and_identity_together() {
        let directory = fixture("replacement-snapshot");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("current.txt");
        let replacement = directory.join("replacement.txt");
        std::fs::write(&path, b"old!").unwrap();
        std::fs::write(&replacement, b"new!").unwrap();
        let before = metadata_with_stable_identity(&path).unwrap();
        let replacement_before = metadata_with_stable_identity(&replacement).unwrap();

        std::fs::remove_file(&path).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let after = metadata_with_stable_identity(&path).unwrap();

        assert_eq!(before.metadata.len(), after.metadata.len());
        assert_ne!(before.stable_identity, after.stable_identity);
        assert_eq!(replacement_before.stable_identity, after.stable_identity);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    #[allow(unsafe_code)]
    fn unix_workspace_read_preserves_nonblocking_flag() {
        use std::os::fd::AsRawFd as _;

        let directory = fixture("nonblocking");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("note.txt");
        std::fs::write(&path, b"note").unwrap();
        let file = open_workspace_file_for_read(&path).unwrap();

        // SAFETY: `file.as_raw_fd()` 是本作用域内仍有效的只读文件描述符，`F_GETFL` 只查询
        // 描述符标志，不写入指针或转移所有权。
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        assert_ne!(flags, -1);
        assert_ne!(flags & libc::O_NONBLOCK, 0);

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_stable_identity_keeps_existing_wire_format() {
        use std::os::unix::fs::MetadataExt as _;

        let directory = fixture("unix-identity-format");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("note.txt");
        std::fs::write(&path, b"note").unwrap();
        let snapshot = metadata_with_stable_identity(&path).unwrap();
        let metadata = snapshot.metadata;
        assert_eq!(
            snapshot.stable_identity,
            format!("dev:{}:ino:{}", metadata.dev(), metadata.ino())
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_stable_identity_uses_volume_and_file_index() {
        let directory = fixture("windows-identity-format");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("note.txt");
        std::fs::write(&path, b"note").unwrap();
        let identity = metadata_with_stable_identity(&path)
            .unwrap()
            .stable_identity;
        assert!(identity.starts_with("volume:"));
        assert!(identity.contains(":file:"));
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_stable_read_refuses_existing_or_new_writers() {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };

        let directory = fixture("stable-read-sharing");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("note.txt");
        std::fs::write(&path, b"note").unwrap();

        let existing_writer = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .open(&path)
            .unwrap();
        assert!(open_stable_workspace_file_for_read(&path).is_err());
        drop(existing_writer);

        let stable_reader = open_stable_workspace_file_for_read(&path).unwrap();
        assert!(
            OpenOptions::new()
                .write(true)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .open(&path)
                .is_err()
        );
        drop(stable_reader);

        std::fs::remove_dir_all(&directory).unwrap();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::FileMetadataSnapshot;
    use std::fs::{self, File, Metadata, OpenOptions};
    use std::io;
    use std::path::Path;

    pub fn path_for_comparison(path: &Path) -> std::path::PathBuf {
        path.to_path_buf()
    }

    pub fn sync_directory(path: &Path) -> io::Result<()> {
        File::open(path).and_then(|directory| directory.sync_all())
    }

    pub fn configure_workspace_read(options: &mut OpenOptions) {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
    }

    pub fn configure_stable_workspace_read(options: &mut OpenOptions) {
        configure_workspace_read(options);
    }

    pub fn configure_no_follow(options: &mut OpenOptions) {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }

    pub fn is_reparse_point(_metadata: &Metadata) -> bool {
        false
    }

    pub fn path_metadata_snapshot(
        path: &Path,
        follow_symlinks: bool,
    ) -> io::Result<FileMetadataSnapshot> {
        let metadata = if follow_symlinks {
            fs::metadata(path)?
        } else {
            fs::symlink_metadata(path)?
        };
        Ok(snapshot(metadata))
    }

    pub fn opened_file_metadata_snapshot(file: &File) -> io::Result<FileMetadataSnapshot> {
        Ok(snapshot(file.metadata()?))
    }

    fn snapshot(metadata: Metadata) -> FileMetadataSnapshot {
        use std::os::unix::fs::MetadataExt as _;
        let stable_identity = format!("dev:{}:ino:{}", metadata.dev(), metadata.ino());
        FileMetadataSnapshot {
            metadata,
            stable_identity,
        }
    }

    pub fn is_cloud_placeholder_file(metadata: &Metadata) -> bool {
        use std::os::macos::fs::MetadataExt as _;
        const SF_DATALESS: u32 = 0x4000_0000;
        metadata.st_flags() & SF_DATALESS != 0
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use super::FileMetadataSnapshot;
    use std::fmt::Write as _;
    use std::fs::{File, Metadata, OpenOptions};
    use std::io;
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::os::windows::io::AsRawHandle as _;
    use std::path::Path;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo,
        GetFileInformationByHandleEx,
    };

    pub fn path_for_comparison(path: &Path) -> std::path::PathBuf {
        use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
        use std::path::{Component, PathBuf, Prefix};
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            return path.to_path_buf();
        };
        let original: Vec<u16> = path.as_os_str().encode_wide().collect();
        match prefix.kind() {
            Prefix::VerbatimDisk(_) => PathBuf::from(std::ffi::OsString::from_wide(&original[4..])),
            Prefix::VerbatimUNC(_, _) => {
                let mut plain = vec![u16::from(b'\\'), u16::from(b'\\')];
                plain.extend_from_slice(&original[8..]);
                PathBuf::from(std::ffi::OsString::from_wide(&plain))
            }
            _ => path.to_path_buf(),
        }
    }

    pub fn sync_directory(path: &Path) -> io::Result<()> {
        // CreateFile 打开目录需要 BACKUP_SEMANTICS；FlushFileBuffers 要求 GENERIC_WRITE。
        // 只读普通文件句柄在 Windows 上不能履行目录刷新契约。
        OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?
            .sync_all()
    }

    pub fn configure_workspace_read(_options: &mut OpenOptions) {}

    pub fn configure_stable_workspace_read(options: &mut OpenOptions) {
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE);
    }

    pub fn configure_no_follow(options: &mut OpenOptions) {
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    pub fn is_reparse_point(metadata: &Metadata) -> bool {
        use std::os::windows::fs::MetadataExt as _;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    pub fn path_metadata_snapshot(
        path: &Path,
        follow_symlinks: bool,
    ) -> io::Result<FileMetadataSnapshot> {
        let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
        if !follow_symlinks {
            flags |= FILE_FLAG_OPEN_REPARSE_POINT;
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(flags);
        let file = options.open(path)?;
        opened_file_metadata_snapshot(&file)
    }

    pub fn opened_file_metadata_snapshot(file: &File) -> io::Result<FileMetadataSnapshot> {
        let metadata = file.metadata()?;
        let stable_identity = identity_from_handle(file.as_raw_handle().cast())?;
        Ok(FileMetadataSnapshot {
            metadata,
            stable_identity,
        })
    }

    fn identity_from_handle(handle: HANDLE) -> io::Result<String> {
        let mut information = FILE_ID_INFO::default();
        // SAFETY: `handle` 由调用方保证在本次调用期间有效；`information` 指向可写且尺寸
        // 正确的结构体，不跨调用保存。
        if unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                (&raw mut information).cast(),
                u32::try_from(std::mem::size_of::<FILE_ID_INFO>())
                    .expect("FILE_ID_INFO size must fit in u32"),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if information.VolumeSerialNumber == 0
            || information.FileId.Identifier.iter().all(|byte| *byte == 0)
            || information
                .FileId
                .Identifier
                .iter()
                .all(|byte| *byte == u8::MAX)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "workspace file has no stable Windows identity",
            ));
        }
        let mut file_id = String::with_capacity(32);
        for byte in information.FileId.Identifier {
            write!(&mut file_id, "{byte:02x}").expect("writing to String cannot fail");
        }
        Ok(format!(
            "volume:{}:file:{file_id}",
            information.VolumeSerialNumber
        ))
    }

    pub fn is_cloud_placeholder_file(metadata: &Metadata) -> bool {
        use std::os::windows::fs::MetadataExt as _;
        // Cloud Files API（OneDrive 等按需文件提供方所依赖的机制）为尚未落地的占位文件
        // 设置该属性位，与 macOS 端 `SF_DATALESS` 语义对等。
        const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
        metadata.file_attributes() & FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS != 0
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::FileMetadataSnapshot;
    use std::fs::{self, File, Metadata, OpenOptions};
    use std::io;
    use std::path::Path;

    pub fn path_for_comparison(path: &Path) -> std::path::PathBuf {
        path.to_path_buf()
    }

    pub fn sync_directory(path: &Path) -> io::Result<()> {
        File::open(path).and_then(|directory| directory.sync_all())
    }

    pub fn configure_workspace_read(options: &mut OpenOptions) {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
    }

    pub fn configure_stable_workspace_read(options: &mut OpenOptions) {
        configure_workspace_read(options);
    }

    pub fn configure_no_follow(options: &mut OpenOptions) {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }

    pub fn is_reparse_point(_metadata: &Metadata) -> bool {
        false
    }

    pub fn path_metadata_snapshot(
        path: &Path,
        follow_symlinks: bool,
    ) -> io::Result<FileMetadataSnapshot> {
        let metadata = if follow_symlinks {
            fs::metadata(path)?
        } else {
            fs::symlink_metadata(path)?
        };
        Ok(snapshot(metadata))
    }

    pub fn opened_file_metadata_snapshot(file: &File) -> io::Result<FileMetadataSnapshot> {
        Ok(snapshot(file.metadata()?))
    }

    fn snapshot(metadata: Metadata) -> FileMetadataSnapshot {
        use std::os::unix::fs::MetadataExt as _;
        let stable_identity = format!("dev:{}:ino:{}", metadata.dev(), metadata.ino());
        FileMetadataSnapshot {
            metadata,
            stable_identity,
        }
    }

    /// 非 macOS 的 Unix 平台不属于本产品的正式目标（见架构基线第 6 节），且缺乏统一的
    /// 云端占位文件标记机制，因此固定返回 `false`（等同于“非占位文件”）。
    pub fn is_cloud_placeholder_file(_metadata: &Metadata) -> bool {
        false
    }
}
