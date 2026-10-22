//! AI 子进程的暂停、恢复、终止控制与生命周期归属。收敛 `ai_runtime` 中原先直接拼装
//! `/bin/kill` 命令行的逻辑，并提供 Windows 等效实现，使调用方只面向平台无关接口。
//!
//! Windows 暂停/恢复通过枚举目标进程的线程逐一挂起/恢复实现（不使用未文档化接口，行为
//! 可预测）。整棵子进程树的生命周期归属由 [`ProcessGroup`] 承担：调用方在
//! `Command::spawn` 之前调用 [`prepare_command_for_supervision`]，spawn 成功后立即调用
//! [`ProcessGroup::adopt`]。Windows 端子进程以挂起态创建，`adopt` 内先将其加入携带
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 Job Object，再恢复其主线程执行——进程从创建
//! 起点就处于 Job 约束范围内，不存在“先启动后补挂”的竞态窗口；宿主进程正常退出或被强制
//! 结束时，系统关闭 Job 句柄会自动终止其下全部成员进程，不残留子进程树。`terminate` 仍
//! 用于业务发起的单次终止（用户点击终止按钮），作用于已跟踪的单个 PID；Unix 端两套机制
//! 均沿用迁移前的 `kill(2)` 语义，不因本次改造改变行为。

use std::io;

/// 暂停目标进程。
///
/// # Errors
///
/// 目标进程不存在或操作失败时返回错误。
pub fn pause(pid: u32) -> io::Result<()> {
    platform::pause(pid)
}

/// 恢复已暂停的目标进程。
///
/// # Errors
///
/// 目标进程不存在或操作失败时返回错误。
pub fn resume(pid: u32) -> io::Result<()> {
    platform::resume(pid)
}

/// 终止目标进程。
///
/// # Errors
///
/// 目标进程不存在或操作失败时返回错误。
pub fn terminate(pid: u32) -> io::Result<()> {
    platform::terminate(pid)
}

/// 为即将启动的子进程准备受监督执行所需的创建参数；必须在 `Command::spawn` 之前调用，
/// 且不与 [`ProcessGroup::adopt`] 中间插入任何与该子进程交互的代码。Windows 端令进程以
/// 挂起态创建；Unix 端为空操作。
pub fn prepare_command_for_supervision(command: &mut std::process::Command) {
    platform::prepare_command_for_supervision(command);
}

/// Prepare a protocol-only helper without opening a visible console window.
pub fn prepare_background_command_for_supervision(command: &mut std::process::Command) {
    platform::prepare_background_command_for_supervision(command);
}

/// 独立服务不能继承启动者的其他 Job；系统不允许脱离时创建失败。
pub fn prepare_retainable_service_command(command: &mut std::process::Command) {
    platform::prepare_retainable_service_command(command);
}

/// 清空服务环境后补回平台运行必需项，不继承其他用户变量。macOS 保持空环境。
///
/// # Errors
///
/// Windows 缺少有效的 `SystemRoot` 时返回错误。
pub fn retain_required_system_environment(command: &mut std::process::Command) -> io::Result<()> {
    platform::retain_required_system_environment(command)
}

/// 一组受统一生命周期管理的子进程：所有通过 [`ProcessGroup::adopt`] 加入的子进程，会在
/// 宿主进程正常退出或异常终止时被整体回收，防止残留子进程树。跨调用方复用同一个进程组，
/// 见 [`shared_process_group`]。
pub struct ProcessGroup(platform::ProcessGroupHandle);

impl ProcessGroup {
    /// 创建一个新的进程组。
    ///
    /// # Errors
    ///
    /// 底层资源创建失败时返回错误。
    pub fn create() -> io::Result<Self> {
        platform::ProcessGroupHandle::create().map(Self)
    }

    /// 将子进程纳入本组的统一生命周期管理。`child` 必须是通过携带
    /// [`prepare_command_for_supervision`] 参数的 `Command` 启动、且此前未与其发生任何
    /// 交互的子进程，以保证 Windows 端的挂起态未被提前打破。
    ///
    /// # Errors
    ///
    /// 归属分配或恢复执行失败时返回错误。
    pub fn adopt(&self, child: &std::process::Child) -> io::Result<()> {
        self.0.adopt(child)
    }
}

/// 进程内唯一共享的进程组，覆盖桌面壳启动的 Owned App Server、内置 Codex Harness、
/// Windows 沙箱控制会话与外部本地 CLI；首次调用时创建，创建失败时返回错误，不静默跳过监督。
///
/// # Errors
///
/// 首次创建底层资源失败时返回错误。
pub fn shared_process_group() -> io::Result<&'static ProcessGroup> {
    static SHARED: std::sync::OnceLock<Result<ProcessGroup, io::Error>> =
        std::sync::OnceLock::new();
    match SHARED.get_or_init(ProcessGroup::create) {
        Ok(group) => Ok(group),
        Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
    }
}

/// 将刚启动的子进程纳入进程内共享的生命周期组。若进程组创建、归属分配或
/// Windows 主线程恢复失败，本函数会先终止并回收该子进程再返回错误，不会留下
/// 永久挂起或失去监督的进程。
///
/// # Errors
///
/// 底层监督初始化或归属失败时返回错误；若同时无法回收子进程，错误中会包含
/// 回收失败详情。
pub fn supervise_spawned_child(child: &mut std::process::Child) -> io::Result<()> {
    if let Err(supervision_error) = shared_process_group().and_then(|group| group.adopt(child)) {
        return Err(terminate_and_reap_after_supervision_failure(
            child,
            supervision_error,
        ));
    }
    Ok(())
}

/// 独立服务最初由启动者监管，只有服务确认已有客户端接续后才可交还生命周期。
pub struct RetainableServiceProcess(platform::ProcessGroupHandle);

impl RetainableServiceProcess {
    /// # Errors
    /// 监督失败时终止并回收刚创建的子进程。
    pub fn adopt(child: &mut std::process::Child) -> io::Result<Self> {
        let result = platform::ProcessGroupHandle::create().and_then(|group| {
            group.adopt(child)?;
            Ok(Self(group))
        });
        result.map_err(|error| terminate_and_reap_after_supervision_failure(child, error))
    }

    /// # Errors
    /// 无法仅解除本服务的启动者退出约束时拒绝交还生命周期。
    pub fn continue_after_owner_exit(&self) -> io::Result<()> {
        self.0.continue_after_owner_exit()
    }
}

fn terminate_and_reap_after_supervision_failure(
    child: &mut std::process::Child,
    supervision_error: io::Error,
) -> io::Error {
    let kill_error = child.kill().err();
    let wait_error = child.wait().err();
    if let Some(wait_error) = wait_error {
        return io::Error::new(
            supervision_error.kind(),
            format!(
                "{supervision_error}; failed to reap unsupervised child: {wait_error}{}",
                kill_error
                    .map(|error| format!("; termination also failed: {error}"))
                    .unwrap_or_default()
            ),
        );
    }
    supervision_error
}

#[cfg(any(test, windows))]
fn require_thread_operation(
    pid: u32,
    operated_threads: usize,
    open_error: Option<io::Error>,
) -> io::Result<()> {
    if operated_threads > 0 {
        return Ok(());
    }
    if let Some(error) = open_error {
        return Err(error);
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("process {pid} has no operable threads"),
    ))
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::process::{Command, Stdio};

    pub fn pause(pid: u32) -> io::Result<()> {
        signal(pid, "-STOP")
    }

    pub fn resume(pid: u32) -> io::Result<()> {
        signal(pid, "-CONT")
    }

    pub fn terminate(pid: u32) -> io::Result<()> {
        signal(pid, "-TERM")
    }

    fn signal(pid: u32, flag: &str) -> io::Result<()> {
        let status = Command::new("/bin/kill")
            .arg(flag)
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "kill {flag} {pid} exited with {status}"
            )))
        }
    }

    /// Unix 端子进程生命周期归属维持迁移前行为：`terminate` 仍只作用于已跟踪的单个 PID，
    /// 不引入进程组/会话级别的整体回收，避免在 macOS 测试调优期间改变既有行为。
    pub fn prepare_command_for_supervision(_command: &mut Command) {}

    pub fn prepare_background_command_for_supervision(command: &mut Command) {
        prepare_command_for_supervision(command);
    }

    pub fn prepare_retainable_service_command(command: &mut Command) {
        prepare_command_for_supervision(command);
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn retain_required_system_environment(_command: &mut Command) -> io::Result<()> {
        Ok(())
    }

    pub struct ProcessGroupHandle;

    impl ProcessGroupHandle {
        #[allow(clippy::unnecessary_wraps)]
        pub fn create() -> io::Result<Self> {
            Ok(Self)
        }

        #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
        pub fn adopt(&self, _child: &std::process::Child) -> io::Result<()> {
            Ok(())
        }

        #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
        pub fn continue_after_owner_exit(&self) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{require_thread_operation, terminate_and_reap_after_supervision_failure};
    use std::io;
    use std::process::{Command, Stdio};

    #[test]
    fn failed_supervision_terminates_and_reaps_the_spawned_child() {
        let mut child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let error = terminate_and_reap_after_supervision_failure(
            &mut child,
            io::Error::other("supervision failed"),
        );
        assert_eq!(error.to_string(), "supervision failed");
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn supervision_never_accepts_zero_operated_threads() {
        assert_eq!(
            require_thread_operation(42, 0, None).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let access_error = io::Error::new(io::ErrorKind::PermissionDenied, "thread denied");
        assert_eq!(
            require_thread_operation(42, 0, Some(access_error))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(require_thread_operation(42, 1, None).is_ok());
    }
}

/// Fixed no-op command for a Windows sandbox execution probe. Resolves the
/// system interpreter without PATH lookup and disables command-processor `AutoRun`.
/// # Errors
/// Reports unsupported platforms or a failed Windows system-directory lookup.
pub fn sandbox_probe_command() -> std::io::Result<Vec<String>> {
    #[cfg(windows)]
    return platform::sandbox_probe_command();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "windows_sandbox_probe_unsupported",
    ))
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use super::require_thread_operation;
    use std::io;
    use std::process::Command;
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_SUSPENDED, OpenProcess, OpenThread, PROCESS_TERMINATE, ResumeThread, SuspendThread,
        THREAD_SUSPEND_RESUME, TerminateProcess,
    };

    pub(super) fn sandbox_probe_command() -> io::Result<Vec<String>> {
        use windows_sys::Win32::System::Com::CoTaskMemFree;
        use windows_sys::Win32::UI::Shell::{FOLDERID_System, SHGetKnownFolderPath};
        let mut raw = ptr::null_mut();
        // SAFETY: fixed system folder, writable output pointer; no environment/PATH lookup.
        let result =
            unsafe { SHGetKnownFolderPath(&FOLDERID_System, 0, ptr::null_mut(), &raw mut raw) };
        if result < 0 || raw.is_null() {
            // SAFETY: the shell permits freeing NULL; release any partial allocation.
            unsafe { CoTaskMemFree(raw.cast()) };
            return Err(io::Error::other(
                "windows_sandbox_probe_system_directory_unavailable",
            ));
        }
        let path =
            std::path::PathBuf::from(crate::workspace_permissions::windows::local_string(raw))
                .join("cmd.exe");
        // SAFETY: release the successful shell allocation exactly once.
        unsafe { CoTaskMemFree(raw.cast()) };
        Ok(vec![
            path.to_string_lossy().into_owned(),
            "/d".into(),
            "/c".into(),
            "exit 0".into(),
        ])
    }

    pub fn pause(pid: u32) -> io::Result<()> {
        for_each_thread(pid, |thread| {
            // SAFETY: `thread` 是本函数刚通过 `OpenThread` 以 `THREAD_SUSPEND_RESUME`
            // 权限打开、仍然有效的线程句柄，调用后不保留额外引用。
            let previous = unsafe { SuspendThread(thread) };
            if previous == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })
    }

    pub fn resume(pid: u32) -> io::Result<()> {
        for_each_thread(pid, |thread| {
            // SAFETY: 同上，`thread` 是本函数刚打开、仍然有效的线程句柄。
            let previous = unsafe { ResumeThread(thread) };
            if previous == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })
    }

    pub fn terminate(pid: u32) -> io::Result<()> {
        // SAFETY: `PROCESS_TERMINATE` 只请求终止所需的最小权限；返回句柄在下方所有
        // 路径上都会被关闭。
        let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `process` 是上面刚打开、仍然有效的进程句柄，退出码 1 表示由本工作台
        // 主动终止，不与目标进程自身的正常退出码混淆。
        let succeeded = unsafe { TerminateProcess(process, 1) };
        // SAFETY: `process` 在此处仍然有效且不再被使用，关闭后不留悬空句柄。
        unsafe {
            CloseHandle(process);
        }
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// 枚举目标进程当前的全部线程并对每个线程执行 `operation`。任一线程操作失败即返回
    /// 该错误；没有成功操作任何目标线程时返回错误，避免把仍处于挂起态的受管进程当作
    /// 已经恢复。
    fn for_each_thread(pid: u32, operation: impl Fn(HANDLE) -> io::Result<()>) -> io::Result<()> {
        // SAFETY: 不传入进程 ID 限定（`0`），按 Win32 契约返回系统级线程快照；
        // 返回句柄在函数结束前经下方 `CloseHandle` 关闭。
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let result = enumerate_and_operate(snapshot, pid, &operation);
        // SAFETY: `snapshot` 在此处仍然有效且不再被使用。
        unsafe {
            CloseHandle(snapshot);
        }
        result
    }

    fn enumerate_and_operate(
        snapshot: HANDLE,
        pid: u32,
        operation: &impl Fn(HANDLE) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut entry = THREADENTRY32 {
            dwSize: u32::try_from(std::mem::size_of::<THREADENTRY32>())
                .expect("THREADENTRY32 size fits in u32"),
            cntUsage: 0,
            th32ThreadID: 0,
            th32OwnerProcessID: 0,
            tpBasePri: 0,
            tpDeltaPri: 0,
            dwFlags: 0,
        };
        // SAFETY: `snapshot` 是调用方刚创建的有效快照句柄；`entry` 已按契约设置
        // `dwSize` 且是本地栈上的合法可写地址。
        let mut has_entry = unsafe { Thread32First(snapshot, &raw mut entry) } != 0;
        if !has_entry {
            return Err(io::Error::last_os_error());
        }
        let mut operated_threads = 0usize;
        let mut first_open_error = None;
        while has_entry {
            if entry.th32OwnerProcessID == pid {
                // SAFETY: `entry.th32ThreadID` 来自刚枚举到的快照条目，`THREAD_SUSPEND_RESUME`
                // 只请求挂起/恢复所需的最小权限；线程可能在枚举与操作之间退出，此时记录
                // 打开错误，只有另一个目标线程成功完成操作时才容忍该竞态。
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    if first_open_error.is_none() {
                        first_open_error = Some(io::Error::last_os_error());
                    }
                } else {
                    let outcome = operation(thread);
                    // SAFETY: `thread` 在此处仍然有效且不再被使用。
                    unsafe {
                        CloseHandle(thread);
                    }
                    outcome?;
                    operated_threads += 1;
                }
            }
            // SAFETY: 同上，`snapshot` 与 `entry` 均仍然有效。
            has_entry = unsafe { Thread32Next(snapshot, &raw mut entry) } != 0;
        }
        require_thread_operation(pid, operated_threads, first_open_error)
    }

    pub fn prepare_command_for_supervision(command: &mut Command) {
        use std::os::windows::process::CommandExt as _;
        // 令进程以挂起态创建：`ProcessGroupHandle::adopt` 会在恢复执行前先完成 Job Object
        // 归属分配，使进程从创建起点就处于监督范围内。
        command.creation_flags(CREATE_SUSPENDED);
    }

    pub fn prepare_background_command_for_supervision(command: &mut Command) {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(
            CREATE_SUSPENDED | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW,
        );
    }

    pub fn prepare_retainable_service_command(command: &mut Command) {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(
            CREATE_SUSPENDED
                | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
                | windows_sys::Win32::System::Threading::CREATE_BREAKAWAY_FROM_JOB,
        );
    }

    pub fn retain_required_system_environment(command: &mut Command) -> io::Result<()> {
        let system_root = std::env::var_os("SystemRoot")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SystemRoot is unavailable"))?;
        // Windows DNS provider loading requires SystemRoot even when the executable is absolute.
        command.env("SystemRoot", system_root);
        Ok(())
    }

    pub struct ProcessGroupHandle {
        job: HANDLE,
    }

    // SAFETY: `job` 是一个内核对象句柄，其生命周期由本类型独占管理；Win32 允许从任意线程
    // 对同一句柄发起 `AssignProcessToJobObject` 等操作，不存在需要额外同步的可变状态。
    unsafe impl Send for ProcessGroupHandle {}
    // SAFETY: 同上，本类型不持有除句柄本身以外的可变状态，多线程只读共享是安全的。
    unsafe impl Sync for ProcessGroupHandle {}

    impl ProcessGroupHandle {
        pub fn create() -> io::Result<Self> {
            // SAFETY: 空名称与空安全属性指针创建一个匿名、不可被其他进程按名打开的 Job
            // Object；返回句柄的所有权转移给本类型，由 `Drop` 负责关闭。
            let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let info_len =
                u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                    .expect("JOBOBJECT_EXTENDED_LIMIT_INFORMATION size fits in u32");
            // SAFETY: `job` 是上面刚创建、仍然有效的 Job 句柄；`info` 是本地栈上按契约
            // 初始化并置零其余字段的合法结构体，`info_len` 与其真实大小一致。
            let configured = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    ptr::from_ref(&info).cast(),
                    info_len,
                )
            };
            if configured == 0 {
                let error = io::Error::last_os_error();
                // SAFETY: `job` 在此处仍然有效且不再被使用，创建失败时立即释放。
                unsafe {
                    CloseHandle(job);
                }
                return Err(error);
            }
            Ok(Self { job })
        }

        pub fn adopt(&self, child: &std::process::Child) -> io::Result<()> {
            use std::os::windows::io::AsRawHandle as _;
            let process = child.as_raw_handle().cast::<core::ffi::c_void>();
            // SAFETY: `self.job` 是本类型持有、仍然有效的 Job 句柄；`process` 是调用方
            // 传入、仍然存活（`child` 未被丢弃）的子进程句柄，本次调用不获取其所有权。
            let assigned = unsafe { AssignProcessToJobObject(self.job, process) };
            if assigned == 0 {
                return Err(io::Error::last_os_error());
            }
            // 子进程以 `CREATE_SUSPENDED` 创建，此刻仅有主线程且处于挂起态，恢复它使进程
            // 真正开始执行；复用与 `resume` 相同的线程枚举逻辑。
            resume(child.id())
        }

        pub fn continue_after_owner_exit(&self) -> io::Result<()> {
            let info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            let info_len = u32::try_from(std::mem::size_of_val(&info))
                .expect("JOBOBJECT_EXTENDED_LIMIT_INFORMATION size fits in u32");
            // SAFETY: 只改变本对象独占的专用服务 Job；其余子进程的共享 Job 保持原策略。
            let configured = unsafe {
                SetInformationJobObject(
                    self.job,
                    JobObjectExtendedLimitInformation,
                    ptr::from_ref(&info).cast(),
                    info_len,
                )
            };
            if configured == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    impl Drop for ProcessGroupHandle {
        fn drop(&mut self) {
            // SAFETY: `self.job` 在此处仍然有效且不再被使用；关闭本进程持有的最后一个
            // 句柄会因 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 触发系统终止其下全部成员进程。
            unsafe {
                CloseHandle(self.job);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::retain_required_system_environment;
        use std::net::ToSocketAddrs as _;
        use std::process::Command;

        #[test]
        fn cleared_service_environment_supports_dns() {
            const CHILD_FLAG: &str = "CILAI_DNS_ENV_TEST_CHILD";
            if std::env::var_os(CHILD_FLAG).is_some() {
                assert!(
                    std::env::var_os("PATH").is_none(),
                    "must not inherit the full user environment"
                );
                assert!(std::env::var_os("SystemRoot").is_some());
                assert!("localhost:80".to_socket_addrs().unwrap().next().is_some());
                return;
            }
            let mut command = Command::new(std::env::current_exe().unwrap());
            command.env_clear().env(CHILD_FLAG, "1").args([
                "--exact",
                "process::platform::tests::cleared_service_environment_supports_dns",
            ]);
            retain_required_system_environment(&mut command).unwrap();
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
