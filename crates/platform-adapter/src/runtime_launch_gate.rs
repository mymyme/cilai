//! 个人官方运行时启动门：登记可核对的进程身份前不运行供应商代码。
use crate::executable_search::{
    BundledExecutable, bundled_executable_path, validate_executable_file,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const COMMAND: &str = "personal-runtime-launch-gate";
pub const PROTECTED_HOME_FLAG: &str = "--protected-home-v1";
pub const PERMIT: &[u8] = b"CILAI_PERSONAL_RUNTIME_PERMIT_V1\n";

fn denied(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason)
}

/// 只接受同目录个人包中通过锁定摘要核对的官方入口。
/// # Errors
/// 路径、包版本、平台或摘要不匹配时拒绝，绝不搜索其他运行时。
pub fn validate_target(target: &Path) -> io::Result<PathBuf> {
    let expected = bundled_executable_path(BundledExecutable::CodexHarness)?;
    for path in [&expected, target] {
        if !path.is_absolute() {
            return Err(denied("model_launch_target_invalid"));
        }
        for ancestor in path.ancestors() {
            if fs::symlink_metadata(ancestor)?.file_type().is_symlink() {
                return Err(denied("model_launch_target_symlink"));
            }
        }
    }
    let expected = expected.canonicalize()?;
    if target.canonicalize()? != expected {
        return Err(denied("model_launch_target_not_bundled"));
    }
    validate_executable_file(&expected)?;
    let root = expected
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| denied("model_launch_package_invalid"))?;
    let description: Value = serde_json::from_slice(&fs::read(root.join("codex-package.json"))?)
        .map_err(|_| denied("model_launch_package_invalid"))?;
    let release: Value = serde_json::from_str(include_str!(
        "../../../third_party/openai-codex.release.json"
    ))
    .map_err(|_| denied("model_launch_lock_invalid"))?;
    let target_triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return Err(denied("model_launch_platform_not_registered")),
    };
    let entry = if cfg!(windows) {
        "bin/codex-app-server.exe"
    } else {
        "bin/codex-app-server"
    };
    if description["layoutVersion"] != 1
        || description["version"] != release["version"]
        || description["target"] != target_triple
        || description["variant"] != "codex-app-server"
        || description["entrypoint"] != entry
    {
        return Err(denied("model_launch_package_invalid"));
    }
    let actual = format!("{:x}", Sha256::digest(fs::read(&expected)?));
    if release["packages"][target_triple]["executableSha256"].as_str() != Some(&actual) {
        return Err(denied("model_launch_digest_mismatch"));
    }
    Ok(expected)
}

fn read_permit(input: &mut impl Read) -> io::Result<bool> {
    for expected in PERMIT {
        let mut byte = [0];
        match input.read_exact(&mut byte) {
            Ok(()) if byte[0] == *expected => (),
            Ok(()) => return Err(denied("model_launch_permit_invalid")),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

/// 从专用 stdin 消费唯一许可行，之后将原 stdin/stdout 直接交给官方进程。
/// # Errors
/// 非官方目标、无效许可、监督失败或运行时执行失败时返回错误。
pub fn run(arguments: &[OsString]) -> io::Result<i32> {
    let protected = arguments
        .first()
        .is_some_and(|value| value == PROTECTED_HOME_FLAG);
    if protected != crate::macos_runtime_isolation::protection_requested() {
        return Err(denied("model_runtime_home_isolation_required"));
    }
    let arguments = if protected {
        &arguments[1..]
    } else {
        arguments
    };
    let Some((separator, rest)) = arguments.split_first() else {
        return Err(denied("model_launch_arguments_invalid"));
    };
    if separator != "--" || rest.is_empty() {
        return Err(denied("model_launch_arguments_invalid"));
    }
    let target = validate_target(Path::new(&rest[0]))?;
    let mut input = unbuffered_stdin()?;
    if !read_permit(&mut input)? {
        return Ok(0);
    }
    let command = crate::macos_runtime_isolation::official_command(&target, &rest[1..])?;
    execute(command)
}

#[cfg(unix)]
#[allow(unsafe_code)]
pub(crate) fn unbuffered_stdin() -> io::Result<fs::File> {
    use std::os::fd::BorrowedFd;
    // SAFETY: stdin is borrowed only to duplicate it; the original descriptor is not closed.
    unsafe { BorrowedFd::borrow_raw(libc::STDIN_FILENO) }
        .try_clone_to_owned()
        .map(fs::File::from)
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) fn unbuffered_stdin() -> io::Result<fs::File> {
    use std::os::windows::io::BorrowedHandle;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE};
    // SAFETY: retrieve and duplicate the inherited standard input handle without taking ownership.
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    unsafe { BorrowedHandle::borrow_raw(handle) }
        .try_clone_to_owned()
        .map(fs::File::from)
}

#[cfg(unix)]
pub(crate) fn execute(mut command: Command) -> io::Result<i32> {
    use std::os::unix::process::CommandExt;
    Err(command.exec())
}

#[cfg(windows)]
pub(crate) fn execute(mut command: Command) -> io::Result<i32> {
    crate::process::prepare_background_command_for_supervision(&mut command);
    let group = crate::process::ProcessGroup::create()?;
    let mut child = command.spawn()?;
    if let Err(error) = group.adopt(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let status = child.wait()?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permit_consumes_exactly_its_line_and_preserves_official_stdin() {
        let mut bytes = PERMIT.to_vec();
        bytes.extend_from_slice(b"{\"id\":1}\n");
        let mut input = std::io::Cursor::new(bytes);
        assert!(read_permit(&mut input).unwrap());
        let mut remainder = String::new();
        input.read_to_string(&mut remainder).unwrap();
        assert_eq!(remainder, "{\"id\":1}\n");
        for end in 0..PERMIT.len() {
            assert!(!read_permit(&mut std::io::Cursor::new(&PERMIT[..end])).unwrap());
        }
        assert_eq!(
            read_permit(&mut std::io::Cursor::new(b"invalid\n"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
