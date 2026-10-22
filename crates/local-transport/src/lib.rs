#![doc = "桌面壳、Local App Server 与 CLI 三方之间的本地传输抽象：监听端、连接端、端点标识与安全校验。"]
#![doc = "对外只暴露平台无关类型，调用方不可见 `UnixListener`、Named Pipe 句柄等平台类型。"]
#![doc = "macOS 使用 Unix Domain Socket，Windows 使用 Named Pipe；行为契约见"]
#![doc = "`词来工作台Windows端适配技术方案.md` 第 4 节，两端由同一套契约测试覆盖。"]

mod endpoint;

#[cfg(unix)]
#[path = "unix.rs"]
mod platform;

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

pub use endpoint::{Endpoint, EndpointIdentity};

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// 本地传输监听端：绑定一个端点并接受经过归属校验的连接。
pub struct LocalListener(platform::PlatformListener);

impl LocalListener {
    /// 绑定端点并开始准备接受连接。
    ///
    /// # Errors
    ///
    /// 端点已被占用、平台资源创建失败或安全边界设置失败时返回错误。
    pub fn bind(endpoint: &Endpoint) -> io::Result<Self> {
        platform::PlatformListener::bind(endpoint).map(Self)
    }

    /// 接受下一个连接；内部已经完成客户端归属校验，只返回校验通过的连接。
    ///
    /// # Errors
    ///
    /// 底层监听端发生不可恢复的 I/O 错误时返回错误。
    pub async fn accept(&self) -> io::Result<LocalConnection> {
        self.0.accept().await.map(LocalConnection)
    }
}

/// 一条已通过归属校验的服务端连接，可拆分为独立的读写半边。
pub struct LocalConnection(platform::PlatformConnection);

impl LocalConnection {
    #[must_use]
    pub fn into_split(self) -> (LocalReadHalf, LocalWriteHalf) {
        let (reader, writer) = self.0.into_split();
        (LocalReadHalf(reader), LocalWriteHalf(writer))
    }
}

pub struct LocalReadHalf(platform::PlatformReadHalf);
pub struct LocalWriteHalf(platform::PlatformWriteHalf);

impl AsyncRead for LocalReadHalf {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_read(context, buffer)
    }
}

impl AsyncWrite for LocalWriteHalf {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().0).poll_write(context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().0).poll_shutdown(context)
    }
}

/// 同步客户端连接：供桌面壳的接管（attach）模式与 CLI 使用。
pub struct LocalStream(platform::PlatformStream);

impl LocalStream {
    /// 以单次尝试连接端点；重试与超时策略由调用方按业务语义决定。
    ///
    /// # Errors
    ///
    /// 端点不存在时返回 [`io::ErrorKind::NotFound`]；端点存在但暂不接受连接时返回
    /// [`io::ErrorKind::ConnectionRefused`]；其他失败原样返回。
    pub fn connect(endpoint: &Endpoint) -> io::Result<Self> {
        platform::PlatformStream::connect(endpoint).map(Self)
    }

    /// 连接一个由 [`EndpointIdentity::parse`] 校验过的稳定端点标识，供不持有
    /// 运行时目录、只持有实例记录端点值的接管调试入口使用。
    ///
    /// # Errors
    ///
    /// 端点不存在、暂不接受连接或底层连接失败时返回对应 I/O 错误。
    pub fn connect_identity(identity: &EndpointIdentity) -> io::Result<Self> {
        platform::PlatformStream::connect_identity(identity).map(Self)
    }

    /// 克隆出一个独立的流句柄，用于读写分离到不同线程。
    ///
    /// # Errors
    ///
    /// 底层平台句柄复制失败时返回错误。
    pub fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Self)
    }

    /// 主动断开连接，使对端与本地其余句柄观察到连接结束。
    ///
    /// # Errors
    ///
    /// 底层平台操作失败时返回错误。
    pub fn disconnect(&self) -> io::Result<()> {
        self.0.disconnect()
    }
}

impl io::Read for LocalStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl io::Write for LocalStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// 探测端点是否仍有实例接受连接，不建立持久连接。
///
/// # Errors
///
/// 探测过程中发生非"未找到/暂不接受连接"以外的 I/O 错误时返回错误。
pub fn probe_liveness(endpoint: &Endpoint) -> io::Result<bool> {
    platform::probe(endpoint)
}

/// 根据已校验的持久化标识探测端点是否仍有实例接受连接。
///
/// # Errors
///
/// 探测过程中发生非"未找到/暂不接受连接"以外的 I/O 错误时返回错误。
pub fn probe_liveness_identity(identity: &EndpointIdentity) -> io::Result<bool> {
    platform::probe_identity(identity)
}

/// 端点是否留有文件系统残留：Unix 端为套接字路径是否存在（不关心是否仍有监听者）；
/// Windows 端管道不产生此类残留，等价于 [`probe_liveness`]。
///
/// # Errors
///
/// 检查过程中发生 I/O 错误时返回错误。
pub fn artifact_exists(endpoint: &Endpoint) -> io::Result<bool> {
    platform::artifact_exists(endpoint)
}

/// 已校验的持久化标识是否留有平台端点残留。
///
/// # Errors
///
/// 检查过程中发生 I/O 错误时返回错误。
pub fn artifact_exists_identity(identity: &EndpointIdentity) -> io::Result<bool> {
    platform::artifact_exists_identity(identity)
}

/// 清理已确认陈旧（无存活监听者）的端点残留。调用方必须先以 [`probe_liveness`] 确认
/// 端点已无存活监听者，本函数不重复探测。
///
/// # Errors
///
/// Unix 端路径存在但不是套接字文件，或移除失败时返回错误；Windows 端恒为 `Ok(())`。
pub fn cleanup_stale(endpoint: &Endpoint) -> io::Result<()> {
    platform::cleanup_stale(endpoint)
}

/// 清理已校验且已确认陈旧的持久化端点残留。
///
/// # Errors
///
/// Unix 端路径存在但不是套接字文件，或移除失败时返回错误；Windows 端恒为 `Ok(())`。
pub fn cleanup_stale_identity(identity: &EndpointIdentity) -> io::Result<()> {
    platform::cleanup_stale_identity(identity)
}

#[cfg(all(test, unix))]
mod tests {
    use super::{EndpointIdentity, LocalStream};
    use std::io;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parsed_identity_is_consumed_by_the_platform_neutral_stream() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::path::PathBuf::from("/tmp").join(format!(
            "cilai-local-transport-identity-{}-{nonce}",
            std::process::id()
        ));
        let socket = root.join("c.sock");
        let identity = EndpointIdentity::parse(socket.to_str().unwrap()).unwrap();
        let Err(error) = LocalStream::connect_identity(&identity) else {
            panic!("nonexistent endpoint unexpectedly accepted a connection");
        };
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
