//! 平台无关的本地传输端点标识。
//!
//! Linux 端沿用运行时目录下固定的 `c.sock` 路径；macOS 与 Windows 端分别使用短 Unix
//! Socket 路径和 Named Pipe，并由运行时目录路径哈希派生稳定名称。运行时目录已经按用户、
//! dev/prod 环境和租户档案分隔，因此仅以运行时目录作为输入即可保证端点互不冲突。
//!
//! 调用方在解析端点前必须确保运行时目录已存在：Windows 端点名称的稳定性依赖于对该目录的
//! 规范化路径求哈希，若目录尚不存在则无法规范化。

#[cfg(any(windows, target_os = "macos"))]
use sha2::{Digest, Sha256};
#[cfg(any(windows, target_os = "macos"))]
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const MACOS_SUN_PATH_MAX_BYTES: usize = 103;
#[cfg(target_os = "macos")]
const MACOS_SOCKET_PREFIX: &str = "/tmp/cilai-workbench-";
#[cfg(any(windows, test))]
const WINDOWS_PIPE_PREFIX: &str = r"\\.\pipe\cilai-workbench-";

/// 可跨进程传递的本地传输端点标识。只接受当前平台由 [`Endpoint::identity`]
/// 产生的稳定形态，不向调用方暴露 Unix Socket 或 Named Pipe 类型。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointIdentity {
    #[cfg(unix)]
    address: PathBuf,
    #[cfg(windows)]
    address: String,
}

impl EndpointIdentity {
    /// 解析一个已持久化或由命令行传入的端点标识。
    ///
    /// # Errors
    ///
    /// 标识不符合当前平台的端点命名契约时返回 [`io::ErrorKind::InvalidInput`]。
    pub fn parse(identity: &str) -> io::Result<Self> {
        let address = parse_platform_identity(identity)?;
        Ok(Self { address })
    }

    /// 返回与实例记录中 `ipcEndpoint` 字段相同的稳定字符串。
    #[must_use]
    pub fn as_str(&self) -> std::borrow::Cow<'_, str> {
        #[cfg(unix)]
        {
            self.address.to_string_lossy()
        }
        #[cfg(windows)]
        {
            std::borrow::Cow::Borrowed(self.address.as_str())
        }
    }

    #[cfg(unix)]
    pub(crate) fn unix_path(&self) -> &Path {
        &self.address
    }

    #[cfg(windows)]
    pub(crate) fn pipe_path(&self) -> &str {
        &self.address
    }
}

/// 本地传输端点：由运行时目录唯一确定，不向调用方暴露具体的套接字路径或管道名形态。
#[derive(Clone, Debug)]
pub struct Endpoint {
    runtime_dir: PathBuf,
    #[cfg(unix)]
    address: PathBuf,
    #[cfg(windows)]
    address: String,
}

impl Endpoint {
    /// 从运行时目录解析本地传输端点。
    ///
    /// # Errors
    ///
    /// Windows 端运行时目录尚不存在或无法规范化时返回错误；macOS 端要求绝对运行时目录，
    /// 并验证派生出的短套接字路径不超过平台限制。
    pub fn resolve(runtime_dir: &Path) -> io::Result<Self> {
        let address = platform_address(runtime_dir)?;
        Ok(Self {
            runtime_dir: runtime_dir.to_path_buf(),
            address,
        })
    }

    /// 该端点所属的运行时目录，用于实例记录与锁文件等仍落文件系统的用途。
    #[must_use]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// 端点的稳定字符串标识，供实例记录持久化与一致性比对使用。
    #[must_use]
    pub fn identity(&self) -> std::borrow::Cow<'_, str> {
        #[cfg(unix)]
        {
            self.address.to_string_lossy()
        }
        #[cfg(windows)]
        {
            std::borrow::Cow::Borrowed(self.address.as_str())
        }
    }

    /// 判断已持久化的端点标识是否属于当前运行时目录。
    ///
    /// macOS 除当前哈希短路径外，还接受升级前位于同一运行时目录的
    /// `c.sock`；其他平台只接受当前标识。
    #[must_use]
    pub fn matches_persisted_identity(&self, identity: &str) -> bool {
        if identity == self.identity().as_ref() {
            return true;
        }
        #[cfg(target_os = "macos")]
        {
            let legacy = self.runtime_dir.join("c.sock");
            identity == legacy.to_string_lossy().as_ref() && validate_unix_path(&legacy).is_ok()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    #[cfg(unix)]
    pub(crate) fn unix_path(&self) -> &Path {
        &self.address
    }

    #[cfg(windows)]
    pub(crate) fn pipe_path(&self) -> &str {
        &self.address
    }
}

#[cfg(target_os = "macos")]
fn platform_address(runtime_dir: &Path) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    if !runtime_dir.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "macOS IPC runtime directory must be absolute",
        ));
    }
    let digest = digest_hex(runtime_dir.as_os_str().as_bytes());
    let socket_path = PathBuf::from(format!("{MACOS_SOCKET_PREFIX}{digest}.sock"));
    validate_unix_path(&socket_path)?;
    Ok(socket_path)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_address(runtime_dir: &Path) -> io::Result<PathBuf> {
    Ok(runtime_dir.join("c.sock"))
}

#[cfg(target_os = "macos")]
fn parse_platform_identity(identity: &str) -> io::Result<PathBuf> {
    if identity.is_empty() || identity.as_bytes().contains(&0) {
        return Err(invalid_identity());
    }
    let path = PathBuf::from(identity);
    if !path.is_absolute() {
        return Err(invalid_identity());
    }
    let legacy = path.file_name().and_then(|name| name.to_str()) == Some("c.sock");
    let hashed = identity
        .strip_prefix(MACOS_SOCKET_PREFIX)
        .and_then(|value| value.strip_suffix(".sock"))
        .is_some_and(valid_lower_hex_digest);
    if !legacy && !hashed {
        return Err(invalid_identity());
    }
    validate_unix_path(&path)?;
    Ok(path)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn parse_platform_identity(identity: &str) -> io::Result<PathBuf> {
    if identity.is_empty() || identity.as_bytes().contains(&0) {
        return Err(invalid_identity());
    }
    let path = PathBuf::from(identity);
    if !path.is_absolute() || path.file_name().and_then(|name| name.to_str()) != Some("c.sock") {
        return Err(invalid_identity());
    }
    Ok(path)
}

#[cfg(target_os = "macos")]
fn validate_unix_path(path: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    if path.as_os_str().as_bytes().len() > MACOS_SUN_PATH_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "IPC socket path exceeds the macOS Unix socket path limit",
        ));
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn validate_unix_path(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn platform_address(runtime_dir: &Path) -> io::Result<String> {
    let canonical = std::fs::canonicalize(runtime_dir).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("canonicalize runtime directory: {error}"),
        )
    })?;
    let hex = digest_hex(canonical.as_os_str().to_string_lossy().as_bytes());
    Ok(format!(r"\\.\pipe\cilai-workbench-{hex}"))
}

#[cfg(windows)]
fn parse_platform_identity(identity: &str) -> io::Result<String> {
    validate_windows_identity(identity)?;
    Ok(identity.to_owned())
}

#[cfg(any(windows, test))]
fn validate_windows_identity(identity: &str) -> io::Result<()> {
    let Some(digest) = identity.strip_prefix(WINDOWS_PIPE_PREFIX) else {
        return Err(invalid_identity());
    };
    if !valid_lower_hex_digest(digest) {
        return Err(invalid_identity());
    }
    Ok(())
}

#[cfg(any(windows, target_os = "macos"))]
fn digest_hex(input: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(32);
    for byte in &digest[..16] {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    hex
}

#[cfg(any(windows, target_os = "macos", test))]
fn valid_lower_hex_digest(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid_identity() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "local IPC endpoint identity is invalid for this platform",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn unix_identity_parser_accepts_the_stable_socket_shape() {
        let identity = EndpointIdentity::parse("/tmp/cilai/run/c.sock").unwrap();
        assert_eq!(identity.as_str(), "/tmp/cilai/run/c.sock");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_endpoint_is_stable_and_short_for_realistic_profile_paths() {
        let runtime = PathBuf::from(
            "/Users/example/Library/Application Support/com.cilai.workbench/dev/profiles/0123456789abcdef0123456789abcdef/run",
        );
        let endpoint = Endpoint::resolve(&runtime).unwrap();
        assert!(endpoint.identity().starts_with(MACOS_SOCKET_PREFIX));
        assert!(endpoint.identity().ends_with(".sock"));
        assert!(endpoint.identity().len() <= MACOS_SUN_PATH_MAX_BYTES);
        assert_eq!(
            endpoint.identity(),
            Endpoint::resolve(&runtime).unwrap().identity()
        );
        assert_ne!(
            endpoint.identity(),
            Endpoint::resolve(&runtime.with_file_name("other"))
                .unwrap()
                .identity(),
        );
        assert!(EndpointIdentity::parse(&endpoint.identity()).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_endpoint_accepts_only_its_own_valid_legacy_identity() {
        let runtime = PathBuf::from("/tmp/cilai-profile/run");
        let endpoint = Endpoint::resolve(&runtime).unwrap();
        assert!(endpoint.matches_persisted_identity(endpoint.identity().as_ref()));
        assert!(endpoint.matches_persisted_identity("/tmp/cilai-profile/run/c.sock"));
        assert!(!endpoint.matches_persisted_identity("/tmp/other-profile/run/c.sock"));

        let long_runtime = PathBuf::from(format!("/tmp/{}/run", "a".repeat(110)));
        let long_endpoint = Endpoint::resolve(&long_runtime).unwrap();
        assert!(
            !long_endpoint
                .matches_persisted_identity(long_runtime.join("c.sock").to_string_lossy().as_ref())
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_identity_parser_rejects_empty_nul_and_other_socket_names() {
        for invalid in [
            "",
            "relative/run/c.sock",
            "/tmp/cilai/run/other.sock",
            "/tmp/cilai/run/c.sock\0",
        ] {
            assert_eq!(
                EndpointIdentity::parse(invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    #[test]
    fn windows_identity_shape_requires_the_product_prefix_and_lower_hex_digest() {
        assert!(
            validate_windows_identity(r"\\.\pipe\cilai-workbench-0123456789abcdef0123456789abcdef")
                .is_ok()
        );
        for invalid in [
            r"\\.\pipe\other-0123456789abcdef0123456789abcdef",
            r"\\.\pipe\cilai-workbench-0123456789abcdef",
            r"\\.\pipe\cilai-workbench-0123456789ABCDEf0123456789abcdef",
            r"\\.\pipe\cilai-workbench-0123456789abcdef0123456789abcdeg",
        ] {
            assert_eq!(
                validate_windows_identity(invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }
}
