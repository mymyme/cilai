use crate::supervisor::AppServerSupervisor;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) const THREAD_PROTOCOL_VERSION: u32 = 1;

#[derive(Clone)]
pub(crate) struct ThreadJournal {
    pub writer_process: Value,
    pub runtime_scope: Option<Value>,
    pub model_execution_lease_id: Option<String>,
    pub history_consent_confirmed: bool,
    pub application_scope: Option<contexthub_protocol::AiThreadApplicationScope>,
    pub server: AppServerSupervisor,
    pub conversation_id: String,
    pub run_id: String,
    pub owner_instance_id: String,
}

#[derive(Clone)]
pub(crate) struct PersistentThread {
    pub journal: ThreadJournal,
    pub native_thread_id: Option<String>,
    pub fork_before_turn_id: Option<String>,
    pub workbench_history_fork: bool,
    pub cross_runtime_fork: bool,
    pub workbench_history: Vec<Value>,
    pub name: String,
    pub history: Vec<Value>,
    pub message_id: String,
    pub client_message_id: String,
    pub content_digest: String,
    pub writer_process: Value,
}

pub(crate) fn owner_instance_id() -> String {
    static OWNER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    OWNER
        .get_or_init(|| uuid::Uuid::new_v4().to_string())
        .clone()
}

impl ThreadJournal {
    pub(crate) fn state(&self) -> Result<Value, String> {
        self.server.request_result(
            "aiThread/get",
            &json!({"conversationId":self.conversation_id}),
        )
    }

    pub(crate) fn prepare(&self, suffix: &str, kind: &str, input: Value) -> Result<(), String> {
        let state = self.state()?;
        let revision = state
            .pointer("/binding/bindingRevision")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let mut request = json!({
            "conversationId":self.conversation_id,"operationId":self.operation_id(suffix),
            "runId":self.run_id,"ownerInstanceId":self.owner_instance_id,
            "expectedBindingRevision":revision,"protocolVersion":THREAD_PROTOCOL_VERSION,"kind":kind,
        });
        if let Some(scope) = &self.application_scope {
            request["applicationScope"] = serde_json::to_value(scope)
                .map_err(|_| "application_execution_scope_invalid".to_owned())?;
        }
        let mut input = input;
        if let Some(scope) = &self.runtime_scope {
            let object = input.as_object_mut().ok_or("model_thread_input_invalid")?;
            object.insert("runtimeScope".into(), scope.clone());
            object.insert("writerProcess".into(), self.writer_process.clone());
            object.insert(
                "modelExecutionLeaseId".into(),
                json!(self.model_execution_lease_id),
            );
            object.insert(
                "historyConsentConfirmed".into(),
                json!(self.history_consent_confirmed),
            );
        }
        request["input"] = input;
        let result = self.server.request_result("aiThread/prepare", &request)?;
        if result["replayed"] == true {
            return Err("message_delivery_uncertain".to_owned());
        }
        Ok(())
    }

    pub(crate) fn record(&self, suffix: &str, event: &str, receipt: Value) -> Result<(), String> {
        let mut request = json!({
            "conversationId":self.conversation_id,"operationId":self.operation_id(suffix),
            "runId":self.run_id,"ownerInstanceId":self.owner_instance_id,"event":event,
        });
        request["receipt"] = receipt;
        self.server
            .request_result("aiThread/acknowledge", &request)?;
        Ok(())
    }

    pub(crate) fn release(&self) -> Result<(), String> {
        let state = self.state()?;
        if state["binding"].is_null() || state["binding"]["activeRunId"].is_null() {
            return Ok(());
        }
        self.server.request_result(
            "aiThread/release",
            &json!({
                "conversationId":self.conversation_id,"runId":self.run_id,
                "ownerInstanceId":self.owner_instance_id,"processReaped":true,
            }),
        )?;
        Ok(())
    }

    pub(crate) fn park(&self) -> Result<(), String> {
        self.server.request_result(
            "aiThread/release",
            &json!({
                "conversationId":self.conversation_id,"runId":self.run_id,
                "ownerInstanceId":self.owner_instance_id,"processReaped":false,"retainWriter":true,
            }),
        )?;
        Ok(())
    }

    fn operation_id(&self, suffix: &str) -> String {
        format!("{}:{suffix}", self.run_id)
    }

    pub(crate) fn prepare_input(
        &self,
        message_id: &str,
        client_message_id: &str,
        digest: &str,
        steer: bool,
        content: &str,
    ) -> Result<(), String> {
        self.prepare(&format!("input:{client_message_id}"),if steer { "steer" } else { "send" },json!({"messageId":message_id,"clientMessageId":client_message_id,"contentDigest":digest,"content":content,"createdAt":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|_|"ai_clock_invalid")?.as_millis()}))?;
        self.record(
            &format!("input:{client_message_id}"),
            "dispatch",
            Value::Null,
        )
    }

    pub(crate) fn accept_input(
        &self,
        client_message_id: &str,
        native_thread_id: &str,
        native_turn_id: &str,
    ) -> Result<(), String> {
        self.record(&format!("input:{client_message_id}"),"acknowledge",json!({"nativeThreadId":native_thread_id,"nativeTurnId":native_turn_id,"clientMessageId":client_message_id}))
    }
}

pub(crate) fn digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

pub(crate) fn validate_binding(state: &Value, owner: &str) -> Result<Option<String>, String> {
    let Some(binding) = state.get("binding").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    if binding["protocolVersion"] != THREAD_PROTOCOL_VERSION {
        return Err("history_migration_required".to_owned());
    }
    if binding["activeRunId"].is_string()
        && !(binding["writerParked"] == true && binding["ownerInstanceId"] == owner)
    {
        return Err(if binding["ownerInstanceId"] == owner {
            "thread_owner_busy"
        } else {
            "thread_owner_recovery_required"
        }
        .to_owned());
    }
    if binding["status"] == "blocked"
        && binding["nativeThreadId"].is_null()
        && state["operations"].as_array().is_some_and(|operations| {
            !operations.is_empty()
                && operations
                    .iter()
                    .all(|operation| operation["state"] == "rejected")
        })
    {
        return Ok(None);
    }
    if binding["status"] == "blocked" {
        return Err("thread_binding_blocked".to_owned());
    }
    if binding["status"] != "ready" {
        return Err("message_delivery_uncertain".to_owned());
    }
    binding["nativeThreadId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(|id| Some(id.to_owned()))
        .ok_or_else(|| "thread_binding_invalid".to_owned())
}

/// 编辑目标对应的回退轮次；`None` 表示目标从未派发到原生线程（例如运行在沙箱或能力目录检查时
/// 就已失败）：线程里没有这条消息，重新生成直接接在线程末尾。只要曾派发过——即使送达结果未确认——
/// 仍按轮次映射回退，映射缺失时报错，不猜测送达状态。
pub(crate) fn regeneration_fork(state: &Value, message_id: &str) -> Result<Option<String>, String> {
    let linked = state["messages"].as_array().is_some_and(|messages| {
        messages
            .iter()
            .any(|message| message["messageId"] == message_id)
    });
    let dispatched = state["operations"].as_array().is_some_and(|operations| {
        operations
            .iter()
            .any(|operation| operation["request"]["input"]["messageId"] == message_id)
    });
    if !linked && !dispatched {
        return Ok(None);
    }
    fork_boundary(state, message_id).map(Some)
}

pub(crate) fn fork_boundary(state: &Value, message_id: &str) -> Result<String, String> {
    let generation = &state["binding"]["generation"];
    let messages = state["messages"]
        .as_array()
        .ok_or_else(|| "thread_message_mapping_missing".to_owned())?;
    let target = messages
        .iter()
        .rev()
        .find(|message| message["generation"] == *generation && message["messageId"] == message_id)
        .ok_or_else(|| "thread_message_mapping_missing".to_owned())?;
    let turn = target["nativeTurnId"]
        .as_str()
        .ok_or_else(|| "thread_message_mapping_missing".to_owned())?;
    if messages.iter().any(|message| {
        message["generation"] == *generation
            && message["nativeTurnId"] == turn
            && message["messageId"] != message_id
    }) {
        return Err("thread_edit_prefix_unavailable".to_owned());
    }
    Ok(turn.to_owned())
}

pub(crate) fn history_batches(items: &[Value]) -> Result<Vec<Vec<Value>>, String> {
    const LIMIT: usize = 4 * 1024 * 1024;
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    let mut bytes = 0;
    for item in items {
        let size = item.to_string().len();
        if size > LIMIT {
            return Err("history_item_too_large".to_owned());
        }
        if !batch.is_empty() && (batch.len() == 32 || bytes + size > LIMIT) {
            batches.push(std::mem::take(&mut batch));
            bytes = 0;
        }
        batch.push(item.clone());
        bytes += size;
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    Ok(batches)
}

pub(crate) fn native_request(
    executable: &str,
    home: &std::path::Path,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    let mut command = recovery_command(executable, home)?;
    contexthub_platform_adapter::process::prepare_command_for_supervision(&mut command);
    let mut child = command
        .spawn()
        .map_err(|_| "thread_recovery_service_unavailable".to_owned())?;
    let result = (|| {
        contexthub_platform_adapter::process::supervise_spawned_child(&mut child)
            .map_err(|_| "thread_recovery_service_unavailable".to_owned())?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or("thread_recovery_service_unavailable")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("thread_recovery_service_unavailable")?;
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut answer = Value::Null;
        for (id, method, params) in [
            (
                1,
                "initialize",
                json!({"clientInfo":{"name":"cilai-thread-recovery","version":"1"},"capabilities":{"experimentalApi":true}}),
            ),
            (2, method, params),
        ] {
            writeln!(
                stdin,
                "{}",
                json!({"id":id,"method":method,"params":params})
            )
            .map_err(|_| "thread_recovery_transport_failed")?;
            stdin
                .flush()
                .map_err(|_| "thread_recovery_transport_failed")?;
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let line = receiver
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .map_err(|_| "thread_recovery_timed_out")?
                    .map_err(|_| "thread_recovery_transport_failed")?;
                let message: Value =
                    serde_json::from_str(&line).map_err(|_| "thread_recovery_response_invalid")?;
                if message["id"] != id {
                    continue;
                }
                if message.get("error").is_some() {
                    return Err("thread_recovery_read_failed".to_owned());
                }
                answer = message
                    .get("result")
                    .cloned()
                    .ok_or("thread_recovery_response_invalid")?;
                break;
            }
            if id == 1 {
                writeln!(stdin, "{}", json!({"method":"initialized","params":{}}))
                    .map_err(|_| "thread_recovery_transport_failed")?;
                stdin
                    .flush()
                    .map_err(|_| "thread_recovery_transport_failed")?;
            }
        }
        Ok(answer)
    })();
    let _ = child.kill();
    child
        .wait()
        .map_err(|_| "thread_recovery_process_unreaped".to_owned())?;
    result
}

fn native_turn_for_client(thread: &Value, client_id: &str) -> Result<Option<String>, String> {
    let turns = thread
        .pointer("/thread/turns")
        .and_then(Value::as_array)
        .ok_or("thread_recovery_response_invalid")?;
    let mut found = None;
    for turn in turns {
        for item in turn["items"]
            .as_array()
            .ok_or("thread_recovery_response_invalid")?
        {
            if item["type"] == "userMessage" && item["clientId"] == client_id {
                if found.is_some() {
                    return Err("thread_recovery_duplicate_receipt".to_owned());
                }
                found = Some(
                    turn["id"]
                        .as_str()
                        .ok_or("thread_recovery_response_invalid")?
                        .to_owned(),
                );
            }
        }
    }
    Ok(found)
}

pub(crate) fn recover(
    server: &AppServerSupervisor,
    conversation_id: &str,
    mut state: Value,
    executable: &str,
    home: &std::path::Path,
) -> Result<Value, String> {
    let current = &state["binding"];
    if let Some(run_id) = current["activeRunId"].as_str() {
        if current["ownerInstanceId"] == owner_instance_id() {
            return Ok(state);
        }
        let operation = state["operations"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|operation| {
                    operation["request"]["runId"] == run_id
                        && matches!(
                            operation["request"]["kind"].as_str(),
                            Some("create" | "resume" | "fork")
                        )
                })
            })
            .ok_or("thread_owner_recovery_required")?;
        let identity = &operation["request"]["input"]["writerProcess"];
        let pid = identity["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or("thread_owner_recovery_required")?;
        let fingerprint = identity["startFingerprint"]
            .as_str()
            .ok_or("thread_owner_recovery_required")?;
        let live = contexthub_platform_adapter::instance::process_start_fingerprint(pid)
            .map_err(|_| "thread_owner_recovery_required")?;
        if live.as_deref() == Some(fingerprint) {
            return Err("thread_owner_recovery_required".to_owned());
        }
        server.request_result("aiThread/release",&json!({"conversationId":conversation_id,"runId":run_id,"ownerInstanceId":current["ownerInstanceId"],"processReaped":true}))?;
        state =
            server.request_result("aiThread/get", &json!({"conversationId":conversation_id}))?;
    }
    if state["binding"]["status"] != "reconciling" {
        return Ok(state);
    }
    let thread_id = state["binding"]["nativeThreadId"]
        .as_str()
        .ok_or("message_delivery_uncertain")?;
    let thread = native_request(
        executable,
        home,
        "thread/read",
        json!({"threadId":thread_id,"includeTurns":true}),
    )?;
    if thread.pointer("/thread/id").and_then(Value::as_str) != Some(thread_id) {
        return Err("thread_recovery_response_invalid".to_owned());
    }
    for operation in state["operations"]
        .as_array()
        .ok_or("thread_recovery_response_invalid")?
    {
        if operation["state"] != "uncertain" {
            continue;
        }
        let request = &operation["request"];
        let mut receipt = json!({"nativeThreadId":thread_id});
        match request["kind"].as_str() {
            Some("send" | "steer") => {
                let client = request["input"]["clientMessageId"]
                    .as_str()
                    .ok_or("thread_recovery_response_invalid")?;
                let Some(turn_id) = native_turn_for_client(&thread, client)? else {
                    continue;
                };
                receipt["nativeTurnId"] = json!(turn_id);
                receipt["clientMessageId"] = json!(client);
            }
            Some("resume") => {}
            _ => continue,
        }
        server.request_result("aiThread/reconcile",&json!({"conversationId":conversation_id,"operationId":request["operationId"],"runId":request["runId"],"ownerInstanceId":request["ownerInstanceId"],"event":"acknowledge","receipt":receipt}))?;
    }
    server.request_result("aiThread/get", &json!({"conversationId":conversation_id}))
}

fn recovery_command(
    executable: &str,
    home: &std::path::Path,
) -> Result<std::process::Command, String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(executable);
    crate::model_connections::isolate_runtime_environment(&mut command, home)?;
    crate::codex_upgrade::configure_aliases(&mut command, home)?;
    command
        .env("CODEX_HOME", home)
        .env("CODEX_SQLITE_HOME", home)
        .args([
            "--listen",
            "stdio://",
            "-c",
            "cli_auth_credentials_store=\"file\"",
            "-c",
            "features.secret_auth_storage=false",
            "-c",
            "features.memories=false",
            "-c",
            "features.plugins=false",
            "-c",
            "check_for_update_on_startup=false",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::model_connections::direct_native_network()?.apply_native(&mut command);
    command.args(["-c", "skills.bundled.enabled=false"]);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parked_binding_is_reusable_only_by_its_owner() {
        let mut state = json!({"binding":{"nativeThreadId":"t","protocolVersion":1,"status":"ready","ownerInstanceId":"host-1","activeRunId":"old","writerParked":true}});
        assert_eq!(
            validate_binding(&state, "host-1").unwrap(),
            Some("t".into())
        );
        assert!(validate_binding(&state, "host-2").is_err());
        state["binding"]["writerParked"] = json!(false);
        assert!(validate_binding(&state, "host-1").is_err());
    }

    #[test]
    fn reconciliation_requires_one_exact_native_client_receipt() {
        let history = json!({"thread":{"turns":[{"id":"turn-1","items":[{"type":"userMessage","clientId":"a"},{"type":"userMessage","clientId":"b"}]}]}});
        assert_eq!(
            native_turn_for_client(&history, "b").unwrap().as_deref(),
            Some("turn-1")
        );
        assert_eq!(native_turn_for_client(&history, "missing").unwrap(), None);
        let mut duplicate = history.clone();
        duplicate["thread"]["turns"]
            .as_array_mut()
            .unwrap()
            .push(history["thread"]["turns"][0].clone());
        assert!(native_turn_for_client(&duplicate, "b").is_err());
    }

    #[test]
    fn import_batches_preserve_order_and_reject_oversized_items() {
        let history = (0..65)
            .map(|index| json!({"text":index}))
            .collect::<Vec<_>>();
        let batches = history_batches(&history).unwrap();
        assert_eq!(
            batches.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![32, 32, 1]
        );
        assert_eq!(batches.into_iter().flatten().collect::<Vec<_>>(), history);
        assert!(history_batches(&[json!({"text":"x".repeat(4*1024*1024)})]).is_err());
    }

    #[test]
    fn binding_never_selects_an_older_matching_model() {
        let state = json!({"binding":{"nativeThreadId":"current","protocolVersion":1,"status":"ready","generation":2},"runs":[{"model":"A","sessionId":"old"}]});
        assert_eq!(
            validate_binding(&state, "host").unwrap().as_deref(),
            Some("current")
        );
        let mut uncertain = state;
        uncertain["binding"]["status"] = json!("reconciling");
        assert!(validate_binding(&uncertain, "host").is_err());
        uncertain["binding"]["status"] = json!("blocked");
        assert_eq!(
            validate_binding(&uncertain, "host").unwrap_err(),
            "thread_binding_blocked"
        );
    }

    #[test]
    fn editing_a_shared_turn_requires_a_verified_prefix() {
        let mut state = json!({"binding":{"generation":1},"messages":[{"generation":1,"messageId":"m1","nativeTurnId":"turn"}]});
        assert_eq!(fork_boundary(&state, "m1").unwrap(), "turn");
        state["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({"generation":1,"messageId":"steer","nativeTurnId":"turn"}));
        assert_eq!(
            fork_boundary(&state, "steer").unwrap_err(),
            "thread_edit_prefix_unavailable"
        );
    }

    #[test]
    fn regenerating_a_message_that_never_reached_the_thread_appends_at_the_tail() {
        let thread_only = json!({
            "binding":{"generation":1},
            "operations":[{"request":{"kind":"resume","input":{"sourceThreadId":"thread"}},"state":"acknowledged"}],
            "messages":[],
        });
        assert_eq!(
            regeneration_fork(&thread_only, "failed-early").unwrap(),
            None
        );

        let delivered = json!({
            "binding":{"generation":1},
            "operations":[{"request":{"kind":"send","input":{"messageId":"m1"}},"state":"acknowledged"}],
            "messages":[{"generation":1,"messageId":"m1","nativeTurnId":"turn"}],
        });
        assert_eq!(
            regeneration_fork(&delivered, "m1").unwrap(),
            Some("turn".to_owned())
        );

        let unconfirmed = json!({
            "binding":{"generation":1},
            "operations":[{"request":{"kind":"send","input":{"messageId":"m2"}},"state":"dispatched"}],
            "messages":[],
        });
        assert_eq!(
            regeneration_fork(&unconfirmed, "m2").unwrap_err(),
            "thread_message_mapping_missing"
        );
    }
}
