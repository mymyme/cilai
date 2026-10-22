//! Workbench-owned LPAC launcher. No Codex protocol or implementation is involved.
//! Package identities are per user and per canonical package/data pair. Grants never
//! include another capability, user profile, workspace, network capability or child process.

use sha2::{Digest as _, Sha256};
use std::os::windows::fs::MetadataExt as _;
use std::{ffi::OsStr, fs, io, mem, os::windows::ffi::OsStrExt as _, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, GENERIC_ALL, HANDLE,
        INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0,
    },
    Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
            GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT, SE_WINDOW_OBJECT,
            SetNamedSecurityInfoW, SetSecurityInfo,
        },
        DACL_SECURITY_INFORMATION, DeriveCapabilitySidsFromName, EqualSid, FreeSid,
        GetSecurityDescriptorDacl, GetSecurityDescriptorSacl, GetTokenInformation,
        Isolation::{CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName},
        LABEL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES,
        SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenIsAppContainer,
        TokenUser,
    },
    Storage::FileSystem::{DELETE, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE},
    System::{
        Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        SystemServices::SE_GROUP_ENABLED,
        Threading::{
            CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateMutexW, CreateProcessW,
            DETACHED_PROCESS, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
            GetCurrentProcess, GetExitCodeProcess, InitializeProcThreadAttributeList,
            LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcessToken,
            PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
            PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ReleaseMutex,
            ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
            UpdateProcThreadAttribute, WaitForSingleObject,
        },
        WindowsProgramming::{
            PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT,
            PROCESS_CREATION_CHILD_PROCESS_RESTRICTED,
        },
    },
};

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
fn win(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: all Handle values own one successfully acquired, non-pseudo handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Local(*mut core::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: allocations originate from the documented LocalAlloc security APIs.
        unsafe {
            LocalFree(self.0);
        }
    }
}
struct Sid(PSID);
impl Drop for Sid {
    fn drop(&mut self) {
        // SAFETY: CreateAppContainerProfile/DeriveAppContainerSidFromAppContainerName
        // transfer a SID allocation which must be released with FreeSid.
        unsafe {
            FreeSid(self.0);
        }
    }
}
fn profile(package: &Path, data: &Path) -> io::Result<Sid> {
    let mut digest = Sha256::new();
    for path in [package, data] {
        for unit in path.as_os_str().encode_wide() {
            digest.update(unit.to_le_bytes());
        }
        digest.update([0, 0]);
    }
    let hash = format!("{:x}", digest.finalize());
    let name = wide(format!("Cilai.Personal.{}", &hash[..40]));
    let mut sid = ptr::null_mut();
    // SAFETY: strings and SID output live through the call; zero capabilities
    // explicitly grants neither internet nor private/loopback network access.
    let result = unsafe {
        CreateAppContainerProfile(
            name.as_ptr(),
            name.as_ptr(),
            name.as_ptr(),
            ptr::null(),
            0,
            &raw mut sid,
        )
    };
    if result == 0 {
        return Ok(Sid(sid));
    }
    if result != 0x8007_00b7_u32.cast_signed() {
        return Err(io::Error::from_raw_os_error(result));
    }
    // SAFETY: the same validated, per-user profile name and writable output pointer.
    let result = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &raw mut sid) };
    if result < 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    Ok(Sid(sid))
}
fn sid_string(sid: &Sid) -> io::Result<String> {
    let mut text = ptr::null_mut();
    // SAFETY: SID is owned/live; API allocates a NUL-terminated string.
    win(unsafe { ConvertSidToStringSidW(sid.0, &raw mut text) })?;
    let _allocation = Local(text.cast());
    Ok(local_string(text))
}
fn local_string(text: *const u16) -> String {
    let mut len = 0;
    // SAFETY: all callers pass a live NUL-terminated Win32 string allocation.
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    }
}
fn descriptor(sddl: &str) -> io::Result<Local> {
    let text = wide(sddl);
    let mut descriptor = ptr::null_mut();
    // SAFETY: bounded, NUL-terminated SDDL and writable allocation output.
    win(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    })?;
    Ok(Local(descriptor))
}
fn grant(path: &Path, sid: &str, write: bool) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_attributes() & 0x400 != 0 || (!metadata.is_file() && !metadata.is_dir()) {
        return Err(io::Error::other("personal_sandbox_reparse_or_special_file"));
    }
    let mut path = wide(path);
    let mut original = ptr::null_mut();
    let mut owner = ptr::null_mut();
    // SAFETY: writable outputs; returned owner is borrowed from `original`.
    let result = unsafe {
        GetNamedSecurityInfoW(
            path.as_mut_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &raw mut owner,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut original,
        )
    };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result.cast_signed()));
    }
    let _original = Local(original);
    // A trusted runtime can be Administrators-owned; writable private data must
    // belong to the launching user. Never take ownership of an existing object.
    if write {
        verify_owner(owner)?;
    }
    let mut text = ptr::null_mut();
    // SAFETY: descriptor is live; the output is independently LocalAlloc-owned.
    win(unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            original,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &raw mut text,
            ptr::null_mut(),
        )
    })?;
    let _text = Local(text.cast());
    let mut sddl = local_string(text);
    if !sddl.starts_with("D:") || sddl.contains("NO_ACCESS_CONTROL") {
        return Err(io::Error::other("personal_sandbox_invalid_dacl"));
    }
    let rights = FILE_GENERIC_READ
        | FILE_GENERIC_EXECUTE
        | if write {
            FILE_GENERIC_WRITE | DELETE
        } else {
            0
        };
    let inheritance = if metadata.is_dir() { "OICI" } else { "" };
    let ace = format!("(A;{inheritance};0x{rights:x};;;{sid})");
    // AppContainer checks both the user and package principals. Our private
    // directory initially has a non-inheritable OWNER RIGHTS ACE; without an
    // inheritable owner ACE, newly created files pass only the package check.
    let owner_ace = "(A;OICI;FA;;;OW)";
    let add_owner = write && metadata.is_dir() && !sddl.contains(owner_ace);
    if !sddl.contains(&ace) || add_owner {
        if !sddl.contains(&ace) {
            sddl.push_str(&ace);
        }
        if add_owner {
            sddl.push_str(owner_ace);
        }
        let descriptor = descriptor(&sddl)?;
        let mut dacl = ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        // SAFETY: descriptor is valid; all output addresses remain live.
        win(unsafe {
            GetSecurityDescriptorDacl(
                descriptor.0,
                &raw mut present,
                &raw mut dacl,
                &raw mut defaulted,
            )
        })?;
        // SAFETY: only append the isolated package SID; preserve owner, existing
        // restrictions and DACL protection. No host/global rights are added.
        let result = unsafe {
            SetNamedSecurityInfoW(
                path.as_mut_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null_mut(),
            )
        };
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result.cast_signed()));
        }
    }
    if write {
        low_label(&mut path)?;
    }
    Ok(())
}
fn low_label(path: &mut [u16]) -> io::Result<()> {
    let label = descriptor("S:(ML;OICI;NW;;;LW)")?;
    let mut sacl = ptr::null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    // SAFETY: valid descriptor and writable SACL output parameters.
    win(unsafe {
        GetSecurityDescriptorSacl(label.0, &raw mut present, &raw mut sacl, &raw mut defaulted)
    })?;
    // SAFETY: setting only the low mandatory label permits the LPAC to write
    // its data. DACL and ownership still constrain access to that identity.
    let result = unsafe {
        SetNamedSecurityInfoW(
            path.as_mut_ptr(),
            SE_FILE_OBJECT,
            LABEL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            sacl,
        )
    };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result.cast_signed()));
    }
    Ok(())
}
fn verify_owner(owner: PSID) -> io::Result<()> {
    let mut token = ptr::null_mut();
    // SAFETY: valid pseudo process handle and output; the real token is RAII-owned.
    win(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) })?;
    let token = Handle(token);
    let mut bytes = 0;
    // SAFETY: zero-buffer size query is explicitly supported by this API.
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &raw mut bytes);
    }
    let mut buffer = vec![0_usize; (bytes as usize).div_ceil(mem::size_of::<usize>())];
    // SAFETY: aligned buffer has the queried capacity and owns TOKEN_USER data.
    win(unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            bytes,
            &raw mut bytes,
        )
    })?;
    // SAFETY: successful TOKEN_USER fill, borrowed SID points into this buffer.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    // SAFETY: both SID pointers remain live during this call.
    if owner.is_null() || unsafe { EqualSid(owner, user.User.Sid) } == 0 {
        return Err(io::Error::other("personal_sandbox_data_owner_mismatch"));
    }
    Ok(())
}
fn grant_tree(root: &Path, sid: &str, write: bool, count: &mut usize) -> io::Result<()> {
    *count += 1;
    if *count > 10_000 {
        return Err(io::Error::other("personal_sandbox_tree_limit"));
    }
    grant(root, sid, write)?;
    if root.is_dir() {
        for entry in fs::read_dir(root)? {
            grant_tree(&entry?.path(), sid, write, count)?;
        }
    }
    Ok(())
}
struct Attributes {
    storage: Vec<usize>,
}
impl Attributes {
    fn new() -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: documented size query; four attributes will be initialized.
        unsafe {
            InitializeProcThreadAttributeList(ptr::null_mut(), 4, 0, &raw mut bytes);
        }
        let mut storage = vec![0; bytes.div_ceil(mem::size_of::<usize>())];
        // SAFETY: aligned storage meets the queried byte size and lives through launch.
        win(unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 4, 0, &raw mut bytes)
        })?;
        Ok(Self { storage })
    }
    fn raw(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
    fn set<T>(&mut self, kind: u32, value: &mut T) -> io::Result<()> {
        // SAFETY: API borrows the value; callers keep all values alive until after
        // CreateProcessW returns. Each type matches its fixed attribute contract.
        win(unsafe {
            UpdateProcThreadAttribute(
                self.raw(),
                0,
                kind as usize,
                ptr::from_mut(value).cast(),
                mem::size_of::<T>(),
                ptr::null_mut(),
                ptr::null(),
            )
        })
        .map_err(|error| io::Error::other(format!("attribute_{kind}: {error}")))
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: list was initialized in `new` and is not used after this drop.
        unsafe {
            DeleteProcThreadAttributeList(self.raw());
        }
    }
}
fn inherited(std: STD_HANDLE) -> io::Result<Handle> {
    // SAFETY: only this helper's three standard handles are queried.
    let source = unsafe { GetStdHandle(std) };
    if source.is_null() || source == INVALID_HANDLE_VALUE {
        return Err(io::Error::other("personal_sandbox_stdio_missing"));
    }
    let mut target = ptr::null_mut();
    // SAFETY: duplicate an existing handle in this process, explicitly inheritable;
    // the attribute allowlist prevents any other handle reaching the child.
    win(unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            GetCurrentProcess(),
            &raw mut target,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    })?;
    Ok(Handle(target))
}
pub(super) fn prepare_helper_environment(command: &mut std::process::Command) -> io::Result<()> {
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{
            FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_RoamingAppData, SHGetKnownFolderPath,
        },
    };
    for (name, id) in [
        ("LOCALAPPDATA", FOLDERID_LocalAppData),
        ("APPDATA", FOLDERID_RoamingAppData),
        ("USERPROFILE", FOLDERID_Profile),
    ] {
        let mut raw = ptr::null_mut();
        // SAFETY: query a fixed folder for the current token, no impersonation;
        // result is a NUL-terminated CoTaskMem allocation.
        let result =
            unsafe { SHGetKnownFolderPath(&raw const id, 0, ptr::null_mut(), &raw mut raw) };
        if result < 0 {
            return Err(io::Error::from_raw_os_error(result));
        }
        let value = local_string(raw);
        // SAFETY: release the successful shell allocation exactly once.
        unsafe {
            CoTaskMemFree(raw.cast());
        }
        command.env(name, value);
    }
    Ok(())
}
pub(super) struct Lease {
    _station: StationAccess,
    _lock: crate::security::RestrictedFileLock,
    node: std::path::PathBuf,
}
impl Lease {
    pub(super) fn node(&self) -> &Path {
        &self.node
    }
}
fn runtime_hash(path: &Path) -> io::Result<String> {
    use std::io::Read as _;
    let mut file = fs::File::open(path)?;
    if file.metadata()?.len() > 256 * 1024 * 1024 {
        return Err(io::Error::other("personal_sandbox_runtime_too_large"));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn private_runtime(source: &Path, data: &Path) -> io::Result<std::path::PathBuf> {
    // Installed binaries may be Administrators-owned. Never change their ACLs.
    // Each capability uses a host-owned, content-addressed copy outside its
    // writable data; the LPAC receives read/execute access only.
    let expected = runtime_hash(source)?;
    let mut name = data.as_os_str().to_owned();
    name.push(".personal-runtime");
    let root = std::path::PathBuf::from(name);
    let directory = root.join(&expected);
    for path in [&root, &directory] {
        match fs::symlink_metadata(path) {
            Ok(kind) if !kind.is_dir() || kind.file_attributes() & 0x400 != 0 => {
                return Err(io::Error::other(
                    "personal_sandbox_runtime_cache_path_invalid",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        crate::security::ensure_restricted_directory(path)?;
    }
    let target = directory.join("node.exe");
    if target.try_exists()? {
        let kind = fs::symlink_metadata(&target)?;
        if !kind.is_file()
            || kind.file_attributes() & 0x400 != 0
            || runtime_hash(&target)? != expected
        {
            return Err(io::Error::other("personal_sandbox_runtime_copy_changed"));
        }
        return target.canonicalize();
    }
    let pending = directory.join(format!(
        "node-{}-{}.pending",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos()
    ));
    let outcome = (|| {
        let mut file = crate::security::create_new_restricted_file(&pending)?;
        std::io::copy(&mut fs::File::open(source)?, &mut file)?;
        file.sync_all()?;
        drop(file);
        if runtime_hash(&pending)? != expected {
            return Err(io::Error::other("personal_sandbox_runtime_copy_changed"));
        }
        fs::rename(&pending, &target)?;
        target.canonicalize()
    })();
    if outcome.is_err() {
        // Only this invocation's newly created private cache candidate is removed.
        let _ = fs::remove_file(&pending);
    }
    outcome
}
pub(super) fn prepare(node: &Path, package: &Path, data: &Path) -> io::Result<Lease> {
    let mut lock_name = data.as_os_str().to_owned();
    lock_name.push(".personal-sandbox.lock");
    let lock = crate::security::try_lock_restricted_file(Path::new(&lock_name))?
        .ok_or_else(|| io::Error::other("personal_sandbox_already_running"))?;
    let context = |step: &str, error: io::Error| {
        io::Error::other(format!("personal_sandbox_{step}: {error}"))
    };
    let node = private_runtime(node, data).map_err(|error| context("runtime_copy", error))?;
    let sid = profile(package, data).map_err(|error| context("profile", error))?;
    let text = sid_string(&sid)?;
    grant(
        node.parent()
            .ok_or_else(|| io::Error::other("personal_sandbox_runtime_parent_missing"))?,
        &text,
        false,
    )
    .map_err(|error| context("runtime_directory_acl", error))?;
    grant(&node, &text, false).map_err(|error| context("runtime_acl", error))?;
    grant_tree(package, &text, false, &mut 0).map_err(|error| context("package_acl", error))?;
    grant_tree(data, &text, true, &mut 0).map_err(|error| context("data_acl", error))?;
    // Host owns the station grant and lock: killing a stuck helper still
    // revokes the grant after its process tree has been reaped.
    // SAFETY: only queries the current process station; handle is borrowed.
    let station_name = name_for_station(unsafe {
        windows_sys::Win32::System::StationsAndDesktops::GetProcessWindowStation()
    })?;
    let station = StationAccess::new(&station_name, &sid)?;
    Ok(Lease {
        _station: station,
        _lock: lock,
        node,
    })
}
pub(super) fn run(node: &Path, package: &Path, data: &Path, args: &[String]) -> io::Result<i32> {
    let sid = profile(package, data)?;
    launch(node, args, data, &sid)
}
fn launch(executable: &Path, args: &[String], data: &Path, sid: &Sid) -> io::Result<i32> {
    let mut desktop = PrivateDesktop::new(sid)
        .map_err(|error| io::Error::other(format!("private_desktop: {error}")))?;
    let stdin = inherited(STD_INPUT_HANDLE)?;
    let stdout = inherited(STD_OUTPUT_HANDLE)?;
    let stderr = inherited(STD_ERROR_HANDLE)?;
    let mut handles = [
        stdin.0,
        stdout.0,
        stderr.0,
        desktop.station,
        desktop.desktop,
    ];
    let (mut registry, _allocations) = registry_read()?;
    let mut capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        Capabilities: &raw mut registry,
        CapabilityCount: 1,
        Reserved: 0,
    };
    let mut lpac = PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT;
    let mut child_policy = PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
    let mut attributes = Attributes::new()?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
        &mut capabilities,
    )?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
        &mut lpac,
    )?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY,
        &mut child_policy,
    )?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &mut handles)?;
    // SAFETY: zero is the documented initial state for both Win32 structures.
    let mut startup: STARTUPINFOEXW = unsafe { mem::zeroed() };
    startup.StartupInfo.cb = u32::try_from(mem::size_of::<STARTUPINFOEXW>()).expect("startup size");
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin.0;
    startup.StartupInfo.hStdOutput = stdout.0;
    startup.StartupInfo.hStdError = stderr.0;
    // The explicit inherited station and private desktop handles select the UI
    // objects. Never ask Windows to open the caller's default desktop.
    startup.StartupInfo.lpDesktop = desktop.name.as_mut_ptr();
    startup.lpAttributeList = attributes.raw();
    let executable_w = wide(executable);
    let mut command = wide(super::windows_command_line(executable, args));
    let cwd = wide(data);
    let system_root =
        std::env::var_os("SystemRoot").ok_or_else(|| io::Error::other("SystemRoot missing"))?;
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| io::Error::other("APPDATA missing"))?;
    let local_appdata =
        std::env::var_os("LOCALAPPDATA").ok_or_else(|| io::Error::other("LOCALAPPDATA missing"))?;
    let profile =
        std::env::var_os("USERPROFILE").ok_or_else(|| io::Error::other("USERPROFILE missing"))?;
    let mut environment = Vec::new();
    for (name, value) in [
        ("APPDATA", appdata.as_os_str()),
        ("LOCALAPPDATA", local_appdata.as_os_str()),
        ("SystemRoot", system_root.as_os_str()),
        ("TEMP", data.as_os_str()),
        ("TMP", data.as_os_str()),
        ("USERPROFILE", profile.as_os_str()),
        ("WINDIR", system_root.as_os_str()),
    ] {
        environment.extend(wide(format!("{name}={}", value.to_string_lossy())));
    }
    environment.push(0);
    // SAFETY: zero initializes all optional/reserved fields; all borrowed buffers
    // and attribute values stay live through CreateProcessW.
    let mut info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    // A strict child policy/one-process Job must not initialize a console host.
    // Protocol stdio are explicit inherited pipes, independent of a console.
    let window_flags = DETACHED_PROCESS;
    let environment_pointer = environment.as_ptr().cast();
    // SAFETY: fixed executable, correctly quoted mutable command, minimal environment,
    // LPAC attributes and explicit standard-handle allowlist; starts suspended.
    win(unsafe {
        CreateProcessW(
            executable_w.as_ptr(),
            command.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT
                | window_flags,
            environment_pointer,
            cwd.as_ptr(),
            &raw const startup.StartupInfo,
            &raw mut info,
        )
    })
    .map_err(|error| io::Error::other(format!("create_process: {error}")))?;
    let process = Handle(info.hProcess);
    let thread = Handle(info.hThread);
    let result = supervise(&process, &thread);
    if result.is_err() {
        // SAFETY: always terminate a successfully created child on failed setup;
        // a suspended child must never be orphaned or resumed without its Job.
        unsafe {
            TerminateProcess(process.0, 1);
        }
    }
    result
}

struct PrivateDesktop {
    station: windows_sys::Win32::System::StationsAndDesktops::HWINSTA,
    desktop: windows_sys::Win32::System::StationsAndDesktops::HDESK,
    name: Vec<u16>,
}
impl PrivateDesktop {
    fn new(sid: &Sid) -> io::Result<Self> {
        use windows_sys::Win32::{
            System::StationsAndDesktops::{
                CreateDesktopW, GetUserObjectInformationW, OpenWindowStationW, UOI_NAME,
            },
            UI::WindowsAndMessaging::{WINSTA_ACCESSGLOBALATOMS, WINSTA_READATTRIBUTES},
        };
        // SAFETY: borrow the caller's existing station only to query its name.
        let current =
            unsafe { windows_sys::Win32::System::StationsAndDesktops::GetProcessWindowStation() };
        let mut bytes = 0;
        // SAFETY: documented zero-size query, then correctly sized UTF-16 buffer.
        unsafe {
            GetUserObjectInformationW(current, UOI_NAME, ptr::null_mut(), 0, &raw mut bytes);
        }
        let mut name = vec![0_u16; (bytes as usize).div_ceil(2)];
        // SAFETY: live station handle and buffer has the queried byte capacity.
        win(unsafe {
            GetUserObjectInformationW(
                current,
                UOI_NAME,
                name.as_mut_ptr().cast(),
                bytes,
                &raw mut bytes,
            )
        })?;
        // SAFETY: open only a query handle, inheritable and explicitly included in
        // HANDLE_LIST. No clipboard, screen or host desktop access. The host
        // holds the narrowly scoped station grant for this launch.
        let station = unsafe {
            OpenWindowStationW(
                name.as_ptr(),
                1,
                (WINSTA_READATTRIBUTES | WINSTA_ACCESSGLOBALATOMS).cast_unsigned(),
            )
        };
        if station.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut result = Self {
            station,
            desktop: ptr::null_mut(),
            name: Vec::new(),
        };
        let descriptor = descriptor(&format!(
            "D:P(A;;GA;;;SY)(A;;GA;;;OW)(A;;GA;;;{})S:(ML;;NW;;;LW)",
            sid_string(sid)?
        ))?;
        let security = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(mem::size_of::<SECURITY_ATTRIBUTES>()).expect("security size"),
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 1,
        };
        let name = wide(format!(
            "Cilai.Personal.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_nanos()
        ));
        let station_name = local_string(name_for_station(current)?.as_ptr());
        result.name = wide(format!("{station_name}\\{}", local_string(name.as_ptr())));
        // SAFETY: new private desktop in the current station, scoped DACL and low
        // label. Only this desktop, not the host desktop, reaches the child.
        let desktop = unsafe {
            CreateDesktopW(
                name.as_ptr(),
                ptr::null(),
                ptr::null(),
                0,
                GENERIC_ALL,
                &raw const security,
            )
        };
        if desktop.is_null() {
            return Err(io::Error::last_os_error());
        }
        result.desktop = desktop;
        Ok(result)
    }
}

fn name_for_station(station: HANDLE) -> io::Result<Vec<u16>> {
    use windows_sys::Win32::System::StationsAndDesktops::{GetUserObjectInformationW, UOI_NAME};
    let mut bytes = 0;
    // SAFETY: documented size query for a live borrowed window-station handle.
    unsafe {
        GetUserObjectInformationW(station, UOI_NAME, ptr::null_mut(), 0, &raw mut bytes);
    }
    let mut name = vec![0_u16; (bytes as usize).div_ceil(2)];
    // SAFETY: buffer has the queried capacity and remains live through this call.
    win(unsafe {
        GetUserObjectInformationW(
            station,
            UOI_NAME,
            name.as_mut_ptr().cast(),
            bytes,
            &raw mut bytes,
        )
    })?;
    Ok(name)
}
struct StationAccess {
    station: windows_sys::Win32::System::StationsAndDesktops::HWINSTA,
    ace: String,
    sid_suffix: String,
}
struct StationLock(Handle);
impl StationLock {
    fn acquire() -> io::Result<Self> {
        let name = wide("Local\\Cilai.Personal.WindowStationAcl");
        // SAFETY: session-local mutex with the launching token's default DACL;
        // no inheritable handle and no capability SID access. Serializes ACL
        // edits across trusted hosts, including different Workbench profiles.
        let raw = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        let handle = Handle(raw);
        // SAFETY: live mutex handle; both success and abandoned status transfer
        // ownership to this thread. No await or cross-thread transfer follows.
        match unsafe { WaitForSingleObject(handle.0, 5_000) } {
            WAIT_OBJECT_0 | windows_sys::Win32::Foundation::WAIT_ABANDONED => Ok(Self(handle)),
            _ => Err(io::Error::other("personal_sandbox_station_acl_busy")),
        }
    }
}
impl Drop for StationLock {
    fn drop(&mut self) {
        // SAFETY: this same thread successfully acquired and still owns mutex.
        if let Err(error) = win(unsafe { ReleaseMutex(self.0.0) }) {
            eprintln!("personal_sandbox_station_mutex_release_failed:{error}");
        }
    }
}
impl StationAccess {
    fn new(name: &[u16], sid: &Sid) -> io::Result<Self> {
        use windows_sys::Win32::{
            Storage::FileSystem::{READ_CONTROL, WRITE_DAC},
            System::StationsAndDesktops::OpenWindowStationW,
            UI::WindowsAndMessaging::{
                WINSTA_ACCESSGLOBALATOMS, WINSTA_ENUMDESKTOPS, WINSTA_ENUMERATE,
                WINSTA_READATTRIBUTES,
            },
        };
        let sid = sid_string(sid)?;
        let guard = StationLock::acquire()?;
        // SAFETY: name is a queried NUL-terminated station name. This control
        // handle is not inheritable and is excluded from the child HANDLE_LIST.
        let station = unsafe { OpenWindowStationW(name.as_ptr(), 0, READ_CONTROL | WRITE_DAC) };
        if station.is_null() {
            return Err(io::Error::last_os_error());
        }
        let rights = (WINSTA_READATTRIBUTES
            | WINSTA_ENUMDESKTOPS
            | WINSTA_ENUMERATE
            | WINSTA_ACCESSGLOBALATOMS)
            .cast_unsigned();
        let value = Self {
            station,
            ace: format!("(A;;0x{rights:x};;;{sid})"),
            sid_suffix: format!(";;;{sid})"),
        };
        let outcome = (|| {
            let mut sddl = value.read()?;
            if !sddl.contains(&value.ace) {
                sddl.push_str(&value.ace);
                value.write(&sddl)?;
            }
            Ok::<_, io::Error>(())
        })();
        drop(guard);
        outcome?;
        Ok(value)
    }
    fn read(&self) -> io::Result<String> {
        let mut raw = ptr::null_mut();
        // SAFETY: live station control handle, valid security descriptor output.
        let status = unsafe {
            GetSecurityInfo(
                self.station,
                SE_WINDOW_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &raw mut raw,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status.cast_signed()));
        }
        let _descriptor = Local(raw);
        let mut text = ptr::null_mut();
        // SAFETY: descriptor remains live, independent allocated SDDL output.
        win(unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                raw,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &raw mut text,
                ptr::null_mut(),
            )
        })?;
        let _text = Local(text.cast());
        Ok(local_string(text))
    }
    fn write(&self, sddl: &str) -> io::Result<()> {
        let descriptor = descriptor(sddl)?;
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = ptr::null_mut();
        // SAFETY: valid descriptor and writable borrowed-DACL output parameters.
        win(unsafe {
            GetSecurityDescriptorDacl(
                descriptor.0,
                &raw mut present,
                &raw mut dacl,
                &raw mut defaulted,
            )
        })?;
        // SAFETY: changes only our package SID ACE on this session's station.
        // No clipboard, screen, host desktop or filesystem rights are granted.
        let status = unsafe {
            SetSecurityInfo(
                self.station,
                SE_WINDOW_OBJECT,
                DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null_mut(),
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status.cast_signed()));
        }
        Ok(())
    }
}
impl Drop for StationAccess {
    fn drop(&mut self) {
        let guard = StationLock::acquire();
        if guard.is_ok()
            && let Ok(current) = self.read()
        {
            // SDDL canonicalization may rename access-mask bits. Match our exact
            // package SID, preserving all unrelated concurrent grants and controls.
            let mut next = String::new();
            let mut tail = current.as_str();
            while let Some(start) = tail.find('(') {
                next.push_str(&tail[..start]);
                tail = &tail[start..];
                let Some(end) = tail.find(')') else {
                    break;
                };
                let ace = &tail[..=end];
                if !ace.ends_with(&self.sid_suffix) {
                    next.push_str(ace);
                }
                tail = &tail[end + 1..];
            }
            next.push_str(tail);
            if current != next
                && let Err(error) = self.write(&next)
            {
                eprintln!("personal_sandbox_station_cleanup_failed:{error}");
            }
        }
        if let Err(error) = &guard {
            eprintln!("personal_sandbox_station_cleanup_failed:{error}");
        }
        // SAFETY: privately owned non-inheritable control handle is no longer used.
        unsafe {
            windows_sys::Win32::System::StationsAndDesktops::CloseWindowStation(self.station);
        }
    }
}
impl Drop for PrivateDesktop {
    fn drop(&mut self) {
        use windows_sys::Win32::System::StationsAndDesktops::{CloseDesktop, CloseWindowStation};
        // SAFETY: privately owned UI object handles; the child exits or its Job
        // closes before these guards. The caller's borrowed objects are not closed.
        unsafe {
            if !self.desktop.is_null() {
                CloseDesktop(self.desktop);
            }
            CloseWindowStation(self.station);
        }
    }
}
fn registry_read() -> io::Result<(SID_AND_ATTRIBUTES, Vec<Local>)> {
    let name = wide("registryRead");
    let mut groups = ptr::null_mut();
    let mut group_count = 0;
    let mut sids = ptr::null_mut();
    let mut count = 0;
    // SAFETY: fixed OS registry-read capability only. Windows owns all output
    // allocations until success, then each array/SID is released with LocalFree.
    win(unsafe {
        DeriveCapabilitySidsFromName(
            name.as_ptr(),
            &raw mut groups,
            &raw mut group_count,
            &raw mut sids,
            &raw mut count,
        )
    })?;
    let _groups = Local(groups.cast());
    let _sids = Local(sids.cast());
    let mut allocations = Vec::new();
    for index in 0..group_count as usize {
        // SAFETY: successful API returned `group_count` allocated SID pointers.
        allocations.push(Local(unsafe { *groups.add(index) }));
    }
    let mut sid = ptr::null_mut();
    for index in 0..count as usize {
        // SAFETY: successful API returned `count` allocated SID pointers.
        sid = unsafe { *sids.add(index) };
        allocations.push(Local(sid));
    }
    if count != 1 || sid.is_null() {
        return Err(io::Error::other(
            "personal_sandbox_registry_capability_invalid",
        ));
    }
    Ok((
        SID_AND_ATTRIBUTES {
            Sid: sid,
            Attributes: SE_GROUP_ENABLED.cast_unsigned(),
        },
        allocations,
    ))
}
fn supervise(process: &Handle, thread: &Handle) -> io::Result<i32> {
    verify_isolation_token(process)?;
    // SAFETY: anonymous job, no inheritable handle or global name.
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = Handle(raw);
    // SAFETY: zero is the documented initial state for Job limit information.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    limits.ProcessMemoryLimit = 256 * 1024 * 1024;
    // SAFETY: live anonymous job and correctly sized immutable limit structure.
    win(unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            ptr::from_ref(&limits).cast(),
            u32::try_from(mem::size_of_val(&limits)).expect("job size"),
        )
    })?;
    // SAFETY: child still suspended; assignment precedes any untrusted instruction.
    win(unsafe { AssignProcessToJobObject(job.0, process.0) })?;
    // SAFETY: child primary thread is valid and was created suspended exactly once.
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: live child handle. Timeout is a hard stop; closing the job kills it.
    if unsafe { WaitForSingleObject(process.0, 25_000) } != WAIT_OBJECT_0 {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "personal_sandbox_timeout",
        ));
    }
    let mut code = 0;
    // SAFETY: process has exited; output is writable.
    win(unsafe { GetExitCodeProcess(process.0, &raw mut code) })?;
    Ok(code.cast_signed())
}

fn verify_isolation_token(process: &Handle) -> io::Result<()> {
    let mut raw = ptr::null_mut();
    // SAFETY: suspended child is owned/live and token is opened only for query.
    win(unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &raw mut raw) })?;
    let token = Handle(raw);
    // Class 46 is not a supported Win32 query on Windows 10. LPAC is required
    // through the documented creation attribute above, with no retry/fallback;
    // the native contract additionally probes AAP-only vs ARAP file access.
    let mut value = 0_u32;
    let mut bytes = 0;
    // SAFETY: TokenIsAppContainer returns one DWORD; correct buffer size.
    win(unsafe {
        GetTokenInformation(
            token.0,
            TokenIsAppContainer,
            ptr::from_mut(&mut value).cast(),
            4,
            &raw mut bytes,
        )
    })?;
    if value != 1 {
        return Err(io::Error::other("personal_sandbox_token_not_lpac"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "executes Node without Node permission flags inside a real Windows LPAC"]
    #[expect(
        clippy::used_underscore_binding,
        reason = "native contract inspects the host-owned RAII grant before and after drop"
    )]
    fn native_lpac_enforces_files_network_processes_and_separate_capabilities() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!("cilai-native-lpac-{}", std::process::id()));
        let package = root.join("package");
        let data = root.join("data");
        let other = root.join("other-data");
        for path in [&root, &package, &data, &other] {
            crate::security::ensure_restricted_directory(path).unwrap();
        }
        fs::write(root.join("outside.txt"), "private host").unwrap();
        fs::write(other.join("secret.txt"), "another capability").unwrap();
        let ordinary_access = root.join("aap-only.txt");
        let restricted_access = root.join("arap.txt");
        fs::write(&ordinary_access, "AAP only").unwrap();
        fs::write(&restricted_access, "ARAP").unwrap();
        grant(&ordinary_access, "S-1-15-2-1", false).unwrap();
        grant(&restricted_access, "S-1-15-2-2", false).unwrap();
        let ordinary_access = serde_json::to_string(&ordinary_access).unwrap();
        let restricted_access = serde_json::to_string(&restricted_access).unwrap();
        let script = package.join("probe.mjs");
        let outside = serde_json::to_string(&root.join("outside.txt")).unwrap();
        let other = serde_json::to_string(&other.join("secret.txt")).unwrap();
        let package_write = serde_json::to_string(&package.join("tampered.txt")).unwrap();
        let port = listener.local_addr().unwrap().port();
        fs::write(&script, format!(r"
import fs from 'node:fs'; import net from 'node:net'; import {{spawnSync}} from 'node:child_process';
fs.writeFileSync('allowed.txt','private');
const result={{own:fs.readFileSync('allowed.txt','utf8')==='private',aap:false,arap:false,outside:false,other:false,packageWrite:false,child:false,network:false}};
try {{fs.readFileSync({ordinary_access});result.aap=true;}} catch {{}}
try {{result.arap=fs.readFileSync({restricted_access},'utf8')==='ARAP';}} catch {{}}
try {{fs.readFileSync({outside});result.outside=true;}} catch {{}}
try {{fs.readFileSync({other});result.other=true;}} catch {{}}
try {{fs.writeFileSync({package_write},'bad');result.packageWrite=true;}} catch {{}}
try {{const child=spawnSync(process.execPath,['--version'],{{stdio:'inherit',timeout:1000}});result.child=!child.error;}} catch {{}}
result.network=await new Promise(resolve=>{{const socket=net.connect({{host:'127.0.0.1',port:{port}}}); socket.on('error',()=>resolve(false));socket.on('connect',()=>{{socket.destroy();resolve(true)}});socket.setTimeout(1000,()=>{{socket.destroy();resolve(true)}});}});
console.log(JSON.stringify(result));
if(!result.own || result.aap || !result.arap || result.outside || result.other || result.packageWrite || result.child || result.network) process.exitCode=1;
")).unwrap();
        let node = crate::executable_search::find_executable(&["node"]).expect("Node 24 required");
        // No --permission flags: these assertions exercise Windows enforcement alone.
        let args = vec![
            "--preserve-symlinks-main".into(),
            "--preserve-symlinks".into(),
            script.to_string_lossy().into_owned(),
        ];
        let lease = prepare(&node, &package, &data).unwrap();
        assert!(
            prepare(&node, &package, &data).is_err(),
            "same capability cannot race another launch"
        );
        // Keep an independent query handle across lease drop to verify the host
        // actually removes the grant, rather than merely closing its handle.
        // SAFETY: query the process's borrowed station; the ACL-edit handle
        // deliberately has no WINSTA_READATTRIBUTES permission.
        let name = name_for_station(unsafe {
            windows_sys::Win32::System::StationsAndDesktops::GetProcessWindowStation()
        })
        .unwrap();
        // SAFETY: queried live station name; read-only non-inherited test handle.
        let raw = unsafe {
            windows_sys::Win32::System::StationsAndDesktops::OpenWindowStationW(
                name.as_ptr(),
                0,
                windows_sys::Win32::Storage::FileSystem::READ_CONTROL,
            )
        };
        assert!(!raw.is_null());
        let observer = StationAccess {
            station: raw,
            ace: String::new(),
            sid_suffix: "not-a-sid".into(),
        };
        let suffix = lease._station.sid_suffix.clone();
        assert!(observer.read().unwrap().contains(&suffix));
        assert_eq!(run(lease.node(), &package, &data, &args).unwrap(), 0);
        assert_eq!(run(lease.node(), &package, &data, &args).unwrap(), 0);
        assert_eq!(
            fs::read_to_string(data.join("allowed.txt")).unwrap(),
            "private"
        );
        drop(lease);
        assert!(!observer.read().unwrap().contains(&suffix));
        fs::remove_dir_all(&root).unwrap();
    }
}
