//! macOS/Unix 本地传输实现：Unix Domain Socket。
//!
//! 行为与迁移前分散在 `crates/app-server/src/local_ipc.rs`、`crates/app-server/src/instance.rs`、
//! `crates/workbench-runtime/src/supervisor.rs` 与 `apps/cli/src/main.rs` 的实现逐字一致，仅做
//! 位置迁移与类型包装，不改变权限位、校验顺序或错误语义。

use crate::{Endpoint, EndpointIdentity};
use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::PathBuf;
use tokio::net::UnixListener as TokioUnixListener;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub struct PlatformListener {
    listener: TokioUnixListener,
    owner_uid: u32,
    _cleanup: SocketCleanup,
}

impl PlatformListener {
    pub fn bind(endpoint: &Endpoint) -> io::Result<Self> {
        let socket_path = endpoint.unix_path();
        match fs::symlink_metadata(socket_path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    "IPC socket path already exists; stale cleanup requires instance validation",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(ipc_error("inspect socket path", &error)),
        }
        let listener = TokioUnixListener::bind(socket_path)
            .map_err(|error| ipc_error("bind socket", &error))?;
        let cleanup = SocketCleanup::new(socket_path.to_path_buf());
        fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))
            .map_err(|error| ipc_error("set socket permissions", &error))?;
        let owner_uid = fs::metadata(endpoint.runtime_dir())
            .map_err(|error| ipc_error("read runtime directory owner", &error))?
            .uid();
        verify_socket_metadata(socket_path, owner_uid)?;
        Ok(Self {
            listener,
            owner_uid,
            _cleanup: cleanup,
        })
    }

    pub async fn accept(&self) -> io::Result<PlatformConnection> {
        loop {
            let (stream, _address) = self.listener.accept().await?;
            let credentials = stream.peer_cred()?;
            if credentials.uid() != self.owner_uid {
                eprintln!(
                    "Local IPC connection rejected: peer user does not own the runtime directory"
                );
                continue;
            }
            return Ok(PlatformConnection { stream });
        }
    }
}

pub struct PlatformConnection {
    stream: tokio::net::UnixStream,
}

impl PlatformConnection {
    pub fn into_split(self) -> (OwnedReadHalf, OwnedWriteHalf) {
        self.stream.into_split()
    }
}

pub type PlatformReadHalf = OwnedReadHalf;
pub type PlatformWriteHalf = OwnedWriteHalf;

fn ipc_error(operation: &str, source: &io::Error) -> io::Error {
    io::Error::new(source.kind(), format!("{operation}: {source}"))
}

fn verify_socket_metadata(socket_path: &std::path::Path, owner_uid: u32) -> io::Result<()> {
    let metadata =
        fs::metadata(socket_path).map_err(|error| ipc_error("read socket metadata", &error))?;
    if !metadata.file_type().is_socket()
        || metadata.permissions().mode() & 0o777 != 0o600
        || metadata.uid() != owner_uid
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "IPC socket must be owned by the runtime directory owner with mode 0600",
        ));
    }
    Ok(())
}

struct SocketCleanup {
    path: PathBuf,
}

impl SocketCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for SocketCleanup {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.file_type().is_socket()
            && let Err(error) = fs::remove_file(&self.path)
        {
            eprintln!("Local IPC socket cleanup failed: {error}");
        }
    }
}

// ---- 同步客户端：供 supervisor（attach 模式）与 CLI 使用 ----

pub struct PlatformStream {
    stream: StdUnixStream,
}

impl PlatformStream {
    pub fn connect(endpoint: &Endpoint) -> io::Result<Self> {
        StdUnixStream::connect(endpoint.unix_path()).map(|stream| Self { stream })
    }

    pub fn connect_identity(identity: &EndpointIdentity) -> io::Result<Self> {
        StdUnixStream::connect(identity.unix_path()).map(|stream| Self { stream })
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        self.stream.try_clone().map(|stream| Self { stream })
    }

    pub fn disconnect(&self) -> io::Result<()> {
        self.stream.shutdown(std::net::Shutdown::Both)
    }
}

impl io::Read for PlatformStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buffer)
    }
}

impl io::Write for PlatformStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.stream.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// 探测端点是否仍有实例接受连接，语义与既有 `UnixStream::connect(..).is_ok()` 判定一致。
pub fn probe(endpoint: &Endpoint) -> io::Result<bool> {
    probe_path(endpoint.unix_path())
}

pub fn probe_identity(identity: &EndpointIdentity) -> io::Result<bool> {
    probe_path(identity.unix_path())
}

fn probe_path(socket_path: &std::path::Path) -> io::Result<bool> {
    match StdUnixStream::connect(socket_path) {
        Ok(_stream) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// 端点是否在文件系统上留有残留：套接字路径存在即为真，不关心是否仍有监听者。
pub fn artifact_exists(endpoint: &Endpoint) -> io::Result<bool> {
    artifact_exists_path(endpoint.unix_path())
}

pub fn artifact_exists_identity(identity: &EndpointIdentity) -> io::Result<bool> {
    artifact_exists_path(identity.unix_path())
}

fn artifact_exists_path(socket_path: &std::path::Path) -> io::Result<bool> {
    match fs::symlink_metadata(socket_path) {
        Ok(_metadata) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ipc_error("inspect socket path", &error)),
    }
}

/// 清理已确认陈旧（无存活监听者）的端点残留：仅当路径确实是套接字文件时移除，
/// 不存在时视为已清理，不是失败。
pub fn cleanup_stale(endpoint: &Endpoint) -> io::Result<()> {
    cleanup_stale_path(endpoint.unix_path())
}

pub fn cleanup_stale_identity(identity: &EndpointIdentity) -> io::Result<()> {
    cleanup_stale_path(identity.unix_path())
}

fn cleanup_stale_path(socket_path: &std::path::Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(socket_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(ipc_error("inspect stale socket path", &error)),
    };
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "stale IPC endpoint is not a Unix socket",
        ));
    }
    fs::remove_file(socket_path).map_err(|error| ipc_error("remove stale socket path", &error))
}
