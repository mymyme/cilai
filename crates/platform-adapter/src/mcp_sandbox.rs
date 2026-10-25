//! 任意 MCP 可执行文件的独立权限计划。固定 Node 个人包沙箱不参与此路径。
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use std::fs;
use std::io::{self, Read as _};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Permissions {
    pub read_only: Vec<PathBuf>,
    pub writable: Vec<PathBuf>,
    pub network: bool,
    pub child_processes: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Grant {
    requested: PathBuf,
    resolved: PathBuf,
    directory: bool,
}
impl Grant {
    fn capture(path: &Path) -> io::Result<Self> {
        if !path.is_absolute()
            || path.parent().is_none()
            || path.components().any(|c| {
                !matches!(
                    c,
                    Component::Prefix(_) | Component::RootDir | Component::Normal(_)
                )
            })
        {
            return Err(denied("mcp_file_permissions_invalid"));
        }
        let resolved = path
            .canonicalize()
            .map_err(|_| denied("mcp_file_permissions_invalid"))?;
        let metadata = fs::metadata(&resolved)?;
        if resolved.parent().is_none() || (!metadata.is_file() && !metadata.is_dir()) {
            return Err(denied("mcp_file_permissions_invalid"));
        }
        Ok(Self {
            requested: path.into(),
            resolved,
            directory: metadata.is_dir(),
        })
    }
    fn validate(&self) -> io::Result<()> {
        if Self::capture(&self.requested)? != *self {
            return Err(denied("mcp_file_permissions_changed"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Plan {
    executable: Grant,
    args: Vec<String>,
    cwd: Grant,
    home: Grant,
    reads: Vec<Grant>,
    writes: Vec<Grant>,
    network: bool,
    protected: Vec<PathBuf>,
    managed: Option<Grant>,
    executable_digest: String,
    argument_files: Vec<(PathBuf, Grant, String)>,
}
fn denied(code: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, code)
}
fn supported(permissions: &Permissions) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        if permissions.child_processes {
            Err(denied("mcp_child_processes_not_supported"))
        } else {
            Ok(())
        }
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        let _ = permissions;
        Err(denied("mcp_local_isolation_not_supported"))
    }
}
#[cfg(target_os = "macos")]
fn overlaps(root: &Path, protected: &Path) -> bool {
    root.starts_with(protected) || protected.starts_with(root)
}
#[cfg(target_os = "macos")]
fn protected_roots() -> io::Result<Vec<PathBuf>> {
    crate::macos_runtime_isolation::ChatgptIsolationSnapshot::capture()
        .map(|s| s.map_or_else(Vec::new, |s| s.protected_roots().to_vec()))
}
fn file_digest(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    if file.metadata()?.len() > 512 * 1024 * 1024 {
        return Err(denied("mcp_program_too_large"));
    }
    let mut buffer = [0; 8192];
    let mut hash = Sha256::new();
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
impl Plan {
    /// # Errors
    /// 受管命名空间不能授权给外部命令；只有本次租约的私有 HOME 允许读写。
    pub fn capture_managed(
        program: &Path,
        args: &[String],
        cwd: &Path,
        private_home: &Path,
        permissions: &Permissions,
        namespace: &Path,
    ) -> io::Result<Self> {
        supported(permissions)?;
        let managed = Grant::capture(namespace)?;
        let home = Grant::capture(private_home)?;
        let grants = std::iter::once(program)
            .chain(std::iter::once(cwd))
            .chain(permissions.read_only.iter().map(PathBuf::as_path))
            .chain(permissions.writable.iter().map(PathBuf::as_path))
            .map(Grant::capture)
            .collect::<io::Result<Vec<_>>>()?;
        Self::managed_boundary(&managed, &home, &grants.iter().collect::<Vec<_>>())?;
        let mut plan = Self::capture(program, args, cwd, private_home, permissions)?;
        plan.managed = Some(managed);
        plan.validate_managed()?;
        Ok(plan)
    }
    fn validate_managed(&self) -> io::Result<()> {
        let Some(managed) = &self.managed else {
            return Ok(());
        };
        managed.validate()?;
        Self::managed_boundary(
            managed,
            &self.home,
            &[&self.executable, &self.cwd]
                .into_iter()
                .chain(self.reads.iter())
                .chain(self.writes.iter())
                .collect::<Vec<_>>(),
        )
    }
    fn managed_boundary(managed: &Grant, home: &Grant, grants: &[&Grant]) -> io::Result<()> {
        if !managed.directory
            || managed.resolved.file_name().and_then(|v| v.to_str())
                != Some("cn.cilai.personal.workbench")
            || home.resolved == managed.resolved
            || !home.resolved.starts_with(&managed.resolved)
        {
            return Err(denied("mcp_managed_storage_invalid"));
        }
        for grant in grants {
            if [&grant.requested, &grant.resolved].into_iter().any(|path| {
                [&managed.requested, &managed.resolved]
                    .into_iter()
                    .any(|root| path.starts_with(root) || root.starts_with(path))
            }) {
                return Err(denied("mcp_managed_storage_protected"));
            }
        }
        Ok(())
    }
    /// # Errors
    /// 不支持的平台/子进程权限、受保护路径、路径别名变化或不可执行来源均明确拒绝。
    pub fn capture(
        program: &Path,
        args: &[String],
        cwd: &Path,
        private_home: &Path,
        permissions: &Permissions,
    ) -> io::Result<Self> {
        #[cfg(any(windows, target_os = "linux"))]
        {
            let _ = (program, args, cwd, private_home, permissions);
            Err(denied("mcp_local_isolation_not_supported"))
        }
        #[cfg(target_os = "macos")]
        {
            supported(permissions)?;
            Self::capture_protected(
                program,
                args,
                cwd,
                private_home,
                permissions,
                protected_roots()?,
            )
        }
    }
    #[cfg(target_os = "macos")]
    fn capture_protected(
        program: &Path,
        args: &[String],
        cwd: &Path,
        home: &Path,
        permissions: &Permissions,
        protected: Vec<PathBuf>,
    ) -> io::Result<Self> {
        let executable = Grant::capture(program)?;
        let cwd = Grant::capture(cwd)?;
        let home = Grant::capture(home)?;
        let reads = permissions
            .read_only
            .iter()
            .map(|p| Grant::capture(p))
            .collect::<io::Result<Vec<_>>>()?;
        let writes = permissions
            .writable
            .iter()
            .map(|p| Grant::capture(p))
            .collect::<io::Result<Vec<_>>>()?;
        if executable.directory
            || !cwd.directory
            || !home.directory
            || args.len() > 128
            || reads.len() + writes.len() > 64
        {
            return Err(denied("mcp_file_permissions_invalid"));
        }
        for grant in [&executable, &cwd, &home]
            .into_iter()
            .chain(reads.iter())
            .chain(writes.iter())
        {
            if protected
                .iter()
                .any(|p| overlaps(&grant.requested, p) || overlaps(&grant.resolved, p))
            {
                return Err(denied("mcp_original_home_permissions_protected"));
            }
        }
        crate::executable_search::validate_executable_file(&executable.resolved)?;
        let executable_digest = file_digest(&executable.resolved)?;
        let mut argument_files = Vec::new();
        let mut total = 0;
        for arg in args {
            let candidate = Path::new(arg);
            let candidate = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                cwd.resolved.join(candidate)
            };
            if !candidate.is_file() {
                continue;
            }
            let grant = Grant::capture(&candidate.canonicalize()?)?;
            if protected.iter().any(|path| overlaps(&grant.resolved, path))
                || !reads
                    .iter()
                    .chain(writes.iter())
                    .chain(std::iter::once(&home))
                    .any(|root| {
                        grant.resolved == root.resolved
                            || root.directory && grant.resolved.starts_with(&root.resolved)
                    })
            {
                return Err(denied("mcp_source_arguments_outside_permissions"));
            }
            total += fs::metadata(&grant.resolved)?.len();
            if total > 64 * 1024 * 1024 {
                return Err(denied("mcp_source_snapshot_too_large"));
            }
            let hash = file_digest(&grant.resolved)?;
            argument_files.push((candidate, grant, hash));
        }
        Ok(Self {
            executable,
            args: args.into(),
            cwd,
            home,
            reads,
            writes,
            network: permissions.network,
            protected,
            managed: None,
            executable_digest,
            argument_files,
        })
    }
    /// # Errors
    /// 启动及每次工具请求前核对来源和权限路径；变化后必须重新验证。
    pub fn validate(&self) -> io::Result<()> {
        self.validate_managed()?;
        #[cfg(target_os = "macos")]
        if protected_roots()? != self.protected {
            return Err(denied("mcp_original_home_protection_changed"));
        }
        for grant in [&self.executable, &self.cwd, &self.home]
            .into_iter()
            .chain(self.reads.iter())
            .chain(self.writes.iter())
        {
            grant.validate()?;
        }
        if file_digest(&self.executable.resolved)? != self.executable_digest {
            return Err(denied("mcp_source_changed"));
        }
        for (requested, grant, hash) in &self.argument_files {
            if requested.canonicalize()? != grant.resolved {
                return Err(denied("mcp_source_changed"));
            }
            grant.validate()?;
            if file_digest(&grant.resolved)? != *hash {
                return Err(denied("mcp_source_changed"));
            }
        }
        Ok(())
    }
    /// # Errors
    /// 仅返回权限计划摘要，不包含环境秘密。
    pub fn fingerprint(&self) -> io::Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(io::Error::other)?)
        ))
    }
    /// # Errors
    /// 来源摘要忽略每次执行的临时 HOME，保留程序、参数、工作目录与实际权限路径。
    pub fn source_fingerprint(&self) -> io::Result<String> {
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    &self.executable,
                    &self.args,
                    &self.cwd,
                    &self.reads,
                    &self.writes,
                    self.network,
                    &self.protected,
                    &self.managed,
                    &self.executable_digest,
                    &self.argument_files
                ))
                .map_err(io::Error::other)?
            )
        ))
    }
    /// # Errors
    /// 不支持的平台或权限核验失败时不生成裸命令。
    pub fn command(&self) -> io::Result<Command> {
        self.validate()?;
        #[cfg(target_os = "macos")]
        {
            if protected_roots()? != self.protected {
                return Err(denied("mcp_original_home_protection_changed"));
            }
            let mut command = Command::new("/usr/bin/sandbox-exec");
            command
                .args(["-p", &self.policy()?])
                .arg(&self.executable.resolved)
                .args(&self.args)
                .current_dir(&self.cwd.resolved)
                .env_clear();
            for name in [
                "HOME",
                "USERPROFILE",
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_STATE_HOME",
                "XDG_CACHE_HOME",
                "TMPDIR",
                "TMP",
                "TEMP",
                "CODEX_HOME",
                "CODEX_SQLITE_HOME",
            ] {
                command.env(name, &self.home.resolved);
            }
            command.env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
            Ok(command)
        }
        #[cfg(any(windows, target_os = "linux"))]
        Err(denied("mcp_local_isolation_not_supported"))
    }
    #[cfg(target_os = "macos")]
    fn policy(&self) -> io::Result<String> {
        use std::fmt::Write as _;
        let quote =
            |p: &Path| serde_json::to_string(&p.to_string_lossy()).map_err(io::Error::other);
        let mut policy = format!(
            "(version 1)\n(deny default)\n(allow process-exec (literal {}))\n(allow process-info* (target self))\n(allow signal (target self))\n(allow sysctl-read)\n(allow mach-lookup (global-name \"com.apple.system.logger\"))\n(allow file-read-metadata)\n(allow file-read* (literal \"/\") (literal \"/dev/null\") (literal \"/dev/urandom\") (literal \"/dev/random\") (subpath \"/System/Library\") (subpath \"/usr/lib\") (subpath \"/private/var/db/dyld\") (subpath \"/usr/share/zoneinfo\"))\n(allow file-map-executable (literal {}) (subpath \"/System/Library\") (subpath \"/usr/lib\"))\n(allow file-read-data file-write-data (subpath \"/dev/fd\"))\n(allow file-write* (literal \"/dev/null\"))\n",
            quote(&self.executable.resolved)?,
            quote(&self.executable.resolved)?
        );
        for grant in [&self.executable, &self.home]
            .into_iter()
            .chain(self.reads.iter())
            .chain(self.writes.iter())
        {
            let filter = if grant.directory {
                "subpath"
            } else {
                "literal"
            };
            writeln!(
                policy,
                "(allow file-read* file-map-executable ({filter} {}))",
                quote(&grant.resolved)?
            )
            .map_err(io::Error::other)?;
        }
        for grant in std::iter::once(&self.home).chain(self.writes.iter()) {
            let filter = if grant.directory {
                "subpath"
            } else {
                "literal"
            };
            writeln!(
                policy,
                "(allow file-write* ({filter} {}))",
                quote(&grant.resolved)?
            )
            .map_err(io::Error::other)?;
        }
        if self.network {
            policy.push_str("(allow network*)\n");
        }
        policy.push_str("(deny file-read* file-write*");
        for protected in &self.protected {
            write!(
                policy,
                " (literal {}) (subpath {})",
                quote(protected)?,
                quote(protected)?
            )
            .map_err(io::Error::other)?;
        }
        policy.push_str(")\n");
        if let Some(managed) = &self.managed {
            writeln!(
                policy,
                "(deny file-read* file-write* (require-all (subpath {}) (require-not (subpath {}))))",
                quote(&managed.resolved)?,
                quote(&self.home.resolved)?
            )
            .map_err(io::Error::other)?;
        }
        Ok(policy)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let root = crate::test_support::personal_build_root()
                .join("mcp-kernel-fixtures")
                .join(format!(
                    "probe-{}-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos(),
                    COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                ));
            crate::security::ensure_restricted_directory(&root).unwrap();
            Self(root.canonicalize().unwrap())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if std::thread::panicking() {
                eprintln!("mcp_kernel_fixture_failure={}", self.0.display());
            } else {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }
    fn node() -> PathBuf {
        crate::executable_search::find_executables(&["node"])
            .into_iter()
            .next()
            .unwrap()
            .canonicalize()
            .unwrap()
    }
    #[test]
    fn kernel_enforces_files_network_and_child_permissions_for_arbitrary_command() {
        let fixture = Fixture::new();
        let home = fixture.0.join("home");
        let source = fixture.0.join("source");
        let outside = fixture.0.join("outside.txt");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(&outside, "private canary").unwrap();
        fs::write(source.join("input.txt"), "allowed").unwrap();
        let program = node();
        let outside_literal = serde_json::to_string(&outside).unwrap();
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let code = format!(
            r#"const fs=require('node:fs'),cp=require('node:child_process'),net=require('node:net');let own=fs.readFileSync('input.txt','utf8')==='allowed',outside=false,child=false;fs.writeFileSync(process.env.HOME+'/result.txt','own');try{{fs.readFileSync({outside_literal},'utf8');outside=true}}catch{{}}try{{cp.execFileSync(process.execPath,['-e','process.stdout.write("child")']);child=true}}catch{{}}let socket=net.connect({{host:'127.0.0.1',port:{port}}});socket.on('error',()=>{{console.log(JSON.stringify({{own,outside,child,network:false}}))}});socket.on('connect',()=>{{socket.destroy();console.log(JSON.stringify({{own,outside,child,network:true}}))}});socket.setTimeout(1000,()=>socket.destroy());"#
        );
        let plan = Plan::capture(
            &program,
            &["-e".into(), code],
            &source,
            &home,
            &Permissions {
                read_only: vec![source.clone()],
                writable: vec![],
                network: false,
                child_processes: false,
            },
        )
        .unwrap();
        let output = plan.command().unwrap().output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            result,
            serde_json::json!({"own":true,"outside":false,"child":false,"network":false})
        );
        assert_eq!(fs::read_to_string(home.join("result.txt")).unwrap(), "own");
    }
    #[test]
    fn original_home_or_alias_ancestor_grants_are_rejected_before_launch() {
        let fixture = Fixture::new();
        let private = fixture.0.join("private");
        let fake_home = fixture.0.join("fake-original");
        fs::create_dir_all(&private).unwrap();
        fs::create_dir_all(fake_home.join(".codex")).unwrap();
        let error = Plan::capture_protected(
            &node(),
            &[],
            &private,
            &private,
            &Permissions {
                read_only: vec![fake_home.clone()],
                writable: vec![],
                network: false,
                child_processes: false,
            },
            vec![fake_home.join(".codex")],
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "mcp_original_home_permissions_protected");
        assert!(
            Plan::capture(
                &node(),
                &[],
                &private,
                &private,
                &Permissions {
                    read_only: vec![],
                    writable: vec![],
                    network: false,
                    child_processes: true
                }
            )
            .is_err()
        );
    }
    #[test]
    fn declared_network_access_reaches_only_the_requested_local_test_server() {
        let fixture = Fixture::new();
        let home = fixture.0.join("home");
        fs::create_dir_all(&home).unwrap();
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let code = format!(
            "const net=require('node:net');const socket=net.connect({{host:'127.0.0.1',port:{port}}});socket.on('connect',()=>{{socket.destroy();process.stdout.write('connected')}});socket.on('error',()=>process.exitCode=1);"
        );
        let plan = Plan::capture(
            &node(),
            &["-e".into(), code],
            &home,
            &home,
            &Permissions {
                read_only: vec![],
                writable: vec![],
                network: true,
                child_processes: false,
            },
        )
        .unwrap();
        let output = plan.command().unwrap().output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"connected");
    }
    #[test]
    fn entry_file_alias_cannot_change_after_source_digest_was_confirmed() {
        let fixture = Fixture::new();
        let home = fixture.0.join("home");
        let source = fixture.0.join("source");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("a.mjs"), "console.log('a')").unwrap();
        fs::write(source.join("b.mjs"), "console.log('b')").unwrap();
        let entry = source.join("entry.mjs");
        std::os::unix::fs::symlink(source.join("a.mjs"), &entry).unwrap();
        let plan = Plan::capture(
            &node(),
            &[entry.to_string_lossy().into_owned()],
            &source,
            &home,
            &Permissions {
                read_only: vec![source.clone()],
                writable: vec![],
                network: false,
                child_processes: false,
            },
        )
        .unwrap();
        fs::remove_file(&entry).unwrap();
        std::os::unix::fs::symlink(source.join("b.mjs"), &entry).unwrap();
        assert_eq!(
            plan.validate().unwrap_err().to_string(),
            "mcp_source_changed"
        );
    }
    #[test]
    fn managed_namespace_and_aliases_cannot_be_granted_to_external_commands() {
        let fixture = Fixture::new();
        let namespace = fixture.0.join("cn.cilai.personal.workbench");
        let profile = namespace.join("business/profiles/local");
        let home = profile.join("ai-runtime/mcp-runtime/connection/lease");
        let project = fixture.0.join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&project).unwrap();
        let alias = fixture.0.join("alias");
        std::os::unix::fs::symlink(&profile, &alias).unwrap();
        for root in [&namespace, &profile, &fixture.0, &alias] {
            for writable in [false, true] {
                let result = Plan::capture_managed(
                    &node(),
                    &[],
                    &project,
                    &home,
                    &Permissions {
                        read_only: if writable { vec![] } else { vec![root.into()] },
                        writable: if writable { vec![root.into()] } else { vec![] },
                        network: false,
                        child_processes: false,
                    },
                    &namespace,
                );
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "mcp_managed_storage_protected"
                );
            }
        }
    }
    #[test]
    fn kernel_blocks_business_state_while_retaining_private_home_and_user_project() {
        let fixture = Fixture::new();
        let namespace = fixture.0.join("cn.cilai.personal.workbench");
        let home =
            namespace.join("business/profiles/local/ai-runtime/mcp-runtime/connection/lease");
        let project = fixture.0.join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&project).unwrap();
        let database = namespace.join("business/contexthub.db");
        fs::write(&database, "original").unwrap();
        fs::write(project.join("read.txt"), "project").unwrap();
        let code = format!(
            "const fs=require('node:fs');const denied=f=>{{try{{f();process.exitCode=1}}catch(e){{if(!['EPERM','EACCES'].includes(e.code))throw e}}}};denied(()=>fs.readFileSync({0}));denied(()=>fs.writeFileSync({0},'changed'));if(fs.readFileSync('read.txt','utf8')!=='project')process.exitCode=1;fs.writeFileSync(process.env.HOME+'/own.txt','private');",
            serde_json::to_string(&database).unwrap()
        );
        let plan = Plan::capture_managed(
            &node(),
            &["-e".into(), code],
            &project,
            &home,
            &Permissions {
                read_only: vec![project.clone()],
                writable: vec![],
                network: false,
                child_processes: false,
            },
            &namespace,
        )
        .unwrap();
        let output = plan.command().unwrap().output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read_to_string(database).unwrap(), "original");
        assert_eq!(fs::read_to_string(home.join("own.txt")).unwrap(), "private");
    }
}
