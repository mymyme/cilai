use crate::app_bridge::validate_tool_arguments;
use crate::supervisor::AppServerSupervisor;
use contexthub_platform_adapter::{executable_search, managed_component, process};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write as _};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const MAX_LINE: u64 = 1024 * 1024;
struct Running(Child);
impl Running {
    fn stop_checked(&mut self) -> Result<(), String> {
        if self
            .0
            .try_wait()
            .map_err(|_| "personal_writer_process_exit_unconfirmed")?
            .is_none()
        {
            self.0
                .kill()
                .map_err(|_| "personal_writer_process_exit_unconfirmed")?;
        }
        self.0
            .wait()
            .map_err(|_| "personal_writer_process_exit_unconfirmed")?;
        Ok(())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// # Errors
/// Returns an error when installation validation, sandbox startup, or MCP communication fails.
pub fn invoke(
    server: &AppServerSupervisor,
    creation_id: &str,
    tool: Option<&str>,
    arguments: &Value,
    expected_digest: Option<&str>,
) -> Result<Value, String> {
    if arguments.to_string().len() > 256 * 1024 {
        return Err("个人能力输入超过限制。".into());
    }
    let binding = server.request_result(
        "personalCapability/runtime",
        &json!({"creationProjectId":creation_id}),
    )?;
    if expected_digest.is_some_and(|digest| binding["packageDigest"].as_str() != Some(digest)) {
        return Err("个人能力版本已变化，请重新打开。".into());
    }
    invoke_binding(server, &binding, tool, arguments)
}

/// # Errors
/// Returns an error for an invalid binding, oversized input, or a failed sandbox call.
pub fn invoke_binding(
    server: &AppServerSupervisor,
    binding: &Value,
    tool: Option<&str>,
    arguments: &Value,
) -> Result<Value, String> {
    if arguments.to_string().len() > 256 * 1024 {
        return Err("个人能力输入超过限制。".into());
    }
    let path = |key: &str| -> Result<PathBuf, String> {
        binding[key]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| "个人能力安装信息不完整。".into())
    };
    let node = executable_search::bundled_executable_path(
        executable_search::BundledExecutable::PersonalNode,
    )
    .map_err(|error| error.to_string())?;
    executable_search::validate_executable_file(&node)
        .map_err(|_| "随包个人能力运行时缺失，请修复工作台安装。")?;
    let helper = process_helper()?;
    let command =
        managed_component::command(&node, &path("packageRoot")?, &path("dataRoot")?, &helper)
            .map_err(|error| format!("个人能力沙箱无法启动：{error}"))?;
    let writer = ProfileWriter::prepare(server, binding)?;
    run_command(
        command,
        tool,
        arguments,
        &binding["packageDigest"],
        Duration::from_secs(20),
        Some(writer),
    )
}

fn run_command(
    launch: managed_component::Launch,
    tool: Option<&str>,
    arguments: &Value,
    package_digest: &Value,
    timeout: Duration,
    mut writer: Option<ProfileWriter>,
) -> Result<Value, String> {
    let managed_component::Launch {
        mut command,
        lease: _lease,
    } = launch;
    if writer.is_some() {
        command = contexthub_platform_adapter::writer_launch_gate::wrap_command(
            &command,
            &process_helper()?,
        )
        .map_err(|_| "personal_writer_launch_gate_unavailable")?;
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::prepare_background_command_for_supervision(&mut command);
    let child = command
        .spawn()
        .map_err(|error| format!("个人能力无法启动：{error}"))?;
    if let Some(writer) = writer.as_mut() {
        writer.launched = true;
    }
    let mut running = Running(child);
    process::supervise_spawned_child(&mut running.0).map_err(|error| error.to_string())?;
    if let Some(writer) = writer.as_ref() {
        writer.register(running.0.id())?;
        let stdin = running
            .0
            .stdin
            .as_mut()
            .ok_or("personal_writer_launch_pipe_unavailable")?;
        stdin
            .write_all(contexthub_platform_adapter::writer_launch_gate::PERMIT)
            .and_then(|()| stdin.flush())
            .map_err(|_| "personal_writer_launch_unconfirmed")?;
    }
    let mut stderr = running.0.stderr.take().ok_or("个人能力诊断输出不可用。")?;
    let (diagnostics, received) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("personal-mcp-stderr".into())
        .spawn(move || {
            let mut saved = Vec::new();
            let mut buffer = [0_u8; 4096];
            while let Ok(count) = stderr.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                let retain = count.min(4096_usize.saturating_sub(saved.len()));
                saved.extend_from_slice(&buffer[..retain]);
            }
            let _ = diagnostics.send(String::from_utf8_lossy(&saved).trim().to_owned());
        })
        .map_err(|error| format!("个人能力诊断无法启动：{error}"))?;
    let result = communicate(&mut running.0, tool, arguments, package_digest, timeout);
    // Child teardown precedes the host-owned permission lease even on timeout.
    running.stop_checked()?;
    if let Some(writer) = writer.as_mut() {
        writer.finish()?;
    }
    drop(running);
    result.map_err(
        |error| match received.recv_timeout(Duration::from_millis(100)) {
            Ok(detail) if !detail.is_empty() => format!("{error}\n{detail}"),
            _ => error,
        },
    )
}

fn process_helper() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        crate::model_connections::launch_gate_executable()
    }
    #[cfg(not(test))]
    {
        std::env::current_exe().map_err(|_| "personal_writer_helper_unavailable".into())
    }
}

struct ProfileWriter {
    server: AppServerSupervisor,
    lease: contexthub_protocol::PersonalWriterLease,
    launched: bool,
    finished: bool,
}
impl ProfileWriter {
    fn prepare(server: &AppServerSupervisor, binding: &Value) -> Result<Self, String> {
        let digest = binding["packageDigest"]
            .as_str()
            .ok_or("personal_writer_binding_invalid")?;
        let data = binding["dataRoot"]
            .as_str()
            .ok_or("personal_writer_binding_invalid")?;
        let owner = crate::model_connections::execution_owner()?;
        let result = server.request_result("profileWriter/prepare", &json!({
            "clientOperationId":format!("personal-package-{}",uuid::Uuid::new_v4().simple()),
            "kind":"personal_mcp","resourceId":crate::persistent_threads::digest(&json!({"dataRoot":data})),
            "resourceRevision":digest,"owner":owner,"launchProtocol":1,"processMode":"local_process"
        }))?;
        let lease =
            serde_json::from_value(result).map_err(|_| "personal_writer_response_invalid")?;
        Ok(Self {
            server: server.clone(),
            lease,
            launched: false,
            finished: false,
        })
    }
    fn register(&self, pid: u32) -> Result<(), String> {
        let fingerprint = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
            .map_err(|_| "personal_writer_process_unconfirmed")?
            .ok_or("personal_writer_process_unconfirmed")?;
        self.server.request_result(
            "profileWriter/registerProcess",
            &json!({
                "leaseId":self.lease.lease_id,"process":{"pid":pid,"startFingerprint":fingerprint}
            }),
        )?;
        Ok(())
    }
    fn finish(&mut self) -> Result<(), String> {
        self.server.request_result(
            "profileWriter/release",
            &json!({"leaseId":self.lease.lease_id}),
        )?;
        self.finished = true;
        Ok(())
    }
}
impl Drop for ProfileWriter {
    fn drop(&mut self) {
        if !self.finished && !self.launched {
            if self
                .server
                .request_result(
                    "profileWriter/abortUnlaunched",
                    &json!({"leaseId":self.lease.lease_id,"owner":self.lease.owner}),
                )
                .is_err()
            {
                eprintln!("personal_writer_abort_unconfirmed");
            }
        } else if !self.finished {
            eprintln!("personal_writer_release_unconfirmed");
        }
    }
}
fn communicate(
    child: &mut Child,
    tool: Option<&str>,
    arguments: &Value,
    package_digest: &Value,
    timeout: Duration,
) -> Result<Value, String> {
    let stdin = child.stdin.take().ok_or("个人能力输入不可用。")?;
    let stdout = child.stdout.take().ok_or("个人能力输出不可用。")?;
    let (sender, receiver) = mpsc::sync_channel(2);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let result = reader
                .by_ref()
                .take(MAX_LINE + 1)
                .read_until(b'\n', &mut line);
            if matches!(result, Ok(0)) {
                break;
            }
            let item = result.map_err(|error| error.to_string()).and_then(|_| {
                if line.len() as u64 > MAX_LINE {
                    Err("个人能力输出超过限制。".into())
                } else {
                    serde_json::from_slice::<Value>(&line)
                        .map_err(|_| "个人能力返回的消息格式无效。".into())
                }
            });
            let failed = item.is_err();
            if sender.send(item).is_err() || failed {
                break;
            }
        }
    });
    let deadline = Instant::now() + timeout;
    let mut transport = crate::personal_exec::Transport::new(stdin, receiver)?;
    transport.send(
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"cilai-personal","version":"1.0"}}}),
    )?;
    let initialized = response(&mut transport, 1, deadline)?;
    if !matches!(
        initialized["protocolVersion"].as_str(),
        Some("2024-11-05" | "2025-03-26" | "2025-06-18")
    ) || !initialized["capabilities"]["tools"].is_object()
    {
        return Err("个人能力尚未实现受支持的 MCP 工具协议。".into());
    }
    transport.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
    transport.send(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))?;
    let catalog = response(&mut transport, 2, deadline)?;
    let tools = catalog["tools"]
        .as_array()
        .filter(|items| items.len() <= 128)
        .ok_or("个人能力工具目录无效。")?;
    if catalog
        .get("nextCursor")
        .is_some_and(|value| !value.is_null())
    {
        return Err("个人能力工具目录超过单包限制。".into());
    }
    let Some(name) = tool else {
        return Ok(json!({"tools":tools,"packageDigest":package_digest}));
    };
    let selected = tools
        .iter()
        .find(|item| item["name"] == name)
        .ok_or("所选工具当前不可用。")?;
    validate_tool_arguments(&selected["inputSchema"], arguments)?;
    transport.send(
        &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":arguments}}),
    )?;
    let result = response(&mut transport, 3, deadline)?;
    Ok(json!({"result":result,"packageDigest":package_digest}))
}
fn response(
    transport: &mut crate::personal_exec::Transport,
    id: u64,
    deadline: Instant,
) -> Result<Value, String> {
    for _ in 0..128 {
        let message = transport.next(deadline)?;
        if message["jsonrpc"] != "2.0" {
            return Err("个人能力协议无效。".into());
        }
        if message.get("method").is_some() {
            if !message["id"].is_null() {
                transport.send(
                    &json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32601,"message":"Host access is not granted"}}),
                )?;
            }
            continue;
        }
        if message["id"] != id {
            return Err("个人能力响应编号不匹配。".into());
        }
        if let Some(error) = message.get("error") {
            return Err(format!("个人能力返回错误：{error}"));
        }
        return message
            .get("result")
            .cloned()
            .ok_or_else(|| "个人能力缺少执行结果。".into());
    }
    Err("个人能力协议消息超过限制。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mcp_protocol_reads_catalog_calls_tools_and_terminates_invalid_or_silent_servers() {
        let root = std::env::temp_dir().join(format!("cilai-mcp-{}", uuid::Uuid::new_v4()));
        let package = root.join("package");
        let data = root.join("data");
        std::fs::create_dir_all(package.join("source/mcp")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        let node = executable_search::find_executable(&["node"]).unwrap();
        let source = package.join("source/mcp/server.mjs");
        std::fs::write(&source,r"import readline from 'node:readline';
for await (const line of readline.createInterface({input:process.stdin})) {
 const m=JSON.parse(line); if(!m.id) continue;
 const result=m.method==='initialize'?{protocolVersion:'2024-11-05',capabilities:{tools:{}}}:m.method==='tools/list'?{tools:[{name:'echo',inputSchema:{type:'object',properties:{text:{type:'string'}},required:['text'],additionalProperties:false}}]}:{content:[{type:'text',text:m.params.arguments.text}]};
 console.log(JSON.stringify({jsonrpc:'2.0',id:m.id,result}));
}").unwrap();
        let command = || {
            // This fixture tests MCP framing, validation and process lifetime. Native sandbox
            // enforcement belongs to platform-adapter; cargo test binaries have no bundled Harness.
            let mut command = std::process::Command::new(&node);
            command.arg(&source).current_dir(&data);
            managed_component::Launch::direct(command)
        };
        let catalog = run_command(
            command(),
            None,
            &json!({}),
            &json!("digest"),
            Duration::from_secs(3),
            None,
        )
        .unwrap();
        assert_eq!(catalog["tools"][0]["name"], "echo");
        let result = run_command(
            command(),
            Some("echo"),
            &json!({"text":"真实调用"}),
            &json!("digest"),
            Duration::from_secs(3),
            None,
        )
        .unwrap();
        assert_eq!(result["result"]["content"][0]["text"], "真实调用");
        assert!(
            run_command(
                command(),
                Some("echo"),
                &json!({"text":1}),
                &json!("digest"),
                Duration::from_secs(3),
                None,
            )
            .is_err()
        );
        std::fs::write(&source, "setInterval(()=>{},1000)").unwrap();
        let started = Instant::now();
        assert!(
            run_command(
                command(),
                None,
                &json!({}),
                &json!("digest"),
                Duration::from_millis(100),
                None,
            )
            .unwrap_err()
            .contains("结果未确认")
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        // A valid server can stop reading once discovery completed. A large
        // tools/call must still obey the host deadline instead of hanging in
        // the synchronous OS pipe write.
        std::fs::write(&source,r"let pending=''; process.stdin.on('data',chunk=>{
 pending+=chunk; let at;
 while((at=pending.indexOf('\n'))>=0){ const m=JSON.parse(pending.slice(0,at));pending=pending.slice(at+1);if(!m.id)continue;
 const result=m.method==='initialize'?{protocolVersion:'2024-11-05',capabilities:{tools:{}}}:{tools:[{name:'echo',inputSchema:{type:'object',properties:{text:{type:'string'}},required:['text'],additionalProperties:false}}]};
 console.log(JSON.stringify({jsonrpc:'2.0',id:m.id,result}));
 if(m.method==='tools/list'){process.stdin.pause();setInterval(()=>{},1000);return;}
 }});").unwrap();
        let started = Instant::now();
        let error = run_command(
            command(),
            Some("echo"),
            &json!({"text":"x".repeat(200*1024)}),
            &json!("digest"),
            Duration::from_millis(500),
            None,
        )
        .unwrap_err();
        assert!(error.contains("结果未确认"));
        assert!(started.elapsed() < Duration::from_secs(3));
        std::fs::write(&source, "throw new Error('native-bootstrap-fixture');").unwrap();
        assert!(
            run_command(
                command(),
                None,
                &json!({}),
                &json!("digest"),
                Duration::from_secs(3),
                None,
            )
            .unwrap_err()
            .contains("native-bootstrap-fixture")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "requires a built Workbench host service helper on Windows"]
    fn workbench_native_sandbox_mcp_denies_host_access() {
        assert_eq!(
            contexthub_platform_adapter::current_operating_system(),
            contexthub_platform_adapter::OperatingSystem::Windows
        );
        let helper = PathBuf::from(
            std::env::var("CILAI_NATIVE_SANDBOX_HELPER").expect("built Workbench helper required"),
        );
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let node = executable_search::find_executable(&["node"]).unwrap();
        let root =
            std::env::temp_dir().join(format!("cilai-official-mcp-{}", uuid::Uuid::new_v4()));
        let package = root.join("package");
        let data = root.join("private data");
        std::fs::create_dir_all(package.join("source/mcp")).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(root.join("outside.txt"), "private fixture").unwrap();
        let outside = serde_json::to_string(&root.join("outside.txt")).unwrap();
        let readonly = serde_json::to_string(&package.join("tampered.txt")).unwrap();
        std::fs::write(package.join("source/mcp/server.mjs"), format!(r"
import readline from 'node:readline'; import fs from 'node:fs'; import cp from 'node:child_process';
for await (const line of readline.createInterface({{input:process.stdin}})) {{
 const m=JSON.parse(line); if(!m.id) continue;
 let result;
 if(m.method==='initialize') result={{protocolVersion:'2024-11-05',capabilities:{{tools:{{}}}}}};
 else if(m.method==='tools/list') result={{tools:[{{name:'probe',inputSchema:{{type:'object',properties:{{}},additionalProperties:false}}}}]}};
 else {{ const scope={{private:false,outside:false,packageWrite:false,child:false,network:false}};
 try {{ fs.writeFileSync('allowed.txt','允许中文'); scope.private=fs.readFileSync('allowed.txt','utf8')==='允许中文'; }} catch {{}}
 try {{ fs.readFileSync({outside}); scope.outside=true; }} catch {{}}
 try {{ fs.writeFileSync({readonly},'bad'); scope.packageWrite=true; }} catch {{}}
 try {{ cp.execFileSync(process.execPath,['--version']); scope.child=true; }} catch {{}}
 try {{ await fetch('http://127.0.0.1:{port}', {{signal:AbortSignal.timeout(1000)}}); scope.network=true; }} catch {{}}
 result={{content:[{{type:'text',text:JSON.stringify(scope)+'中文'.repeat(10000)}}]}};
 }}
 console.log(JSON.stringify({{jsonrpc:'2.0',id:m.id,result}}));
}}
")).unwrap();
        let launch = || managed_component::command(&node, &package, &data, &helper).unwrap();
        let catalog = run_command(
            launch(),
            None,
            &json!({}),
            &json!("fixture"),
            Duration::from_secs(20),
            None,
        )
        .unwrap();
        assert_eq!(catalog["tools"][0]["name"], "probe");
        let result = run_command(
            launch(),
            Some("probe"),
            &json!({}),
            &json!("fixture"),
            Duration::from_secs(20),
            None,
        )
        .unwrap();
        let text = result["result"]["content"][0]["text"].as_str().unwrap();
        let (scope, remainder) = text.split_once('}').unwrap();
        let scope: Value = serde_json::from_str(&format!("{scope}}}")).unwrap();
        assert_eq!(
            scope,
            json!({"private":true,"outside":false,"packageWrite":false,"child":false,"network":false})
        );
        assert_eq!(remainder, "中文".repeat(10000));
        assert!(!std::fs::read_dir(&data).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".cilai-mcp-")
        }));
        std::fs::remove_dir_all(root).unwrap();
    }
}
