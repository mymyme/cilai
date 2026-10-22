use contexthub_local_transport::Endpoint;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::Path;

pub const INSTANCE_SCHEMA_VERSION: u32 = 1;
const INSTANCE_RECORD_MAX_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParentProcessIdentity {
    pub pid: u32,
    pub start_fingerprint: String,
}

/// # Errors
/// 父进程不存在、指纹不可核对或核对期间归属改变时拒绝返回身份。
pub fn current_parent_process() -> io::Result<ParentProcessIdentity> {
    process_parent_identity(std::process::id())
}

/// # Errors
/// 目标及其父进程必须仍存活，核对期间的归属及两个启动指纹必须稳定。
pub fn process_parent_identity(child: u32) -> io::Result<ParentProcessIdentity> {
    let started =
        process_start_fingerprint(child)?.ok_or_else(|| io::Error::other("process_unavailable"))?;
    let pid = parent_process_id(child)?;
    if pid == child {
        return Err(io::Error::other("parent_process_identity_invalid"));
    }
    let fingerprint = process_start_fingerprint(pid)?
        .ok_or_else(|| io::Error::other("parent_process_unavailable"))?;
    if parent_process_id(child)? != pid
        || process_start_fingerprint(pid)?.as_deref() != Some(fingerprint.as_str())
        || process_start_fingerprint(child)?.as_deref() != Some(started.as_str())
    {
        return Err(io::Error::other("parent_process_identity_changed"));
    }
    Ok(ParentProcessIdentity {
        pid,
        start_fingerprint: fingerprint,
    })
}

/// # Errors
/// 父链不存在、出现环、层级过深或核对期间任何归属改变时拒绝判断；不操作目标进程。
pub fn process_is_descendant(child: u32, ancestor: u32) -> io::Result<bool> {
    let ancestor_fingerprint = process_start_fingerprint(ancestor)?
        .ok_or_else(|| io::Error::other("process_ancestor_unavailable"))?;
    let mut current = child;
    let mut seen = std::collections::BTreeSet::new();
    let mut chain = Vec::new();
    loop {
        if !seen.insert(current) || chain.len() >= 64 {
            return Err(io::Error::other("process_parent_chain_invalid"));
        }
        if current == ancestor {
            break;
        }
        let started = process_start_fingerprint(current)?
            .ok_or_else(|| io::Error::other("process_unavailable"))?;
        let parent = process_parent_identity(current)?;
        chain.push((current, started, parent.clone()));
        current = parent.pid;
    }
    for (pid, started, parent) in chain {
        if process_start_fingerprint(pid)?.as_deref() != Some(started.as_str())
            || process_parent_identity(pid)? != parent
        {
            return Err(io::Error::other("process_parent_identity_changed"));
        }
    }
    if process_start_fingerprint(ancestor)?.as_deref() != Some(ancestor_fingerprint.as_str()) {
        return Err(io::Error::other("process_ancestor_identity_changed"));
    }
    Ok(true)
}

#[cfg(target_os = "macos")]
fn parent_process_id(child: u32) -> io::Result<u32> {
    use std::process::{Command, Stdio};
    let output = Command::new("/bin/ps")
        .args(["-o", "ppid=", "-p", &child.to_string()])
        .env_clear()
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("parent_process_unavailable"));
    }
    parent_id_from_text(
        std::str::from_utf8(&output.stdout)
            .map_err(|_| io::Error::other("parent_process_identity_invalid"))?,
    )
}

#[cfg(target_os = "linux")]
fn parent_process_id(child: u32) -> io::Result<u32> {
    let stat = fs::read_to_string(format!("/proc/{child}/stat"))?;
    let boundary = stat
        .rfind(')')
        .ok_or_else(|| io::Error::other("parent_process_identity_invalid"))?;
    parent_id_from_text(
        stat[boundary + 1..]
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| io::Error::other("parent_process_identity_invalid"))?,
    )
}

#[cfg(windows)]
fn parent_process_id(child: u32) -> io::Result<u32> {
    let pid = windows_process::parent_process_id(child)?;
    if pid == 0 {
        return Err(io::Error::other("parent_process_identity_invalid"));
    }
    let creation = |id| -> io::Result<u64> {
        windows_process::process_start_fingerprint(id)?
            .and_then(|value| {
                value
                    .strip_prefix("windows-creation-ticks:")
                    .and_then(|ticks| ticks.parse().ok())
            })
            .ok_or_else(|| io::Error::other("parent_process_unavailable"))
    };
    if creation(pid)? > creation(child)? {
        return Err(io::Error::other("parent_process_identity_changed"));
    }
    Ok(pid)
}

#[cfg(unix)]
fn parent_id_from_text(text: &str) -> io::Result<u32> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(io::Error::other("parent_process_identity_invalid"));
    }
    let pid = text
        .parse::<u32>()
        .map_err(|_| io::Error::other("parent_process_identity_invalid"))?;
    if pid == 0 {
        return Err(io::Error::other("parent_process_identity_invalid"));
    }
    Ok(pid)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LifecycleMode {
    Owned,
    Transient,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstanceDescriptor {
    pub schema_version: u32,
    pub pid: u32,
    pub process_start_fingerprint: String,
    pub server_instance_id: String,
    pub lifecycle_mode: LifecycleMode,
    pub environment: String,
    pub data_dir_hash: String,
    pub protocol_version: String,
    pub ipc_endpoint: String,
    pub started_at_epoch_ms: u64,
}

/// 读取并校验当前用户私有的实例描述文件。
///
/// # Errors
///
/// 文件类型、权限、所有者、大小或 JSON schema 不符合契约，或底层文件访问失败时返回错误。
pub fn read_descriptor(record_path: &Path) -> io::Result<Option<InstanceDescriptor>> {
    let metadata = match fs::symlink_metadata(record_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(contextual_error("inspect instance record", &error)),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > INSTANCE_RECORD_MAX_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "instance record must be a regular file within the size limit",
        ));
    }
    verify_owner(record_path, &metadata)?;
    let bytes =
        fs::read(record_path).map_err(|error| contextual_error("read instance record", &error))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(unix)]
fn verify_owner(record_path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let parent = record_path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "instance record has no parent")
    })?;
    if metadata.permissions().mode() & 0o777 != 0o600
        || metadata.uid() != fs::metadata(parent)?.uid()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "instance record must be a regular 0600 file owned by its parent directory's owner",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn verify_owner(record_path: &Path, _metadata: &fs::Metadata) -> io::Result<()> {
    crate::security::verify_current_user_private_file(record_path)
}

/// 校验实例描述是否属于指定环境、数据目录，并对应仍存活的原进程。
///
/// # Errors
///
/// 描述与目录身份不匹配、进程不存在、PID 已复用或系统进程信息不可读取时返回错误。
pub fn validate_descriptor(
    descriptor: &InstanceDescriptor,
    data_dir: &Path,
    environment: &str,
    endpoint: &Endpoint,
) -> io::Result<()> {
    if descriptor.schema_version != INSTANCE_SCHEMA_VERSION
        || descriptor.environment != environment
        || descriptor.data_dir_hash != hash_canonical_path(data_dir)?
        || !endpoint.matches_persisted_identity(&descriptor.ipc_endpoint)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "instance record does not match the requested environment and data directory",
        ));
    }
    match process_start_fingerprint(descriptor.pid)? {
        Some(fingerprint) if fingerprint == descriptor.process_start_fingerprint => Ok(()),
        _ => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "recorded App Server process is no longer active",
        )),
    }
}

/// 计算规范化路径的 SHA-256 十六进制身份摘要。
///
/// # Errors
///
/// 路径无法规范化或底层文件系统拒绝访问时返回错误。
pub fn hash_canonical_path(path: &Path) -> io::Result<String> {
    let canonical = fs::canonicalize(path)
        .map_err(|error| contextual_error("canonicalize data directory", &error))?;
    let digest = Sha256::digest(path_bytes(&canonical));
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(encoded)
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

#[cfg(windows)]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().to_string_lossy().into_owned().into_bytes()
}

#[cfg(target_os = "macos")]
/// 读取 macOS 进程的稳定启动时刻指纹。
///
/// # Errors
///
/// 固定系统命令无法执行或返回非 UTF-8 数据时返回错误。
pub fn process_start_fingerprint(pid: u32) -> io::Result<Option<String>> {
    use std::process::{Command, Stdio};
    let output = Command::new("/bin/ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .env_clear()
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|error| contextual_error("read process start fingerprint", &error))?;
    if !output.status.success() {
        return Ok(None);
    }
    let started = String::from_utf8(output.stdout)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let started = started.trim();
    Ok((!started.is_empty()).then(|| format!("macos-lstart:{started}")))
}

#[cfg(target_os = "linux")]
/// 读取 Linux 进程的启动 tick 指纹。
///
/// # Errors
///
/// `/proc` 内容无法读取或格式不符合内核契约时返回错误。
pub fn process_start_fingerprint(pid: u32) -> io::Result<Option<String>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(contextual_error("read process start fingerprint", &error)),
    };
    let end_of_name = stat.rfind(')').ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "process stat has no command boundary",
        )
    })?;
    let fields: Vec<&str> = stat[end_of_name + 1..].split_whitespace().collect();
    let start_ticks = fields.get(19).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "process stat has no start time")
    })?;
    Ok(Some(format!("linux-start-ticks:{start_ticks}")))
}

#[cfg(windows)]
/// 读取仍在运行的 Windows 进程的创建时间指纹。
///
/// # Errors
///
/// 无权限查询或系统调用失败时返回错误；不存在或已退出的进程返回 `Ok(None)`。
pub fn process_start_fingerprint(pid: u32) -> io::Result<Option<String>> {
    windows_process::process_start_fingerprint(pid)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_process {
    use std::io;
    use std::os::windows::io::{FromRawHandle as _, OwnedHandle};
    use windows_sys::Win32::Foundation::{
        ERROR_INVALID_PARAMETER, FILETIME, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    };

    pub(super) fn parent_process_id(child: u32) -> io::Result<u32> {
        // SAFETY: 只创建进程列表只读快照，不请求目标进程控制权限。
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: 系统成功返回真实独占句柄，由 OwnedHandle 在全部路径关闭。
        let _snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
        // SAFETY: Windows要求结构体先置零并填入准确长度。
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize =
            u32::try_from(std::mem::size_of_val(&entry)).expect("PROCESSENTRY32W fits u32");
        // SAFETY: snapshot有效，entry具有契约要求的长度且可写。
        let mut present = unsafe { Process32FirstW(snapshot, &raw mut entry) } != 0;
        while present {
            if entry.th32ProcessID == child {
                return Ok(entry.th32ParentProcessID);
            }
            // SAFETY: 快照及可写entry在本轮枚举期间仍有效。
            present = unsafe { Process32NextW(snapshot, &raw mut entry) } != 0;
        }
        Err(io::Error::other("parent_process_unavailable"))
    }

    pub(super) fn process_start_fingerprint(pid: u32) -> io::Result<Option<String>> {
        // SAFETY: 只请求查询时间及零等待存活检查所需权限，不修改目标进程。
        let process = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if process.is_null() {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == i32::try_from(ERROR_INVALID_PARAMETER).ok() {
                Ok(None)
            } else {
                Err(error)
            };
        }
        // SAFETY: OpenProcess 成功返回独占拥有的真实句柄，转交 OwnedHandle 在所有路径关闭。
        let _process_handle = unsafe { OwnedHandle::from_raw_handle(process) };
        let mut creation_time = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit_time = creation_time;
        let mut kernel_time = creation_time;
        let mut user_time = creation_time;
        // SAFETY: 句柄在整个调用期间存活；四个输出地址均为有效可写 FILETIME。
        let succeeded = unsafe {
            GetProcessTimes(
                process,
                &raw mut creation_time,
                &raw mut exit_time,
                &raw mut kernel_time,
                &raw mut user_time,
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        // 已退出进程在仍有外部句柄时依然能读取创建时间，不能仅据此判断存活。
        // SAFETY: 句柄拥有 SYNCHRONIZE 权限，零超时立即返回，不会阻塞或关闭句柄。
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_OBJECT_0 => return Ok(None),
            WAIT_TIMEOUT => {}
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            status => {
                return Err(io::Error::other(format!(
                    "unexpected process wait status: {status}"
                )));
            }
        }
        let ticks = (u64::from(creation_time.dwHighDateTime) << 32)
            | u64::from(creation_time.dwLowDateTime);
        Ok(Some(format!("windows-creation-ticks:{ticks}")))
    }
}

fn contextual_error(operation: &str, source: &io::Error) -> io::Error {
    io::Error::new(source.kind(), format!("{operation}: {source}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{create_new_restricted_file, ensure_restricted_directory};

    #[test]
    fn parent_process_identity_matches_the_real_process_that_spawned_the_child() {
        let identity = process_start_fingerprint(std::process::id())
            .unwrap()
            .unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "instance::tests::parent_process_identity_child",
                "--ignored",
                "--nocapture",
            ])
            .env("CILAI_PARENT_PROBE_PID", std::process::id().to_string())
            .env("CILAI_PARENT_PROBE_FINGERPRINT", identity)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "由真实父进程专项启动"]
    fn parent_process_identity_child() {
        let expected = std::env::var("CILAI_PARENT_PROBE_PID")
            .unwrap()
            .parse::<u32>()
            .unwrap();
        let identity = current_parent_process().unwrap();
        assert_eq!(identity.pid, expected);
        assert_eq!(
            identity.start_fingerprint,
            std::env::var("CILAI_PARENT_PROBE_FINGERPRINT").unwrap()
        );
        assert!(process_is_descendant(std::process::id(), expected).unwrap());
        assert!(process_is_descendant(expected, std::process::id()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn exited_process_with_an_open_handle_has_no_live_fingerprint() {
        use std::os::windows::process::CommandExt as _;
        use std::process::{Command, Stdio};
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        assert!(
            process_start_fingerprint(std::process::id())
                .unwrap()
                .is_some()
        );
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(process_start_fingerprint(child.id()).unwrap(), None);
        drop(child);
    }

    #[test]
    fn restricted_instance_record_passes_owner_and_access_validation() {
        let root = std::env::temp_dir().join(format!(
            "ch-instance-security-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        ensure_restricted_directory(&root).unwrap();
        let record_path = root.join("instance.json");
        let descriptor = InstanceDescriptor {
            schema_version: INSTANCE_SCHEMA_VERSION,
            pid: std::process::id(),
            process_start_fingerprint: "test-start".to_owned(),
            server_instance_id: "test-instance".to_owned(),
            lifecycle_mode: LifecycleMode::Owned,
            environment: "test".to_owned(),
            data_dir_hash: "test-data".to_owned(),
            protocol_version: "test-protocol".to_owned(),
            ipc_endpoint: "test-endpoint".to_owned(),
            started_at_epoch_ms: 0,
        };
        let mut file = create_new_restricted_file(&record_path).unwrap();
        serde_json::to_writer(&mut file, &descriptor).unwrap();
        drop(file);

        assert_eq!(read_descriptor(&record_path).unwrap(), Some(descriptor));

        fs::remove_file(record_path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
