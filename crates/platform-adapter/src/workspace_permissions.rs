//! User-initiated, single-directory Windows permission repair. The elevated
//! helper never starts the application, model, shell, or sandbox setup.
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

pub const HELPER_ARGUMENT: &str = "--repair-workspace-permissions";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepairPlan {
    directory: String,
    user_sid: String,
    identity: String,
    security_digest: String,
    parent_pid: u32,
    parent_fingerprint: String,
    expires_at: u64,
}

#[derive(Debug)]
pub struct Inspection {
    pub process_elevated: bool,
    pub directory: String,
    pub user_sid: String,
    pub can_update_permissions: bool,
    pub security_backup: String,
    pub plan: RepairPlan,
}

/// Read-only; never starts UAC or writes an ACL.
/// # Errors
/// Returns an explicit error for unsupported platforms, unsafe paths, or
/// inaccessible metadata. Elevated callers are checked without admin grants.
pub fn inspect(directory: &Path) -> io::Result<Inspection> {
    #[cfg(windows)]
    return windows::inspect(directory);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let _ = directory;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "workspace_permission_unsupported",
        ))
    }
}

/// Called only after an explicit desktop confirmation. Revalidates the plan,
/// invokes a dedicated elevated mode of the current executable, then verifies
/// `WRITE_DAC` for the original account without administrative grants.
/// # Errors
/// Cancellation, stale plans, ACL conflicts, and failed verification stay errors.
pub fn repair(plan: &RepairPlan) -> io::Result<Inspection> {
    #[cfg(windows)]
    return windows::repair(plan);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let _ = plan;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "workspace_permission_unsupported",
        ))
    }
}

/// Recognize the narrowly scoped helper mode before any app/profile startup.
/// The parent receives only an exit code; elevated code never writes a user-
/// supplied output file. The actual ACL backup is saved by the regular host.
#[must_use]
pub fn helper_exit_code(arguments: &[String]) -> Option<i32> {
    if arguments.first().map(String::as_str) != Some(HELPER_ARGUMENT) {
        return None;
    }
    #[cfg(windows)]
    return Some(
        windows::helper(arguments).map_or_else(|error| error.raw_os_error().unwrap_or(1), |()| 0),
    );
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Some(1)
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub(crate) mod windows;
