//! Protect original runtime data and shell startup files before official code executes.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::OsString;
#[cfg(target_os = "macos")]
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const PROTECTION_ENV: &str = "CILAI_PERSONAL_ORIGINAL_HOME_PROTECTION";
const PROTECTED_HOME_ENTRIES: [&str; 11] = [
    ".codex",
    ".agents",
    ".zshenv",
    ".zprofile",
    ".zshrc",
    ".zlogin",
    ".zlogout",
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".profile",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatgptIsolationSnapshot {
    home: PathBuf,
    protected_roots: Vec<PathBuf>,
    file_access: Option<FileAccessSnapshot>,
    digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileAccessSnapshot {
    read_only: Vec<AccessRoot>,
    writable: Vec<AccessRoot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AccessRoot {
    path: PathBuf,
    resolved: PathBuf,
    directory: bool,
}

impl AccessRoot {
    fn capture(path: &Path) -> io::Result<Self> {
        if !path.is_absolute()
            || path.parent().is_none()
            || path
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(invalid("model_runtime_home_permissions_invalid"));
        }
        checked_path(path)?;
        let resolved = path
            .canonicalize()
            .map_err(|_| invalid("model_runtime_home_permissions_metadata_failed"))?;
        if resolved.parent().is_none() {
            return Err(invalid("model_runtime_home_permissions_invalid"));
        }
        checked_path(&resolved)?;
        let metadata = fs::metadata(&resolved)
            .map_err(|_| invalid("model_runtime_home_permissions_metadata_failed"))?;
        if !metadata.is_dir() && !metadata.is_file() {
            return Err(invalid("model_runtime_home_permissions_invalid"));
        }
        Ok(Self {
            path: path.to_owned(),
            resolved,
            directory: metadata.is_dir(),
        })
    }
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, reason)
}

impl ChatgptIsolationSnapshot {
    /// Capture trusted OS account metadata. Does not read any protected file contents.
    /// # Errors
    /// Missing/unsafe home metadata or unresolved protected symlinks reject the launch.
    pub fn capture() -> io::Result<Option<Self>> {
        #[cfg(target_os = "macos")]
        return platform::capture();
        #[cfg(any(
            windows,
            target_os = "linux",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "android",
            target_os = "ios"
        ))]
        Ok(None)
    }

    fn from_home(home: &Path) -> io::Result<Self> {
        if !home.is_absolute()
            || home.parent().is_none()
            || home
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(invalid("model_runtime_home_isolation_invalid"));
        }
        checked_path(home)?;
        let canonical_home = home
            .canonicalize()
            .map_err(|_| invalid("model_runtime_home_isolation_metadata_failed"))?;
        if !canonical_home.is_dir() {
            return Err(invalid("model_runtime_home_isolation_invalid"));
        }
        let mut roots = BTreeSet::new();
        for folder in PROTECTED_HOME_ENTRIES {
            roots.insert(home.join(folder));
            let canonical_parent = canonical_home.join(folder);
            roots.insert(canonical_parent.clone());
            match fs::symlink_metadata(&canonical_parent) {
                Ok(_) => {
                    let target = canonical_parent
                        .canonicalize()
                        .map_err(|_| invalid("model_runtime_home_isolation_metadata_failed"))?;
                    if target.parent().is_none() || target == canonical_home {
                        return Err(invalid("model_runtime_home_isolation_invalid"));
                    }
                    checked_path(&target)?;
                    roots.insert(target);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(_) => return Err(invalid("model_runtime_home_isolation_metadata_failed")),
            }
        }
        let protected_roots: Vec<_> = roots.into_iter().collect();
        let mut snapshot = Self {
            home: home.to_owned(),
            protected_roots,
            file_access: None,
            digest: String::new(),
        };
        snapshot.refresh_digest()?;
        Ok(snapshot)
    }

    /// Freeze the exact file access granted to this official process.
    /// # Errors
    /// Missing, ambiguous or non-file paths reject the launch.
    pub fn with_file_access(
        &self,
        read_only: &[PathBuf],
        writable: &[PathBuf],
    ) -> io::Result<Self> {
        fn capture(paths: &[PathBuf]) -> io::Result<Vec<AccessRoot>> {
            paths
                .iter()
                .map(|path| AccessRoot::capture(path))
                .collect::<io::Result<BTreeSet<_>>>()
                .map(|roots| roots.into_iter().collect())
        }
        let mut snapshot = self.clone();
        let access = FileAccessSnapshot {
            read_only: capture(read_only)?,
            writable: capture(writable)?,
        };
        if access
            .read_only
            .iter()
            .chain(&access.writable)
            .any(|grant| {
                self.protected_roots.iter().any(|protected| {
                    grant.path.starts_with(protected) || grant.resolved.starts_with(protected)
                })
            })
        {
            return Err(invalid("model_runtime_home_permissions_protected"));
        }
        snapshot.file_access = Some(access);
        snapshot.refresh_digest()?;
        Ok(snapshot)
    }

    fn refresh_digest(&mut self) -> io::Result<()> {
        let encoded = serde_json::to_vec(&(
            "closed-home-v1",
            &self.home,
            &self.protected_roots,
            &self.file_access,
        ))
        .map_err(|_| invalid("model_runtime_home_isolation_invalid"))?;
        self.digest = format!("{:x}", Sha256::digest(encoded));
        Ok(())
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    #[must_use]
    pub fn protected_roots(&self) -> &[PathBuf] {
        &self.protected_roots
    }

    /// # Errors
    /// A protection snapshot without frozen file grants cannot launch.
    pub fn apply_environment(&self, command: &mut Command) -> io::Result<()> {
        if self.file_access.is_none() {
            return Err(invalid("model_runtime_home_permissions_required"));
        }
        let payload = serde_json::to_string(self)
            .map_err(|_| invalid("model_runtime_home_isolation_invalid"))?;
        command
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env(PROTECTION_ENV, payload);
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn profile(&self, runtime_root: Option<&Path>) -> io::Result<String> {
        let access = self
            .file_access
            .as_ref()
            .ok_or_else(|| invalid("model_runtime_home_permissions_required"))?;
        let mut read = system_read_roots()?;
        read.extend(access.read_only.iter().chain(&access.writable).cloned());
        if let Some(root) = runtime_root {
            read.push(AccessRoot::capture(root)?);
        }
        let mut policy = String::from("(version 1)\n(allow default)\n");
        append_access_deny(&mut policy, "file-read-data file-read-xattr", &read, true)?;
        append_access_deny(&mut policy, "file-write*", &access.writable, false)?;
        policy.push_str("(deny file-read* file-write*");
        for path in &self.protected_roots {
            let quoted = serde_json::to_string(checked_path(path)?)
                .map_err(|_| invalid("model_runtime_home_isolation_invalid"))?;
            write!(policy, "\n (literal {quoted}) (subpath {quoted})")
                .map_err(|_| invalid("model_runtime_home_isolation_invalid"))?;
        }
        policy.push_str(")\n");
        Ok(policy)
    }
}

#[cfg(target_os = "macos")]
fn system_read_roots() -> io::Result<Vec<AccessRoot>> {
    [
        "/System",
        "/usr",
        "/bin",
        "/sbin",
        "/private/etc",
        "/dev",
        "/Library/Apple",
        "/Library/Preferences",
        "/private/var/db/timezone",
        "/private/var/db/dyld",
        "/private/var/run",
        "/private/var/select",
    ]
    .into_iter()
    .filter(|path| Path::new(path).exists())
    .map(|path| AccessRoot::capture(Path::new(path)))
    .collect()
}

#[cfg(target_os = "macos")]
fn append_access_deny(
    policy: &mut String,
    operations: &str,
    roots: &[AccessRoot],
    read: bool,
) -> io::Result<()> {
    write!(policy, "(deny {operations} (require-not (require-any")
        .map_err(|_| invalid("model_runtime_home_isolation_invalid"))?;
    for root in roots {
        for path in [&root.path, &root.resolved] {
            let quoted = serde_json::to_string(checked_path(path)?)
                .map_err(|_| invalid("model_runtime_home_isolation_invalid"))?;
            let filter = if root.directory { "subpath" } else { "literal" };
            write!(policy, "\n ({filter} {quoted})")
                .map_err(|_| invalid("model_runtime_home_isolation_invalid"))?;
        }
    }
    if read {
        policy.push_str("\n (literal \"/\")");
    } else {
        policy.push_str("\n (literal \"/dev/null\") (subpath \"/dev/fd\")");
    }
    policy.push_str(")))\n");
    Ok(())
}

fn checked_path(path: &Path) -> io::Result<&str> {
    path.to_str()
        .filter(|value| !value.chars().any(char::is_control))
        .ok_or_else(|| invalid("model_runtime_home_isolation_invalid"))
}

pub(crate) fn protection_requested() -> bool {
    std::env::var_os(PROTECTION_ENV).is_some()
}

pub(crate) fn official_command(target: &Path, arguments: &[OsString]) -> io::Result<Command> {
    let Some(payload) = std::env::var_os(PROTECTION_ENV) else {
        let mut command = Command::new(target);
        command.args(arguments);
        return Ok(command);
    };
    let payload = payload
        .to_str()
        .filter(|value| value.len() <= 131_072)
        .ok_or_else(|| invalid("model_runtime_home_isolation_marker_invalid"))?;
    let supplied: ChatgptIsolationSnapshot = serde_json::from_str(payload)
        .map_err(|_| invalid("model_runtime_home_isolation_marker_invalid"))?;
    let home =
        std::env::var_os("HOME").ok_or_else(|| invalid("model_runtime_home_isolation_invalid"))?;
    if Path::new(&home) != supplied.home {
        return Err(invalid("model_runtime_home_isolation_changed"));
    }
    let access = supplied
        .file_access
        .as_ref()
        .ok_or_else(|| invalid("model_runtime_home_permissions_required"))?;
    let read_only = access
        .read_only
        .iter()
        .map(|root| root.path.clone())
        .collect::<Vec<_>>();
    let writable = access
        .writable
        .iter()
        .map(|root| root.path.clone())
        .collect::<Vec<_>>();
    let snapshot = ChatgptIsolationSnapshot::from_home(Path::new(&home))?
        .with_file_access(&read_only, &writable)?;
    if snapshot != supplied {
        return Err(invalid("model_runtime_home_isolation_changed"));
    }
    platform::command(&snapshot, target, arguments)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{
        ChatgptIsolationSnapshot, Command, OsString, PROTECTION_ENV, Path, PathBuf, invalid, io,
    };
    pub(super) fn capture() -> io::Result<Option<ChatgptIsolationSnapshot>> {
        let user = objc2_foundation::NSUserName();
        let home = objc2_foundation::NSHomeDirectoryForUser(Some(&user))
            .ok_or_else(|| invalid("model_runtime_home_isolation_metadata_failed"))?;
        ChatgptIsolationSnapshot::from_home(&PathBuf::from(home.to_string())).map(Some)
    }
    pub(super) fn command(
        snapshot: &ChatgptIsolationSnapshot,
        target: &Path,
        arguments: &[OsString],
    ) -> io::Result<Command> {
        let mut command = Command::new("/usr/bin/sandbox-exec");
        command
            .args([
                "-p",
                &snapshot.profile(target.parent().and_then(Path::parent))?,
            ])
            .arg(target)
            .args(arguments)
            .env_remove(PROTECTION_ENV);
        Ok(command)
    }
}

#[cfg(any(
    windows,
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "android",
    target_os = "ios"
))]
mod platform {
    use super::{ChatgptIsolationSnapshot, Command, OsString, Path, invalid, io};
    pub(super) fn command(
        _: &ChatgptIsolationSnapshot,
        _: &Path,
        _: &[OsString],
    ) -> io::Result<Command> {
        Err(invalid("model_runtime_home_isolation_platform_invalid"))
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    static FIXTURE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let project = Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(4)
                .unwrap();
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = project
                .join(".personal-build/runtime-isolation-fixtures")
                .join(format!(
                    "mac-home-{}-{nonce}-{}",
                    std::process::id(),
                    FIXTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
            fs::create_dir_all(root.join("home")).unwrap();
            Self(root)
        }
        fn home(&self) -> PathBuf {
            self.0.join("home")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn frozen_protection_contains_both_roots_and_never_reads_file_contents() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.home().join(".codex")).unwrap();
        fs::write(fixture.home().join(".codex/auth.json"), "synthetic-bait").unwrap();
        let snapshot = ChatgptIsolationSnapshot::from_home(&fixture.home()).unwrap();
        assert!(
            snapshot
                .protected_roots()
                .contains(&fixture.home().join(".codex"))
        );
        assert!(
            snapshot
                .protected_roots()
                .contains(&fixture.home().join(".agents"))
        );
        for entry in PROTECTED_HOME_ENTRIES {
            assert!(
                snapshot
                    .protected_roots()
                    .contains(&fixture.home().join(entry))
            );
        }
        assert!(snapshot.profile(None).is_err());
        assert!(
            snapshot
                .apply_environment(&mut Command::new("unused-fixture-command"))
                .is_err()
        );
        let snapshot = snapshot
            .with_file_access(&[], std::slice::from_ref(&fixture.0))
            .unwrap();
        assert!(!snapshot.profile(None).unwrap().contains("synthetic-bait"));
        assert_eq!(snapshot.digest().len(), 64);
        let mut command = Command::new("unused-fixture-command");
        snapshot.apply_environment(&mut command).unwrap();
        assert!(command.get_envs().any(|(key, value)| key == PROTECTION_ENV
            && value.is_some_and(|payload| payload.to_string_lossy().contains(snapshot.digest()))));
    }
    #[test]
    fn ambiguous_protection_paths_are_errors() {
        for home in [
            Path::new("/"),
            Path::new("relative"),
            Path::new("/tmp/../other"),
            Path::new("/tmp/bad\nname"),
        ] {
            assert!(ChatgptIsolationSnapshot::from_home(home).is_err());
        }
    }

    #[test]
    fn file_grants_are_explicit_and_change_the_snapshot_digest() {
        let fixture = Fixture::new();
        let snapshot = ChatgptIsolationSnapshot::from_home(&fixture.home()).unwrap();
        assert!(
            snapshot
                .with_file_access(&[PathBuf::from("/")], &[])
                .is_err()
        );
        assert!(
            snapshot
                .with_file_access(&[fixture.0.join("missing")], &[])
                .is_err()
        );
        fs::create_dir_all(fixture.home().join(".codex")).unwrap();
        assert!(
            snapshot
                .with_file_access(&[fixture.home().join(".codex")], &[])
                .is_err()
        );
        let read_only = snapshot
            .with_file_access(std::slice::from_ref(&fixture.0), &[])
            .unwrap();
        let writable = snapshot
            .with_file_access(&[], std::slice::from_ref(&fixture.0))
            .unwrap();
        assert_ne!(snapshot.digest(), read_only.digest());
        assert_ne!(read_only.digest(), writable.digest());
    }

    #[cfg(target_os = "macos")]
    fn bait(fixture: &Fixture) -> (Vec<PathBuf>, PathBuf) {
        use std::os::unix::fs::symlink;
        let external = fixture.0.join("original-codex-target");
        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("auth.json"), "synthetic-codex-bait").unwrap();
        symlink(&external, fixture.home().join(".codex")).unwrap();
        let skill = fixture.home().join(".agents/skills/bait/SKILL.md");
        fs::create_dir_all(skill.parent().unwrap()).unwrap();
        fs::write(&skill, "synthetic-skill-bait").unwrap();
        symlink(&external, fixture.0.join("outside-alias")).unwrap();
        let allowed = fixture.0.join("personal-runtime/config.toml");
        fs::create_dir_all(allowed.parent().unwrap()).unwrap();
        fs::write(&allowed, "isolated-config").unwrap();
        let mut paths = vec![
            fixture.home().join(".codex/auth.json"),
            external.join("auth.json"),
            fixture.0.join("outside-alias/auth.json"),
            skill,
        ];
        for entry in PROTECTED_HOME_ENTRIES.into_iter().skip(2) {
            let path = fixture.home().join(entry);
            fs::write(&path, "synthetic-shell-startup-bait").unwrap();
            paths.push(path);
        }
        let external_startup = fixture.0.join("shell-startup-target");
        fs::write(&external_startup, "synthetic-shell-startup-bait").unwrap();
        fs::remove_file(fixture.home().join(".zshenv")).unwrap();
        symlink(&external_startup, fixture.home().join(".zshenv")).unwrap();
        paths.push(external_startup);
        (paths, allowed)
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn kernel_denies_original_roots_and_symlink_aliases_with_permission_errors() {
        let fixture = Fixture::new();
        let (paths, allowed) = bait(&fixture);
        let unknown = fixture.0.join("unapproved");
        fs::create_dir_all(&unknown).unwrap();
        fs::write(unknown.join("hidden"), "synthetic-only").unwrap();
        let snapshot = ChatgptIsolationSnapshot::from_home(&fixture.home())
            .unwrap()
            .with_file_access(
                &[PathBuf::from("/Library/Developer/CommandLineTools")],
                &[allowed.parent().unwrap().to_owned()],
            )
            .unwrap();
        let script = r"import os,sys,json,subprocess
paths=json.loads(sys.argv[1]); allowed=sys.argv[2]; result=[]
for path in paths:
 item={}
 for operation in ['read','metadata','write']:
  try:
   if operation=='read':
    with open(path,'rb') as stream: stream.read()
   elif operation=='metadata': os.stat(path)
   else:
    with open(path,'ab') as stream: stream.write(b'changed')
   item[operation]=0
  except OSError as error: item[operation]=error.errno
 result.append(item)
with open(allowed,'r') as stream: allowed_read=stream.read()=='isolated-config'
with open(allowed+'.new','w') as stream: stream.write('isolated-write')
nested=subprocess.run(['/bin/cat',paths[0]],capture_output=True,timeout=3)
unknown=sys.argv[3]; unknown_errors=[]
for operation in ['read','list']:
 try:
  if operation=='read':open(unknown+'/hidden','rb').read()
  else:os.listdir(unknown)
  unknown_errors.append(0)
 except OSError as error:unknown_errors.append(error.errno)
xattr=subprocess.run(['/usr/bin/xattr','-p','user.synthetic',unknown+'/hidden'],capture_output=True,timeout=3)
print(json.dumps({'checks':result,'allowedRead':allowed_read,'nestedDenied':nested.returncode!=0 and not nested.stdout,'markerAbsent':'CILAI_PERSONAL_ORIGINAL_HOME_PROTECTION' not in os.environ,'unknownErrors':unknown_errors,'unknownXattrDenied':xattr.returncode!=0 and not xattr.stdout and b'Operation not permitted' in xattr.stderr,'unknownMetadataAllowed':os.stat(unknown+'/hidden').st_size>0}))
";
        let args = [
            OsString::from("-I"),
            OsString::from("-c"),
            OsString::from(script),
            OsString::from(serde_json::to_string(&paths).unwrap()),
            allowed.as_os_str().to_owned(),
            unknown.into_os_string(),
        ];
        let mut command = platform::command(
            &snapshot,
            Path::new("/Library/Developer/CommandLineTools/usr/bin/python3"),
            &args,
        )
        .unwrap();
        command
            .current_dir(allowed.parent().unwrap())
            .env_clear()
            .env("HOME", fixture.home());
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["allowedRead"], true);
        assert_eq!(result["nestedDenied"], true);
        assert_eq!(result["markerAbsent"], true);
        assert_eq!(result["unknownMetadataAllowed"], true);
        assert_eq!(result["unknownXattrDenied"], true);
        assert!(
            result["unknownErrors"]
                .as_array()
                .unwrap()
                .iter()
                .all(|error| matches!(error.as_i64(), Some(1 | 13))),
            "{result}"
        );
        for check in result["checks"].as_array().unwrap() {
            for operation in ["read", "metadata", "write"] {
                assert!(
                    matches!(check[operation].as_i64(), Some(1 | 13)),
                    "{result}"
                );
            }
        }
        assert_eq!(
            fs::read_to_string(&paths[1]).unwrap(),
            "synthetic-codex-bait"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn kernel_protects_a_symlinked_home_spelling_and_its_canonical_roots() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let (paths, allowed) = bait(&fixture);
        let alias = fixture.0.join("home-alias");
        symlink(fixture.home(), &alias).unwrap();
        let snapshot = ChatgptIsolationSnapshot::from_home(&alias)
            .unwrap()
            .with_file_access(
                &[PathBuf::from("/Library/Developer/CommandLineTools")],
                &[allowed.parent().unwrap().to_owned()],
            )
            .unwrap();
        assert!(snapshot.protected_roots().contains(&alias.join(".codex")));
        assert!(
            snapshot
                .protected_roots()
                .contains(&fixture.home().join(".codex"))
        );
        let script = r"import os,sys,json
result=[]
for path in sys.argv[1:]:
 try:
  with open(path,'rb') as stream: stream.read()
  result.append(0)
 except OSError as error: result.append(error.errno)
print(json.dumps(result))
";
        let mut args = vec![
            OsString::from("-I"),
            OsString::from("-c"),
            OsString::from(script),
            alias.join(".codex/auth.json").into_os_string(),
        ];
        args.extend(paths.into_iter().map(PathBuf::into_os_string));
        let mut command = platform::command(
            &snapshot,
            Path::new("/Library/Developer/CommandLineTools/usr/bin/python3"),
            &args,
        )
        .unwrap();
        command
            .current_dir(allowed.parent().unwrap())
            .env_clear()
            .env("HOME", &alias)
            .env("CODEX_HOME", allowed.parent().unwrap());
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let denied: Vec<i32> = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            denied.iter().all(|error| matches!(error, 1 | 13)),
            "{denied:?}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_changed_protected_symlink_changes_the_frozen_digest_before_launch() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        bait(&fixture);
        let snapshot = ChatgptIsolationSnapshot::from_home(&fixture.home()).unwrap();
        let replacement = fixture.0.join("replacement");
        fs::create_dir_all(&replacement).unwrap();
        fs::remove_file(fixture.home().join(".codex")).unwrap();
        symlink(replacement, fixture.home().join(".codex")).unwrap();
        assert_ne!(
            snapshot.digest(),
            ChatgptIsolationSnapshot::from_home(&fixture.home())
                .unwrap()
                .digest()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn kernel_rejects_new_symlink_targets_after_start_and_protected_targets_inside_grants() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let target = fixture.0.join("authorized/original-codex");
        let unapproved = fixture.0.join("unapproved");
        let runtime = fixture.0.join("runtime");
        for path in [&target, &unapproved, &runtime] {
            fs::create_dir_all(path).unwrap();
        }
        for path in [&target, &unapproved] {
            fs::write(path.join("auth.json"), "synthetic-only").unwrap();
        }
        symlink(&target, fixture.home().join(".codex")).unwrap();
        let snapshot = ChatgptIsolationSnapshot::from_home(&fixture.home())
            .unwrap()
            .with_file_access(
                &[
                    target.parent().unwrap().to_owned(),
                    PathBuf::from("/Library/Developer/CommandLineTools"),
                ],
                std::slice::from_ref(&runtime),
            )
            .unwrap();
        let script = r"import pathlib,time,sys,json
base=pathlib.Path(sys.argv[1]); result=[]
def attempt():
 try:
  (base/'home/.codex/auth.json').read_bytes(); result.append(0)
 except OSError as error: result.append(error.errno)
attempt();(base/'runtime/ready').write_text('')
while not (base/'runtime/go').exists():time.sleep(.01)
attempt();print(json.dumps(result))
";
        let args = [
            OsString::from("-I"),
            OsString::from("-c"),
            OsString::from(script),
            fixture.0.clone().into_os_string(),
        ];
        let mut command = platform::command(
            &snapshot,
            Path::new("/Library/Developer/CommandLineTools/usr/bin/python3"),
            &args,
        )
        .unwrap();
        command
            .current_dir(&runtime)
            .env_clear()
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !runtime.join("ready").exists()
            && std::time::Instant::now() < deadline
            && child.try_wait().unwrap().is_none()
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !runtime.join("ready").exists() {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("{}", String::from_utf8_lossy(&output.stderr));
        }
        fs::remove_file(fixture.home().join(".codex")).unwrap();
        symlink(&unapproved, fixture.home().join(".codex")).unwrap();
        fs::write(runtime.join("go"), "").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let denied: Vec<i32> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(denied.len(), 2);
        assert!(
            denied.iter().all(|error| matches!(error, 1 | 13)),
            "{denied:?}"
        );
    }
}
