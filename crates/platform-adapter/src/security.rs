//! 本地数据安全边界：受限目录与受限文件。macOS 用权限位 0600/0700 保护数据库文件、运行时
//! 目录、凭证文件和便携投影目录；Windows 用显式 DACL 达到对等强度——仅授予当前用户与
//! `SYSTEM` 完全控制，移除继承的访问控制项，不授予 `Users` 与 `Authenticated Users`，不接受
//! 以账户默认权限替代。四个入口覆盖既有调用方的四种既有用法，只替换平台专属实现，不改变
//! 调用方的创建语义（是否允许已存在、是否截断）。设置失败时返回明确错误，不继续写入数据；
//! 设置后均回读校验，校验失败同样视为失败。
//!
//! Windows 端新目录在创建时显式指定当前用户为所有者；新文件通过创建句柄设置所有者。
//! 原生创建、权限收紧与回读校验均封装在下方的 Windows 私有子模块，以最小范围 `unsafe`
//! 完成，对外只暴露 `io::Result`，不暴露原始句柄或平台结构。既有对象不自动变更所有者。

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

/// 确保目录存在，并将其访问权限收紧为仅当前用户（Windows 端另加 `SYSTEM`）可访问；
/// 设置后回读校验，失败时返回错误，不允许继续在该目录中写入。
///
/// # Errors
///
/// 目录创建失败、权限设置失败或回读校验不一致时返回错误。
pub fn ensure_restricted_directory(directory: &Path) -> io::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(directory)
        && metadata.file_type().is_symlink()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "restricted directory path must not be a symlink",
        ));
    }
    #[cfg(windows)]
    {
        platform::create_private_directory(directory)?;
        // 对已满足精确私有权限且属于当前用户的目录只做校验。
        // 重复设置父目录 DACL 会传播到已归档的子项，改变其继承权限。
        if platform::verify_private_path(directory).is_ok() {
            return Ok(());
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fs::create_dir_all(directory)
        .map_err(|error| contextual_error("create restricted directory", &error))?;
    restrict_existing_path(directory)
        .map_err(|error| contextual_error("restrict directory permissions", &error))?;
    verify_restricted_path(directory, true)
}

/// Ensure an application-owned runtime tree remains writable when Windows creates
/// child directories beneath a private directory whose DACL does not inherit.
/// Existing ACEs are preserved, including sandbox-specific deny rules. Only
/// directory ACLs are updated directly; inheritance keeps sandbox-created files
/// accessible to the trusted application user even when their creator differs.
/// Symbolic links are left untouched.
///
/// # Errors
///
/// Returns an error if the root is not owned by the current user, a child is
/// neither user-owned nor an already accessible administrator-owned directory,
/// an ACL cannot be updated, or children cannot be enumerated.
pub fn ensure_owner_writable_runtime_tree(directory: &Path) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    #[cfg(windows)]
    {
        // Setup creates this group on demand. Resolve its actual local SID once
        // per preparation, so the first check after setup sees the new identity.
        let sandbox_group_sid = platform::resolve_sandbox_runtime_group_sid()?;
        repair_runtime_directory(directory, true, sandbox_group_sid.as_deref())?;
    }
    Ok(())
}

#[cfg(windows)]
fn repair_runtime_directory(
    directory: &Path,
    root: bool,
    sandbox_group_sid: Option<&str>,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime directory must not be a symbolic link",
        ));
    }
    platform::grant_inheritable_owner_access(directory, root, sandbox_group_sid).map_err(
        |error| {
            contextual_error(
                &format!("prepare runtime directory {}", directory.display()),
                &error,
            )
        },
    )?;
    for child in fs::read_dir(directory)? {
        let child = child?;
        let file_type = child.file_type()?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        if root && child.file_name().to_string_lossy().starts_with(".sandbox") {
            continue;
        }
        repair_runtime_directory(&child.path(), false, sandbox_group_sid)?;
    }
    Ok(())
}

/// 以受限权限创建一个新文件：路径已存在时失败（`create_new` 语义），不截断既有内容。
/// 设置后回读校验。
///
/// # Errors
///
/// 文件已存在、创建失败、权限设置失败或回读校验不一致时返回错误。
pub fn create_new_restricted_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    platform::configure_new_private_file(&mut options);
    let file = options
        .open(path)
        .map_err(|error| contextual_error("create restricted file", &error))?;
    // Only a file freshly created by this call may have its owner normalized.
    // Never apply this to existing private files or relax their reader checks.
    #[cfg(windows)]
    platform::set_new_file_current_user_owner(&file)?;
    finish_restricting_file(path, file)
}

/// 以受限权限打开一个文件用于读写：文件不存在时创建，存在时不截断（供锁文件等需要跨进程
/// 重新打开同一文件的场景使用）。设置后回读校验；由于会复用已存在的文件（不像
/// [`create_new_restricted_file`] 那样以“不存在即失败”天然拒绝抢先写入的文件），额外校验
/// 该文件的属主与其所在目录的属主一致，防止其他账户在本进程创建文件前抢先放入一个权限位
/// 正确但属主不同的同名文件；Windows 端校验既有文件属于当前进程用户。
///
/// # Errors
///
/// 打开失败、权限设置失败或回读校验不一致时返回错误。
pub fn open_restricted_file(path: &Path) -> io::Result<File> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "restricted file path must be a regular file",
        ));
    }
    // Atomically distinguish new locks from existing ones. New Windows files
    // need an explicit TokenUser owner even when the creating token is elevated.
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    platform::configure_new_private_file(&mut options);
    match options.open(path) {
        Ok(file) => {
            #[cfg(windows)]
            platform::set_new_file_current_user_owner(&file)?;
            return finish_restricting_file(path, file);
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(contextual_error("create restricted file", &error)),
    }
    platform::verify_owned_by_parent(path)
        .map_err(|error| contextual_error("verify restricted file ownership", &error))?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .truncate(false)
        .open(path)
        .map_err(|error| contextual_error("open restricted file", &error))?;
    let file = finish_restricting_file(path, file)?;
    platform::verify_owned_by_parent(path)
        .map_err(|error| contextual_error("verify restricted file ownership", &error))?;
    Ok(file)
}

/// 持有受限文件的排他锁；句柄关闭或进程退出时由操作系统释放。
#[must_use]
pub struct RestrictedFileLock {
    _file: File,
}

/// 非阻塞取得当前用户私有锁文件的排他权。已被占用返回 `None`，不抢占运行中的任务。
/// # Errors
/// 文件、所有者、权限不符合约束或系统锁操作失败时返回错误。
pub fn try_lock_restricted_file(path: &Path) -> io::Result<Option<RestrictedFileLock>> {
    let file = open_restricted_file(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(RestrictedFileLock { _file: file })),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

fn finish_restricting_file(path: &Path, file: File) -> io::Result<File> {
    restrict_existing_path(path)
        .map_err(|error| contextual_error("restrict file permissions", &error))?;
    verify_restricted_path(path, false)?;
    Ok(file)
}

/// 将已存在的路径（文件或目录）权限收紧为仅当前用户（Windows 端另加 `SYSTEM`）可访问；
/// 用于对已经由第三方库（如 SQLite）创建的文件做事后收紧。不含回读校验，调用方需要时
/// 自行调用 [`verify_restricted_path`]。
///
/// # Errors
///
/// 路径不存在或权限设置失败时返回错误。
pub fn restrict_existing_path(path: &Path) -> io::Result<()> {
    platform::restrict(path)
}

/// 回读校验路径的权限是否符合受限契约。
///
/// # Errors
///
/// 权限不符合契约或读取失败时返回错误。
pub fn verify_restricted_path(path: &Path, is_directory: bool) -> io::Result<()> {
    platform::verify(path, is_directory)
}

/// 校验一个既有文件（通常由本进程之外的流程写入，例如企业登录写入的凭据文件）没有对
/// 同组或其他账户开放访问：Unix 端要求组、其他位均为零，不要求与本进程创建的文件一样
/// 精确等于 0600（属主可以有超出读写的位，例如历史遗留的可执行位）；Windows 端要求 DACL
/// 中不包含对 `Everyone`、`Authenticated Users`、`Users` 等内置共享主体的允许项。
///
/// # Errors
///
/// 路径不存在、读取失败或存在共享访问权限时返回错误。
pub fn verify_no_shared_access(path: &Path) -> io::Result<()> {
    platform::verify_no_shared_access(path)
}

/// 校验私有文件的所属者与访问控制边界。Unix 端要求文件权限为 `0600`且属主
/// 与父目录一致；Windows 端要求文件属于当前进程用户，并且受保护 DACL 仅授予
/// `SYSTEM` 与所属者完全控制。
///
/// # Errors
///
/// 所属者、权限或 DACL 不符合私有文件契约，或底层安全信息无法读取时返回错误。
pub fn verify_current_user_private_file(path: &Path) -> io::Result<()> {
    platform::verify_current_user_private_file(path)
}

#[cfg(any(windows, test))]
fn is_restrictive_windows_dacl_sddl(value: &str) -> bool {
    value.starts_with("D:P")
        && value.matches('(').count() == 2
        && value.contains("(A;;FA;;;SY)")
        && value.contains("(A;;FA;;;OW)")
}

fn contextual_error(operation: &str, source: &io::Error) -> io::Error {
    io::Error::new(source.kind(), format!("{operation}: {source}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn unique_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "ch-platform-security-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn directory_is_created_and_restricted() {
        let root = unique_root("directory");
        ensure_restricted_directory(&root).unwrap();
        assert!(root.is_dir());
        verify_restricted_path(&root, true).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn rechecking_private_directory_preserves_archived_child_access() {
        let root = unique_root("archive-parent");
        let source = unique_root("archive-source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("history.txt"), b"preserved history").unwrap();
        ensure_restricted_directory(&root).unwrap();
        let archived = root.join("source");
        fs::rename(&source, &archived).unwrap();
        assert_eq!(
            fs::read(archived.join("history.txt")).unwrap(),
            b"preserved history"
        );
        ensure_restricted_directory(&root).unwrap();
        verify_restricted_path(&root, true).unwrap();
        assert_eq!(
            fs::read(archived.join("history.txt")).unwrap(),
            b"preserved history"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn private_task_lock_is_exclusive_and_released_when_its_owner_closes() {
        let root = unique_root("exclusive-task");
        ensure_restricted_directory(&root).unwrap();
        let path = root.join("memory.lock");
        let guard = try_lock_restricted_file(&path).unwrap().unwrap();
        assert!(try_lock_restricted_file(&path).unwrap().is_none());
        drop(guard);
        let reacquired = try_lock_restricted_file(&path).unwrap().unwrap();
        drop(reacquired);
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn runtime_tree_repairs_non_inheriting_private_directories() {
        let root = unique_root("runtime-inheritance");
        ensure_restricted_directory(&root).unwrap();
        let locks = root.join("thread-writer-locks");
        fs::create_dir(&locks).unwrap();

        ensure_owner_writable_runtime_tree(&root).unwrap();
        fs::write(locks.join(".coordination.lock"), b"ready").unwrap();
        let next = root.join("tool-snapshots");
        fs::create_dir(&next).unwrap();
        fs::write(next.join("snapshot.txt"), b"ready").unwrap();

        fs::remove_dir_all(&root).unwrap();
    }

    // Windows 端创建符号链接通常需要管理员权限或已开启开发者模式，不适合作为可移植的
    // 自动化测试前提；该校验分支在两端共用同一段跨平台代码（`file_type().is_symlink()`），
    // 此处仅在 Unix 侧覆盖，Windows 侧的等效路径随实机验证补充契约测试。
    #[cfg(unix)]
    #[test]
    fn ensure_restricted_directory_rejects_symlink() {
        let root = unique_root("symlink-dir");
        let target = root.join("target");
        let link = root.join("link");
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(
            ensure_restricted_directory(&link).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::remove_file(&link).unwrap();
        fs::remove_dir(&target).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn new_file_is_restricted_and_rejects_existing_path() {
        let root = unique_root("new-file");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("secret.txt");
        let mut file = create_new_restricted_file(&path).unwrap();
        file.write_all(b"secret").unwrap();
        drop(file);
        verify_restricted_path(&path, false).unwrap();
        assert!(create_new_restricted_file(&path).is_err());
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn open_restricted_file_does_not_truncate_existing_content() {
        let root = unique_root("open-file");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("lock");
        {
            let mut file = open_restricted_file(&path).unwrap();
            file.write_all(b"hold").unwrap();
        }
        {
            let file = open_restricted_file(&path).unwrap();
            assert_eq!(file.metadata().unwrap().len(), 4);
        }
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn verify_no_shared_access_rejects_group_or_other_bits() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = unique_root("shared-access");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("credential");
        fs::write(&path, b"secret").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        verify_no_shared_access(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            verify_no_shared_access(&path).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn restrict_existing_path_tightens_an_already_created_file() {
        let root = unique_root("existing-file");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("db");
        fs::write(&path, b"data").unwrap();
        restrict_existing_path(&path).unwrap();
        verify_restricted_path(&path, false).unwrap();
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn windows_private_dacl_contract_rejects_additional_principals() {
        assert!(is_restrictive_windows_dacl_sddl(
            "D:P(A;;FA;;;SY)(A;;FA;;;OW)"
        ));
        assert!(!is_restrictive_windows_dacl_sddl(
            "D:P(A;;FA;;;SY)(A;;FA;;;OW)(A;;FR;;;BU)"
        ));
        assert!(!is_restrictive_windows_dacl_sddl(
            "D:(A;;FA;;;SY)(A;;FA;;;OW)"
        ));
    }

    #[cfg(windows)]
    #[test]
    fn restricted_file_is_owned_by_current_user_and_has_exact_private_dacl() {
        let root = unique_root("current-user-file");
        ensure_restricted_directory(&root).unwrap();
        let path = root.join("instance.json");
        drop(create_new_restricted_file(&path).unwrap());
        verify_current_user_private_file(&path).unwrap();
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&root).unwrap();
    }
}

#[cfg(unix)]
mod platform {
    use super::contextual_error;
    use std::fs;
    use std::io;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;

    const DIRECTORY_MODE: u32 = 0o700;
    const FILE_MODE: u32 = 0o600;

    pub(super) fn restrict(path: &Path) -> io::Result<()> {
        let mode = if path.is_dir() {
            DIRECTORY_MODE
        } else {
            FILE_MODE
        };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    }

    pub(super) fn verify(path: &Path, is_directory: bool) -> io::Result<()> {
        let expected = if is_directory {
            DIRECTORY_MODE
        } else {
            FILE_MODE
        };
        let metadata =
            fs::metadata(path).map_err(|error| contextual_error("read restricted path", &error))?;
        if metadata.permissions().mode() & 0o777 != expected {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("restricted path does not have expected mode {expected:o}"),
            ));
        }
        Ok(())
    }

    pub(super) fn verify_no_shared_access(path: &Path) -> io::Result<()> {
        let metadata = fs::metadata(path)
            .map_err(|error| contextual_error("read file permissions", &error))?;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file grants access to its group or other accounts",
            ));
        }
        Ok(())
    }

    pub(super) fn verify_current_user_private_file(path: &Path) -> io::Result<()> {
        verify(path, false)?;
        verify_owned_by_parent(path)
    }

    pub(super) fn verify_owned_by_parent(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::MetadataExt as _;
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "restricted file has no parent")
        })?;
        let file_uid = fs::metadata(path)?.uid();
        let parent_uid = fs::metadata(parent)?.uid();
        if file_uid != parent_uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "restricted file owner does not match its directory owner",
            ));
        }
        Ok(())
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use super::contextual_error;
    use std::io;
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;
    use std::ptr;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_NONE_MAPPED, HANDLE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
        ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, EqualSid, GetSecurityDescriptorDacl, GetTokenInformation,
        IsWellKnownSid, LookupAccountNameW, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SidTypeAlias, TOKEN_QUERY,
        TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::core::PWSTR;

    // 与 `crates/local-transport` 的 Named Pipe 安全描述符使用同一条 SDDL：仅授予
    // SYSTEM 与当前用户（此处以 Owner 表达，`SetNamedSecurityInfoW` 调用时不携带
    // OWNER_SECURITY_INFORMATION，因此文件/目录的实际属主仍是创建者本身，OW 在应用阶段
    // 解析为该属主）完全控制，`P` 标志阻止继承访问控制项。
    const RESTRICTIVE_DACL_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FA;;;OW)";

    const INHERITABLE_SYSTEM_ACCESS: &str = "(A;OICI;FA;;;SY)";
    const INHERITABLE_OWNER_ACCESS: &str = "(A;OICI;FA;;;OW)";

    pub(super) fn grant_inheritable_owner_access(
        path: &Path,
        root: bool,
        sandbox_group_sid: Option<&str>,
    ) -> io::Result<()> {
        // OWNER_RIGHTS follows each child's creator. A sandbox account may
        // create the child, so the trusted host user also needs an explicit ACE.
        let host_sid = current_user_sddl_sid()?;
        grant_runtime_host_access(path, root, &host_sid, sandbox_group_sid)
    }

    fn grant_runtime_host_access(
        path: &Path,
        root: bool,
        host_sid: &str,
        sandbox_group_sid: Option<&str>,
    ) -> io::Result<()> {
        let host_access = inheritable_host_access(host_sid)?;
        let mut dacl_sddl = fetch_dacl_sddl(path)?;
        if !runtime_acl_requires_update(
            path_owner(path)?,
            root,
            &dacl_sddl,
            host_sid,
            sandbox_group_sid,
        )? {
            // An elevated helper may leave an Administrators-owned tmp/arg0
            // child. Keep its sufficient host access without changing its ACL.
            return Ok(());
        }
        if dacl_sddl.contains(INHERITABLE_SYSTEM_ACCESS)
            && dacl_sddl.contains(INHERITABLE_OWNER_ACCESS)
            && dacl_sddl.contains(&host_access)
        {
            return Ok(());
        }
        if !dacl_sddl.contains(INHERITABLE_SYSTEM_ACCESS) {
            dacl_sddl.push_str(INHERITABLE_SYSTEM_ACCESS);
        }
        if !dacl_sddl.contains(INHERITABLE_OWNER_ACCESS) {
            dacl_sddl.push_str(INHERITABLE_OWNER_ACCESS);
        }
        if !dacl_sddl.contains(&host_access) {
            dacl_sddl.push_str(&host_access);
        }
        let descriptor = SecurityDescriptor::from_sddl(&dacl_sddl)?;
        let dacl = descriptor.dacl()?;
        let mut wide_path = wide_path(path);
        // SAFETY: `wide_path` and `descriptor` remain valid for the call. Only
        // the DACL changes; the current owner and all existing ACEs are kept.
        let result = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null_mut(),
            )
        };
        if result != 0 {
            return Err(io::Error::from_raw_os_error(
                i32::try_from(result).unwrap_or(-1),
            ));
        }
        let observed = fetch_dacl_sddl(path)?;
        if !observed.contains(INHERITABLE_SYSTEM_ACCESS)
            || !observed.contains(INHERITABLE_OWNER_ACCESS)
            || !observed.contains(&host_access)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "runtime directory DACL did not retain inheritable owner access",
            ));
        }
        Ok(())
    }

    fn inheritable_host_access(host_sid: &str) -> io::Result<String> {
        // Windows serializes some account SIDs as aliases (for example the
        // local built-in administrator as LA). Canonicalize our expected ACE
        // through the same native formatter as the observed DACL, so a grant
        // does not fail verification or get appended again on every retry.
        let descriptor = SecurityDescriptor::from_sddl(&format!("D:(A;OICI;FA;;;{host_sid})"))?;
        let canonical = describe_as_sddl(descriptor.raw)?;
        canonical
            .strip_prefix("D:")
            .filter(|ace| {
                ace.starts_with("(A;OICI;FA;;;")
                    && ace.ends_with(')')
                    && ace.bytes().filter(|byte| *byte == b'(').count() == 1
            })
            .map(str::to_owned)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid canonical host ACE"))
    }

    fn runtime_acl_requires_update(
        owner: PathOwner,
        root: bool,
        dacl: &str,
        host_sid: &str,
        sandbox_group_sid: Option<&str>,
    ) -> io::Result<bool> {
        match owner {
            PathOwner::CurrentUser => Ok(true),
            PathOwner::Administrators
                if !root
                    && admin_child_has_access_with_sandbox_group(
                        dacl,
                        host_sid,
                        sandbox_group_sid,
                    ) =>
            {
                Ok(false)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "runtime directory is not owned by the current user or an accessible administrator child",
            )),
        }
    }

    #[cfg(test)]
    fn admin_child_has_inherited_host_access(dacl: &str, host_sid: &str) -> bool {
        admin_child_has_access_with_sandbox_group(dacl, host_sid, None)
    }

    fn admin_child_has_access_with_sandbox_group(
        dacl: &str,
        host_sid: &str,
        sandbox_group_sid: Option<&str>,
    ) -> bool {
        let Ok(host_access) = inheritable_host_access(host_sid) else {
            return false;
        };
        let host = &host_access[1..host_access.len() - 1];
        // Codex's read-ACL helper grants this exact RX mask with OI/CI. It can
        // complete after readiness reports ready, and its grant then inherits
        // into administrator-owned children. Accept only the resolved local
        // sandbox group, with no new grants or changes to owners/deny rules.
        let sandbox_read_access = sandbox_group_sid.map(|sid| format!("A;OICI;0x1200a9;;;{sid}"));
        let normalized = dacl.replace(";OICIID;", ";OICI;");
        let mut entries = normalized.split('(');
        if !entries
            .next()
            .is_some_and(|header| header.starts_with("D:"))
        {
            return false;
        }
        let mut has_host = false;
        let mut has_system = false;
        for entry in entries {
            match entry.strip_suffix(')') {
                Some(entry) if entry == host => has_host = true,
                Some("A;OICI;FA;;;SY") => has_system = true,
                // These existing entries appear in directories created by the
                // elevated Harness. Accept them only in this administrator-
                // owned child gate, after requiring full inheritable host and
                // SYSTEM access. Do not rewrite owners or sandbox ACLs.
                Some("A;OICI;FA;;;OW" | "A;;FA;;;SY" | "A;;FA;;;BA") => {}
                Some(entry) if is_runtime_read_execute_logon_access(entry) => {}
                Some(entry) if sandbox_read_access.as_deref() == Some(entry) => {}
                _ => return false,
            }
        }
        has_host && has_system
    }

    fn is_runtime_read_execute_logon_access(entry: &str) -> bool {
        let Some(logon_id) = entry.strip_prefix("A;;0x1200a9;;;S-1-5-5-") else {
            return false;
        };
        let mut components = logon_id.split('-');
        components
            .next()
            .is_some_and(|value| value.parse::<u32>().is_ok())
            && components
                .next()
                .is_some_and(|value| value.parse::<u32>().is_ok())
            && components.next().is_none()
    }

    fn current_user_sddl_sid() -> io::Result<String> {
        let user = current_user_sid()?;
        let mut text: PWSTR = ptr::null_mut();
        // SAFETY: `user` owns the aligned TOKEN_USER buffer and its SID remains
        // valid throughout this call. Win32 allocates the output with LocalAlloc.
        if unsafe { ConvertSidToStringSidW(user.sid()?, &raw mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful conversion returned a valid NUL-terminated string.
        let sid = unsafe { pwstr_to_string(text) };
        // SAFETY: this function is the sole owner of the LocalAlloc output.
        unsafe { LocalFree(text.cast()) };
        Ok(sid)
    }

    pub(super) fn resolve_sandbox_runtime_group_sid() -> io::Result<Option<String>> {
        // Keep the fixed local group name in sync with the bundled Codex setup
        // helper. Never trust a RID (e.g. 1000), a report SID, or a domain group
        // with the same unqualified name. No group or account is created here.
        resolve_local_alias_sid("CodexSandboxUsers")
    }

    fn resolve_local_alias_sid(name: &str) -> io::Result<Option<String>> {
        let mut computer = [0_u16; 16];
        let mut computer_chars = 16;
        // SAFETY: the fixed buffer holds MAX_COMPUTERNAME_LENGTH + 1 UTF-16
        // characters; the size parameter describes that buffer and is writable.
        if unsafe {
            windows_sys::Win32::System::WindowsProgramming::GetComputerNameW(
                computer.as_mut_ptr(),
                &raw mut computer_chars,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let computer = String::from_utf16_lossy(&computer[..computer_chars as usize]);
        resolve_alias_sid_for_account(&format!("{computer}\\{name}"))
    }

    fn resolve_alias_sid_for_account(account: &str) -> io::Result<Option<String>> {
        let account: Vec<u16> = account.encode_utf16().chain([0]).collect();
        let mut sid_bytes = 0;
        let mut domain_chars = 0;
        let mut kind = 0;
        // SAFETY: account is NUL-terminated and all outputs are writable. Null
        // buffers with zero sizes request the required allocation sizes only.
        unsafe {
            LookupAccountNameW(
                ptr::null(),
                account.as_ptr(),
                ptr::null_mut(),
                &raw mut sid_bytes,
                ptr::null_mut(),
                &raw mut domain_chars,
                &raw mut kind,
            );
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == i32::try_from(ERROR_NONE_MAPPED).ok() {
            return Ok(None); // Setup has not provisioned this group yet.
        }
        if error.raw_os_error() != i32::try_from(ERROR_INSUFFICIENT_BUFFER).ok() {
            return Err(error);
        }
        if sid_bytes == 0 || sid_bytes > 68 || domain_chars == 0 || domain_chars > 32768 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid local group SID allocation",
            ));
        }
        let mut sid = vec![0_u32; (sid_bytes as usize).div_ceil(4)];
        let mut domain = vec![0_u16; domain_chars as usize];
        // SAFETY: aligned SID and UTF-16 domain buffers meet the sizes returned
        // by Windows; all buffers and outputs remain live throughout the call.
        if unsafe {
            LookupAccountNameW(
                ptr::null(),
                account.as_ptr(),
                sid.as_mut_ptr().cast(),
                &raw mut sid_bytes,
                domain.as_mut_ptr(),
                &raw mut domain_chars,
                &raw mut kind,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if kind != SidTypeAlias {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "sandbox runtime identity is not a local group",
            ));
        }
        let mut text: PWSTR = ptr::null_mut();
        // SAFETY: a successful lookup returned a valid SID in the aligned live
        // buffer. Win32 allocates the string; it is freed once below.
        if unsafe { ConvertSidToStringSidW(sid.as_mut_ptr().cast(), &raw mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: conversion succeeded and returned a NUL-terminated string.
        let value = unsafe { pwstr_to_string(text) };
        // SAFETY: this function exclusively owns the LocalAlloc output.
        unsafe { LocalFree(text.cast()) };
        Ok(Some(value))
    }

    pub(super) fn set_new_file_current_user_owner(file: &std::fs::File) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle as _;
        let user = current_user_sid()?;
        // SAFETY: file is a newly-created, still-open handle owned by the caller;
        // user owns the SID buffer. No existing file or path lookup is involved.
        let result = unsafe {
            windows_sys::Win32::Security::Authorization::SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                user.sid()?,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if result != 0 {
            return Err(io::Error::from_raw_os_error(
                i32::try_from(result).unwrap_or(-1),
            ));
        }
        Ok(())
    }

    pub(super) fn configure_new_private_file(options: &mut std::fs::OpenOptions) {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
        use windows_sys::Win32::Storage::FileSystem::WRITE_OWNER;
        options.access_mode(GENERIC_READ | GENERIC_WRITE | WRITE_OWNER);
    }

    pub(super) fn create_private_directory(path: &Path) -> io::Result<()> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "private directory must not be a link",
                    ));
                }
                return verify_current_user_owner(path);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("private directory has no parent"))?;
        if !parent.exists() {
            create_private_directory(parent)?;
        }
        let sid = current_user_sddl_sid()?;
        let descriptor = SecurityDescriptor::from_sddl(&format!("O:{sid}{RESTRICTIVE_DACL_SDDL}"))?;
        let attributes = windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<
                windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
            >())
            .unwrap(),
            lpSecurityDescriptor: descriptor.raw,
            bInheritHandle: 0,
        };
        let encoded = wide_path(path);
        // SAFETY: descriptor, attributes and terminated path live through this
        // atomic creation; an existing object is never assigned a new owner here.
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::CreateDirectoryW(
                encoded.as_ptr(),
                &raw const attributes,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        verify_current_user_owner(path)
    }

    pub(super) fn restrict(path: &Path) -> io::Result<()> {
        let descriptor = SecurityDescriptor::from_sddl(RESTRICTIVE_DACL_SDDL)?;
        let dacl = descriptor.dacl()?;
        let mut wide_path = wide_path(path);
        // SAFETY: `wide_path` 是本次调用期间存活的合法 NUL 结尾宽字符路径；`dacl`
        // 指向 `descriptor` 拥有的内存，`descriptor` 在本次调用返回前不会被释放；
        // 不设置 `psidOwner`/`psidGroup`（传空指针且不在标志位中包含对应信息位），
        // 因此本次调用只修改 DACL，不改变对象属主。
        let result = unsafe {
            SetNamedSecurityInfoW(
                wide_path.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null_mut(),
            )
        };
        if result != 0 {
            return Err(io::Error::from_raw_os_error(
                i32::try_from(result).unwrap_or(-1),
            ));
        }
        Ok(())
    }

    /// Windows 端由目录自身的 DACL 阻止其他账户在本进程创建文件前抢先写入同名文件，
    /// 不存在与 Unix 端等价的“属主与父目录属主不一致”场景需要额外校验，是空操作。
    pub(super) fn verify_owned_by_parent(path: &Path) -> io::Result<()> {
        verify_current_user_owner(path)
    }

    pub(super) fn verify(path: &Path, _is_directory: bool) -> io::Result<()> {
        let observed = fetch_dacl_sddl(path)?;
        if !observed.contains(":P") || !observed.contains(";SY)") || !observed.contains(";OW)") {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("restricted path does not carry the expected protected DACL: {observed}"),
            ));
        }
        Ok(())
    }

    /// 内置共享主体的 SDDL 简写：`WD` = Everyone，`AU` = Authenticated Users，
    /// `BU` = 内置 Users 组。任一简写作为允许（`A;`）项的主体出现即视为存在共享访问。
    const SHARED_PRINCIPAL_SDDL_ABBREVIATIONS: [&str; 3] = [";WD)", ";AU)", ";BU)"];

    pub(super) fn verify_no_shared_access(path: &Path) -> io::Result<()> {
        let observed = fetch_dacl_sddl(path)?;
        if SHARED_PRINCIPAL_SDDL_ABBREVIATIONS
            .iter()
            .any(|principal| observed.contains(principal))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("file grants access to a shared account: {observed}"),
            ));
        }
        Ok(())
    }

    pub(super) fn verify_current_user_private_file(path: &Path) -> io::Result<()> {
        verify_private_path(path)
    }

    pub(super) fn verify_private_path(path: &Path) -> io::Result<()> {
        let observed = fetch_dacl_sddl(path)?;
        if !super::is_restrictive_windows_dacl_sddl(&observed) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("private file does not carry the exact restricted DACL: {observed}"),
            ));
        }
        verify_current_user_owner(path)
    }

    fn verify_current_user_owner(path: &Path) -> io::Result<()> {
        if path_owner(path)? != PathOwner::CurrentUser {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private file is not owned by the current user",
            ));
        }
        Ok(())
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum PathOwner {
        CurrentUser,
        Administrators,
        Other,
    }

    fn path_owner(path: &Path) -> io::Result<PathOwner> {
        let mut wide_path = wide_path(path);
        let mut owner: PSID = ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `wide_path` 是本次调用期间存活的 NUL 结尾宽字符路径；只请求
        // 所属者信息，`owner` 借用自返回的安全描述符，描述符由下方 RAII 对象释放。
        let query_result = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_mut_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &raw mut owner,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut descriptor,
            )
        };
        if query_result != 0 {
            return Err(io::Error::from_raw_os_error(
                i32::try_from(query_result).unwrap_or(-1),
            ));
        }
        let _descriptor = SecurityDescriptor { raw: descriptor };
        if owner.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private file security descriptor has no owner",
            ));
        }
        let current_user = current_user_sid()?;
        // SAFETY: `owner` 指向 `_descriptor` 持有且仍然存活的安全描述符；
        // `current_user.sid()` 指向按 `TOKEN_USER` 对齐且在本调用期间存活的缓冲区。
        if unsafe { EqualSid(owner, current_user.sid()?) } != 0 {
            return Ok(PathOwner::CurrentUser);
        }
        // SAFETY: `owner` remains borrowed from the live `_descriptor`.
        if unsafe { IsWellKnownSid(owner, WinBuiltinAdministratorsSid) } != 0 {
            return Ok(PathOwner::Administrators);
        }
        Ok(PathOwner::Other)
    }

    fn current_user_sid() -> io::Result<TokenUserBuffer> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: `GetCurrentProcess` 返回的伪句柄在当前进程内始终有效；`token`
        // 是合法的可写输出参数，成功返回的令牌句柄立即交给 RAII 守卫。
        let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) };
        if opened == 0 {
            return Err(io::Error::last_os_error());
        }
        let _token = HandleGuard(token);
        let mut needed = 0_u32;
        // SAFETY: 首次调用仅探测缓冲区大小，Win32 契约允许空指针与零长度。
        unsafe {
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &raw mut needed);
        }
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let word_count = (needed as usize).div_ceil(std::mem::size_of::<usize>());
        let mut buffer = vec![0_usize; word_count];
        let buffer_bytes = u32::try_from(word_count * std::mem::size_of::<usize>())
            .expect("token user buffer size fits in u32");
        // SAFETY: `buffer` 按 `usize` 对齐且容量不小于探测结果，满足 `TOKEN_USER`
        // 输出结构及其 SID 数据的对齐和长度要求。
        let filled = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                buffer_bytes,
                &raw mut needed,
            )
        };
        if filled == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(TokenUserBuffer { buffer })
    }

    struct TokenUserBuffer {
        buffer: Vec<usize>,
    }

    impl TokenUserBuffer {
        fn sid(&self) -> io::Result<PSID> {
            if self.buffer.len() * std::mem::size_of::<usize>() < std::mem::size_of::<TOKEN_USER>()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "token user buffer is too small",
                ));
            }
            // SAFETY: `buffer` 由 `GetTokenInformation(TokenUser)` 成功填充且按 `usize`
            // 对齐，因此可在其生命周期内读取 `TOKEN_USER` 头及内部 SID 指针。
            let sid = unsafe { (&*self.buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
            if sid.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "current user token has no SID",
                ));
            }
            Ok(sid)
        }
    }

    struct HandleGuard(HANDLE);

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            // SAFETY: 本类型只由成功的 `OpenProcessToken` 构造，是该句柄的唯一
            // 所有者，析构时关闭一次后不再使用。
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    fn fetch_dacl_sddl(path: &Path) -> io::Result<String> {
        let mut wide_path = wide_path(path);
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `wide_path` 是本次调用期间存活的合法 NUL 结尾宽字符路径；只请求
        // `DACL_SECURITY_INFORMATION`，输出的安全描述符经下方 `LocalFree` 释放。
        let query_result = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut descriptor,
            )
        };
        if query_result != 0 {
            return Err(io::Error::from_raw_os_error(
                i32::try_from(query_result).unwrap_or(-1),
            ));
        }
        let observed = describe_as_sddl(descriptor);
        // SAFETY: `descriptor` 由上面的 `GetNamedSecurityInfoW` 通过 LocalAlloc 分配，
        // 本函数是其唯一所有者，此处是其生命周期内唯一一次释放。
        unsafe {
            LocalFree(descriptor.cast());
        }
        observed.map_err(|error| contextual_error("describe observed DACL", &error))
    }

    fn describe_as_sddl(descriptor: PSECURITY_DESCRIPTOR) -> io::Result<String> {
        let mut sddl: PWSTR = ptr::null_mut();
        // SAFETY: `descriptor` is a validated, live security descriptor owned
        // by the caller, obtained from GetNamedSecurityInfoW or constructed by
        // SecurityDescriptor::from_sddl. The output is freed exactly once.
        let succeeded = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &raw mut sddl,
                ptr::null_mut(),
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `sddl` 刚由上面的调用成功写入，指向一段 NUL 结尾的合法宽字符串。
        let text = unsafe { pwstr_to_string(sddl) };
        // SAFETY: `sddl` 由 LocalAlloc 分配，本函数是其唯一所有者。
        unsafe {
            LocalFree(sddl.cast());
        }
        Ok(text)
    }

    /// # Safety
    ///
    /// `pointer` 必须指向一段有效、NUL 结尾的宽字符缓冲区。
    unsafe fn pwstr_to_string(pointer: PWSTR) -> String {
        let mut length = 0usize;
        // SAFETY: 由调用方保证 `pointer` 有效且 NUL 结尾，逐字扫描直至遇到 NUL。
        unsafe {
            while *pointer.add(length) != 0 {
                length += 1;
            }
            let slice = std::slice::from_raw_parts(pointer, length);
            String::from_utf16_lossy(slice)
        }
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    struct SecurityDescriptor {
        raw: PSECURITY_DESCRIPTOR,
    }

    impl SecurityDescriptor {
        fn from_sddl(sddl: &str) -> io::Result<Self> {
            let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
            let mut raw: PSECURITY_DESCRIPTOR = ptr::null_mut();
            // SAFETY: `wide` 是以 NUL 结尾、在本次调用期间存活的合法 UTF-16 缓冲区；
            // `raw` 是本地栈上的合法可写输出参数，成功时由 Win32 通过 LocalAlloc 分配，
            // 所有权转移给本类型，由 `Drop` 负责 `LocalFree` 释放。
            let succeeded = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    SDDL_REVISION_1,
                    &raw mut raw,
                    ptr::null_mut(),
                )
            };
            if succeeded == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { raw })
        }

        fn dacl(&self) -> io::Result<*mut ACL> {
            let mut present = 0;
            let mut dacl: *mut ACL = ptr::null_mut();
            let mut defaulted = 0;
            // SAFETY: `self.raw` 是本类型持有、仍然有效的安全描述符；三个输出参数均为
            // 本地栈上的合法可写地址。返回的 `dacl` 指针借用自 `self.raw` 拥有的内存，
            // 只在 `self` 存活期间使用，不单独释放。
            let succeeded = unsafe {
                GetSecurityDescriptorDacl(
                    self.raw,
                    &raw mut present,
                    &raw mut dacl,
                    &raw mut defaulted,
                )
            };
            if succeeded == 0 || present == 0 || dacl.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "constructed security descriptor has no DACL",
                ));
            }
            Ok(dacl)
        }
    }

    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            if !self.raw.is_null() {
                // SAFETY: `self.raw` 由 `ConvertStringSecurityDescriptorToSecurityDescriptorW`
                // 通过 LocalAlloc 分配；本类型是其唯一所有者，此处是其生命周期内唯一一次释放。
                unsafe {
                    LocalFree(self.raw.cast());
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn builtin_administrator_sid() -> String {
            let descriptor = SecurityDescriptor::from_sddl("O:LAD:(A;;FA;;;SY)").unwrap();
            let mut owner = ptr::null_mut();
            let mut defaulted = 0;
            // SAFETY: descriptor owns the validated security descriptor for the
            // entire call and the returned owner SID is borrowed from it.
            assert_ne!(
                unsafe {
                    windows_sys::Win32::Security::GetSecurityDescriptorOwner(
                        descriptor.raw,
                        &raw mut owner,
                        &raw mut defaulted,
                    )
                },
                0
            );
            let mut text: PWSTR = ptr::null_mut();
            // SAFETY: owner remains valid while descriptor is alive; Windows
            // allocates the output, which this test frees exactly once.
            assert_ne!(unsafe { ConvertSidToStringSidW(owner, &raw mut text) }, 0);
            // SAFETY: successful conversion produced a NUL-terminated string.
            let sid = unsafe { pwstr_to_string(text) };
            // SAFETY: this test owns the output allocation exclusively.
            unsafe { LocalFree(text.cast()) };
            sid
        }

        #[test]
        fn runtime_builtin_administrator_ace_survives_native_readback_and_retry() {
            let root = std::env::temp_dir().join(format!(
                "cilai-runtime-alias-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            super::super::ensure_restricted_directory(&root).unwrap();
            let sid = builtin_administrator_sid();
            grant_runtime_host_access(&root, true, &sid, None).unwrap();
            let before = fetch_dacl_sddl(&root).unwrap();
            assert!(before.contains("(A;OICI;FA;;;LA)"), "{before}");
            grant_runtime_host_access(&root, true, &sid, None).unwrap();
            assert_eq!(fetch_dacl_sddl(&root).unwrap(), before);
            std::fs::remove_dir(root).unwrap();
        }

        #[test]
        fn runtime_captured_admin_child_accepts_existing_safe_legacy_entries() {
            let sid = builtin_administrator_sid();
            let captured = "D:AI(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x1200a9;;;S-1-5-5-0-290976)(A;OICIID;FA;;;SY)(A;OICIID;FA;;;OW)(A;OICIID;FA;;;LA)";
            let descriptor = SecurityDescriptor::from_sddl(captured).unwrap();
            let observed = describe_as_sddl(descriptor.raw).unwrap();
            assert!(
                admin_child_has_inherited_host_access(&observed, &sid),
                "{observed}"
            );
            assert!(
                !runtime_acl_requires_update(
                    PathOwner::Administrators,
                    false,
                    &observed,
                    &sid,
                    None
                )
                .unwrap()
            );
            assert!(
                runtime_acl_requires_update(PathOwner::Administrators, true, &observed, &sid, None)
                    .is_err()
            );
            assert!(
                runtime_acl_requires_update(PathOwner::Other, false, &observed, &sid, None)
                    .is_err()
            );
            for rejected in [
                captured.replace("0x1200a9", "0x1201bf"),
                captured.replace("0x1200a9", "FA"),
                captured.replace("A;;FA;;;BA", "A;OICI;FA;;;WD"),
                captured.replace(
                    "A;;0x1200a9;;;S-1-5-5-0-290976",
                    "D;;0x1200a9;;;S-1-5-5-0-290976",
                ),
                captured.replace("(A;OICIID;FA;;;LA)", ""),
                captured.replace("(A;OICIID;FA;;;SY)", ""),
                captured.replace("S-1-5-5-0-290976", "S-1-5-21-1-2-3-1001"),
                captured.replace("S-1-5-5-0-290976", "S-1-5-5-0-290976-1"),
            ] {
                assert!(
                    !admin_child_has_inherited_host_access(&rejected, &sid),
                    "{rejected}"
                );
            }
        }

        #[test]
        fn runtime_captured_sandbox_read_grants_survive_post_execution_checks() {
            let host_sid = builtin_administrator_sid();
            let sandbox_group_sid = "S-1-5-21-1-2-3-1000";
            // These are the two complete DACL shapes in the returned v4 report.
            // All 53 active-profile children were rejected on this same RX ACE.
            for captured in [
                format!(
                    "D:AI(A;OICIID;0x1200a9;;;{sandbox_group_sid})(A;OICIID;FA;;;SY)(A;OICIID;FA;;;OW)(A;OICIID;FA;;;LA)"
                ),
                format!(
                    "D:AI(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x1200a9;;;S-1-5-5-0-290976)(A;OICIID;0x1200a9;;;{sandbox_group_sid})(A;OICIID;FA;;;SY)(A;OICIID;FA;;;OW)(A;OICIID;FA;;;LA)"
                ),
            ] {
                let descriptor = SecurityDescriptor::from_sddl(&captured).unwrap();
                let observed = describe_as_sddl(descriptor.raw).unwrap();
                assert!(
                    admin_child_has_access_with_sandbox_group(
                        &observed,
                        &host_sid,
                        Some(sandbox_group_sid),
                    ),
                    "captured sandbox read grant rejected: {observed}"
                );
                assert!(
                    !runtime_acl_requires_update(
                        PathOwner::Administrators,
                        false,
                        &observed,
                        &host_sid,
                        Some(sandbox_group_sid),
                    )
                    .unwrap()
                );
                // Both the unresolved identity and a different local group's
                // SID must still be rejected, regardless of its numeric RID.
                for candidate in [None, Some("S-1-5-21-4-5-6-1000")] {
                    assert!(
                        runtime_acl_requires_update(
                            PathOwner::Administrators,
                            false,
                            &observed,
                            &host_sid,
                            candidate,
                        )
                        .is_err()
                    );
                }
                for owner in [PathOwner::Other, PathOwner::Administrators] {
                    assert!(
                        runtime_acl_requires_update(
                            owner,
                            true,
                            &observed,
                            &host_sid,
                            Some(sandbox_group_sid),
                        )
                        .is_err()
                    );
                }
            }
        }

        #[test]
        fn sandbox_group_read_compatibility_rejects_write_deny_unknown_and_missing_host_access() {
            let host_sid = builtin_administrator_sid();
            let group_sid = "S-1-5-21-1-2-3-1000";
            let valid = format!(
                "D:AI(A;OICIID;0x1200a9;;;{group_sid})(A;OICIID;FA;;;SY)(A;OICIID;FA;;;OW)(A;OICIID;FA;;;LA)"
            );
            for rejected in [
                valid.replace("0x1200a9", "0x1201bf"),
                valid.replace("0x1200a9", "FA"),
                valid.replace("A;OICIID;0x1200a9", "D;OICIID;0x1200a9"),
                valid.replace("A;OICIID;0x1200a9", "A;OICIIO;0x1200a9"),
                valid.replace("(A;OICIID;FA;;;LA)", ""),
                valid.replace("(A;OICIID;FA;;;SY)", ""),
                format!("{valid}(A;OICI;FA;;;WD)"),
                format!("{valid}(A;OICI;0x1200a9;;;S-1-5-21-1-2-3-1001)"),
            ] {
                let descriptor = SecurityDescriptor::from_sddl(&rejected).unwrap();
                let observed = describe_as_sddl(descriptor.raw).unwrap();
                assert!(
                    runtime_acl_requires_update(
                        PathOwner::Administrators,
                        false,
                        &observed,
                        &host_sid,
                        Some(group_sid),
                    )
                    .is_err(),
                    "unexpected runtime grant accepted: {observed}"
                );
            }
        }

        #[test]
        fn runtime_group_lookup_resolves_local_aliases_and_does_not_require_setup() {
            use windows_sys::Win32::NetworkManagement::NetManagement::{
                LOCALGROUP_INFO_0, MAX_PREFERRED_LENGTH, NetApiBufferFree, NetLocalGroupEnum,
            };
            let mut buffer = ptr::null_mut();
            let mut count = 0;
            let mut total = 0;
            // SAFETY: this read-only local enumeration owns its returned buffer;
            // all output parameters are writable, and no resume handle is needed.
            let result = unsafe {
                NetLocalGroupEnum(
                    ptr::null(),
                    0,
                    &raw mut buffer,
                    MAX_PREFERRED_LENGTH,
                    &raw mut count,
                    &raw mut total,
                    ptr::null_mut(),
                )
            };
            assert_eq!(result, 0);
            assert!(!buffer.is_null() && count > 0);
            assert_eq!(
                buffer.align_offset(std::mem::align_of::<LOCALGROUP_INFO_0>()),
                0
            );
            // SAFETY: successful level-0 enumeration returned count valid
            // LOCALGROUP_INFO_0 entries, each with a live NUL-terminated name.
            #[expect(
                clippy::cast_ptr_alignment,
                reason = "NetApi level-0 buffer alignment is verified before reading its typed entries"
            )]
            let groups = unsafe {
                std::slice::from_raw_parts(buffer.cast::<LOCALGROUP_INFO_0>(), count as usize)
            };
            let names = groups
                .iter()
                .map(|group| {
                    // SAFETY: name pointer is valid while the enumeration buffer lives.
                    unsafe { pwstr_to_string(group.lgrpi0_name) }
                })
                .collect::<Vec<_>>();
            // SAFETY: this test is the sole owner of the NetApi allocation.
            unsafe { NetApiBufferFree(buffer.cast()) };
            for name in names {
                let builtin = resolve_alias_sid_for_account(&format!("BUILTIN\\{name}")).unwrap();
                let local = resolve_local_alias_sid(&name).unwrap();
                // BUILTIN aliases have a different authority from machine SAM
                // aliases. A qualified machine lookup must not resolve those
                // accidentally, just as it must not resolve a domain namesake.
                if let Some(sid) = builtin {
                    assert!(sid.starts_with("S-1-5-32-"));
                    assert_eq!(
                        local, None,
                        "BUILTIN alias accepted as machine group: {name}"
                    );
                } else {
                    assert!(
                        local.is_some_and(|sid| sid.starts_with("S-1-5-21-")),
                        "machine alias not resolved: {name}"
                    );
                }
            }
            assert!(resolve_alias_sid_for_account("NT AUTHORITY\\SYSTEM").is_err());
            let missing = format!("cilai_missing_{}", std::process::id());
            assert_eq!(resolve_local_alias_sid(&missing).unwrap(), None);
            // Optional before setup, but resolution errors must propagate.
            resolve_sandbox_runtime_group_sid().unwrap();
        }

        #[test]
        fn runtime_native_read_grant_inheritance_survives_repeated_preparation() {
            let root = std::env::temp_dir().join(format!(
                "cilai-runtime-post-setup-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ));
            super::super::ensure_restricted_directory(&root).unwrap();
            let host_sid = current_user_sddl_sid().unwrap();
            let group_sid = "S-1-5-21-1-2-3-1000";
            super::super::ensure_owner_writable_runtime_tree(&root).unwrap();
            let existing = root.join("sessions");
            std::fs::create_dir(&existing).unwrap();
            let content = existing.join("fixture.txt");
            std::fs::write(&content, b"preserved across sandbox read grants").unwrap();
            let owner_before = path_owner(&existing).unwrap();
            let dacl_before = fetch_dacl_sddl(&root).unwrap();
            let descriptor = SecurityDescriptor::from_sddl(&format!(
                "{dacl_before}(A;OICI;0x1200a9;;;{group_sid})"
            ))
            .unwrap();
            let mut path = wide_path(&root);
            // SAFETY: this test owns the new root and live descriptor. The
            // unknown fixture SID receives RX only on this isolated tree.
            assert_eq!(
                unsafe {
                    SetNamedSecurityInfoW(
                        path.as_mut_ptr(),
                        SE_FILE_OBJECT,
                        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        descriptor.dacl().unwrap(),
                        ptr::null_mut(),
                    )
                },
                0
            );
            let late = root.join("tool-snapshots");
            std::fs::create_dir(&late).unwrap();
            // Exercise native inheritance both for an existing child and one
            // created after the helper's read grant, then replay the BA gate.
            for child in [&existing, &late] {
                let observed = fetch_dacl_sddl(child).unwrap();
                assert!(
                    observed.contains(&format!("(A;OICIID;0x1200a9;;;{group_sid})")),
                    "{observed}"
                );
                assert!(
                    runtime_acl_requires_update(
                        PathOwner::Administrators,
                        false,
                        &observed,
                        &host_sid,
                        None,
                    )
                    .is_err()
                );
                assert!(
                    !runtime_acl_requires_update(
                        PathOwner::Administrators,
                        false,
                        &observed,
                        &host_sid,
                        Some(group_sid),
                    )
                    .unwrap()
                );
            }
            super::super::ensure_owner_writable_runtime_tree(&root).unwrap();
            let prepared = fetch_dacl_sddl(&existing).unwrap();
            super::super::ensure_owner_writable_runtime_tree(&root).unwrap();
            assert_eq!(fetch_dacl_sddl(&existing).unwrap(), prepared);
            assert!(path_owner(&existing).unwrap() == owner_before);
            assert_eq!(
                std::fs::read(&content).unwrap(),
                b"preserved across sandbox read grants"
            );
            assert!(prepared.contains(&format!("0x1200a9;;;{group_sid})")));
            assert!(!prepared.contains(";;;WD)") && !prepared.contains(";;;BU)"));
            std::fs::remove_dir_all(root).unwrap();
        }

        #[test]
        #[ignore = "requires the returned read-only site diagnostic via CILAI_CAPTURED_SANDBOX_REPORT"]
        fn runtime_replays_every_active_site_directory_after_sandbox_initialization() {
            let path =
                std::env::var_os("CILAI_CAPTURED_SANDBOX_REPORT").expect("diagnostic fixture path");
            let bytes = std::fs::read(path).unwrap();
            // Windows PowerShell 5.1 diagnostic JSON intentionally carries BOM.
            let report: serde_json::Value =
                serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
                    .unwrap();
            let remote_host = report["currentUser"]["userSid"].as_str().unwrap();
            assert!(
                remote_host.ends_with("-500"),
                "this captured fixture requires the built-in administrator mapping"
            );
            // SDDL's LA alias resolves on the replay computer. Map only this
            // fixture's built-in administrator identity, keeping all ACEs/masks.
            let host_sid = builtin_administrator_sid();
            let mut total = 0;
            for snapshot in report["snapshots"].as_array().unwrap() {
                for profile in snapshot["profiles"].as_array().unwrap() {
                    if profile["profile"]
                        .as_str()
                        .unwrap()
                        .ends_with("\\signed-out")
                    {
                        continue;
                    }
                    let issues = profile["runtimeIssues"].as_array().unwrap();
                    let root = &profile["metadata"][2];
                    assert_eq!(root["ownerSid"].as_str(), Some(remote_host));
                    let group_sid = issues
                        .iter()
                        .flat_map(|issue| issue["aclPrincipals"].as_array().unwrap())
                        .find(|entry| {
                            entry["account"]
                                .as_str()
                                .is_some_and(|name| name.ends_with("\\CodexSandboxUsers"))
                        })
                        .and_then(|entry| entry["sid"].as_str())
                        .unwrap();
                    assert_eq!(
                        issues.len() as u64,
                        profile["ownershipGateRejections"].as_u64().unwrap()
                    );
                    assert_eq!(profile["runtimeScanTruncated"], false);
                    for issue in issues {
                        assert_eq!(issue["ownerSid"], "S-1-5-32-544");
                        assert_eq!(issue["isLink"], false);
                        let descriptor =
                            SecurityDescriptor::from_sddl(issue["dacl"].as_str().unwrap()).unwrap();
                        let observed = describe_as_sddl(descriptor.raw).unwrap();
                        assert!(
                            runtime_acl_requires_update(
                                PathOwner::Administrators,
                                false,
                                &observed,
                                &host_sid,
                                None,
                            )
                            .is_err(),
                            "fixture no longer reproduces old rejection"
                        );
                        assert!(
                            !runtime_acl_requires_update(
                                PathOwner::Administrators,
                                false,
                                &observed,
                                &host_sid,
                                Some(group_sid),
                            )
                            .unwrap(),
                            "captured runtime directory requires mutation"
                        );
                        total += 1;
                    }
                }
            }
            assert!(total > 0);
            println!("captured active runtime directory checks passed: {total}");
        }

        #[test]
        fn runtime_legacy_tree_preparation_preserves_owners_rules_and_file_bytes() {
            let root = std::env::temp_dir().join(format!(
                "cilai-runtime-legacy-tree-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            super::super::ensure_restricted_directory(&root).unwrap();
            let child = root.join("skills");
            std::fs::create_dir(&child).unwrap();
            let sid = current_user_sddl_sid().unwrap();
            // Replay the signed-out profile's legacy child, which did not yet
            // inherit any host grant. Parent preparation must restore it.
            let captured = "D:AI(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x1200a9;;;S-1-5-5-0-290976)";
            let content = child.join("fixture.txt");
            std::fs::write(&content, b"unchanged runtime content").unwrap();
            let descriptor = SecurityDescriptor::from_sddl(captured).unwrap();
            let dacl = descriptor.dacl().unwrap();
            let mut path = wide_path(&child);
            // SAFETY: child is this test's new directory; path and the owned
            // descriptor stay live for the call. Only its DACL is changed.
            assert_eq!(
                unsafe {
                    SetNamedSecurityInfoW(
                        path.as_mut_ptr(),
                        SE_FILE_OBJECT,
                        DACL_SECURITY_INFORMATION,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        dacl,
                        ptr::null_mut(),
                    )
                },
                0
            );
            let owner_before = path_owner(&child).unwrap();
            super::super::ensure_owner_writable_runtime_tree(&root).unwrap();
            super::super::ensure_owner_writable_runtime_tree(&root).unwrap();
            assert!(path_owner(&child).unwrap() == owner_before);
            let observed = fetch_dacl_sddl(&child).unwrap();
            assert!(observed.contains("(A;;FA;;;BA)"), "{observed}");
            assert!(
                observed.contains("(A;;0x1200a9;;;S-1-5-5-0-290976)"),
                "{observed}"
            );
            assert!(
                observed.contains(
                    &inheritable_host_access(&sid)
                        .unwrap()
                        .replace(";OICI;", ";OICIID;")
                ) || observed.contains(&inheritable_host_access(&sid).unwrap()),
                "{observed}"
            );
            assert!(!observed.contains(";;;WD)") && !observed.contains(";;;BU)"));
            assert_eq!(
                std::fs::read(&content).unwrap(),
                b"unchanged runtime content"
            );
            std::fs::remove_file(content).unwrap();
            std::fs::remove_dir(child).unwrap();
            std::fs::remove_dir(root).unwrap();
        }

        #[test]
        fn runtime_children_inherit_explicit_host_access_not_only_creator_rights() {
            let root = std::env::temp_dir().join(format!(
                "cilai-runtime-host-access-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            super::super::ensure_restricted_directory(&root).unwrap();
            let sid = current_user_sddl_sid().unwrap();
            grant_inheritable_owner_access(&root, true, None).unwrap();
            let child = root.join("sandbox-output.md");
            std::fs::write(&child, "index").unwrap();
            let acl = fetch_dacl_sddl(&child).unwrap();
            assert!(
                acl.contains(
                    &inheritable_host_access(&sid)
                        .unwrap()
                        .replace(";OICI;", ";ID;")
                ),
                "{acl}"
            );
            assert!(!acl.contains(";;;BU)") && !acl.contains(";;;AU)"), "{acl}");
            assert_eq!(std::fs::read_to_string(&child).unwrap(), "index");
            std::fs::remove_file(child).unwrap();
            std::fs::remove_dir(root).unwrap();
        }

        #[test]
        fn administrator_child_requires_existing_inheritable_host_and_system_access() {
            let host_sid = current_user_sddl_sid().unwrap();
            let observed =
                format!("D:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;OW)(A;OICIID;FA;;;{host_sid})");
            let descriptor = SecurityDescriptor::from_sddl(&observed).unwrap();
            let normalized = describe_as_sddl(descriptor.raw).unwrap();
            assert!(admin_child_has_inherited_host_access(
                &normalized,
                &host_sid
            ));
            assert!(
                !runtime_acl_requires_update(
                    PathOwner::Administrators,
                    false,
                    &normalized,
                    &host_sid,
                    None,
                )
                .unwrap()
            );
            assert!(
                runtime_acl_requires_update(
                    PathOwner::Administrators,
                    true,
                    &normalized,
                    &host_sid,
                    None,
                )
                .is_err()
            );
            assert!(
                runtime_acl_requires_update(PathOwner::Other, false, &normalized, &host_sid, None)
                    .is_err()
            );
            assert!(
                runtime_acl_requires_update(
                    PathOwner::CurrentUser,
                    true,
                    &normalized,
                    &host_sid,
                    None
                )
                .unwrap()
            );
            for rejected in [
                "D:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;OW)".to_owned(),
                format!("D:AI(A;OICIID;FA;;;SY)(A;OICIID;FR;;;{host_sid})"),
                format!("D:AI(A;OICIID;FA;;;SY)(A;OICIID;FA;;;{host_sid})(A;OICIID;FA;;;WD)"),
                format!("D:AI(D;OICI;FA;;;{host_sid})(A;OICIID;FA;;;SY)(A;OICIID;FA;;;{host_sid})"),
                format!("D:AI(A;OICIID;FA;;;SY)(A;IO;FA;;;{host_sid})"),
                format!("D:AI(A;OICIID;FA;;;{host_sid})"),
            ] {
                assert!(
                    !admin_child_has_inherited_host_access(&rejected, &host_sid),
                    "{rejected}"
                );
            }
        }
    }
}
