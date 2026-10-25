#![doc = "系统目录、`IPC`、实例锁、通知和凭证等跨平台能力边界。"]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatingSystem {
    MacOs,
    Windows,
    Other,
}

impl OperatingSystem {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MacOs => "macos",
            Self::Windows => "windows",
            Self::Other => "other",
        }
    }
}

#[must_use]
pub fn current_operating_system() -> OperatingSystem {
    match std::env::consts::OS {
        "macos" => OperatingSystem::MacOs,
        "windows" => OperatingSystem::Windows,
        _ => OperatingSystem::Other,
    }
}

pub mod cloud_hydration;
pub mod credential_vault;
pub mod developer_tools;
pub mod executable_search;
pub mod filesystem;
pub mod instance;
pub mod macos_disk_access;
pub mod macos_runtime_isolation;
pub mod managed_component;
pub mod mcp_sandbox;
pub mod power_epoch;
pub mod powershell_policy;
pub mod process;
pub mod runtime_launch_gate;
pub mod security;
pub mod startup_permissions;
pub mod system;
pub mod system_proxy;
pub mod workspace_permissions;
pub mod writer_launch_gate;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod tests {
    use super::{OperatingSystem, current_operating_system};

    #[test]
    fn current_operating_system_is_a_forward_platform_value() {
        let expected = match std::env::consts::OS {
            "macos" => OperatingSystem::MacOs,
            "windows" => OperatingSystem::Windows,
            _ => OperatingSystem::Other,
        };
        assert_eq!(current_operating_system(), expected);
        assert_eq!(
            current_operating_system().as_str(),
            match std::env::consts::OS {
                "macos" => "macos",
                "windows" => "windows",
                _ => "other",
            }
        );
    }
}

pub mod backup_link;
