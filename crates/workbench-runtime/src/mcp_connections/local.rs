use super::{Execution, Secret, credential_map, digest, rpc};
use contexthub_platform_adapter::{
    mcp_sandbox::{Permissions, Plan},
    process, security, writer_launch_gate,
};
use contexthub_protocol::{McpConnection, McpExecutionLease, McpSource, ModelExecutionProcess};
use serde_json::{Value, json};
use std::io::{BufRead as _, Read as _, Write as _};
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Process(Child);
impl std::ops::Deref for Process {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl std::ops::DerefMut for Process {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        if self.0.wait().is_err() {
            eprintln!("mcp_local_exit_unconfirmed");
        }
    }
}
pub(super) struct Session {
    child: Process,
    sender: Option<mpsc::SyncSender<Value>>,
    receiver: mpsc::Receiver<Result<Value, String>>,
    readers: Vec<std::thread::JoinHandle<()>>,
    sequence: u64,
    plan: Plan,
    secrets: std::collections::BTreeMap<String, Secret>,
    config: contexthub_protocol::McpConnectionConfig,
}
fn reason(error: &std::io::Error) -> String {
    let text = error.to_string();
    if text.starts_with("mcp_")
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        text
    } else {
        "mcp_local_process_failed".into()
    }
}
fn managed_namespace<'a>(
    runtime: &'a std::path::Path,
    profile_id: &str,
) -> Result<&'a std::path::Path, String> {
    let profile = runtime.parent().ok_or("mcp_managed_storage_invalid")?;
    let profiles = profile.parent().ok_or("mcp_managed_storage_invalid")?;
    let business = profiles.parent().ok_or("mcp_managed_storage_invalid")?;
    let namespace = business.parent().ok_or("mcp_managed_storage_invalid")?;
    if runtime.file_name().and_then(|s| s.to_str()) != Some("ai-runtime")
        || profiles.file_name().and_then(|s| s.to_str()) != Some("profiles")
        || business.file_name().and_then(|s| s.to_str()) != Some("business")
        || namespace.file_name().and_then(|s| s.to_str()) != Some("cn.cilai.personal.workbench")
        || profile.file_name().and_then(|s| s.to_str()) != Some(profile_id)
    {
        return Err("mcp_managed_storage_invalid".into());
    }
    let id = uuid::Uuid::parse_str(profile_id).map_err(|_| "mcp_managed_storage_invalid")?;
    if id.get_version_num() != 4 || id.simple().to_string() != profile_id {
        return Err("mcp_managed_storage_invalid".into());
    }
    Ok(namespace)
}
fn spawn_registered(
    execution: &mut Execution,
    plan: &Plan,
    secrets: &std::collections::BTreeMap<String, Secret>,
) -> Result<Process, String> {
    let mut original = plan.command().map_err(|e| reason(&e))?;
    for (name, value) in secrets {
        original.env(name, value.expose());
    }
    let mut command = writer_launch_gate::wrap_command(
        &original,
        &crate::model_connections::launch_gate_executable()?,
    )
    .map_err(|_| "mcp_launch_gate_unavailable")?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    process::prepare_background_command_for_supervision(&mut command);
    execution.validate()?;
    let mut child = Process(command.spawn().map_err(|_| "mcp_local_spawn_failed")?);
    execution.local_started = true;
    if process::supervise_spawned_child(&mut child).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("mcp_local_supervision_failed".into());
    }
    let register = (|| {
        let fingerprint =
            contexthub_platform_adapter::instance::process_start_fingerprint(child.id())
                .map_err(|_| "mcp_process_unconfirmed")?
                .ok_or("mcp_process_unconfirmed")?;
        let process = ModelExecutionProcess {
            pid: child.id(),
            start_fingerprint: fingerprint,
        };
        let lease: McpExecutionLease = rpc(
            &execution.server,
            "mcpExecution/registerProcess",
            &json!({"leaseId":execution.lease.lease_id,"snapshotDigest":execution.lease.snapshot_digest,"process":process}),
        )?;
        execution.lease = lease;
        plan.validate().map_err(|e| reason(&e))?;
        execution.validate()?;
        child
            .stdin
            .as_mut()
            .ok_or("mcp_stdio_unavailable")?
            .write_all(writer_launch_gate::PERMIT)
            .and_then(|()| {
                child
                    .stdin
                    .as_mut()
                    .ok_or_else(|| std::io::Error::other("mcp_stdio_unavailable"))?
                    .flush()
            })
            .map_err(|_| "mcp_launch_permit_unconfirmed")?;
        Ok::<_, String>(())
    })();
    if let Err(error) = register {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    Ok(child)
}
impl Session {
    pub fn open(execution: &mut Execution, conn: &McpConnection) -> Result<Self, String> {
        let McpSource::LocalCommand {
            program,
            args,
            cwd,
            permissions,
        } = &conn.config.source
        else {
            return Err("mcp_local_source_invalid".into());
        };
        let home = execution
            .data_dir
            .join("mcp-runtime")
            .join(&conn.connection_id)
            .join(&execution.lease.lease_id);
        security::ensure_restricted_directory(&home).map_err(|_| "mcp_runtime_home_invalid")?;
        let plan = Plan::capture_managed(
            std::path::Path::new(program),
            args,
            std::path::Path::new(cwd),
            &home,
            &Permissions {
                read_only: permissions.read_roots.iter().map(Into::into).collect(),
                writable: permissions.write_roots.iter().map(Into::into).collect(),
                network: permissions.network,
                child_processes: permissions.child_processes,
            },
            managed_namespace(&execution.data_dir, &conn.profile_id)?,
        )
        .map_err(|e| reason(&e))?;
        if execution.lease.intent == contexthub_protocol::McpExecutionIntent::ToolCall {
            let validation = conn.validation.as_ref().ok_or("mcp_validation_required")?;
            let source = digest(
                &json!({"source":plan.source_fingerprint().map_err(|_|"mcp_source_invalid")?,"config":conn.config,"serverName":validation.server_name,"serverVersion":validation.server_version}),
            )?;
            if validation.source_fingerprint.as_ref() != Some(&source) {
                return Err("mcp_source_changed".into());
            }
        }
        let secrets = credential_map(&execution.data_dir, conn)?;
        let mut child = spawn_registered(execution, &plan, &secrets)?;
        let stdin = child.stdin.take().ok_or("mcp_stdio_unavailable")?;
        let stdout = child.stdout.take().ok_or("mcp_stdio_unavailable")?;
        let (sender, pending_input) = mpsc::sync_channel::<Value>(1);
        let (responses, receiver) = mpsc::sync_channel(16);
        let errors = responses.clone();
        let writer = std::thread::Builder::new()
            .name("user-mcp-input".into())
            .spawn(move || {
                let mut stdin = stdin;
                for value in pending_input {
                    if writeln!(stdin, "{value}")
                        .and_then(|()| stdin.flush())
                        .is_err()
                    {
                        let _ = errors.try_send(Err("mcp_stdio_write_failed".into()));
                        break;
                    }
                }
            })
            .map_err(|_| "mcp_stdio_unavailable")?;
        let reader = std::thread::Builder::new()
            .name("user-mcp-output".into())
            .spawn(move || {
                let mut reader = std::io::BufReader::new(stdout);
                loop {
                    let mut bytes = Vec::new();
                    match reader
                        .by_ref()
                        .take(1024 * 1024 + 1)
                        .read_until(b'\n', &mut bytes)
                    {
                        Ok(0) => {
                            let _ = responses.try_send(Err("mcp_stdio_closed".into()));
                            break;
                        }
                        Ok(_) if bytes.len() <= 1024 * 1024 => {
                            let value = serde_json::from_slice(&bytes)
                                .map_err(|_| "mcp_response_invalid".to_owned());
                            if responses.try_send(value).is_err() {
                                break;
                            }
                        }
                        _ => {
                            let _ = responses.try_send(Err("mcp_response_too_large".into()));
                            break;
                        }
                    }
                }
            })
            .map_err(|_| "mcp_stdio_unavailable")?;
        Ok(Self {
            child,
            sender: Some(sender),
            receiver,
            readers: vec![writer, reader],
            sequence: 0,
            plan,
            secrets,
            config: conn.config.clone(),
        })
    }
    pub async fn request(
        &mut self,
        execution: &mut Execution,
        method: &str,
        params: Value,
        side_effect: bool,
    ) -> Result<Value, String> {
        self.plan.validate().map_err(|e| reason(&e))?;
        execution.validate()?;
        self.sequence += 1;
        let id = self.sequence;
        execution.request_sent |= side_effect;
        self.send(
            execution,
            &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        )
        .await?;
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            execution.validate()?;
            match self.receiver.try_recv() {
                Ok(Ok(value)) => {
                    if value.get("method").is_some() {
                        if value.get("id").is_some() {
                            return Err("mcp_server_requests_not_supported".into());
                        }
                        if value["method"] == "notifications/tools/list_changed" {
                            return Err("mcp_catalog_changed".into());
                        }
                        continue;
                    }
                    let result = super::remote::parse_response(
                        "application/json",
                        value.to_string().as_bytes(),
                        id,
                    )?;
                    super::safe_response(&result, self.secrets.values())?;
                    self.plan.validate().map_err(|e| reason(&e))?;
                    return Ok(result);
                }
                Ok(Err(code)) => {
                    return Err(if side_effect {
                        "mcp_tool_outcome_unconfirmed".into()
                    } else {
                        code
                    });
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(if side_effect {
                        "mcp_tool_outcome_unconfirmed"
                    } else {
                        "mcp_stdio_closed"
                    }
                    .into());
                }
                Err(mpsc::TryRecvError::Empty) => (),
            }
            if Instant::now() >= deadline {
                return Err(if side_effect {
                    "mcp_tool_outcome_unconfirmed"
                } else {
                    "mcp_local_timeout"
                }
                .into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    async fn send(&self, execution: &Execution, value: &Value) -> Result<(), String> {
        let mut value = value.clone();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            execution.validate()?;
            match self
                .sender
                .as_ref()
                .ok_or("mcp_stdio_closed")?
                .try_send(value)
            {
                Ok(()) => return Ok(()),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err("mcp_stdio_write_unconfirmed".into());
                }
                Err(mpsc::TrySendError::Full(pending)) => value = pending,
            }
            if Instant::now() >= deadline {
                return Err("mcp_stdio_write_unconfirmed".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    pub async fn notify(&self, execution: &Execution, value: &Value) -> Result<(), String> {
        self.send(execution, value).await
    }
    pub fn fingerprint(&self, name: &str, version: &str) -> Result<String, String> {
        digest(
            &json!({"source":self.plan.source_fingerprint().map_err(|_|"mcp_source_invalid")?,"config":self.config,"serverName":name,"serverVersion":version}),
        )
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.sender.take();
        let _ = self.child.kill();
        if self.child.wait().is_err() {
            eprintln!("mcp_local_exit_unconfirmed");
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}
