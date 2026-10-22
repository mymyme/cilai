//! 本地可执行文件发现：跨平台搜索根目录、候选文件名与可执行性判定。

use crate::{OperatingSystem, current_operating_system};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(any(windows, test))]
const DEFAULT_WINDOWS_PATHEXT: [&str; 3] = [".EXE", ".CMD", ".BAT"];

/// 词来桌面程序同目录随包交付的可执行组件。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundledExecutable {
    AppServer,
    CodexHarness,
    PersonalNode,
}

/// 从当前桌面进程路径解析指定的随包可执行文件路径。
///
/// # Errors
///
/// 无法读取当前进程路径，或该路径没有父目录时返回错误。
pub fn bundled_executable_path(executable: BundledExecutable) -> io::Result<PathBuf> {
    let host = std::env::current_exe()?;
    bundled_executable_path_next_to(&host, executable)
}

/// 在给定宿主可执行文件的同目录内，按当前平台的固定文件名解析随包组件。
///
/// # Errors
///
/// 宿主路径没有父目录时返回 [`io::ErrorKind::InvalidInput`]。
pub fn bundled_executable_path_next_to(
    host: &Path,
    executable: BundledExecutable,
) -> io::Result<PathBuf> {
    host.parent()
        .map(|directory| directory.join(bundled_file_name(executable, current_operating_system())))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "host executable has no parent directory",
            )
        })
}

/// 确认指定路径是当前平台可执行的常规文件。
///
/// # Errors
///
/// 文件不存在、不是常规文件，或当前平台不允许执行时返回错误。
pub fn validate_executable_file(path: &Path) -> io::Result<()> {
    let metadata = path.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "executable path is not a regular file",
        ));
    }
    if !platform::is_explicit_executable_file(path, &metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "file is not executable on this platform",
        ));
    }
    Ok(())
}

fn bundled_file_name(executable: BundledExecutable, os: OperatingSystem) -> &'static str {
    match (executable, os) {
        (BundledExecutable::PersonalNode, OperatingSystem::Windows) => "cilai-node.exe",
        (BundledExecutable::PersonalNode, OperatingSystem::MacOs | OperatingSystem::Other) => {
            "cilai-node"
        }
        (BundledExecutable::AppServer, OperatingSystem::Windows) => "cilai-app-server.exe",
        (BundledExecutable::CodexHarness, OperatingSystem::Windows) => {
            "codex-runtime/bin/codex-app-server.exe"
        }
        (BundledExecutable::AppServer, OperatingSystem::MacOs | OperatingSystem::Other) => {
            "cilai-app-server"
        }
        (BundledExecutable::CodexHarness, OperatingSystem::MacOs | OperatingSystem::Other) => {
            "codex-runtime/bin/codex-app-server"
        }
    }
}

/// 返回本机可执行文件搜索根目录（已去重，保持发现顺序）。
#[must_use]
pub fn search_roots() -> Vec<PathBuf> {
    let mut roots = platform::search_roots();
    let mut seen = HashSet::new();
    roots.retain(|path| seen.insert(path.clone()));
    roots
}

/// 在搜索根目录中查找匹配任一候选名称的全部可执行文件路径（保持首次发现顺序，
/// 不含重复项）。候选名称不带扩展名；Windows 端按 `PATHEXT` 逐一尝试扩展名，
/// macOS/Unix 端按原名直接匹配。
#[must_use]
pub fn find_executables(names: &[&str]) -> Vec<PathBuf> {
    let roots = search_roots();
    let mut matches = Vec::new();
    let mut seen = HashSet::new();
    for name in names {
        for root in &roots {
            for candidate in platform::candidate_file_names(name) {
                let path = root.join(&candidate);
                if is_search_candidate_file(&path) && seen.insert(path.clone()) {
                    matches.push(path);
                }
            }
        }
    }
    matches
}

/// 查找匹配任一候选名称的第一个可执行文件路径。
#[must_use]
pub fn find_executable(names: &[&str]) -> Option<PathBuf> {
    find_executables(names).into_iter().next()
}

/// 判断给定路径是否为可执行文件。
#[must_use]
pub fn is_executable_file(path: &Path) -> bool {
    validate_executable_file(path).is_ok()
}

fn is_search_candidate_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    metadata.is_file() && platform::is_search_candidate_file(path, &metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_name_has_no_matches() {
        assert!(find_executables(&["contexthub-definitely-not-a-real-executable"]).is_empty());
    }

    #[test]
    fn search_roots_are_deduplicated() {
        let roots = search_roots();
        let mut seen = HashSet::new();
        assert!(roots.iter().all(|path| seen.insert(path.clone())));
    }

    #[test]
    fn bundled_file_names_are_owned_by_the_platform_boundary() {
        assert_eq!(
            bundled_file_name(BundledExecutable::AppServer, OperatingSystem::MacOs),
            "cilai-app-server"
        );
        assert_eq!(
            bundled_file_name(BundledExecutable::AppServer, OperatingSystem::Windows),
            "cilai-app-server.exe"
        );
        assert_eq!(
            bundled_file_name(BundledExecutable::CodexHarness, OperatingSystem::MacOs),
            "codex-runtime/bin/codex-app-server"
        );
        assert_eq!(
            bundled_file_name(BundledExecutable::CodexHarness, OperatingSystem::Windows),
            "codex-runtime/bin/codex-app-server.exe"
        );
    }

    #[test]
    fn current_test_binary_is_an_executable_file() {
        validate_executable_file(&std::env::current_exe().unwrap()).unwrap();
    }

    #[test]
    fn windows_pathext_candidates_are_trimmed_and_deduplicated() {
        assert_eq!(
            windows_candidate_file_names("codex", Some(" .CMD ;.EXE;;.cmd ")),
            ["codex.CMD", "codex.EXE"]
        );
        assert_eq!(
            windows_candidate_file_names("codex", None),
            ["codex.EXE", "codex.CMD", "codex.BAT"]
        );
        assert_eq!(
            windows_candidate_file_names("codex", Some(".PS1;.EXE")),
            ["codex.EXE"]
        );
        assert_eq!(
            windows_candidate_file_names("codex", Some(".JS;.COM;.BAT;.VBS")),
            ["codex.COM", "codex.BAT"]
        );
    }

    #[test]
    fn windows_explicit_executables_do_not_depend_on_pathext() {
        for path in ["sidecar.exe", "npm.cmd", "script.bat", "legacy.com"] {
            assert!(windows_has_directly_executable_extension(Path::new(path)));
        }
        for path in ["script.ps1", "extensionless"] {
            assert!(!windows_has_directly_executable_extension(Path::new(path)));
        }
    }

    #[test]
    fn windows_known_roots_only_add_documented_install_directories() {
        let roots = windows_search_roots(
            vec![PathBuf::from("/windows/system32")],
            Some(Path::new("/users/test/appdata/roaming")),
            Some(Path::new("/users/test/appdata/local")),
            Some(Path::new("/users/test")),
        );
        assert_eq!(
            roots,
            [
                PathBuf::from("/windows/system32"),
                PathBuf::from("/users/test/appdata/roaming/npm"),
                PathBuf::from("/users/test/appdata/local/Programs/OpenAI/Codex/bin"),
                PathBuf::from("/users/test/.local/bin"),
            ]
        );
    }
}

#[cfg(any(windows, test))]
fn windows_candidate_file_names(name: &str, pathext: Option<&str>) -> Vec<String> {
    windows_pathext(pathext)
        .into_iter()
        .map(|extension| format!("{name}{extension}"))
        .collect()
}

#[cfg(any(windows, test))]
fn windows_pathext(pathext: Option<&str>) -> Vec<String> {
    let values: Vec<String> = pathext
        .filter(|value| !value.trim().is_empty())
        .map_or_else(
            || {
                DEFAULT_WINDOWS_PATHEXT
                    .iter()
                    .map(ToString::to_string)
                    .collect()
            },
            |value| {
                value
                    .split(';')
                    .map(str::trim)
                    .filter(|extension| !extension.is_empty())
                    .map(|extension| {
                        if extension.starts_with('.') {
                            extension.to_owned()
                        } else {
                            format!(".{extension}")
                        }
                    })
                    .collect()
            },
        );
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|extension| windows_is_directly_executable_extension(extension))
        .filter(|extension| seen.insert(extension.to_ascii_lowercase()))
        .collect()
}

#[cfg(any(windows, test))]
fn windows_has_directly_executable_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(windows_is_directly_executable_extension)
}

#[cfg(any(windows, test))]
fn windows_is_directly_executable_extension(extension: &str) -> bool {
    let extension = extension.strip_prefix('.').unwrap_or(extension);
    extension.eq_ignore_ascii_case("exe")
        || extension.eq_ignore_ascii_case("com")
        || extension.eq_ignore_ascii_case("cmd")
        || extension.eq_ignore_ascii_case("bat")
}

#[cfg(any(windows, test))]
fn windows_search_roots(
    mut path_roots: Vec<PathBuf>,
    appdata: Option<&Path>,
    local_appdata: Option<&Path>,
    user_profile: Option<&Path>,
) -> Vec<PathBuf> {
    if let Some(appdata) = appdata {
        path_roots.push(appdata.join("npm"));
    }
    if let Some(local_appdata) = local_appdata {
        path_roots.push(local_appdata.join("Programs/OpenAI/Codex/bin"));
    }
    if let Some(user_profile) = user_profile {
        path_roots.push(user_profile.join(".local/bin"));
    }
    path_roots
}

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::HashSet;
    use std::fs::{self, Metadata};
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    pub fn search_roots() -> Vec<PathBuf> {
        let path_roots = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .unwrap_or_default();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut roots = path_roots;
        if let Some(home) = home.as_ref() {
            roots.extend([
                home.join(".local/bin"),
                home.join(".opencode/bin"),
                home.join(".cursor/bin"),
                home.join(".npm-global/bin"),
                home.join(".bun/bin"),
            ]);
            let nvm_versions = home.join(".nvm/versions/node");
            if let Ok(entries) = fs::read_dir(nvm_versions) {
                let mut versioned_roots = entries
                    .flatten()
                    .map(|entry| entry.path().join("bin"))
                    .filter(|path| path.is_dir())
                    .collect::<Vec<_>>();
                versioned_roots.sort();
                versioned_roots.reverse();
                roots.extend(versioned_roots);
            }
        }
        roots.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ]);
        let mut seen = HashSet::new();
        roots.retain(|path| seen.insert(path.clone()));
        roots
    }

    pub fn candidate_file_names(name: &str) -> Vec<String> {
        vec![name.to_owned()]
    }

    pub fn is_explicit_executable_file(_path: &Path, metadata: &Metadata) -> bool {
        metadata.permissions().mode() & 0o111 != 0
    }

    pub fn is_search_candidate_file(_path: &Path, metadata: &Metadata) -> bool {
        metadata.permissions().mode() & 0o111 != 0
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::fs::Metadata;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    /// 非 macOS 的 Unix 平台不属于本产品的正式目标（见架构基线第 6 节），此处仅复用
    /// `PATH` 与标准系统目录，不追加发行版专属的搜索路径。
    pub fn search_roots() -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default();
        roots.push(PathBuf::from("/usr/local/bin"));
        roots.push(PathBuf::from("/usr/bin"));
        roots
    }

    pub fn candidate_file_names(name: &str) -> Vec<String> {
        vec![name.to_owned()]
    }

    pub fn is_explicit_executable_file(_path: &Path, metadata: &Metadata) -> bool {
        metadata.permissions().mode() & 0o111 != 0
    }

    pub fn is_search_candidate_file(_path: &Path, metadata: &Metadata) -> bool {
        metadata.permissions().mode() & 0o111 != 0
    }
}

#[cfg(windows)]
mod platform {
    use std::fs::Metadata;
    use std::path::{Path, PathBuf};

    pub fn search_roots() -> Vec<PathBuf> {
        let roots: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default();
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
        let local_appdata = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let user_profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
        super::windows_search_roots(
            roots,
            appdata.as_deref(),
            local_appdata.as_deref(),
            user_profile.as_deref(),
        )
    }

    pub fn candidate_file_names(name: &str) -> Vec<String> {
        let pathext = std::env::var("PATHEXT").ok();
        super::windows_candidate_file_names(name, pathext.as_deref())
    }

    pub fn is_explicit_executable_file(path: &Path, _metadata: &Metadata) -> bool {
        super::windows_has_directly_executable_extension(path)
    }

    pub fn is_search_candidate_file(path: &Path, _metadata: &Metadata) -> bool {
        super::windows_has_directly_executable_extension(path)
    }
}
