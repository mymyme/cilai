use super::{Change, ChangePlan, Engine, EngineState, HELPER_ARGUMENT, Policy, Scopes};
use crate::workspace_permissions::windows as native;
use native::{Handle, changed, current_token, elevated, failed, information, now, user_sid};
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Read as _};
use std::os::windows::fs::OpenOptionsExt as _;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr;
use std::time::{Duration, Instant};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA, WRITE_DAC, WRITE_OWNER,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramFiles, FOLDERID_System, SHGetKnownFolderPath};

struct Installation {
    root: PathBuf,
    executable: PathBuf,
    _pins: Vec<File>,
    identity: String,
}

fn known_folder(id: &windows_sys::core::GUID) -> io::Result<PathBuf> {
    let mut value = ptr::null_mut();
    // SAFETY: fixed system folder GUID, no user-supplied token or output path.
    let result = unsafe { SHGetKnownFolderPath(id, 0, ptr::null_mut(), &raw mut value) };
    let path = if result >= 0 && !value.is_null() {
        Some(PathBuf::from(native::local_string(value)))
    } else {
        None
    };
    // SAFETY: shell allocation is released once, including the permitted NULL case.
    unsafe { CoTaskMemFree(value.cast()) };
    path.ok_or_else(|| failed("powershell_policy_installation_unavailable"))
}

fn open(path: &Path, executable: bool) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(if executable {
            FILE_SHARE_READ
        } else {
            FILE_SHARE_READ | FILE_SHARE_WRITE
        })
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let info = information(&file)?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(failed("powershell_policy_unsafe_installation"));
    }
    Ok(file)
}
fn not_writable(path: &Path) -> io::Result<()> {
    let mut rights = vec![
        FILE_WRITE_DATA,
        FILE_WRITE_ATTRIBUTES,
        FILE_WRITE_EA,
        DELETE,
        WRITE_DAC,
        WRITE_OWNER,
    ];
    if path.is_dir() {
        rights.push(FILE_DELETE_CHILD);
    }
    for access in rights {
        match OpenOptions::new()
            .access_mode(access)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
        {
            Ok(_) => return Err(failed("powershell_policy_unsafe_installation")),
            Err(e) if e.raw_os_error() == Some(5) => {}
            Err(e) if e.raw_os_error() == Some(32) => {
                return Err(failed("powershell_policy_installation_busy"));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn installation(engine: Engine) -> io::Result<Installation> {
    let (root, executable) = match engine {
        Engine::WindowsPowerShell => {
            let root = known_folder(&FOLDERID_System)?;
            let exe = root.join("WindowsPowerShell/v1.0/powershell.exe");
            (root, exe)
        }
        Engine::PowerShell7 => {
            let root = known_folder(&FOLDERID_ProgramFiles)?;
            let exe = root.join("PowerShell/7/pwsh.exe");
            (root, exe)
        }
    };
    let mut pins = Vec::new();
    let relative = executable.strip_prefix(&root).map_err(|_| changed())?;
    let mut path = root.clone();
    pins.push(open(&path, false)?);
    not_writable(&path)?;
    for component in relative.components() {
        path.push(component);
        not_writable(&path)?;
        pins.push(open(&path, path == executable)?);
    }
    // PowerShell 7 takes an exclusive lock. Validate then release its config
    // handle; the pinned, non-user-writable parent and denied file DELETE/
    // WRITE_DAC rights prevent a normal user from replacing it.
    let config = executable
        .parent()
        .ok_or_else(changed)?
        .join("powershell.config.json");
    if config.try_exists()? {
        regular_config(&config)?;
        not_writable(&config)?;
    }
    let mut file = File::open(&executable)?;
    let mut digest = Sha256::new();
    let mut bytes = [0_u8; 8192];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(Installation {
        root,
        executable,
        _pins: pins,
        identity: format!("{:x}", digest.finalize()),
    })
}

fn regular_config(path: &Path) -> io::Result<()> {
    let file = open(path, false)?;
    let info = information(&file)?;
    if info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY) != 0
        || info.nNumberOfLinks != 1
    {
        return Err(failed("powershell_policy_unsafe_installation"));
    }
    Ok(())
}

fn encoded(script: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let mut output = String::new();
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or_default();
        let c = chunk.get(2).copied().unwrap_or_default();
        output.push(ALPHABET[(a >> 2) as usize] as char);
        output.push(ALPHABET[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}
fn run(installation: &Installation, script: &str) -> io::Result<String> {
    let system = known_folder(&FOLDERID_System)?;
    let windows = system.parent().ok_or_else(changed)?;
    let mut child = Command::new(&installation.executable)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encoded(script),
        ])
        .env_clear()
        .env("SystemRoot", windows)
        .env("WINDIR", windows)
        .env("PATH", &system)
        .env(
            "PSModulePath",
            installation
                .executable
                .parent()
                .ok_or_else(changed)?
                .join("Modules"),
        )
        .current_dir(&installation.root)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(failed("powershell_policy_query_timed_out"));
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(failed("powershell_policy_command_failed"));
    }
    String::from_utf8(output.stdout)
        .map(|s| s.trim_start_matches('\u{feff}').trim().to_owned())
        .map_err(io::Error::other)
}
const READ_SCRIPT: &str = r"$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new();
$s=@{}; foreach($p in (Microsoft.PowerShell.Security\Get-ExecutionPolicy -List)){$s[$p.Scope.ToString()]=$p.ExecutionPolicy.ToString()};
@{machinePolicy=$s.MachinePolicy;userPolicy=$s.UserPolicy;currentUser=$s.CurrentUser;localMachine=$s.LocalMachine;effective=(Microsoft.PowerShell.Security\Get-ExecutionPolicy).ToString()} | Microsoft.PowerShell.Utility\ConvertTo-Json -Compress";
fn scopes(installation: &Installation) -> io::Result<Scopes> {
    serde_json::from_str(&run(installation, READ_SCRIPT)?).map_err(io::Error::other)
}
fn write(installation: &Installation, policy: Policy) -> io::Result<()> {
    if !matches!(
        policy,
        Policy::RemoteSigned | Policy::Restricted | Policy::AllSigned | Policy::Undefined
    ) {
        return Err(changed());
    }
    // Policy is an enum, never arbitrary code. An override warning may follow a
    // successful LocalMachine write; verify that exact scope in the catch block.
    let script = format!(
        r"$ErrorActionPreference='Stop'; try {{ Microsoft.PowerShell.Security\Set-ExecutionPolicy -Scope LocalMachine -ExecutionPolicy {policy:?} -Force }} catch {{ if ((Microsoft.PowerShell.Security\Get-ExecutionPolicy -Scope LocalMachine).ToString() -ne '{policy:?}') {{ throw }} }}"
    );
    run(installation, &script).map(|_| ())
}

pub(super) fn inspect() -> io::Result<Vec<EngineState>> {
    let mut states = Vec::new();
    for engine in [Engine::WindowsPowerShell, Engine::PowerShell7] {
        let installation = match native::with_current_user_access_checks(|| installation(engine)) {
            Ok(value) => value,
            Err(e) if engine == Engine::PowerShell7 && e.kind() == io::ErrorKind::NotFound => {
                continue;
            }
            Err(e) => return Err(e),
        };
        states.push(EngineState {
            engine,
            path: installation.executable.to_string_lossy().into_owned(),
            identity: installation.identity.clone(),
            scopes: scopes(&installation)?,
        });
    }
    Ok(states)
}
pub(super) fn plan(changes: Vec<Change>) -> io::Result<ChangePlan> {
    let token = current_token()?;
    Ok(ChangePlan {
        changes,
        parent_pid: std::process::id(),
        parent_fingerprint: crate::instance::process_start_fingerprint(std::process::id())?
            .ok_or_else(changed)?,
        user_sid: user_sid(&token)?,
        expires_at: now() + 300,
    })
}
pub(super) fn apply(plan: &ChangePlan) -> io::Result<Vec<EngineState>> {
    if plan.parent_pid != std::process::id() || plan.user_sid != user_sid(&current_token()?)? {
        return Err(changed());
    }
    let states = inspect()?;
    for change in &plan.changes {
        let state = states
            .iter()
            .find(|state| state.engine == change.engine)
            .ok_or_else(changed)?;
        if state.identity != change.identity
            || state.scopes.managed()
            || state.scopes.local_machine != change.before
        {
            return Err(changed());
        }
    }
    let bytes = serde_json::to_vec(plan).map_err(io::Error::other)?;
    let mut payload = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut payload, "{byte:02x}").map_err(io::Error::other)?;
    }
    native::launch_helper(HELPER_ARGUMENT, &payload)?;
    let after = inspect()?;
    if plan.changes.iter().any(|change| {
        !after.iter().any(|s| {
            s.engine == change.engine
                && s.identity == change.identity
                && s.scopes.local_machine == change.after
                && !s.scopes.managed()
        })
    }) {
        return Err(failed("powershell_policy_verification_failed"));
    }
    Ok(after)
}

fn trusted_for_parent(parent: &Handle, engine: Engine) -> io::Result<Installation> {
    native::with_user_access_checks(parent.0, || installation(engine))
}
fn valid_plan(plan: &ChangePlan) -> bool {
    plan.expires_at >= now()
        && plan.expires_at <= now() + 300
        && !plan.changes.is_empty()
        && plan.changes.len() <= 2
        && plan.changes.iter().enumerate().all(|(index, c)| {
            !plan.changes[..index]
                .iter()
                .any(|old| old.engine == c.engine)
                && ((!c.before.permits_local_scripts() && c.after == Policy::RemoteSigned)
                    || (c.before == Policy::RemoteSigned && !c.after.permits_local_scripts()))
        })
}
pub(super) fn helper(arguments: &[String]) -> io::Result<()> {
    if arguments.len() != 2
        || arguments[1].len() > 12000
        || !arguments[1].len().is_multiple_of(2)
        || !arguments[1].bytes().all(|b| b.is_ascii_hexdigit())
        || !elevated(&current_token()?)?
    {
        return Err(changed());
    }
    let bytes = (0..arguments[1].len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&arguments[1][i..i + 2], 16).map_err(io::Error::other))
        .collect::<io::Result<Vec<_>>>()?;
    let plan: ChangePlan = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if !valid_plan(&plan) {
        return Err(changed());
    }
    let parent =
        native::verified_parent(plan.parent_pid, &plan.parent_fingerprint, &plan.user_sid)?;
    let installs = plan
        .changes
        .iter()
        .map(|c| trusted_for_parent(&parent, c.engine))
        .collect::<io::Result<Vec<_>>>()?;
    for (install, change) in installs.iter().zip(&plan.changes) {
        let state = scopes(install)?;
        if install.identity != change.identity
            || state.machine_policy != Policy::Undefined
            || state.local_machine != change.before
        {
            return Err(changed());
        }
    }
    super::transaction(
        &plan.changes,
        |index| {
            let state = scopes(&installs[index])?;
            if state.machine_policy != Policy::Undefined {
                return Err(changed());
            }
            Ok(state.local_machine)
        },
        |index, policy| write(&installs[index], policy),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    #[test]
    fn config_check_rejects_hardlinks_and_does_not_block_powershell_exclusive_write() {
        let path = std::env::temp_dir().join(format!(
            "cilai-policy-lock-{}-{}.json",
            std::process::id(),
            now()
        ));
        std::fs::write(&path, b"{}").unwrap();
        regular_config(&path).unwrap();
        let link = path.with_extension("hardlink");
        std::fs::hard_link(&path, &link).unwrap();
        assert!(regular_config(&path).is_err());
        std::fs::remove_file(link).unwrap();
        let mut writer = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert_eq!(
            not_writable(&path).unwrap_err().to_string(),
            "powershell_policy_installation_busy"
        );
        writer.write_all(b"{}").unwrap();
        drop(writer);
        assert_eq!(
            not_writable(&path).unwrap_err().to_string(),
            "powershell_policy_unsafe_installation"
        );
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn invalid_helper_and_read_only_probe_never_change_machine_policy() {
        assert!(helper(&[HELPER_ARGUMENT.into(), "00".into()]).is_err());
        let before = inspect().unwrap();
        let after = inspect().unwrap();
        assert_eq!(before, after);
        assert!(before.iter().any(|s| s.engine == Engine::WindowsPowerShell));
    }
    #[test]
    fn helper_plan_rejects_permissive_targets_duplicates_and_expiry() {
        let change = Change {
            engine: Engine::WindowsPowerShell,
            identity: "same".into(),
            before: Policy::Restricted,
            after: Policy::RemoteSigned,
        };
        let mut p = ChangePlan {
            changes: vec![change.clone()],
            parent_pid: 1,
            parent_fingerprint: "x".into(),
            user_sid: "x".into(),
            expires_at: now() + 100,
        };
        assert!(valid_plan(&p));
        p.changes[0].after = Policy::Bypass;
        assert!(!valid_plan(&p));
        p.changes = vec![change.clone(), change];
        assert!(!valid_plan(&p));
        p.changes.truncate(1);
        p.expires_at = 0;
        assert!(!valid_plan(&p));
    }
}
