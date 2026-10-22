use super::{
    Conflict, DiagnosticError, Diagnostics, HELPER_ARGUMENT, Inspection, RepairPlan, RepairResult,
    layout,
};
use crate::workspace_permissions::windows as native;
use native::{
    Handle, LocalMemory, check, current_token, elevated, identity, information, now, snapshot,
    user_sid, wide, win_result,
};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::io::AsRawHandle as _;
use std::path::{Path, PathBuf};
use std::ptr;
use windows_sys::Win32::Foundation::{GetLastError, LUID};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SE_FILE_OBJECT,
    SetSecurityInfo,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl,
    GetSecurityDescriptorOwner, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_DUPLICATE, TOKEN_IMPERSONATE, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath};

const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const FOREIGN_OWNER: i32 = 10001;
const UNSAFE_OBJECT: i32 = 10002;
const BUSY: i32 = 10003;
const VERIFICATION: i32 = 10004;
const ROLLBACK: i32 = 10005;
const PRIVATE_DACL: &str = "D:P(A;;FA;;;OW)(A;;FA;;;SY)";
const ADMINISTRATORS_SID: &str = "S-1-5-32-544";

mod journal;
mod legacy;
use journal::RepairJournal;

fn error(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}
fn conflict(kind: Conflict) -> io::Error {
    error(kind as i32)
}
fn local_data(token: Option<&Handle>) -> io::Result<PathBuf> {
    let mut pointer = ptr::null_mut();
    // SAFETY: fixed known-folder GUID; token, if present, has query/impersonate
    // rights and belongs to the verified live original process.
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            0,
            token.map_or(ptr::null_mut(), |token| token.0),
            &raw mut pointer,
        )
    };
    let path = if result >= 0 && !pointer.is_null() {
        Some(PathBuf::from(native::local_string(pointer)))
    } else {
        None
    };
    // SAFETY: SHGetKnownFolderPath allocates with CoTaskMem; NULL is allowed.
    unsafe { CoTaskMemFree(pointer.cast()) };
    path.ok_or_else(|| error(5))
}

fn open(path: &Path, directory: bool, access: u32) -> io::Result<File> {
    let file = OpenOptions::new()
        .access_mode(access)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let info = information(&file)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        || (!directory && info.nNumberOfLinks != 1)
    {
        return Err(error(UNSAFE_OBJECT));
    }
    Ok(file)
}

fn pin_chain(path: &Path) -> io::Result<Vec<File>> {
    native::validate_path(path)?;
    path.ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|path| open(path, true, 0))
        .collect()
}

fn root(plan: &RepairPlan) -> PathBuf {
    layout::root(Path::new(&plan.local_data))
}
fn profile(plan: &RepairPlan) -> PathBuf {
    root(plan).join("profiles").join(&plan.profile_id)
}

fn validate(plan: &RepairPlan, expected_local: &Path) -> io::Result<()> {
    if !layout::valid_scope(&plan.environment, &plan.profile_id) {
        return Err(conflict(Conflict::InvalidRequest));
    }
    let current_time = now();
    if current_time > plan.expires_at || plan.expires_at > current_time + 300 {
        return Err(conflict(Conflict::Expired));
    }
    if Path::new(&plan.local_data) != expected_local {
        // Known Folder APIs can return different spelling for the same Windows
        // directory (case, 8.3 names, verbatim prefix). Compare actual objects,
        // pinning both complete chains and rejecting reparse points throughout.
        // Text lowercasing/canonicalization alone cannot establish that identity.
        let planned = pin_chain(Path::new(&plan.local_data))?;
        let expected = pin_chain(expected_local)?;
        if identity(
            planned
                .last()
                .ok_or_else(|| conflict(Conflict::InvalidRequest))?,
        )? != identity(
            expected
                .last()
                .ok_or_else(|| conflict(Conflict::InvalidRequest))?,
        )? {
            return Err(conflict(Conflict::LocalDataChanged));
        }
    }
    Ok(())
}

fn anchor_paths(plan: &RepairPlan) -> Vec<PathBuf> {
    vec![
        PathBuf::from(&plan.local_data),
        root(plan).parent().unwrap().to_owned(),
        root(plan),
        root(plan).join("profiles"),
        profile(plan),
    ]
}

fn anchors(plan: &RepairPlan) -> io::Result<Vec<(String, String)>> {
    let mut output = Vec::new();
    for path in anchor_paths(plan) {
        match open(&path, true, 0) {
            Ok(file) => output.push((path.to_string_lossy().into_owned(), identity(&file)?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(output)
}

pub(super) fn inspect(environment_root: &Path, profile_id: &str) -> io::Result<Inspection> {
    let token = current_token()?;
    let local = local_data(None)?;
    let _pins = pin_chain(&local)?;
    let environment = environment_root
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    let mut plan = RepairPlan {
        local_data: local.to_string_lossy().into_owned(),
        environment: environment.into(),
        profile_id: profile_id.into(),
        user_sid: user_sid(&token)?,
        parent_pid: std::process::id(),
        parent_fingerprint: crate::instance::process_start_fingerprint(std::process::id())?
            .ok_or_else(|| conflict(Conflict::CallerChanged))?,
        expires_at: now() + 300,
        anchors: Vec::new(),
    };
    validate(&plan, &local)?;
    if root(&plan) != environment_root {
        return Err(error(UNSAFE_OBJECT));
    }
    plan.anchors = anchors(&plan)?;
    Ok(Inspection {
        directory: profile(&plan).to_string_lossy().into_owned(),
        user_sid: plan.user_sid.clone(),
        process_elevated: elevated(&token)?,
        plan,
    })
}

pub(super) fn diagnose(environment_root: &Path, profile_id: &str) -> io::Result<Diagnostics> {
    let mut result = Diagnostics {
        process_elevated: elevated(&current_token()?)?,
        errors: Vec::new(),
        repair_attempts: Vec::new(),
    };
    let inspection = match inspect(environment_root, profile_id) {
        Ok(value) => value,
        Err(error) => {
            result.errors.push(DiagnosticError {
                scope: "managedPath",
                code: "startup_path_validation_failed",
                owner_category: None,
                os_error: error.raw_os_error(),
            });
            return Ok(result);
        }
    };
    match journal::recent(&inspection.plan) {
        Ok(attempts) => result.repair_attempts = attempts,
        Err(error) => result.errors.push(DiagnosticError {
            scope: "managedPath",
            code: "startup_repair_journal_unavailable",
            owner_category: None,
            os_error: error.raw_os_error(),
        }),
    }
    Ok(diagnose_plan(&inspection.plan, result))
}

fn diagnose_plan(plan: &RepairPlan, mut result: Diagnostics) -> Diagnostics {
    let env = root(plan);
    let profile = profile(plan);
    for (scope, path, directory) in [
        ("applicationData", env.parent().unwrap().to_owned(), true),
        ("environment", env.clone(), true),
        ("profiles", env.join("profiles"), true),
        ("profile", profile.clone(), true),
        ("localProfile", env.join("local-profile.json"), false),
        ("instance", profile.join("run/instance.json"), false),
    ] {
        let checked: io::Result<Option<DiagnosticError>> = (|| {
            let _pins = pin_chain(path.parent().ok_or_else(|| error(UNSAFE_OBJECT))?)?;
            let file = open(&path, directory, READ_CONTROL)?;
            let sddl = snapshot(&file)?;
            let actual_owner = owner(&sddl)?;
            if actual_owner != plan.user_sid {
                return Ok(Some(DiagnosticError {
                    scope,
                    code: "private_file_owner_mismatch",
                    owner_category: Some(if actual_owner == ADMINISTRATORS_SID {
                        "administrators"
                    } else {
                        "otherAccount"
                    }),
                    os_error: None,
                }));
            }
            crate::security::verify_current_user_private_file(&path)?;
            Ok(None)
        })();
        match checked {
            Ok(Some(error)) => result.errors.push(error),
            Ok(None) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => result.errors.push(DiagnosticError {
                scope,
                code: "private_file_validation_failed",
                owner_category: None,
                os_error: error.raw_os_error(),
            }),
        }
    }
    result
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    plan: RepairPlan,
    backup_name: String,
    backup_identity: String,
    journal_identity: String,
}

fn backup_path(request: &Request) -> io::Result<PathBuf> {
    if request.backup_name.len() != 41
        || Path::new(&request.backup_name).extension() != Some(OsStr::new("json"))
        || !request.backup_name[..36]
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-')
    {
        return Err(conflict(Conflict::InvalidRequest));
    }
    Ok(layout::recovery(Path::new(&request.plan.local_data)).join(&request.backup_name))
}

fn encode(request: &Request) -> io::Result<String> {
    let bytes = serde_json::to_vec(request).map_err(io::Error::other)?;
    let mut text = String::new();
    for byte in bytes {
        write!(&mut text, "{byte:02x}").unwrap();
    }
    if text.len() > 24_000 {
        return Err(conflict(Conflict::InvalidRequest));
    }
    Ok(text)
}
fn decode(text: &str) -> io::Result<Request> {
    if text.len() > 24_000
        || !text.len().is_multiple_of(2)
        || !text.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(conflict(Conflict::InvalidRequest));
    }
    let bytes = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(io::Error::other))
        .collect::<io::Result<Vec<_>>>()?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

fn owner(sddl: &str) -> io::Result<String> {
    // SDDL can abbreviate an owner (BA, LA, etc.). Normalize through Win32
    // rather than comparing that abbreviation with the numeric TokenUser SID.
    let text = wide(OsStr::new(sddl));
    let mut descriptor = ptr::null_mut();
    // SAFETY: NUL-terminated input; the allocated descriptor is retained below.
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    })?;
    let _descriptor = LocalMemory(descriptor);
    let mut sid = ptr::null_mut();
    let mut defaulted = 0;
    // SAFETY: the returned SID borrows the live descriptor.
    check(unsafe { GetSecurityDescriptorOwner(descriptor, &raw mut sid, &raw mut defaulted) })?;
    if sid.is_null() {
        return Err(error(UNSAFE_OBJECT));
    }
    let mut output = ptr::null_mut();
    // SAFETY: valid borrowed SID and writable output; LocalMemory owns the result.
    check(unsafe { ConvertSidToStringSidW(sid, &raw mut output) })?;
    let _output = LocalMemory(output.cast());
    Ok(native::local_string(output))
}

struct Target {
    path: PathBuf,
    file: File,
    _ancestors: Vec<File>,
    before: String,
    directory: bool,
    runtime: bool,
}

// Fixed startup surfaces only. No recursion, no arbitrary filename/renderer
// path, no model workspace, and no sandbox secrets/accounts/capability ACLs.
fn locations(plan: &RepairPlan) -> Vec<(PathBuf, bool, bool)> {
    let env = root(plan);
    let profile = profile(plan);
    let mut result = vec![
        (env.parent().unwrap().to_owned(), true, false),
        (env.clone(), true, false),
        (env.join("profiles"), true, false),
        (env.join("local-profile.json"), false, false),
    ];
    if profile != env {
        result.push((profile.clone(), true, false));
    }
    for base in [&env, &profile] {
        for directory in ["run", "ai-runtime", "ai-runtime/runs"] {
            result.push((base.join(directory), true, false));
        }
        for file in [
            "contexthub.db",
            "contexthub.db-wal",
            "contexthub.db-shm",
            "workspaces.json",
            "run/instance.json",
            "run/instance.lock",
            "ai-runtime/history-migration-report.json",
        ] {
            result.push((base.join(file), false, false));
        }
        for home in ["cilai-harness", "cilai-application-harness"] {
            result.push((base.join("ai-runtime").join(home), true, true));
            // Database filenames are pinned to the bundled harness schema. Only
            // metadata permissions are touched; no SQL, row reads or migrations.
            for database in [
                "state_5.sqlite",
                "logs_2.sqlite",
                "goals_1.sqlite",
                "memories_1.sqlite",
                "queue_1.sqlite",
                "thread_history_1.sqlite",
            ] {
                for suffix in ["", "-wal", "-shm"] {
                    result.push((
                        base.join("ai-runtime")
                            .join(home)
                            .join(format!("{database}{suffix}")),
                        false,
                        false,
                    ));
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    result.retain(|(path, _, _)| seen.insert(path.clone()));
    result
}

fn collect(plan: &RepairPlan) -> io::Result<Vec<Target>> {
    let mut targets = Vec::new();
    // Keep ALL ancestor and object handles open before any ACL mutation.
    // This prevents replacing a traversed directory with a junction mid-repair.
    for (path, directory, runtime) in locations(plan) {
        // Pin every parent again for this target. A parent that was absent
        // earlier in the fixed list might otherwise appear as a junction before
        // its child is opened, bypassing final-component reparse checks.
        let opened = (|| {
            let ancestors = pin_chain(path.parent().ok_or_else(|| error(UNSAFE_OBJECT))?)?;
            let file = open(&path, directory, MAXIMUM_ALLOWED)?;
            Ok::<_, io::Error>((ancestors, file))
        })();
        match opened {
            Ok((ancestors, file)) => {
                let before = snapshot(&file)?;
                let actual_owner = owner(&before)?;
                if actual_owner != plan.user_sid && actual_owner != ADMINISTRATORS_SID {
                    return Err(error(FOREIGN_OWNER));
                }
                targets.push(Target {
                    path,
                    file,
                    _ancestors: ancestors,
                    before,
                    directory,
                    runtime,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(targets)
}

fn desired(target: &Target, sid: &str) -> String {
    if target.runtime {
        // Keep sandbox-specific rules on the runtime root. Its established
        // normal runtime preparation will handle inheritable access separately.
        let dacl = target
            .before
            .find("D:")
            .map_or(PRIVATE_DACL, |i| &target.before[i..]);
        let mut result = format!("O:{sid}{dacl}");
        for entry in [format!("(A;;FA;;;{sid})"), "(A;;FA;;;SY)".into()] {
            if !result.contains(&entry) {
                result.push_str(&entry);
            }
        }
        result
    } else {
        format!("O:{sid}{PRIVATE_DACL}")
    }
}

fn set_security(file: &File, sddl: &str) -> io::Result<()> {
    let text = wide(OsStr::new(sddl));
    let mut descriptor = ptr::null_mut();
    // SAFETY: live NUL-terminated input; allocated descriptor retained below.
    check(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            1,
            &raw mut descriptor,
            ptr::null_mut(),
        )
    })?;
    let _memory = LocalMemory(descriptor);
    let mut sid = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    // SAFETY: outputs borrow the retained, valid descriptor.
    check(unsafe { GetSecurityDescriptorOwner(descriptor, &raw mut sid, &raw mut defaulted) })?;
    // SAFETY: same retained descriptor and valid output storage.
    check(unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
            &raw mut present,
            &raw mut dacl,
            &raw mut defaulted,
        )
    })?;
    if sid.is_null() || present == 0 || dacl.is_null() {
        return Err(error(UNSAFE_OBJECT));
    }
    let protected = sddl
        .split("D:")
        .nth(1)
        .is_some_and(|part| part.split('(').next().unwrap_or_default().contains('P'));
    let protection = if protected {
        PROTECTED_DACL_SECURITY_INFORMATION
    } else {
        windows_sys::Win32::Security::UNPROTECTED_DACL_SECURITY_INFORMATION
    };
    let change_owner = owner(&snapshot(file)?)? != owner(sddl)?;
    // SAFETY: pinned, non-reparse, single-link object; MAXIMUM_ALLOWED suppresses
    // child propagation. Only owner and DACL change, never contents or group.
    win_result(unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            if change_owner {
                OWNER_SECURITY_INFORMATION
            } else {
                0
            } | DACL_SECURITY_INFORMATION
                | protection,
            if change_owner { sid } else { ptr::null_mut() },
            ptr::null_mut(),
            dacl,
            ptr::null_mut(),
        )
    })
}

fn acquire_locks(targets: &[Target]) -> io::Result<()> {
    for target in targets
        .iter()
        .filter(|target| target.path.file_name() == Some(OsStr::new("instance.lock")))
    {
        target.file.try_lock().map_err(|failure| match failure {
            std::fs::TryLockError::WouldBlock => error(BUSY),
            std::fs::TryLockError::Error(error) => error,
        })?;
    }
    Ok(())
}

fn verify(plan: &RepairPlan) -> io::Result<()> {
    for (path, directory, runtime) in locations(plan) {
        let file = match open(
            &path,
            directory,
            READ_CONTROL | FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC,
        ) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let sddl = snapshot(&file)?;
        if owner(&sddl)? != plan.user_sid {
            return Err(error(VERIFICATION));
        }
        if !runtime {
            crate::security::verify_current_user_private_file(&path)?;
        }
    }
    Ok(())
}

fn apply(request: &Request) -> io::Result<()> {
    let plan = &request.plan;
    let _pins = pin_chain(Path::new(&plan.local_data))?;
    if anchors(plan)? != plan.anchors {
        return Err(conflict(Conflict::DirectoryChanged));
    }
    let targets = collect(plan)?;
    // The first anchor check alone has a gap before collect pins the objects.
    // Recheck while all target/ancestor handles now prevent replacement.
    if anchors(plan)? != plan.anchors {
        return Err(conflict(Conflict::DirectoryChanged));
    }
    acquire_locks(&targets)?;
    for data in [root(plan), profile(plan)] {
        let _runtime = match open(&data.join("run"), true, 0) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let endpoint = contexthub_local_transport::Endpoint::resolve(&data.join("run"))?;
        if contexthub_local_transport::artifact_exists(&endpoint)? {
            return Err(error(BUSY));
        }
    }
    // Probe write rights before writing a backup or changing any object, so a
    // normal attempt can safely request UAC without partial ACL updates.
    for target in &targets {
        let _probe = open(
            &target.path,
            target.directory,
            READ_CONTROL
                | WRITE_DAC
                | if owner(&target.before)? == plan.user_sid {
                    0
                } else {
                    WRITE_OWNER
                },
        )?;
    }
    let backup = backup_path(request)?;
    let _backup_parent = pin_chain(backup.parent().unwrap())?;
    let mut journal_file = journal::open_for_write(request)?;
    let mut journal = RepairJournal::new(request)?;
    journal::save(&mut journal_file, &journal)?;
    let mut file = open(&backup, false, FILE_GENERIC_WRITE | READ_CONTROL)?;
    if identity(&file)? != request.backup_identity || owner(&snapshot(&file)?)? != plan.user_sid {
        return Err(conflict(Conflict::BackupChanged));
    }
    let records = targets
        .iter()
        .map(|target| serde_json::json!({"path":target.path,"securityDescriptor":target.before}))
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({"schemaVersion":1,"scope":"startup-metadata-only","userSid":plan.user_sid,"createdAtEpochSeconds":now(),"objects":records})).map_err(io::Error::other)?;
    file.set_len(0)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    let result = apply_changes(&targets, &plan.user_sid, &mut journal);
    // The original permission backup is never rewritten with operation results.
    // A reporting failure must not hide the actual repair/rollback failure.
    let saved = journal::save(&mut journal_file, &journal);
    match result {
        Err(error) => {
            if saved.is_err() {
                eprintln!("startup_repair_journal_write_failed");
            }
            Err(error)
        }
        Ok(()) => saved,
    }
}

fn restore(targets: &[Target]) -> bool {
    let mut failed = false;
    for target in targets.iter().rev() {
        failed |= !matches!(set_security(&target.file, &target.before)
            .and_then(|()| snapshot(&target.file)), Ok(restored) if restored == target.before);
    }
    failed
}

fn apply_changes(targets: &[Target], sid: &str, journal: &mut RepairJournal) -> io::Result<()> {
    apply_changes_with_snapshot(targets, sid, journal, snapshot)
}

fn apply_changes_with_snapshot(
    targets: &[Target],
    sid: &str,
    journal: &mut RepairJournal,
    mut read_before_write: impl FnMut(&File) -> io::Result<String>,
) -> io::Result<()> {
    for (index, target) in targets.iter().enumerate() {
        journal.target(index, target);
        let conflict = match read_before_write(&target.file) {
            Ok(current) if current == target.before => None,
            Ok(current) if legacy_conversion_after_parent(targets, index, &current) => {
                journal.inherited_source_conversion(index);
                None
            }
            Ok(current) => {
                journal.difference(&target.before, &current);
                Some(conflict(Conflict::PermissionsChanged))
            }
            Err(error) => Some(error),
        };
        if let Some(failure) = conflict {
            let rollback_failed = restore(&targets[..index]);
            journal.failed(&failure, !rollback_failed);
            return Err(if rollback_failed {
                error(ROLLBACK)
            } else {
                failure
            });
        }
        let result = (|| {
            journal.summary.phase = super::RepairPhase::Write;
            set_security(&target.file, &desired(target, sid))?;
            journal.summary.phase = super::RepairPhase::Verify;
            let after = snapshot(&target.file)?;
            if owner(&after)? != sid {
                return Err(error(VERIFICATION));
            }
            Ok(())
        })();
        if let Err(original) = result {
            let rollback_failed = restore(&targets[..=index]);
            journal.failed(&original, !rollback_failed);
            return Err(if rollback_failed {
                error(ROLLBACK)
            } else {
                original
            });
        }
        journal.summary.previous_applied_index = Some(index);
    }
    journal.summary.phase = super::RepairPhase::Complete;
    Ok(())
}

fn legacy_conversion_after_parent(targets: &[Target], index: usize, current: &str) -> bool {
    let target = &targets[index];
    target.directory
        && !target.runtime
        // Only the immediate parent, already successfully made private in this
        // attempt. Do not accept a change on the first target, a sibling, a file
        // or a runtime root whose existing sandbox ACL must be preserved.
        && targets[..index].iter().any(|parent| {
            parent.directory && !parent.runtime && target.path.parent() == Some(parent.path.as_path())
        })
        && legacy::inherited_source_removed(&target.before, current)
}

pub(super) fn repair(plan: &RepairPlan, operation_id: &str) -> io::Result<RepairResult> {
    let token = current_token()?;
    if plan.user_sid != user_sid(&token)? || plan.parent_pid != std::process::id() {
        return Err(conflict(Conflict::CallerChanged));
    }
    validate(plan, &local_data(None)?)?;
    if anchors(plan)? != plan.anchors {
        return Err(conflict(Conflict::DirectoryChanged));
    }
    let directory = layout::recovery(Path::new(&plan.local_data));
    if directory.exists() {
        crate::security::verify_current_user_private_file(&directory)?;
    }
    crate::security::ensure_restricted_directory(&directory)?;
    let _pins = pin_chain(&directory)?;
    let mut request = Request {
        plan: plan.clone(),
        backup_name: format!("{operation_id}.json"),
        backup_identity: String::new(),
        journal_identity: String::new(),
    };
    let path = backup_path(&request)?;
    let file = crate::security::create_new_restricted_file(&path)?;
    request.backup_identity = identity(&file)?;
    let journal_file = crate::security::create_new_restricted_file(&journal::path(&request)?)?;
    request.journal_identity = identity(&journal_file)?;
    // An already elevated host must not enable backup/restore privileges in
    // its long-lived process. Use the same short-lived, identity-bound helper;
    // Windows does not require a second UAC prompt for an elevated parent.
    if elevated(&token)? {
        native::launch_helper(HELPER_ARGUMENT, &encode(&request)?)?;
    } else {
        match apply(&request) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(5) || e.raw_os_error() == Some(1314) => {
                native::launch_helper(HELPER_ARGUMENT, &encode(&request)?)?;
            }
            Err(e) => return Err(e),
        }
    }
    verify(plan)?;
    Ok(RepairResult {
        backup_path: path.to_string_lossy().into_owned(),
    })
}

fn set_privilege(name: &str, enabled: bool) -> io::Result<()> {
    let mut raw = ptr::null_mut();
    // SAFETY: current-process pseudo handle and valid output; RAII owns real token.
    check(unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES,
            &raw mut raw,
        )
    })?;
    let token = Handle(raw);
    let mut luid = LUID::default();
    let name = wide(OsStr::new(name));
    // SAFETY: fixed privilege string, valid writable LUID.
    check(unsafe { LookupPrivilegeValueW(ptr::null(), name.as_ptr(), &raw mut luid) })?;
    let state = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: if enabled { SE_PRIVILEGE_ENABLED } else { 0 },
        }],
    };
    // SAFETY: one-entry privilege structure lives through the call; no prior state requested.
    check(unsafe {
        AdjustTokenPrivileges(
            token.0,
            0,
            &raw const state,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    })?;
    // SAFETY: immediately read documented last-error for partial assignment.
    win_result(unsafe { GetLastError() })
}

pub(super) fn helper(arguments: &[String]) -> io::Result<()> {
    if arguments.len() != 2 || !elevated(&current_token()?)? {
        return Err(conflict(Conflict::InvalidRequest));
    }
    let request = decode(&arguments[1])?;
    let plan = &request.plan;
    // OpenProcessToken requires SeDebugPrivilege for a different account.
    // Use it only while verifying/opening the original host token, never while
    // applying ACL changes. Every early error exits this dedicated process.
    let different_admin = user_sid(&current_token()?)? != plan.user_sid;
    if different_admin {
        set_privilege("SeDebugPrivilege", true)?;
    }
    let parent =
        native::verified_startup_parent(plan.parent_pid, &plan.parent_fingerprint, &plan.user_sid)
            .map_err(|error| {
                if error.raw_os_error() == Some(1237) {
                    conflict(Conflict::ParentChanged)
                } else {
                    error
                }
            })?;
    let mut raw = ptr::null_mut();
    // SAFETY: verified live parent; query rights needed by the known-folder API.
    check(unsafe {
        OpenProcessToken(
            parent.0,
            TOKEN_QUERY | TOKEN_IMPERSONATE | TOKEN_DUPLICATE,
            &raw mut raw,
        )
    })?;
    let parent_token = Handle(raw);
    if different_admin {
        set_privilege("SeDebugPrivilege", false)?;
    }
    validate(plan, &local_data(Some(&parent_token))?)?;
    // Limited to this dedicated short-lived process, never the workbench/model.
    set_privilege("SeBackupPrivilege", true)?;
    set_privilege("SeRestorePrivilege", true)?;
    apply(&request)
}

#[cfg(test)]
mod tests;
