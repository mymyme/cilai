//! 当前工作台的系统磁盘访问设置导航，不查询或更改授权。
use crate::{OperatingSystem, current_operating_system};
use std::io;
#[cfg(any(test, target_os = "macos"))]
use std::path::{Path, PathBuf};

/// 打开系统的完全磁盘访问设置；授权由用户完成。
/// # Errors
/// 非 macOS、非正式应用包上下文、系统版本或系统导航失败时明确返回错误。
pub fn open_settings() -> io::Result<()> {
    require_macos(current_operating_system())?;
    platform::open_settings()
}

/// 在文件管理器中显示当前运行宿主所属的工作台应用。
/// # Errors
/// 非 macOS、应用包验证或文件管理器启动失败时返回错误。
pub fn reveal_application() -> io::Result<()> {
    require_macos(current_operating_system())?;
    platform::reveal_application()
}

fn require_macos(os: OperatingSystem) -> io::Result<()> {
    if os == OperatingSystem::MacOs {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "磁盘访问设置仅适用于 macOS",
        ))
    }
}

#[cfg(any(test, target_os = "macos"))]
fn settings_url(version: &str) -> io::Result<&'static str> {
    let major: u32 = version
        .trim()
        .split('.')
        .next()
        .and_then(|s| s.parse().ok())
        .filter(|major| *major >= 10)
        .ok_or_else(|| io::Error::other("无法识别 macOS 版本"))?;
    Ok(if major >= 13 {
        "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_AllFiles"
    } else {
        "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"
    })
}

#[cfg(any(test, target_os = "macos"))]
fn bundle_candidate(executable: &Path) -> io::Result<PathBuf> {
    let executable = executable.canonicalize()?;
    let app = executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|s| s == "app"))
        .ok_or_else(|| io::Error::other("当前进程不属于工作台应用包"))?;
    if executable.strip_prefix(app).ok()
        != Some(Path::new("Contents/Resources/sidecar/cilai-host-service"))
    {
        return Err(io::Error::other("当前进程不是随包工作台宿主"));
    }
    Ok(app.to_path_buf())
}

#[cfg(any(test, target_os = "macos"))]
fn validate_manifest(app: &Path, manifest: &serde_json::Value) -> io::Result<()> {
    let name = manifest["CFBundleExecutable"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| io::Error::other("工作台应用主程序无效"))?;
    if manifest["CFBundleIdentifier"] != "com.cilai.workbench"
        || manifest["CFBundlePackageType"] != "APPL"
        || Path::new(name).components().count() != 1
        || !matches!(
            Path::new(name).components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err(io::Error::other("工作台应用身份无效"));
    }
    let contents = app.join("Contents").canonicalize()?;
    let plist = contents.join("Info.plist").canonicalize()?;
    let executable = contents.join("MacOS").join(name).canonicalize()?;
    if !contents.starts_with(app)
        || !plist.starts_with(&contents)
        || !executable.starts_with(contents.join("MacOS"))
        || !executable.is_file()
    {
        return Err(io::Error::other("工作台应用路径越界"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{PathBuf, bundle_candidate, io, settings_url, validate_manifest};
    use std::process::{Command, Stdio};

    fn output(program: &str, args: &[&std::ffi::OsStr]) -> io::Result<Vec<u8>> {
        let result = Command::new(program)
            .args(args)
            .env_clear()
            .stdin(Stdio::null())
            .output()?;
        if !result.status.success() {
            return Err(io::Error::other("系统设置导航检查失败"));
        }
        Ok(result.stdout)
    }

    fn current_application() -> io::Result<PathBuf> {
        let app = bundle_candidate(&std::env::current_exe()?)?;
        let plist = app.join("Contents/Info.plist").canonicalize()?;
        if !plist.starts_with(app.join("Contents"))
            || !plist.is_file()
            || plist.metadata()?.len() > 64 * 1024
        {
            return Err(io::Error::other("工作台应用信息路径无效"));
        }
        let bytes = output(
            "/usr/bin/plutil",
            &[
                std::ffi::OsStr::new("-convert"),
                std::ffi::OsStr::new("json"),
                std::ffi::OsStr::new("-o"),
                std::ffi::OsStr::new("-"),
                plist.as_os_str(),
            ],
        )?;
        let manifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        validate_manifest(&app, &manifest)?;
        Ok(app)
    }

    pub fn open_settings() -> io::Result<()> {
        current_application()?;
        let bytes = output(
            "/usr/bin/sw_vers",
            &[std::ffi::OsStr::new("-productVersion")],
        )?;
        let version = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
        crate::system::open_url_with_default_browser(settings_url(version)?)
    }

    pub fn reveal_application() -> io::Result<()> {
        crate::system::reveal_in_file_manager(&current_application()?)
    }
}

#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
mod platform {
    use std::io;
    pub fn open_settings() -> io::Result<()> {
        super::require_macos(crate::OperatingSystem::Other)
    }
    pub fn reveal_application() -> io::Result<()> {
        super::require_macos(crate::OperatingSystem::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_settings_links_and_explicit_platform_boundary() {
        assert!(
            settings_url("13.5")
                .unwrap()
                .contains("settings.PrivacySecurity.extension?Privacy_AllFiles")
        );
        assert!(
            settings_url("12.7")
                .unwrap()
                .contains("preference.security?Privacy_AllFiles")
        );
        assert!(settings_url("invalid").is_err());
        assert!(require_macos(OperatingSystem::Windows).is_err());
    }

    #[test]
    fn current_host_bundle_is_verified_and_cannot_be_arbitrarily_redirected() {
        let root = std::env::temp_dir().join(format!("cilai-fda-{}", std::process::id()));
        let app = root.join("Workbench.app");
        let host = app.join("Contents/Resources/sidecar/cilai-host-service");
        std::fs::create_dir_all(host.parent().unwrap()).unwrap();
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(&host, "host").unwrap();
        std::fs::write(app.join("Contents/Info.plist"), "manifest").unwrap();
        std::fs::write(app.join("Contents/MacOS/Workbench"), "main").unwrap();
        let canonical = app.canonicalize().unwrap();
        assert_eq!(bundle_candidate(&host).unwrap(), canonical);
        assert!(bundle_candidate(&app.join("Contents/MacOS/Workbench")).is_err());
        let manifest = serde_json::json!({"CFBundleIdentifier":"com.cilai.workbench","CFBundlePackageType":"APPL","CFBundleExecutable":"Workbench"});
        validate_manifest(&canonical, &manifest).unwrap();
        let mut wrong = manifest.clone();
        wrong["CFBundleIdentifier"] = serde_json::json!("other.app");
        assert!(validate_manifest(&canonical, &wrong).is_err());
        let mut escape = manifest;
        escape["CFBundleExecutable"] = serde_json::json!("../external");
        assert!(validate_manifest(&canonical, &escape).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
