//! Explicit recovery of known startup data under the original user's `LocalAppData`.
//! No project paths, database reads, shell commands, or sandbox ACL resets.
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

pub const HELPER_ARGUMENT: &str = "--repair-startup-permissions";

#[cfg(any(windows, test))]
mod layout;

// Stable application exit codes cross the elevated process boundary without
// exporting paths, SIDs, descriptors, credentials or arbitrary helper output.
#[derive(Clone, Copy)]
#[repr(i32)]
enum Conflict {
    InvalidRequest = 10011,
    Expired = 10012,
    LocalDataChanged = 10013,
    CallerChanged = 10014,
    DirectoryChanged = 10015,
    BackupChanged = 10016,
    PermissionsChanged = 10017,
    ParentChanged = 10018,
}

/// Safe, stable failure classification for the desktop and exported error log.
#[must_use]
pub fn conflict_reason(code: i32) -> Option<(&'static str, &'static str)> {
    let (kind, name, description) = match code {
        10011 => (
            Conflict::InvalidRequest,
            "startup_repair_invalid_request",
            "修复请求格式或目标范围无效，请重新检查启动权限。",
        ),
        10012 => (
            Conflict::Expired,
            "startup_repair_expired",
            "修复确认已过期，请重新检查启动权限。",
        ),
        10013 => (
            Conflict::LocalDataChanged,
            "startup_repair_local_data_changed",
            "修复助手与工作台识别的 Windows 数据目录不一致，请导出错误日志反馈。",
        ),
        10014 => (
            Conflict::CallerChanged,
            "startup_repair_caller_changed",
            "修复请求对应的 Windows 账户或工作台进程已变化，请重新检查启动权限。",
        ),
        10015 => (
            Conflict::DirectoryChanged,
            "startup_repair_directory_changed",
            "确认后启动数据目录已被创建、删除或替换，请重新检查启动权限。",
        ),
        10016 => (
            Conflict::BackupChanged,
            "startup_repair_backup_changed",
            "权限备份文件的身份或归属校验未通过，已停止修复。",
        ),
        10017 => (
            Conflict::PermissionsChanged,
            "startup_repair_permissions_changed",
            "修复期间目录权限发生变化，已停止修复并还原本次已改权限。请导出错误日志反馈。",
        ),
        10018 => (
            Conflict::ParentChanged,
            "startup_repair_parent_changed",
            "修复助手无法验证原工作台进程的身份，请重新检查启动权限。",
        ),
        _ => return None,
    };
    debug_assert_eq!(kind as i32, code);
    Some((name, description))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepairPlan {
    local_data: String,
    environment: String,
    profile_id: String,
    user_sid: String,
    parent_pid: u32,
    parent_fingerprint: String,
    expires_at: u64,
    anchors: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct Inspection {
    pub directory: String,
    pub user_sid: String,
    pub process_elevated: bool,
    pub plan: RepairPlan,
}

/// Fixed startup metadata only; no paths, SIDs, contents or security descriptors.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    process_elevated: bool,
    errors: Vec<DiagnosticError>,
    repair_attempts: Vec<RepairAttemptDiagnostic>,
}

/// An allowlisted summary only. Raw paths/SIDs/descriptors stay in the private
/// local repair journal and never cross the desktop diagnostics boundary.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepairAttemptDiagnostic {
    operation_id: String,
    at_epoch_seconds: u64,
    component_sha256: String,
    phase: RepairPhase,
    target_index: Option<usize>,
    target_scope: Option<RepairScope>,
    previous_applied_index: Option<usize>,
    changed_parts: Vec<SecurityPart>,
    dacl_flags_before: Vec<DaclFlag>,
    dacl_flags_after: Vec<DaclFlag>,
    os_error: Option<i32>,
    rollback_verified: Option<bool>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum RepairPhase {
    Prepared,
    Compare,
    Write,
    Verify,
    Complete,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum RepairScope {
    ApplicationData,
    Environment,
    Profiles,
    Profile,
    StartupDirectory,
    StartupFile,
    RuntimeDirectory,
    RuntimeDatabase,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum SecurityPart {
    Owner,
    Group,
    DaclFlags,
    DaclEntries,
    Representation,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum DaclFlag {
    Protected,
    AutoInherited,
    AutoInheritRequested,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticError {
    scope: &'static str,
    code: &'static str,
    owner_category: Option<&'static str>,
    os_error: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairResult {
    pub backup_path: String,
}

/// Prepare a read-only confirmation bound to the OS user and managed directory.
/// # Errors
/// Rejects unsupported platforms, links and unmanaged paths. An already
/// elevated caller remains bound to its `TokenUser` and known local data folder.
pub fn inspect(environment_root: &Path, profile_id: &str) -> io::Result<Inspection> {
    #[cfg(windows)]
    return windows::inspect(environment_root, profile_id);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let _ = (environment_root, profile_id);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Diagnose startup permissions even before a tenant profile can be activated.
/// # Errors
/// Unsupported platforms or failure to query the current Windows token.
pub fn diagnose(environment_root: &Path, profile_id: &str) -> io::Result<Diagnostics> {
    #[cfg(windows)]
    return windows::diagnose(environment_root, profile_id);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let _ = (environment_root, profile_id);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Apply only after desktop confirmation, requesting UAC if ordinary rights fail.
/// # Errors
/// Cancellation, changed identity, foreign ownership, busy objects and failed
/// verification are returned, never treated as successful recovery.
pub fn repair(plan: &RepairPlan, operation_id: &str) -> io::Result<RepairResult> {
    #[cfg(windows)]
    return windows::repair(plan, operation_id);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let _ = (plan, operation_id);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Dispatch before any authentication, database or app-server initialization.
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
mod windows;
