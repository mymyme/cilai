//! 本地 AI CLI 的平台安装与升级策略。
//!
//! 调用方负责展示业务状态和执行命令；本模块只决定当前平台允许使用哪种已登记的安装
//! 来源，避免在业务层判断 Homebrew、npm 或平台路径。

use crate::{OperatingSystem, current_operating_system};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaintenanceAction {
    Install,
    Upgrade,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaintenanceProgram {
    SearchPath(&'static [&'static str]),
    InstalledExecutable(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaintenancePlan {
    pub program: MaintenanceProgram,
    pub arguments: Vec<String>,
    pub method: &'static str,
}

/// 返回当前平台已批准的安装或升级策略；没有策略时返回 `None`，调用方应明确显示不支持。
#[must_use]
pub fn maintenance_plan(
    provider: &str,
    action: MaintenanceAction,
    executable: Option<&Path>,
) -> Option<MaintenancePlan> {
    maintenance_plan_for_os(current_operating_system(), provider, action, executable)
}

/// 返回当前安装路径对应的来源名称；未安装时返回当前平台的自动安装来源。
#[must_use]
pub fn installation_method(provider: &str, executable: Option<&Path>) -> Option<&'static str> {
    installation_method_for_os(current_operating_system(), provider, executable)
}

fn npm_plan(package: &str) -> MaintenancePlan {
    MaintenancePlan {
        program: MaintenanceProgram::SearchPath(&["npm"]),
        arguments: vec!["install".to_owned(), "-g".to_owned(), package.to_owned()],
        method: "npm",
    }
}

fn maintenance_plan_for_os(
    os: OperatingSystem,
    provider: &str,
    action: MaintenanceAction,
    executable: Option<&Path>,
) -> Option<MaintenancePlan> {
    let windows_npm_root = windows_default_npm_root();
    maintenance_plan_for_os_with_windows_npm_root(
        os,
        provider,
        action,
        executable,
        windows_npm_root.as_deref(),
    )
}

fn maintenance_plan_for_os_with_windows_npm_root(
    os: OperatingSystem,
    provider: &str,
    action: MaintenanceAction,
    executable: Option<&Path>,
    windows_npm_root: Option<&Path>,
) -> Option<MaintenancePlan> {
    match os {
        OperatingSystem::MacOs => macos_plan(provider, action, executable),
        OperatingSystem::Windows => windows_plan(provider, action, executable, windows_npm_root),
        OperatingSystem::Other => None,
    }
}

fn macos_plan(
    provider: &str,
    action: MaintenanceAction,
    executable: Option<&Path>,
) -> Option<MaintenancePlan> {
    match (provider, action) {
        ("codex", MaintenanceAction::Install) => Some(MaintenancePlan {
            program: MaintenanceProgram::SearchPath(&["brew"]),
            arguments: vec![
                "install".to_owned(),
                "--cask".to_owned(),
                "codex".to_owned(),
            ],
            method: "Homebrew",
        }),
        ("codex", MaintenanceAction::Upgrade) if executable.is_some_and(is_homebrew_codex) => {
            Some(MaintenancePlan {
                program: MaintenanceProgram::SearchPath(&["brew"]),
                arguments: vec![
                    "upgrade".to_owned(),
                    "--cask".to_owned(),
                    "codex".to_owned(),
                ],
                method: "Homebrew",
            })
        }
        ("codex", MaintenanceAction::Upgrade) => Some(npm_plan("@openai/codex@latest")),
        ("claude", MaintenanceAction::Install) => {
            Some(npm_plan("@anthropic-ai/claude-code@latest"))
        }
        ("opencode", MaintenanceAction::Install) => Some(npm_plan("opencode-ai@latest")),
        ("gemini", MaintenanceAction::Install) => Some(npm_plan("@google/gemini-cli@latest")),
        ("claude" | "gemini", MaintenanceAction::Upgrade) => {
            installed_plan(executable, &["update"])
        }
        ("opencode", MaintenanceAction::Upgrade) => {
            installed_plan(executable, &["upgrade", "--method", "curl", "--pure"])
        }
        _ => None,
    }
}

fn windows_plan(
    provider: &str,
    action: MaintenanceAction,
    executable: Option<&Path>,
    npm_root: Option<&Path>,
) -> Option<MaintenancePlan> {
    let package = match provider {
        "codex" => "@openai/codex@latest",
        "claude" => "@anthropic-ai/claude-code@latest",
        "opencode" => "opencode-ai@latest",
        "gemini" => "@google/gemini-cli@latest",
        _ => return None,
    };
    match action {
        MaintenanceAction::Install => Some(npm_plan(package)),
        MaintenanceAction::Upgrade
            if executable.is_some_and(|path| {
                npm_root.is_some_and(|root| is_windows_npm_executable(provider, path, root))
            }) =>
        {
            Some(npm_plan(package))
        }
        MaintenanceAction::Upgrade => None,
    }
}

fn windows_default_npm_root() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("npm"))
}

fn is_windows_npm_executable(provider: &str, executable: &Path, npm_root: &Path) -> bool {
    let expected_file_name = match provider {
        "codex" => "codex.cmd",
        "claude" => "claude.cmd",
        "opencode" => "opencode.cmd",
        "gemini" => "gemini.cmd",
        _ => return false,
    };
    let executable = normalized_path(executable);
    let npm_root = normalized_path(npm_root);
    executable
        .rsplit_once('/')
        .is_some_and(|(parent, file_name)| {
            parent
                .trim_end_matches('/')
                .eq_ignore_ascii_case(npm_root.trim_end_matches('/'))
                && file_name.eq_ignore_ascii_case(expected_file_name)
        })
}

fn windows_installation_method(
    provider: &str,
    executable: Option<&Path>,
    npm_root: Option<&Path>,
) -> Option<&'static str> {
    match provider {
        "codex" | "claude" | "opencode" | "gemini"
            if executable.is_none_or(|path| {
                npm_root.is_some_and(|root| is_windows_npm_executable(provider, path, root))
            }) =>
        {
            Some("npm")
        }
        _ => None,
    }
}

fn installed_plan(executable: Option<&Path>, arguments: &[&str]) -> Option<MaintenancePlan> {
    Some(MaintenancePlan {
        program: MaintenanceProgram::InstalledExecutable(executable?.to_path_buf()),
        arguments: arguments.iter().map(|value| (*value).to_owned()).collect(),
        method: "官方安装器",
    })
}

fn installation_method_for_os(
    os: OperatingSystem,
    provider: &str,
    executable: Option<&Path>,
) -> Option<&'static str> {
    let windows_npm_root = windows_default_npm_root();
    installation_method_for_os_with_windows_npm_root(
        os,
        provider,
        executable,
        windows_npm_root.as_deref(),
    )
}

fn installation_method_for_os_with_windows_npm_root(
    os: OperatingSystem,
    provider: &str,
    executable: Option<&Path>,
    windows_npm_root: Option<&Path>,
) -> Option<&'static str> {
    match os {
        OperatingSystem::MacOs => match provider {
            "codex" if executable.is_none_or(is_homebrew_codex) => Some("Homebrew"),
            "claude"
                if executable.is_some_and(|path| {
                    normalized_path(path).contains("/.local/share/claude/")
                }) =>
            {
                Some("Claude 官方安装器")
            }
            "opencode"
                if executable
                    .is_some_and(|path| normalized_path(path).contains("/.opencode/bin/")) =>
            {
                Some("OpenCode 官方安装器")
            }
            "codex" | "claude" | "opencode" | "gemini" => Some("npm"),
            _ => None,
        },
        OperatingSystem::Windows => {
            windows_installation_method(provider, executable, windows_npm_root)
        }
        OperatingSystem::Other => None,
    }
}

fn is_homebrew_codex(path: &Path) -> bool {
    normalized_path(path).contains("/Caskroom/codex/")
}

fn normalized_path(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_uses_npm_without_inheriting_macos_commands() {
        let npm_root = Path::new("/users/test/appdata/roaming/npm");
        let plan = maintenance_plan_for_os_with_windows_npm_root(
            OperatingSystem::Windows,
            "codex",
            MaintenanceAction::Install,
            None,
            Some(npm_root),
        )
        .unwrap();
        assert_eq!(plan.program, MaintenanceProgram::SearchPath(&["npm"]));
        assert_eq!(plan.arguments, ["install", "-g", "@openai/codex@latest"]);
        assert_eq!(plan.method, "npm");

        let opencode = maintenance_plan_for_os_with_windows_npm_root(
            OperatingSystem::Windows,
            "opencode",
            MaintenanceAction::Upgrade,
            Some(Path::new("/users/test/appdata/roaming/npm/opencode.cmd")),
            Some(npm_root),
        )
        .unwrap();
        assert_eq!(opencode.program, MaintenanceProgram::SearchPath(&["npm"]));
        assert!(!opencode.arguments.iter().any(|value| value == "curl"));
    }

    #[test]
    fn windows_upgrade_requires_a_proven_npm_global_executable() {
        let npm_root = Path::new("/users/test/appdata/roaming/npm");
        let independent = Path::new("/tools/claude.exe");
        assert_eq!(
            maintenance_plan_for_os_with_windows_npm_root(
                OperatingSystem::Windows,
                "claude",
                MaintenanceAction::Upgrade,
                Some(independent),
                Some(npm_root),
            ),
            None
        );
        assert_eq!(
            installation_method_for_os_with_windows_npm_root(
                OperatingSystem::Windows,
                "claude",
                Some(independent),
                Some(npm_root),
            ),
            None
        );
        assert_eq!(
            installation_method_for_os_with_windows_npm_root(
                OperatingSystem::Windows,
                "claude",
                Some(Path::new("/users/test/appdata/roaming/npm/claude.cmd",)),
                Some(npm_root),
            ),
            Some("npm")
        );
        assert_eq!(
            maintenance_plan_for_os_with_windows_npm_root(
                OperatingSystem::Windows,
                "claude",
                MaintenanceAction::Upgrade,
                Some(Path::new("/users/test/appdata/roaming/npm/other.cmd")),
                Some(npm_root),
            ),
            None
        );
        assert_eq!(
            maintenance_plan_for_os_with_windows_npm_root(
                OperatingSystem::Windows,
                "claude",
                MaintenanceAction::Upgrade,
                Some(Path::new("/users/test/appdata/roaming/npm/claude.exe")),
                Some(npm_root),
            ),
            None
        );
    }

    #[test]
    fn macos_preserves_homebrew_and_native_update_sources() {
        let codex = maintenance_plan_for_os(
            OperatingSystem::MacOs,
            "codex",
            MaintenanceAction::Upgrade,
            Some(Path::new("/opt/homebrew/Caskroom/codex/1/bin/codex")),
        )
        .unwrap();
        assert_eq!(codex.program, MaintenanceProgram::SearchPath(&["brew"]));
        assert_eq!(codex.method, "Homebrew");

        let claude = maintenance_plan_for_os(
            OperatingSystem::MacOs,
            "claude",
            MaintenanceAction::Upgrade,
            Some(Path::new("/Users/test/.local/share/claude/claude")),
        )
        .unwrap();
        assert_eq!(
            claude.program,
            MaintenanceProgram::InstalledExecutable(PathBuf::from(
                "/Users/test/.local/share/claude/claude"
            ))
        );
        assert_eq!(claude.arguments, ["update"]);
    }

    #[cfg(unix)]
    #[test]
    fn macos_homebrew_codex_is_recognized_through_its_path_symlink() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "contexthub-developer-tools-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let target = root.join("Caskroom/codex/1/bin/codex");
        let linked = root.join("bin/codex");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::create_dir_all(linked.parent().unwrap()).unwrap();
        std::fs::write(&target, b"codex").unwrap();
        symlink(&target, &linked).unwrap();

        let plan = maintenance_plan_for_os(
            OperatingSystem::MacOs,
            "codex",
            MaintenanceAction::Upgrade,
            Some(&linked),
        )
        .unwrap();
        assert_eq!(plan.program, MaintenanceProgram::SearchPath(&["brew"]));
        assert_eq!(plan.method, "Homebrew");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unsupported_platform_has_no_implicit_install_strategy() {
        assert_eq!(
            maintenance_plan_for_os(
                OperatingSystem::Other,
                "codex",
                MaintenanceAction::Install,
                None
            ),
            None
        );
        assert_eq!(
            installation_method_for_os(OperatingSystem::Other, "codex", None),
            None
        );
    }
}
