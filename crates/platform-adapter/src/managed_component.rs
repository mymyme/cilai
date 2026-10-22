#[cfg(target_os = "macos")]
use std::fmt::Write as _;
use std::io;
use std::path::Path;
use std::process::Command;

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows;

const HELPER_FLAG: &str = "--personal-capability-sandbox";

/// Dispatch the Workbench-owned helper before starting host services.
#[must_use]
pub fn helper_exit_code(arguments: &[String]) -> Option<i32> {
    if arguments.first().is_none_or(|value| value != HELPER_FLAG) {
        return None;
    }
    #[cfg(windows)]
    {
        let result = (|| {
            if arguments.len() != 4 {
                return Err(io::Error::other("personal_sandbox_helper_arguments"));
            }
            let (node, package, data, args) = validated(
                Path::new(&arguments[1]),
                Path::new(&arguments[2]),
                Path::new(&arguments[3]),
            )?;
            windows::run(&node, &package, &data, &args)
        })();
        Some(result.unwrap_or_else(|error| {
            eprintln!("个人能力原生隔离启动失败：{error}");
            1
        }))
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    Some(1)
}

/// 工作台控制的启动计划；Windows 使用工作台原生 LPAC 助手。
pub struct Launch {
    pub command: Command,
    /// Held by the host until the helper has been terminated and reaped.
    pub lease: Lease,
}
pub struct Lease {
    #[cfg(windows)]
    _windows: Option<windows::Lease>,
}
impl Launch {
    #[must_use]
    pub fn direct(command: Command) -> Self {
        Self {
            command,
            lease: Lease {
                #[cfg(windows)]
                _windows: None,
            },
        }
    }
}

/// Builds a process restricted to the capability package and its private data.
///
/// # Errors
/// Returns an error when paths are invalid or the platform cannot enforce isolation.
pub fn command(node: &Path, package: &Path, data: &Path, helper: &Path) -> io::Result<Launch> {
    let (node, package, data, args) = validated(node, package, data)?;
    platform_command(&node, &package, &data, helper, &args)
}

fn validated(
    node: &Path,
    package: &Path,
    data: &Path,
) -> io::Result<(
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
    Vec<String>,
)> {
    let node = node.canonicalize()?;
    let package = package.canonicalize()?;
    let data = data.canonicalize()?;
    let entry = package.join("source/mcp/server.mjs").canonicalize()?;
    if !entry.starts_with(&package)
        || !entry.is_file()
        || package == data
        || data.starts_with(&package)
        || package.starts_with(&data)
        || node.starts_with(&data)
    {
        return Err(io::Error::other("managed_component_scope_invalid"));
    }
    let args = vec![
        "--permission".to_owned(),
        format!("--allow-fs-read={}", node_path(&package)?),
        format!("--allow-fs-read={}", node_path(&data)?),
        format!("--allow-fs-write={}", node_path(&data)?),
        "--max-old-space-size=128".to_owned(),
        "--disable-proto=throw".to_owned(),
        node_path(&entry)?,
    ];
    #[cfg(windows)]
    let args = {
        let mut args = args;
        // Workbench already canonicalizes every scope. Avoid Node resolving
        // parents outside those scopes merely to canonicalize its entry point.
        args.insert(0, "--preserve-symlinks".into());
        args.insert(0, "--preserve-symlinks-main".into());
        args
    };
    Ok((node, package, data, args))
}

#[cfg(windows)]
fn node_path(path: &Path) -> io::Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| io::Error::other("managed_component_path_encoding_invalid"))?;
    // Node's ESM URL and permission paths do not consistently preserve Win32
    // extended namespace prefixes. Only change spelling, then verify identity.
    let plain = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        text.to_owned()
    };
    if Path::new(&plain).canonicalize()? != path {
        return Err(io::Error::other("managed_component_path_alias_invalid"));
    }
    Ok(plain)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn node_path(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| io::Error::other("managed_component_path_encoding_invalid"))
}

#[cfg(target_os = "macos")]
fn platform_command(
    node: &Path,
    package: &Path,
    data: &Path,
    _: &Path,
    args: &[String],
) -> io::Result<Launch> {
    fn literal(path: &Path) -> io::Result<String> {
        serde_json::to_string(&path.to_string_lossy()).map_err(io::Error::other)
    }
    let mut policy = format!(
        "(version 1)\n(deny default)\n(allow process-exec (literal {}))\n(allow process-info* (target self))\n(allow signal (target self))\n(allow sysctl-read)\n(allow mach-lookup (global-name \"com.apple.system.logger\"))\n(allow file-read-metadata)\n(allow file-read* (literal {}) (subpath {}) (subpath {}) (subpath \"/System/Library\") (subpath \"/usr/lib\") (subpath \"/private/var/db/dyld\") (subpath \"/usr/share/zoneinfo\") (literal \"/dev/null\") (literal \"/dev/urandom\") (literal \"/dev/random\"))\n(allow file-write* (subpath {}) (literal \"/dev/null\"))\n",
        literal(node)?,
        literal(node)?,
        literal(package)?,
        literal(data)?,
        literal(data)?
    );
    write!(policy, "(allow file-map-executable (literal {}) (subpath \"/System/Library\") (subpath \"/usr/lib\"))\n(allow file-read-data file-write-data (subpath \"/dev/fd\"))\n(allow file-read* (literal \"/\"))\n",literal(node)?).map_err(io::Error::other)?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .args(["-p", &policy])
        .arg(node)
        .args(args)
        .current_dir(data)
        .env_clear()
        .env("HOME", data)
        .env("TMPDIR", data);
    Ok(Launch::direct(command))
}

#[cfg(windows)]
fn platform_command(
    node: &Path,
    package: &Path,
    data: &Path,
    helper: &Path,
    _: &[String],
) -> io::Result<Launch> {
    let lease = windows::prepare(node, package, data)?;
    let mut command = Command::new(helper);
    command
        .arg(HELPER_FLAG)
        .arg(lease.node())
        .arg(package)
        .arg(data)
        .current_dir(data)
        .env_clear();
    crate::process::retain_required_system_environment(&mut command)?;
    windows::prepare_helper_environment(&mut command)?;
    Ok(Launch {
        command,
        lease: Lease {
            _windows: Some(lease),
        },
    })
}

#[cfg(windows)]
fn windows_command_line(executable: &Path, args: &[String]) -> String {
    fn quote(value: &str) -> String {
        let mut output = String::from("\"");
        let mut slashes = 0;
        for ch in value.chars() {
            if ch == '\\' {
                slashes += 1;
                continue;
            }
            if ch == '\"' {
                output.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
            } else {
                output.extend(std::iter::repeat_n('\\', slashes));
            }
            slashes = 0;
            output.push(ch);
        }
        output.extend(std::iter::repeat_n('\\', slashes * 2));
        output.push('\"');
        output
    }
    std::iter::once(executable.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .map(|value| quote(&value))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "linux")]
fn platform_command(_: &Path, _: &Path, _: &Path, _: &Path, _: &[String]) -> io::Result<Launch> {
    Err(io::Error::other("managed_component_platform_unsupported"))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn component_allows_private_data_and_denies_network_files_and_processes() {
        let node = crate::executable_search::find_executables(&["node"])
            .into_iter()
            .find(|path| {
                Command::new(path)
                    .arg("--version")
                    .output()
                    .is_ok_and(|result| {
                        String::from_utf8_lossy(&result.stdout).trim() == "v24.18.0"
                    })
            })
            .expect("Node 24.18.0");
        let root = std::env::temp_dir().join(format!("cilai-sandbox-test-{}", std::process::id()));
        let package = root.join("package");
        let data = root.join("data");
        std::fs::create_dir_all(package.join("source/mcp")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(root.join("secret.txt"), "secret").unwrap();
        let secret = serde_json::to_string(&root.join("secret.txt").to_string_lossy()).unwrap();
        let source = format!(
            r"import fs from 'node:fs'; import net from 'node:net'; import {{spawnSync}} from 'node:child_process';
const result={{}};
fs.writeFileSync('own.txt','ok'); result.own=fs.readFileSync('own.txt','utf8');
try {{ fs.readFileSync({secret}); result.outside=true; }} catch {{ result.outside=false; }}
try {{ spawnSync('/bin/sh',['-c','echo unsafe']); result.process=true; }} catch {{ result.process=false; }}
result.network=await new Promise(resolve=>{{ const socket=net.connect({{host:'127.0.0.1',port:9}}); socket.on('error',error=>resolve(error.code)); socket.on('connect',()=>{{socket.destroy();resolve('CONNECTED')}}); socket.setTimeout(1000,()=>{{socket.destroy();resolve('TIMEOUT')}}); }});
console.log(JSON.stringify(result));"
        );
        std::fs::write(package.join("source/mcp/server.mjs"), source).unwrap();
        let output = command(&node, &package, &data, Path::new("unused"))
            .unwrap()
            .command
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "status={:?} stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["own"], "ok");
        assert_eq!(result["outside"], false);
        assert_eq!(result["process"], false);
        assert!(
            matches!(result["network"].as_str(), Some("EPERM" | "EACCES")),
            "{result}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    #[test]
    fn windows_component_uses_owned_helper_without_codex_protocol() {
        let root =
            std::env::temp_dir().join(format!("cilai-component 中文 {}", std::process::id()));
        let package = root.join("package");
        let data = root.join("private data");
        std::fs::create_dir_all(package.join("source/mcp")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(package.join("source/mcp/server.mjs"), "").unwrap();
        let node = std::env::current_exe().unwrap();
        let helper = root.join("cilai-host-service.exe");
        let launch = command(&node, &package, &data, &helper).unwrap();
        assert_eq!(launch.command.get_program(), helper);
        let args: Vec<_> = launch
            .command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], HELPER_FLAG);
        let cached = Path::new(&args[1]);
        assert_ne!(cached, node.canonicalize().unwrap());
        assert_eq!(
            std::fs::read(cached).unwrap(),
            std::fs::read(&node).unwrap()
        );
        assert_eq!(args[2], package.canonicalize().unwrap().to_string_lossy());
        assert_eq!(args[3], data.canonicalize().unwrap().to_string_lossy());
        let cached = cached.to_path_buf();
        let modified = std::fs::metadata(&cached).unwrap().modified().unwrap();
        drop(launch);
        let repeated = command(&node, &package, &data, &helper).unwrap();
        assert_eq!(
            std::fs::metadata(&cached).unwrap().modified().unwrap(),
            modified
        );
        drop(repeated);
        std::fs::write(&cached, b"tampered runtime").unwrap();
        assert!(command(&node, &package, &data, &helper).is_err());
        assert!(command(&node, &package, &package, &helper).is_err());
        assert_eq!(helper_exit_code(&[HELPER_FLAG.into()]), Some(1));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn windows_arguments_quote_empty_space_unicode_quotes_and_trailing_slashes() {
        let actual = windows_command_line(
            Path::new(r"C:\Program Files\node.exe"),
            &[
                String::new(),
                "中文 value".into(),
                "a\"b".into(),
                "tail\\".into(),
            ],
        );
        assert_eq!(
            actual,
            r#""C:\Program Files\node.exe" "" "中文 value" "a\"b" "tail\\""#
        );
    }
}
