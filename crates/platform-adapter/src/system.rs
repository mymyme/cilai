//! 本地用户信息查询与系统集成（默认程序打开、文件管理器定位、默认关联应用查询）。
//! macOS 端不再依赖 `USER` 环境变量（某些启动路径下该变量缺失会直接报错，例如通过
//! Launch Services 而非终端启动时），改为通过系统账户命令查询；Windows 端使用
//! 域账户通过 `GetUserNameExW` 读取显示名，本地账户通过 `NetUserGetInfo` 读取账户资料，
//! 账户名由 `GetUserNameW` 读取。两端行为对齐。

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// 当前本机用户的账户名与显示名。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalUser {
    pub username: String,
    pub display_name: String,
}

#[cfg(any(test, windows))]
fn local_user_with_display_name(
    username: String,
    display_name: io::Result<String>,
) -> io::Result<LocalUser> {
    Ok(LocalUser {
        username,
        display_name: display_name?,
    })
}

#[cfg(any(test, windows))]
fn local_account_display_name(username: &str, full_name: &str) -> io::Result<String> {
    if username.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "local account name is empty",
        ));
    }
    // Windows 本地账户的全名是可选资料；查询成功且未设置全名时展示该记录的账户名。
    // 此处不处理查询错误，不能把失败变成另一种身份或环境变量中的名称。
    Ok(if full_name.trim().is_empty() {
        username
    } else {
        full_name
    }
    .to_owned())
}

/// 查询当前本机用户信息。
///
/// # Errors
///
/// 底层系统查询失败或返回空值时返回错误。
pub fn current_user() -> io::Result<LocalUser> {
    platform::current_user()
}

/// 返回当前平台的用户主目录。
///
/// # Errors
///
/// 平台主目录环境变量缺失、为空或不是绝对路径时返回错误。
pub fn user_home_directory() -> io::Result<PathBuf> {
    platform::user_home_directory()
}

/// 返回当前平台存放应用本地数据的根目录。
///
/// macOS 与 Windows 的约定不同，而桌面端的数据目录必须在两端都落到各自平台的惯例位置：
/// 一旦算错，升级后既有工作区、会话与凭据都读不到。差异按平台正向列举，归在本适配层内，
/// 业务层只拿结果。
///
/// # Errors
///
/// 平台主目录或数据目录环境变量缺失、为空或不是绝对路径时返回错误。
pub fn application_data_directory() -> io::Result<PathBuf> {
    platform::application_data_directory()
}

fn home_directory_from_environment(variable: &str) -> io::Result<PathBuf> {
    validate_home_directory(std::env::var_os(variable), variable)
}

fn validate_home_directory(value: Option<OsString>, variable: &str) -> io::Result<PathBuf> {
    let value = value.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{variable} is unavailable"),
        )
    })?;
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{variable} is not an absolute directory"),
        ));
    }
    Ok(path)
}

#[cfg(any(test, windows))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssociationQueryStatus {
    Success,
    InsufficientBuffer,
    NoAssociation,
    Failure(i32),
}

#[cfg(any(test, windows))]
const fn hresult_from_win32(error: u32) -> i32 {
    if error == 0 {
        0
    } else {
        ((error & 0x0000_FFFF) | 0x8007_0000).cast_signed()
    }
}

#[cfg(any(test, windows))]
fn classify_association_hresult(result: i32, no_association: i32) -> AssociationQueryStatus {
    match result {
        0 => AssociationQueryStatus::Success,
        1 => AssociationQueryStatus::InsufficientBuffer,
        value if value == no_association => AssociationQueryStatus::NoAssociation,
        value => AssociationQueryStatus::Failure(value),
    }
}

/// 使用系统关联的默认程序打开文件。
///
/// # Errors
///
/// 系统未能启动关联程序时返回错误。
pub fn open_with_default_handler(target: &Path) -> io::Result<()> {
    platform::open_with_default_handler(target)
}

/// 使用系统默认浏览器打开一个 URL。
///
/// # Errors
///
/// 系统未能启动浏览器时返回错误。
pub fn open_url_with_default_browser(url: &str) -> io::Result<()> {
    platform::open_url_with_default_browser(url)
}

/// 在文件管理器中定位并高亮显示指定的文件或目录。
///
/// # Errors
///
/// 文件管理器未能启动时返回错误。
pub fn reveal_in_file_manager(target: &Path) -> io::Result<()> {
    platform::reveal_in_file_manager(target)
}

/// 使用 Visual Studio Code 打开文件。
///
/// # Errors
///
/// Visual Studio Code 未安装或未能启动时返回错误。
pub fn open_with_visual_studio_code(target: &Path) -> io::Result<()> {
    platform::open_with_visual_studio_code(target)
}

/// 查询系统为该文件关联的默认应用显示名称。仅系统明确未登记关联应用时返回
/// `Ok(None)`。
///
/// # Errors
///
/// 系统关联查询失败或返回无效结果时返回错误。
pub fn default_application_display_name(target: &Path) -> io::Result<Option<String>> {
    platform::default_application_display_name(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_reports_non_empty_username_and_display_name() {
        let user = current_user().unwrap();
        assert!(!user.username.trim().is_empty());
        assert!(!user.display_name.trim().is_empty());
    }

    #[test]
    fn home_directory_requires_an_absolute_path() {
        assert_eq!(
            validate_home_directory(Some(OsString::from("relative/path")), "TEST_HOME")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            validate_home_directory(None, "TEST_HOME")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn default_application_display_name_returns_none_without_extension() {
        let path = Path::new("/tmp/no-extension-file");
        assert_eq!(default_application_display_name(path).unwrap(), None);
    }

    #[test]
    fn association_hresult_classification_only_accepts_explicit_no_association() {
        let no_association = hresult_from_win32(1_155);
        assert_eq!(
            classify_association_hresult(0, no_association),
            AssociationQueryStatus::Success
        );
        assert_eq!(
            classify_association_hresult(1, no_association),
            AssociationQueryStatus::InsufficientBuffer
        );
        assert_eq!(
            classify_association_hresult(no_association, no_association),
            AssociationQueryStatus::NoAssociation
        );
        assert_eq!(
            classify_association_hresult(0x8000_4005_u32.cast_signed(), no_association),
            AssociationQueryStatus::Failure(0x8000_4005_u32.cast_signed())
        );
    }

    #[test]
    fn display_name_failure_is_not_replaced_with_account_name() {
        let error = local_user_with_display_name(
            "account".to_owned(),
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "display name unavailable",
            )),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[cfg(windows)]
    #[test]
    fn application_data_stays_in_the_local_profile() {
        let local = home_directory_from_environment("LOCALAPPDATA").unwrap();
        assert_eq!(application_data_directory().unwrap(), local);
    }

    #[test]
    fn local_account_display_name_accepts_an_unset_optional_full_name() {
        assert_eq!(
            local_account_display_name("tester", "测试员工").unwrap(),
            "测试员工"
        );
        assert_eq!(local_account_display_name("tester", "").unwrap(), "tester");
        assert!(local_account_display_name("", "测试员工").is_err());
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::LocalUser;
    use std::io;
    use std::path::Path;
    use std::process::{Command, Stdio};

    pub fn current_user() -> io::Result<LocalUser> {
        Ok(LocalUser {
            username: run_id(&["-un"])?,
            display_name: run_id(&["-F"])?,
        })
    }

    pub fn user_home_directory() -> io::Result<std::path::PathBuf> {
        super::home_directory_from_environment("HOME")
    }

    pub fn application_data_directory() -> io::Result<std::path::PathBuf> {
        Ok(user_home_directory()?
            .join("Library")
            .join("Application Support"))
    }

    fn run_id(args: &[&str]) -> io::Result<String> {
        let output = Command::new("/usr/bin/id")
            .args(args)
            .env_clear()
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "id {args:?} exited with {}",
                output.status
            )));
        }
        let text = String::from_utf8(output.stdout)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let trimmed = text.trim().to_owned();
        if trimmed.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "id returned empty output",
            ));
        }
        Ok(trimmed)
    }

    pub fn open_with_default_handler(target: &Path) -> io::Result<()> {
        run_open(|command| {
            command.arg(target);
        })
    }

    pub fn open_url_with_default_browser(url: &str) -> io::Result<()> {
        run_open(|command| {
            command.arg(url);
        })
    }

    pub fn reveal_in_file_manager(target: &Path) -> io::Result<()> {
        run_open(|command| {
            command.arg("-R").arg(target);
        })
    }

    pub fn open_with_visual_studio_code(target: &Path) -> io::Result<()> {
        run_open(|command| {
            command.arg("-a").arg("Visual Studio Code").arg(target);
        })
    }

    fn run_open(configure: impl FnOnce(&mut Command)) -> io::Result<()> {
        let mut command = Command::new("/usr/bin/open");
        configure(&mut command);
        let status = command.status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "/usr/bin/open exited with {status}"
            )))
        }
    }

    pub fn default_application_display_name(target: &Path) -> io::Result<Option<String>> {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        let path = target.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "path is not valid UTF-8")
        })?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let workspace = NSWorkspace::sharedWorkspace();
        Ok(workspace
            .URLForApplicationToOpenURL(&url)
            .and_then(|application| application.path())
            .map(|bundle_path| display_name_from_bundle_path(&bundle_path.to_string())))
    }

    fn display_name_from_bundle_path(application_path: &str) -> String {
        Path::new(application_path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(application_path)
            .strip_suffix(".app")
            .unwrap_or_else(|| {
                Path::new(application_path)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(application_path)
            })
            .to_owned()
    }

    #[cfg(test)]
    mod tests {
        use super::display_name_from_bundle_path;

        #[test]
        fn application_name_hides_the_bundle_suffix() {
            assert_eq!(
                display_name_from_bundle_path("/Applications/Microsoft Word.app"),
                "Microsoft Word"
            );
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::LocalUser;
    use std::ffi::OsStr;
    use std::io;
    use std::path::Path;
    use std::process::{Command, Stdio};

    /// 非 macOS 的 Unix 平台不属于本产品的正式目标（见架构基线第 6 节），此处仅提供
    /// 最小可用实现以保持工作区跨三平台编译，不追加 GECOS 全名解析等 macOS 专属逻辑。
    pub fn current_user() -> io::Result<LocalUser> {
        let output = Command::new("/usr/bin/id")
            .arg("-un")
            .env_clear()
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "id -un exited with {}",
                output.status
            )));
        }
        let username = String::from_utf8(output.stdout)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
            .trim()
            .to_owned();
        if username.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "id returned empty output",
            ));
        }
        Ok(LocalUser {
            display_name: username.clone(),
            username,
        })
    }

    pub fn user_home_directory() -> io::Result<std::path::PathBuf> {
        super::home_directory_from_environment("HOME")
    }

    /// 同样出于「非正式目标平台」的定位，按 XDG 约定给出最小实现。
    pub fn application_data_directory() -> io::Result<std::path::PathBuf> {
        super::home_directory_from_environment("XDG_DATA_HOME")
            .or_else(|_| Ok(user_home_directory()?.join(".local").join("share")))
    }

    /// 同样出于「非正式目标平台」的定位，以下几个函数仅提供基于 `xdg-open` 的最小实现，
    /// 不追加 `.desktop` 文件解析等 Linux 桌面环境专属逻辑。
    pub fn open_with_default_handler(target: &Path) -> io::Result<()> {
        run_xdg_open(target.as_os_str())
    }

    pub fn open_url_with_default_browser(url: &str) -> io::Result<()> {
        run_xdg_open(OsStr::new(url))
    }

    pub fn reveal_in_file_manager(target: &Path) -> io::Result<()> {
        run_xdg_open(target.as_os_str())
    }

    pub fn open_with_visual_studio_code(target: &Path) -> io::Result<()> {
        let status = Command::new("code").arg(target).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("code exited with {status}")))
        }
    }

    pub fn default_application_display_name(_target: &Path) -> io::Result<Option<String>> {
        Ok(None)
    }

    fn run_xdg_open(target: &OsStr) -> io::Result<()> {
        let status = Command::new("xdg-open").arg(target).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("xdg-open exited with {status}")))
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use super::{
        AssociationQueryStatus, LocalUser, classify_association_hresult, hresult_from_win32,
        local_account_display_name, local_user_with_display_name,
    };
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_ASSOCIATION};
    use windows_sys::Win32::NetworkManagement::NetManagement::{
        NetApiBufferFree, NetUserGetInfo, USER_INFO_10,
    };
    use windows_sys::Win32::Security::Authentication::Identity::{
        EXTENDED_NAME_FORMAT, GetUserNameExW, NameDisplay, NameSamCompatible,
    };
    use windows_sys::Win32::System::WindowsProgramming::{GetComputerNameW, GetUserNameW};
    use windows_sys::Win32::UI::Shell::{
        ASSOCF_NONE, ASSOCSTR_FRIENDLYAPPNAME, AssocQueryStringW, ShellExecuteW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    pub fn current_user() -> io::Result<LocalUser> {
        let username = account_name()?;
        let display_name = display_name(&username);
        local_user_with_display_name(username, display_name)
    }

    pub fn user_home_directory() -> io::Result<std::path::PathBuf> {
        super::home_directory_from_environment("USERPROFILE")
    }

    /// 业务数据库、凭据与运行时使用本地应用数据目录，保持既有 `app_local_data_dir`
    /// 的位置；环境配置缺失或无效时明确失败，不猜测目录或转入漫游配置。
    pub fn application_data_directory() -> io::Result<std::path::PathBuf> {
        super::home_directory_from_environment("LOCALAPPDATA")
    }

    fn account_name() -> io::Result<String> {
        let mut size: u32 = 0;
        // SAFETY: 空缓冲区、零长度探测调用是 `GetUserNameW` 允许的用法，仅通过 `size`
        // 输出所需字符数（含结尾 NUL），不解引用缓冲区参数。
        unsafe {
            GetUserNameW(std::ptr::null_mut(), &raw mut size);
        }
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0_u16; size as usize];
        // SAFETY: `buffer` 长度等于探测阶段返回的 `size`，满足 `GetUserNameW` 对输出
        // 缓冲区大小的要求。
        let succeeded = unsafe { GetUserNameW(buffer.as_mut_ptr(), &raw mut size) };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(trim_wide(&buffer))
    }

    fn display_name(username: &str) -> io::Result<String> {
        let sam_name = query_extended_name(NameSamCompatible)?;
        let (authority, _) = sam_name.split_once('\\').ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "SAM account has no authority")
        })?;
        let mut computer = [0_u16; 256];
        let mut size = u32::try_from(computer.len()).expect("computer name buffer fits in u32");
        // SAFETY: `computer` 为有效的可写 UTF-16 缓冲区，`size` 是它的实际容量。
        if unsafe { GetComputerNameW(computer.as_mut_ptr(), &raw mut size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if authority.eq_ignore_ascii_case(&trim_wide(&computer)) {
            local_display_name(username)
        } else {
            // 域账户查询失败时保持原错误，不改查同名本地账户。
            query_extended_name(NameDisplay)
        }
    }

    struct NetUserBuffer(*mut u8);

    impl Drop for NetUserBuffer {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: 该缓冲区仅由 NetUserGetInfo 分配，本类型独占并只释放一次。
                unsafe { NetApiBufferFree(self.0.cast()) };
            }
        }
    }

    fn local_display_name(username: &str) -> io::Result<String> {
        let name = to_wide(OsStr::new(username));
        let mut buffer = NetUserBuffer(std::ptr::null_mut());
        // SAFETY: 空服务器参数只查询本机；name 以 NUL 结尾，buffer 在调用期间有效。
        // 调用前已确认当前身份归属于本机，不能以本地同名账户替代域账户。
        let status =
            unsafe { NetUserGetInfo(std::ptr::null(), name.as_ptr(), 10, &raw mut buffer.0) };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(
                i32::try_from(status).unwrap_or(-1),
            ));
        }
        if buffer.0.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "local account record is missing",
            ));
        }
        // SAFETY: 成功的 level 10 查询返回有效 USER_INFO_10，buffer 在读取完成前不会释放。
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "NetUserGetInfo level 10 按 USER_INFO_10 分配并对齐缓冲区"
        )]
        let info = unsafe { &*buffer.0.cast::<USER_INFO_10>() };
        // SAFETY: 两个字符串均由同一成功查询返回，按 NetAPI 契约以 NUL 结尾；读取期间
        // 持有整个分配，拷贝后的 String 不再引用 NetAPI 内存。可选空指针按未设置处理。
        let (account, full_name) = unsafe {
            (
                net_user_string(info.usri10_name),
                net_user_string(info.usri10_full_name),
            )
        };
        local_account_display_name(&account, &full_name)
    }

    unsafe fn net_user_string(value: *const u16) -> String {
        if value.is_null() {
            return String::new();
        }
        let mut length = 0;
        // SAFETY: 调用方保证 value 指向仍存活且以 NUL 结尾的 NetAPI UTF-16 字符串。
        unsafe {
            while *value.add(length) != 0 {
                length += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(value, length))
        }
    }

    fn query_extended_name(format: EXTENDED_NAME_FORMAT) -> io::Result<String> {
        let mut size: u32 = 0;
        // SAFETY: 空缓冲区、零长度探测调用是 `GetUserNameExW` 允许的用法；失败时（预期为
        // `ERROR_MORE_DATA`）通过 `size` 输出所需字符数。
        let probe = unsafe { GetUserNameExW(format, std::ptr::null_mut(), &raw mut size) };
        if probe {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "extended user name is empty",
            ));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(i32::try_from(ERROR_MORE_DATA).unwrap_or(-1)) {
            return Err(error);
        }
        if size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "extended user name is empty",
            ));
        }
        let mut buffer = vec![0_u16; size as usize];
        // SAFETY: `buffer` 长度等于探测阶段返回的 `size`，满足 `GetUserNameExW` 对输出
        // 缓冲区大小的要求。
        let succeeded = unsafe { GetUserNameExW(format, buffer.as_mut_ptr(), &raw mut size) };
        if !succeeded {
            return Err(io::Error::last_os_error());
        }
        let name = trim_wide(&buffer);
        if name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "extended user name is empty",
            ));
        }
        Ok(name)
    }

    fn trim_wide(buffer: &[u16]) -> String {
        let end = buffer
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }

    pub fn open_with_default_handler(target: &Path) -> io::Result<()> {
        shell_execute_open(target.as_os_str())
    }

    pub fn open_url_with_default_browser(url: &str) -> io::Result<()> {
        shell_execute_open(OsStr::new(url))
    }

    fn shell_execute_open(target: &OsStr) -> io::Result<()> {
        let operation = to_wide(OsStr::new("open"));
        let file = to_wide(target);
        // SAFETY: `operation` 与 `file` 均为以 NUL 结尾、生命周期覆盖本次调用的 UTF-16
        // 缓冲区；`ShellExecuteW` 不会在返回后继续持有这些指针。
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // `ShellExecuteW` 返回值 <= 32 表示失败，返回值本身是历史遗留的错误码，不是句柄。
        if (result as usize) <= 32 {
            return Err(io::Error::other(format!(
                "ShellExecuteW failed with code {}",
                result as usize
            )));
        }
        Ok(())
    }

    pub fn reveal_in_file_manager(target: &Path) -> io::Result<()> {
        let mut argument = std::ffi::OsString::from("/select,");
        argument.push(target.as_os_str());
        // `explorer.exe` 在定位到已运行的外壳进程后会立即退出，其退出码历来不代表操作是否
        // 成功（已知系统行为），因此这里只在进程本身无法启动时返回错误，不校验退出状态。
        std::process::Command::new("explorer.exe")
            .arg(argument)
            .spawn()?;
        Ok(())
    }

    pub fn open_with_visual_studio_code(target: &Path) -> io::Result<()> {
        let status = std::process::Command::new("code.cmd")
            .arg(target)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("code.cmd exited with {status}")))
        }
    }

    pub fn default_application_display_name(target: &Path) -> io::Result<Option<String>> {
        let Some(extension) = target.extension() else {
            return Ok(None);
        };
        let mut association_name = OsString::from(".");
        association_name.push(extension);
        let association = to_wide(&association_name);
        let mut size: u32 = 0;
        // SAFETY: 空缓冲区、零长度探测调用是 `AssocQueryStringW` 允许的用法（`pszOut`
        // 为 `NULL` 时仅通过 `pcchOut` 输出所需字符数），不解引用缓冲区参数。
        let probe = unsafe {
            AssocQueryStringW(
                ASSOCF_NONE,
                ASSOCSTR_FRIENDLYAPPNAME,
                association.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                &raw mut size,
            )
        };
        let no_association = hresult_from_win32(ERROR_NO_ASSOCIATION);
        match classify_association_hresult(probe, no_association) {
            AssociationQueryStatus::NoAssociation => return Ok(None),
            AssociationQueryStatus::Success | AssociationQueryStatus::InsufficientBuffer
                if size > 0 => {}
            AssociationQueryStatus::Success | AssociationQueryStatus::InsufficientBuffer => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "AssocQueryStringW returned an empty buffer size",
                ));
            }
            AssociationQueryStatus::Failure(result) => {
                return Err(association_query_error(result));
            }
        }
        let mut buffer = vec![0_u16; size as usize];
        // SAFETY: `buffer` 长度等于探测阶段返回的 `size`，满足 `AssocQueryStringW` 对
        // 输出缓冲区大小的要求。
        let result = unsafe {
            AssocQueryStringW(
                ASSOCF_NONE,
                ASSOCSTR_FRIENDLYAPPNAME,
                association.as_ptr(),
                std::ptr::null(),
                buffer.as_mut_ptr(),
                &raw mut size,
            )
        };
        match classify_association_hresult(result, no_association) {
            AssociationQueryStatus::Success => {}
            AssociationQueryStatus::NoAssociation => return Ok(None),
            AssociationQueryStatus::InsufficientBuffer => {
                return Err(io::Error::other(
                    "AssocQueryStringW association name changed during query",
                ));
            }
            AssociationQueryStatus::Failure(result) => {
                return Err(association_query_error(result));
            }
        }
        let name = trim_wide(&buffer);
        if name.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "AssocQueryStringW returned an empty application name",
            ));
        }
        Ok(Some(name))
    }

    fn association_query_error(result: i32) -> io::Error {
        io::Error::other(format!(
            "AssocQueryStringW failed with HRESULT 0x{:08X}",
            result.cast_unsigned()
        ))
    }

    fn to_wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }
}
