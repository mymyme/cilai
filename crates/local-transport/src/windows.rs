//! Windows 本地传输实现：Named Pipe。
//!
//! 管道创建时携带显式安全描述符，仅授予当前用户与 `SYSTEM` 完全控制、移除继承的访问控制项；
//! 接受连接后再次校验客户端令牌所属用户 SID 与本进程一致，双重防止其他账户接入。首实例标志
//! 防止同名管道被抢占。本模块是 workspace 中少数允许 `unsafe` 的 Windows 私有模块之一，每个
//! `unsafe` 调用点均说明其内存、句柄与生命周期前提，并要求 Windows 契约测试覆盖
//! （见 `词来工作台Windows端适配技术方案.md` 第 2、4 节）。

#![allow(unsafe_code)]

use crate::{Endpoint, EndpointIdentity};
use std::io;
use std::os::windows::io::AsRawHandle;
use std::ptr;
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, ERROR_PIPE_BUSY, HANDLE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::System::IO::CancelIoEx;
use windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId;
use windows_sys::Win32::System::Threading::{
    OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

// 仅授予 SYSTEM 与创建者（当前用户）完全控制，`P` 标志阻止继承访问控制项；
// 未列出的其他主体（含 Users、Authenticated Users）没有匹配的允许项，隐式拒绝访问。
const PIPE_SECURITY_DESCRIPTOR_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;OW)";

// ---- 异步服务端：Local App Server 的 Named Pipe 监听 ----

pub struct PlatformListener {
    pipe_name: String,
    security: SecurityDescriptorGuard,
    next: tokio::sync::Mutex<NamedPipeServer>,
}

impl PlatformListener {
    pub fn bind(endpoint: &Endpoint) -> io::Result<Self> {
        let pipe_name = endpoint.pipe_path().to_owned();
        let security = SecurityDescriptorGuard::current_user_and_system()?;
        let first = create_instance(&pipe_name, &security, true)?;
        Ok(Self {
            pipe_name,
            security,
            next: tokio::sync::Mutex::new(first),
        })
    }

    pub async fn accept(&self) -> io::Result<PlatformConnection> {
        loop {
            let mut guard = self.next.lock().await;
            guard.connect().await?;
            let successor = create_instance(&self.pipe_name, &self.security, false)?;
            let connected = std::mem::replace(&mut *guard, successor);
            drop(guard);
            match verify_peer_is_current_user(&connected) {
                Ok(true) => return Ok(PlatformConnection { pipe: connected }),
                Ok(false) => {
                    eprintln!(
                        "Local IPC connection rejected: peer user does not own the runtime directory"
                    );
                }
                Err(error) => return Err(error),
            }
        }
    }
}

fn create_instance(
    pipe_name: &str,
    security: &SecurityDescriptorGuard,
    is_first: bool,
) -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(is_first)
        .reject_remote_clients(true)
        .pipe_mode(PipeMode::Byte)
        .in_buffer_size(65536)
        .out_buffer_size(65536);
    // SAFETY: `security.as_raw()` 指向本次调用期间保持有效的 `SECURITY_ATTRIBUTES`，其
    // `lpSecurityDescriptor` 由 `SecurityDescriptorGuard` 持有并在其 `Drop` 中释放；
    // `create_with_security_attributes_raw` 仅在本次调用中按值读取该结构体，不保留指针。
    unsafe { options.create_with_security_attributes_raw(pipe_name, security.as_raw()) }.map_err(
        |error| io::Error::new(error.kind(), format!("create named pipe instance: {error}")),
    )
}

pub struct PlatformConnection {
    pipe: NamedPipeServer,
}

impl PlatformConnection {
    pub fn into_split(
        self,
    ) -> (
        tokio::io::ReadHalf<NamedPipeServer>,
        tokio::io::WriteHalf<NamedPipeServer>,
    ) {
        tokio::io::split(self.pipe)
    }
}

pub type PlatformReadHalf = tokio::io::ReadHalf<NamedPipeServer>;
pub type PlatformWriteHalf = tokio::io::WriteHalf<NamedPipeServer>;

/// 持有 `ConvertStringSecurityDescriptorToSecurityDescriptorW` 分配的安全描述符与引用它的
/// `SECURITY_ATTRIBUTES`，析构时以 `LocalFree` 释放，避免每次创建管道实例都重新构造。
struct SecurityDescriptorGuard {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

// SAFETY: 描述符构造后不再被任何 Win32 调用写入，只被以只读方式传递给
// `CreateNamedPipeW`；跨线程共享或移动该守卫不会产生数据竞争。
unsafe impl Send for SecurityDescriptorGuard {}
unsafe impl Sync for SecurityDescriptorGuard {}

impl SecurityDescriptorGuard {
    fn current_user_and_system() -> io::Result<Self> {
        let wide: Vec<u16> = PIPE_SECURITY_DESCRIPTOR_SDDL
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `wide` 是以 NUL 结尾、在本次调用期间存活的合法 UTF-16 缓冲区；
        // `descriptor` 是本地栈上的合法可写输出参数，成功时由 Win32 通过 LocalAlloc
        // 分配，所有权转移给本守卫，由 `Drop` 负责 `LocalFree` 释放。
        let succeeded = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                ptr::null_mut(),
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>())
                .expect("SECURITY_ATTRIBUTES size fits in u32"),
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        Ok(Self {
            descriptor,
            attributes,
        })
    }

    fn as_raw(&self) -> *mut core::ffi::c_void {
        ptr::addr_of!(self.attributes) as *mut core::ffi::c_void
    }
}

impl Drop for SecurityDescriptorGuard {
    fn drop(&mut self) {
        if !self.descriptor.is_null() {
            // SAFETY: `self.descriptor` 由 `ConvertStringSecurityDescriptorToSecurityDescriptorW`
            // 通过 LocalAlloc 分配；本类型是其唯一所有者，此处是其生命周期内唯一一次释放。
            unsafe {
                LocalFree(self.descriptor);
            }
        }
    }
}

/// 持有一个在使用后立即关闭的 Win32 句柄，避免每个失败分支手写 `CloseHandle`。
struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` 是本类型独占持有、由 `OpenProcess`/`OpenProcessToken` 成功
            // 返回的句柄，未被其他代码路径关闭或转移所有权。
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn verify_peer_is_current_user(pipe: &NamedPipeServer) -> io::Result<bool> {
    let handle = pipe.as_raw_handle() as HANDLE;
    let mut client_process_id: u32 = 0;
    // SAFETY: `handle` 是 `pipe` 独占持有、此刻已完成连接的命名管道服务端句柄；
    // `client_process_id` 是本地栈上的合法可写输出参数。
    let succeeded = unsafe { GetNamedPipeClientProcessId(handle, &raw mut client_process_id) };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    let client_sid = process_user_sid(client_process_id)?;
    let current_sid = process_user_sid(std::process::id())?;
    Ok(client_sid == current_sid)
}

fn process_user_sid(pid: u32) -> io::Result<Vec<u8>> {
    // SAFETY: `PROCESS_QUERY_LIMITED_INFORMATION` 只请求读取令牌所需的最小权限；
    // 返回的句柄立即包装进 `HandleGuard`，在函数返回前的所有路径上都会被关闭。
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let _process_guard = HandleGuard(process);

    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `process` 是上面刚打开、仍然有效的进程句柄；`token` 是本地栈上的合法
    // 可写输出参数，成功时获得的令牌句柄同样立即交给 `HandleGuard` 管理。
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) };
    if opened == 0 {
        return Err(io::Error::last_os_error());
    }
    let _token_guard = HandleGuard(token);

    let mut needed: u32 = 0;
    // SAFETY: 首次调用以空缓冲区探测所需长度，`GetTokenInformation` 在此模式下允许
    // 空指针与零长度，只通过 `needed` 输出所需字节数，不解引用缓冲区参数。
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &raw mut needed);
    }
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    // 以 `usize` 为单位分配，保证缓冲区起始地址满足 `TOKEN_USER` 内部 SID 指针字段的
    // 对齐要求（64 位下为 8 字节），而不是仅依赖 `Vec<u8>` 的 1 字节对齐保证。
    let word_count = (needed as usize).div_ceil(std::mem::size_of::<usize>());
    let mut buffer: Vec<usize> = vec![0; word_count];
    let buffer_bytes = u32::try_from(word_count * std::mem::size_of::<usize>())
        .expect("token buffer size fits in u32");
    // SAFETY: `buffer` 刚分配、长度不小于探测阶段返回的 `needed`，且按 `usize` 对齐，
    // 满足 `GetTokenInformation` 对输出缓冲区大小与对齐的要求。
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
    // SAFETY: `buffer` 刚被 `GetTokenInformation` 成功写入且满足 `TOKEN_USER` 的对齐
    // 要求；`token_user.User.Sid` 指向同一缓冲区内、由该次调用保证有效的 SID 结构，
    // 其生命周期不超出 `buffer` 本身。
    let sid_bytes = unsafe {
        let token_user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let sid_ptr = token_user.User.Sid;
        let sid_len = GetLengthSid(sid_ptr) as usize;
        std::slice::from_raw_parts(sid_ptr.cast::<u8>(), sid_len).to_vec()
    };
    Ok(sid_bytes)
}

// ---- 同步客户端：供 supervisor（attach 模式）与 CLI 使用 ----

pub struct PlatformStream {
    file: std::fs::File,
}

impl PlatformStream {
    pub fn connect(endpoint: &Endpoint) -> io::Result<Self> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(endpoint.pipe_path())
            .map(|file| Self { file })
            .map_err(translate_connect_error)
    }

    pub fn connect_identity(identity: &EndpointIdentity) -> io::Result<Self> {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(identity.pipe_path())
            .map(|file| Self { file })
            .map_err(translate_connect_error)
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        self.file.try_clone().map(|file| Self { file })
    }

    /// 取消该句柄上的挂起 I/O，是 Windows 命名管道客户端能达到的最接近
    /// `Unix shutdown(Both)` 的操作。已知限制：无法像 POSIX `shutdown()` 那样对
    /// 通过 `try_clone` 复制出的独立句柄同时生效，读取线程可能延迟到远端关闭连接
    /// 时才解除阻塞；调用方对该时序保持幂等，详见模块顶部说明与 supervisor 的
    /// 分离处理逻辑，标记为待 Windows 实机验证项。
    pub fn disconnect(&self) -> io::Result<()> {
        let handle = self.file.as_raw_handle() as HANDLE;
        // SAFETY: `handle` 由 `self.file` 独占持有且在本次调用期间保持有效；
        // 以 null `lpOverlapped` 调用 `CancelIoEx` 取消该句柄上的全部挂起 I/O。
        let succeeded = unsafe { CancelIoEx(handle, ptr::null()) };
        if succeeded == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(i32::try_from(ERROR_NOT_FOUND).unwrap_or(-1)) {
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }
}

impl io::Read for PlatformStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.file.read(buffer)
    }
}

impl io::Write for PlatformStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.file.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn translate_connect_error(error: io::Error) -> io::Error {
    match error.raw_os_error() {
        Some(code) if code == i32::try_from(ERROR_FILE_NOT_FOUND).unwrap_or(-1) => {
            io::Error::new(io::ErrorKind::NotFound, error)
        }
        Some(code) if code == i32::try_from(ERROR_PIPE_BUSY).unwrap_or(-1) => {
            io::Error::new(io::ErrorKind::ConnectionRefused, error)
        }
        _ => error,
    }
}

/// 探测端点是否仍有实例接受连接：能连接、或管道存在但正忙（`ERROR_PIPE_BUSY`）均视为存活。
pub fn probe(endpoint: &Endpoint) -> io::Result<bool> {
    probe_result(PlatformStream::connect(endpoint))
}

pub fn probe_identity(identity: &EndpointIdentity) -> io::Result<bool> {
    probe_result(PlatformStream::connect_identity(identity))
}

fn probe_result(result: io::Result<PlatformStream>) -> io::Result<bool> {
    match result {
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

/// Named Pipe 随最后一个服务端句柄关闭自动从命名空间消失，不产生独立于存活监听者的
/// 文件系统残留；因此“是否存在残留”与“是否存活”是同一个问题，直接复用 `probe`。
pub fn artifact_exists(endpoint: &Endpoint) -> io::Result<bool> {
    probe(endpoint)
}

pub fn artifact_exists_identity(identity: &EndpointIdentity) -> io::Result<bool> {
    probe_identity(identity)
}

/// Windows 端没有需要清理的陈旧文件系统残留，是空操作；管道实例本身随进程退出自动消失。
/// 签名与 Unix 端保持一致以满足跨平台契约，本实现恒为 `Ok(())`。
#[allow(clippy::unnecessary_wraps)]
pub fn cleanup_stale(_endpoint: &Endpoint) -> io::Result<()> {
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
pub fn cleanup_stale_identity(_identity: &EndpointIdentity) -> io::Result<()> {
    Ok(())
}
