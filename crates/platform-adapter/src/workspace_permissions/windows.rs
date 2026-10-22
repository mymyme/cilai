use super::{HELPER_ARGUMENT, Inspection, RepairPlan};
use sha2::{Digest as _, Sha256};
use std::ffi::{OsStr, c_void};
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;
use std::os::windows::process::CommandExt as _;
use std::path::{Component, Path, PathBuf, Prefix};
use std::process::{Command, Stdio};
use std::ptr;
use std::time::{SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetSecurityInfo, SE_FILE_OBJECT, SetEntriesInAclW,
    SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACL, CreateRestrictedToken, DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE,
    GROUP_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
    ImpersonateLoggedOnUser, LUA_TOKEN, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    RevertToSelf, TOKEN_DUPLICATE, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation,
    TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, READ_CONTROL, WRITE_DAC,
};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::{
    SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const SECURITY_PARTS: u32 =
    OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
const RETRY: i32 = 1237;

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(crate) fn failed(message: &str) -> io::Error {
    io::Error::other(message.to_owned())
}
pub(crate) fn changed() -> io::Error {
    io::Error::from_raw_os_error(RETRY)
}
pub(crate) fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
pub(crate) fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
pub(crate) fn win_result(code: u32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(
            i32::try_from(code).unwrap_or(1),
        ))
    }
}

pub(crate) struct Handle(pub(crate) HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: every Handle owns one successful OpenProcess/Token/ShellExecute handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub(crate) struct LocalMemory(pub(crate) *mut c_void);
struct ComApartment(bool);
impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances this thread's successful CoInitializeEx call.
            unsafe {
                CoUninitialize();
            }
        }
    }
}
impl Drop for LocalMemory {
    fn drop(&mut self) {
        // SAFETY: each pointer comes from a documented LocalAlloc-returning API.
        unsafe {
            LocalFree(self.0);
        }
    }
}
pub(crate) fn local_string(pointer: *mut u16) -> String {
    // SAFETY: callers pass successful Win32 conversion outputs, always NUL terminated.
    unsafe {
        let mut len = 0;
        while *pointer.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(pointer, len))
    }
}

fn token(process: HANDLE) -> io::Result<Handle> {
    let mut result = ptr::null_mut();
    // SAFETY: process is live (or the current-process pseudo handle), output is writable.
    check(unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut result) })?;
    Ok(Handle(result))
}
pub(crate) fn elevated(token: &Handle) -> io::Result<bool> {
    let mut value = TOKEN_ELEVATION::default();
    let mut length = 0;
    // SAFETY: value is an aligned TOKEN_ELEVATION of the advertised size.
    check(unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&raw mut value).cast(),
            u32::try_from(size_of::<TOKEN_ELEVATION>()).unwrap(),
            &raw mut length,
        )
    })?;
    Ok(value.TokenIsElevated != 0)
}
pub(crate) fn user_sid(token: &Handle) -> io::Result<String> {
    let mut length = 0;
    // SAFETY: first call only queries the required buffer length.
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &raw mut length);
    }
    if length < u32::try_from(size_of::<TOKEN_USER>()).unwrap() {
        return Err(io::Error::last_os_error());
    }
    let mut storage = vec![0_usize; (length as usize).div_ceil(size_of::<usize>())];
    // SAFETY: storage is aligned for TOKEN_USER and has at least length writable bytes.
    check(unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            length,
            &raw mut length,
        )
    })?;
    // SAFETY: successful TokenUser query filled the aligned buffer; SID borrows it.
    let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
    let mut text = ptr::null_mut();
    // SAFETY: the SID remains live in storage; output is a fresh LocalAlloc allocation.
    check(unsafe { ConvertSidToStringSidW(user.User.Sid, &raw mut text) })?;
    let _allocation = LocalMemory(text.cast());
    Ok(local_string(text))
}
pub(crate) fn current_token() -> io::Result<Handle> {
    // SAFETY: obtaining a pseudo handle has no preconditions.
    token(unsafe { GetCurrentProcess() })
}

struct Revert;
impl Drop for Revert {
    fn drop(&mut self) {
        // SAFETY: balances the successful impersonation on this synchronous thread.
        if unsafe { RevertToSelf() } == 0 {
            std::process::abort();
        }
    }
}

/// File-access checks only. Never spawn a process inside this closure: child
/// processes would inherit the process token, not this thread's token.
pub(crate) fn with_user_access_checks<T>(
    process: HANDLE,
    inspect: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let mut raw = ptr::null_mut();
    // SAFETY: caller supplies a live process or the current-process pseudo handle.
    check(unsafe { OpenProcessToken(process, TOKEN_QUERY | TOKEN_DUPLICATE, &raw mut raw) })?;
    let original = Handle(raw);
    let restricted = if elevated(&original)? {
        let mut raw = ptr::null_mut();
        // SAFETY: a valid primary token; no pointer arrays are supplied. LUA removes
        // administrative group grants; privileges are disabled, not increased.
        check(unsafe {
            CreateRestrictedToken(
                original.0,
                DISABLE_MAX_PRIVILEGE | LUA_TOKEN,
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                &raw mut raw,
            )
        })?;
        Some(Handle(raw))
    } else {
        None
    };
    let checked = restricted.as_ref().unwrap_or(&original);
    if user_sid(checked)? != user_sid(&original)? {
        return Err(changed());
    }
    // SAFETY: owned primary token, used synchronously and reverted before return.
    check(unsafe { ImpersonateLoggedOnUser(checked.0) })?;
    let guard = Revert;
    let result = inspect();
    drop(guard);
    result
}

pub(crate) fn with_current_user_access_checks<T>(
    inspect: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    // SAFETY: obtaining a pseudo handle has no preconditions.
    with_user_access_checks(unsafe { GetCurrentProcess() }, inspect)
}

pub(crate) fn validate_path(path: &Path) -> io::Result<()> {
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        || !matches!(components.next(), Some(Component::RootDir))
    {
        return Err(failed("workspace_permission_local_directory_required"));
    }
    let mut count = 0;
    for component in components {
        let Component::Normal(name) = component else {
            return Err(failed("workspace_permission_path_invalid"));
        };
        let name = name.to_string_lossy();
        if name.ends_with([' ', '.']) || name.contains([':', '\0']) {
            return Err(failed("workspace_permission_path_invalid"));
        }
        count += 1;
    }
    if count == 0 {
        return Err(failed("workspace_permission_drive_root_forbidden"));
    }
    Ok(())
}

fn open_directory(path: &Path, access: u32) -> io::Result<File> {
    // No delete sharing: the target and each ancestor stay pinned while inspected/repaired.
    let file = OpenOptions::new()
        .access_mode(access)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let info = information(&file)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
    {
        return Err(failed("workspace_permission_reparse_point_forbidden"));
    }
    Ok(file)
}
pub(crate) fn information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file owns a live handle, info is correctly sized/aligned writable storage.
    check(unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut info) })?;
    Ok(info)
}
pub(crate) fn identity(file: &File) -> io::Result<String> {
    let info = information(file)?;
    Ok(format!(
        "{}:{}:{}",
        info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
    ))
}
fn pinned_chain(path: &Path, target_access: u32) -> io::Result<Vec<File>> {
    validate_path(path)?;
    let ancestors = path.ancestors().collect::<Vec<_>>();
    let mut handles = Vec::with_capacity(ancestors.len());
    for ancestor in ancestors.into_iter().rev() {
        handles.push(open_directory(
            ancestor,
            if ancestor == path {
                target_access
            } else {
                FILE_READ_ATTRIBUTES
            },
        )?);
    }
    Ok(handles)
}
pub(crate) fn snapshot(file: &File) -> io::Result<String> {
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: file is live, only descriptor output is requested; Win32 allocates it.
    win_result(unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            SECURITY_PARTS,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut descriptor,
        )
    })?;
    let _descriptor = LocalMemory(descriptor);
    let mut text = ptr::null_mut();
    // SAFETY: descriptor remains allocated until the end of this function.
    check(unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor,
            1,
            SECURITY_PARTS,
            &raw mut text,
            ptr::null_mut(),
        )
    })?;
    let _text = LocalMemory(text.cast());
    Ok(local_string(text))
}
fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn can_update(path: &Path) -> io::Result<bool> {
    with_current_user_access_checks(|| match open_directory(path, WRITE_DAC) {
        Ok(_) => Ok(true),
        Err(error) if error.raw_os_error() == Some(5) => Ok(false),
        Err(error) => Err(error),
    })
}

pub(super) fn inspect(path: &Path) -> io::Result<Inspection> {
    let current = current_token()?;
    let handles = pinned_chain(path, READ_CONTROL | FILE_READ_ATTRIBUTES)?;
    let file = handles.last().ok_or_else(changed)?;
    let directory = path
        .to_str()
        .ok_or_else(|| failed("workspace_permission_path_invalid"))?
        .to_owned();
    let user_sid = user_sid(&current)?;
    let security_backup = snapshot(file)?;
    let parent_pid = std::process::id();
    Ok(Inspection {
        process_elevated: elevated(&current)?,
        can_update_permissions: can_update(path)?,
        plan: RepairPlan {
            directory: directory.clone(),
            user_sid: user_sid.clone(),
            identity: identity(file)?,
            security_digest: digest(&security_backup),
            parent_pid,
            parent_fingerprint: crate::instance::process_start_fingerprint(parent_pid)?
                .ok_or_else(changed)?,
            expires_at: now() + 300,
        },
        directory,
        user_sid,
        security_backup,
    })
}

fn validate_snapshot(plan: &RepairPlan, file: &File) -> io::Result<String> {
    if now() > plan.expires_at || plan.expires_at > now() + 300 || identity(file)? != plan.identity
    {
        return Err(changed());
    }
    let before = snapshot(file)?;
    if digest(&before) != plan.security_digest {
        return Err(changed());
    }
    Ok(before)
}
fn payload(plan: &RepairPlan) -> io::Result<String> {
    let bytes = serde_json::to_vec(plan).map_err(io::Error::other)?;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").expect("writing to a String cannot fail");
    }
    if text.len() > 24_000 {
        return Err(failed("workspace_permission_path_too_long"));
    }
    Ok(text)
}
fn decode(value: &str) -> io::Result<RepairPlan> {
    if value.len() > 24_000
        || !value.len().is_multiple_of(2)
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(changed());
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(io::Error::other))
        .collect::<io::Result<Vec<_>>>()?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub(super) fn repair(plan: &RepairPlan) -> io::Result<Inspection> {
    let current = current_token()?;
    if user_sid(&current)? != plan.user_sid || plan.parent_pid != std::process::id() {
        return Err(changed());
    }
    let path = Path::new(&plan.directory);
    let handles = pinned_chain(path, READ_CONTROL | FILE_READ_ATTRIBUTES)?;
    let file = handles.last().ok_or_else(changed)?;
    validate_snapshot(plan, file)?;
    if can_update(path)? {
        return inspect(path);
    }
    launch_helper(HELPER_ARGUMENT, &payload(plan)?)?;
    let result = inspect(path)?;
    if result.plan.identity != plan.identity || !result.can_update_permissions {
        return Err(failed("workspace_permission_verification_failed"));
    }
    Ok(result)
}

pub(crate) fn launch_helper(argument: &str, payload: &str) -> io::Result<()> {
    if elevated(&current_token()?)? {
        // Like Codex setup, keep an already-elevated launch in the same Windows
        // account/session. Privileges are still enabled only in the child.
        let child = spawn_same_identity_helper(argument, payload)?;
        return wait_helper(child.as_raw_handle());
    }
    let executable = wide(std::env::current_exe()?.as_os_str());
    let arguments = wide(OsStr::new(&format!("{argument} {payload}")));
    let verb = wide(OsStr::new("runas"));
    let directory = wide(
        std::env::current_exe()?
            .parent()
            .ok_or_else(changed)?
            .as_os_str(),
    );
    let mut launch = SHELLEXECUTEINFOW {
        cbSize: u32::try_from(size_of::<SHELLEXECUTEINFOW>()).unwrap(),
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: verb.as_ptr(),
        lpFile: executable.as_ptr(),
        lpParameters: arguments.as_ptr(),
        lpDirectory: directory.as_ptr(),
        nShow: SW_HIDE,
        ..Default::default()
    };
    // SAFETY: this thread performs the paired initialization/uninitialization;
    // an existing apartment (RPC_E_CHANGED_MODE) is retained without uninitializing it.
    let com_result = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    if com_result < 0 && com_result != -2_147_417_850 {
        return Err(failed("workspace_permission_helper_unavailable"));
    }
    let _apartment = ComApartment(com_result >= 0);
    // SAFETY: all UTF-16 buffers and the launch structure outlive this synchronous call.
    check(unsafe { ShellExecuteExW(&raw mut launch) })?;
    if launch.hProcess.is_null() || launch.hProcess == INVALID_HANDLE_VALUE {
        return Err(failed("workspace_permission_helper_unavailable"));
    }
    let process = Handle(launch.hProcess);
    wait_helper(process.0)
}

fn spawn_same_identity_helper(argument: &str, payload: &str) -> io::Result<std::process::Child> {
    let executable = std::env::current_exe()?;
    Command::new(&executable)
        .args([argument, payload])
        .current_dir(executable.parent().ok_or_else(changed)?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .spawn()
}

fn wait_helper(process: HANDLE) -> io::Result<()> {
    // SAFETY: the returned process handle remains owned until after wait/query.
    if unsafe { WaitForSingleObject(process, 120_000) } != WAIT_OBJECT_0 {
        return Err(failed("workspace_permission_result_unknown"));
    }
    let mut code = 1;
    // SAFETY: process handle and writable DWORD output are valid.
    check(unsafe { GetExitCodeProcess(process, &raw mut code) })?;
    win_result(code)
}

fn validate_parent(plan: &RepairPlan) -> io::Result<Handle> {
    verified_parent(plan.parent_pid, &plan.parent_fingerprint, &plan.user_sid)
}

pub(crate) fn verified_parent(pid: u32, fingerprint: &str, sid: &str) -> io::Result<Handle> {
    verified_parent_context(pid, fingerprint, sid)
}

// All repair helpers bind to the original account, including full-token logins.
pub(crate) fn verified_startup_parent(
    pid: u32,
    fingerprint: &str,
    sid: &str,
) -> io::Result<Handle> {
    verified_parent_context(pid, fingerprint, sid)
}

fn verified_parent_context(pid: u32, fingerprint: &str, sid: &str) -> io::Result<Handle> {
    // SAFETY: pid is a scalar; successful handle is transferred immediately to RAII.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = Handle(raw);
    if crate::instance::process_start_fingerprint(pid)?.as_deref() != Some(fingerprint) {
        return Err(changed());
    }
    let parent_token = token(process.0)?;
    if user_sid(&parent_token)? != sid {
        return Err(changed());
    }
    let mut name = vec![0_u16; 32768];
    let mut len = u32::try_from(name.len()).unwrap();
    // SAFETY: buffer has len writable UTF-16 elements and process is queryable.
    check(unsafe { QueryFullProcessImageNameW(process.0, 0, name.as_mut_ptr(), &raw mut len) })?;
    let executable = PathBuf::from(String::from_utf16_lossy(&name[..len as usize]));
    if executable.canonicalize()? != std::env::current_exe()?.canonicalize()? {
        return Err(changed());
    }
    Ok(process)
}

pub(super) fn helper(arguments: &[String]) -> io::Result<()> {
    if arguments.len() != 2 || !elevated(&current_token()?)? {
        return Err(changed());
    }
    let plan = decode(&arguments[1])?;
    let _parent = validate_parent(&plan)?;
    // MAXIMUM_ALLOWED is intentional: SetSecurityInfo documents that this handle
    // mode does NOT propagate existing inheritable ACEs to children. The new ACE
    // has NO inheritance. Do not replace with WRITE_DAC or a named-path setter.
    let handles = pinned_chain(Path::new(&plan.directory), MAXIMUM_ALLOWED)?;
    let file = handles.last().ok_or_else(changed)?;
    let before = validate_snapshot(&plan, file)?;
    grant_directory_write_dac(file, &before, &plan.user_sid)?;
    Ok(())
}

fn grant_directory_write_dac(file: &File, before: &str, sid: &str) -> io::Result<()> {
    let encoded = wide(OsStr::new(before));
    let mut descriptor = ptr::null_mut();
    // SAFETY: encoded is NUL-terminated; output is LocalAlloc-owned.
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            encoded.as_ptr(),
            1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    })?;
    let _descriptor = LocalMemory(descriptor);
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    // SAFETY: descriptor remains live and all outputs have their declared types.
    check(unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
            &raw mut present,
            &raw mut dacl,
            &raw mut defaulted,
        )
    })?;
    if present == 0 || dacl.is_null() {
        return Err(changed());
    }
    let sid_text = wide(OsStr::new(sid));
    let mut sid_memory = ptr::null_mut();
    // SAFETY: valid terminated SID text; output managed by LocalMemory.
    check(unsafe { ConvertStringSidToSidW(sid_text.as_ptr(), &raw mut sid_memory) })?;
    let _sid = LocalMemory(sid_memory);
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: WRITE_DAC,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: sid_memory.cast(),
            ..Default::default()
        },
    };
    let mut updated = ptr::null_mut();
    // SAFETY: entry's SID and old ACL live through the call; output is LocalAlloc-owned.
    win_result(unsafe { SetEntriesInAclW(1, &raw const entry, dacl, &raw mut updated) })?;
    let _updated = LocalMemory(updated.cast());
    if snapshot(file)? != before {
        return Err(changed());
    }
    // SAFETY: target is an identity-checked pinned directory handle opened with
    // MAXIMUM_ALLOWED (no propagation). Only DACL is set, preserving owner/group,
    // protection state, existing allow/deny ACEs; the sole new grant is WRITE_DAC.
    win_result(unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            updated,
            ptr::null_mut(),
        )
    })?;
    let after = snapshot(file)?;
    if before.split('(').next() != after.split('(').next() {
        return Err(failed("workspace_permission_verification_failed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
