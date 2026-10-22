use super::*;
use serde_json::json;

const MAX_HANDOFF_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceRun {
    model_id: String,
    model_label: String,
    started_at: i64,
    completed_at: i64,
    events: Vec<AgentRunEvent>,
}

impl SourceRun {
    fn from_record(record: &ApplicationModelRunRecord) -> Result<Self, String> {
        let snapshot = &record.snapshot;
        let completed_at = snapshot
            .completed_at
            .ok_or("application_handoff_run_invalid")?;
        if snapshot.model_id.is_empty()
            || snapshot.model_label.is_empty()
            || completed_at < snapshot.started_at
        {
            return Err("application_handoff_run_invalid".into());
        }
        Ok(Self {
            model_id: snapshot.model_id.clone(),
            model_label: snapshot.model_label.clone(),
            started_at: snapshot.started_at,
            completed_at,
            events: record.events.clone(),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationHandoffCommitRequest {
    #[serde(rename = "sessionId")]
    session: String,
    #[serde(rename = "handoffId")]
    handoff: String,
    #[serde(rename = "projectId")]
    project: Option<String>,
}

pub(super) struct PendingHandoff {
    ledger: ledger::Pending,
    params: Value,
    conversation: Conversation,
    messages: Vec<ConversationMessage>,
    handoff_id: String,
}

fn completed<'a>(state: &'a ManagerState, handoff_id: &str) -> Option<&'a Conversation> {
    state.persisted.conversations.iter().find(|conversation| {
        conversation
            .application_handoff
            .as_ref()
            .is_some_and(|source| source["handoffId"] == handoff_id)
    })
}

pub(super) fn flush(state: &mut ManagerState) -> Result<(), String> {
    let Some(pending) = state.pending_application_handoff.as_ref() else {
        return Ok(());
    };
    let server = state.server.as_ref().ok_or("ai_runtime_not_initialized")?;
    match ledger_request(server, "aiHistory/execute", &pending.params) {
        Ok(result) => {
            state
                .ledger_baseline
                .acknowledge(&pending.ledger, &result["value"])?;
            let pending = state
                .pending_application_handoff
                .take()
                .ok_or("application_handoff_missing")?;
            state.persisted.conversations.push(pending.conversation);
            state.persisted.messages.extend(pending.messages);
            if let Some(queue) = state
                .persisted
                .conversation_session
                .get_mut("applicationHandoffs")
                .and_then(Value::as_array_mut)
            {
                queue.retain(|entry| entry["handoffId"] != pending.handoff_id);
            }
            state.state_revision = state.state_revision.saturating_add(1);
            state.conversation_session_revision += 1;
            Ok(())
        }
        Err(error) => {
            if error.starts_with("App Server returned protocol error:") {
                state.pending_application_handoff = None;
            }
            Err(error)
        }
    }
}

fn transfer_phase(state: &mut ManagerState, handoff: &str, transfer: &Value) -> Result<(), String> {
    let previous = state.persisted.clone();
    let entry = state
        .persisted
        .conversation_session
        .get_mut("applicationHandoffs")
        .and_then(Value::as_array_mut)
        .and_then(|q| q.iter_mut().find(|e| e["handoffId"] == handoff))
        .ok_or("application_handoff_missing")?;
    entry["transfer"] = transfer.clone();
    persist_locked_or_restore(state, previous)?;
    state.conversation_session_revision += 1;
    Ok(())
}

fn handoff_attachments(
    state: &mut ManagerState,
    handoff: &Value,
    transfer: &mut Value,
) -> Result<Vec<MessageAttachment>, String> {
    let Some(session) = handoff["sourceSessionId"].as_str() else {
        return Ok(Vec::new());
    };
    let server = state.server.as_ref().ok_or("ai_runtime_not_initialized")?;
    let stored =
        AiRuntimeManager::application_store(server, "read", &json!({"sessionId":session}))?;
    let source = stored["messages"]
        .as_array()
        .and_then(|m| m.iter().find(|m| m["runId"] == handoff["runId"]))
        .ok_or("application_handoff_source_missing")?;
    let sources: Vec<MessageAttachment> = serde_json::from_value(source["attachments"].clone())
        .map_err(|_| "application_handoff_attachments_invalid")?;
    let root = data_dir_for_state(state)?.join("attachments");
    let targets = sources
        .iter()
        .map(|attachment| {
            let original = PathBuf::from(&attachment.local_path);
            if original.parent() != Some(root.as_path())
                || !fs::symlink_metadata(&original)
                    .map_err(|_| "application_handoff_attachment_missing")?
                    .file_type()
                    .is_file()
            {
                return Err("application_handoff_attachment_invalid".to_owned());
            }
            let id = format!(
                "attachment_handoff_{}",
                sha256_bytes(
                    format!("{}:{}", handoff["handoffId"], attachment.attachment_id).as_bytes()
                )
            );
            Ok(MessageAttachment {
                attachment_id: id.clone(),
                local_path: root.join(id).to_string_lossy().into_owned(),
                ..attachment.clone()
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    transfer["attachments"] = json!(targets);
    transfer_phase(
        state,
        handoff["handoffId"]
            .as_str()
            .ok_or("application_handoff_invalid")?,
        transfer,
    )?;
    for (source, target) in sources.iter().zip(&targets) {
        let bytes =
            fs::read(&source.local_path).map_err(|_| "application_handoff_attachment_missing")?;
        match fs::symlink_metadata(&target.local_path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                use std::io::Write;
                let copied = fs::read(&target.local_path)
                    .map_err(|_| "application_handoff_attachment_invalid")?;
                if copied != bytes
                    && (transfer["phase"] != "prepared" || !bytes.starts_with(&copied))
                {
                    return Err("application_handoff_attachment_changed".into());
                }
                let mut file = fs::OpenOptions::new()
                    .append(true)
                    .open(&target.local_path)
                    .map_err(|_| "application_handoff_attachment_copy_failed")?;
                file.write_all(&bytes[copied.len()..])
                    .map_err(|_| "application_handoff_attachment_copy_failed")?;
                file.sync_all()
                    .map_err(|_| "application_handoff_attachment_copy_failed")?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                use std::io::Write;
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target.local_path)
                    .map_err(|_| "application_handoff_attachment_copy_failed")?;
                file.write_all(&bytes)
                    .map_err(|_| "application_handoff_attachment_copy_failed")?;
                file.sync_all()
                    .map_err(|_| "application_handoff_attachment_copy_failed")?;
            }
            Ok(_) | Err(_) => return Err("application_handoff_attachment_invalid".into()),
        }
    }
    Ok(targets)
}

impl AiRuntimeManager {
    /// # Errors
    /// 运行归属、结果、容量或持久化校验失败时拒绝转交。
    pub fn create_application_handoff(
        &self,
        application_id: &str,
        run_id: &str,
    ) -> Result<Value, String> {
        let mut state = lock(&self.inner);
        persist_locked(&mut state)?;
        let handoff_id = format!("handoff_{run_id}");
        let mut handoff = if let Some(conversation) = completed(&state, &handoff_id) {
            let source = conversation
                .application_handoff
                .as_ref()
                .ok_or("application_handoff_missing")?;
            if source["applicationId"] != application_id {
                return Err("application_handoff_run_invalid".into());
            }
            json!({"handoffId":handoff_id,"applicationId":application_id,"runId":run_id,"conversationId":conversation.conversation_id,"source":{"applicationName":source["applicationName"]}})
        } else {
            let record = state
                .application_model_runs
                .get(run_id)
                .ok_or("application_model_run_not_found")?;
            if record.snapshot.application_id != application_id
                || record.snapshot.status != "completed"
            {
                return Err("application_handoff_run_invalid".into());
            }
            let output = record
                .snapshot
                .output
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or("application_model_output_missing")?;
            json!({"handoffId":handoff_id,"applicationId":application_id,"runId":run_id,"sourceSessionId":record.persistent_session_id,"source":record.handoff_context,"runSnapshot":SourceRun::from_record(record)?,"output":output,"outputDigest":sha256_bytes(output.as_bytes()),"receipt":record.snapshot.receipt,"createdAt":unix_epoch_millis()?})
        };
        handoff["requestId"] = uuid::Uuid::new_v4().to_string().into();
        if handoff["output"]
            .as_str()
            .is_some_and(|text| text.chars().count() > MAX_MESSAGE_CHARS)
            || handoff
                .pointer("/source/prompt")
                .and_then(Value::as_str)
                .is_some_and(|text| text.chars().count() > MAX_MESSAGE_CHARS)
            || serde_json::to_vec(&handoff)
                .map_err(|error| error.to_string())?
                .len()
                > MAX_HANDOFF_BYTES
        {
            return Err("application_handoff_too_large".into());
        }
        let previous = state.persisted.clone();
        let queue = state
            .persisted
            .conversation_session
            .entry("applicationHandoffs".into())
            .or_insert(json!([]))
            .as_array_mut()
            .ok_or("application_handoff_invalid")?;
        if let Some(entry) = queue
            .iter_mut()
            .find(|value| value["handoffId"] == handoff_id)
        {
            if entry.get("conversationId").is_none() && handoff.get("conversationId").is_none() {
                entry["requestId"] = handoff["requestId"].clone();
                handoff = entry.clone();
            } else {
                *entry = handoff.clone();
            }
        } else {
            if queue.len() >= 20 {
                return Err("application_handoff_queue_full".into());
            }
            queue.push(handoff.clone());
        }
        if serde_json::to_vec(&state.persisted.conversation_session)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_HANDOFF_BYTES
        {
            state.persisted = previous;
            return Err("conversation_session_too_large".into());
        }
        persist_locked_or_restore(&mut state, previous)?;
        state.conversation_session_revision += 1;
        Ok(handoff)
    }

    /// # Errors
    /// 转交记录与应用不匹配时拒绝查询。
    pub fn application_handoff_status(
        &self,
        application_id: &str,
        run_id: &str,
    ) -> Result<Value, String> {
        let state = lock(&self.inner);
        let id = format!("handoff_{run_id}");
        if let Some(conversation) = completed(&state, &id) {
            if conversation
                .application_handoff
                .as_ref()
                .is_none_or(|source| source["applicationId"] != application_id)
            {
                return Err("application_handoff_run_invalid".into());
            }
            return Ok(
                json!({"handoffId":id,"status":"completed","conversationId":conversation.conversation_id}),
            );
        }
        let entry = state
            .persisted
            .conversation_session
            .get("applicationHandoffs")
            .and_then(Value::as_array)
            .and_then(|queue| queue.iter().find(|entry| entry["handoffId"] == id));
        if entry.is_some_and(|entry| entry["applicationId"] != application_id) {
            return Err("application_handoff_run_invalid".into());
        }
        Ok(
            json!({"handoffId":id,"status":if entry.is_some() {"awaiting_project"} else {"cancelled"}}),
        )
    }

    /// # Errors
    /// An uncertain target creation or published conversation cannot be discarded.
    pub fn cancel_application_handoff(
        &self,
        request: &ApplicationHandoffCommitRequest,
    ) -> Result<Value, String> {
        let mut state = lock(&self.inner);
        if request.session != state.conversation_session_id {
            return Err("conversation_session_changed".into());
        }
        persist_locked(&mut state)?;
        if let Some(conversation) = completed(&state, &request.handoff) {
            return Ok(json!({"status":"completed","conversationId":conversation.conversation_id}));
        }
        let handoff = state
            .persisted
            .conversation_session
            .get("applicationHandoffs")
            .and_then(Value::as_array)
            .and_then(|q| q.iter().find(|h| h["handoffId"] == request.handoff))
            .cloned()
            .ok_or("application_handoff_missing")?;
        let mut transfer = handoff["transfer"].clone();
        if transfer["phase"] == "creating" && transfer["nativeThreadId"].is_null() {
            return Err("application_handoff_creation_unconfirmed".into());
        }
        if let Some(thread) = transfer["nativeThreadId"].as_str().map(str::to_owned) {
            transfer["phase"] = json!("deleting");
            transfer_phase(&mut state, &request.handoff, &transfer)?;
            let service = state
                .codex_threads
                .as_ref()
                .ok_or("codex_thread_service_unavailable")?;
            if service.catalog()?.contains_key(&thread) {
                service.request("thread/delete", &json!({"threadId":thread}))?;
            }
        }
        if let Some(attachments) = transfer["attachments"].as_array() {
            let root = data_dir_for_state(&state)?.join("attachments");
            for attachment in attachments {
                let path = PathBuf::from(
                    attachment["localPath"]
                        .as_str()
                        .ok_or("application_handoff_attachments_invalid")?,
                );
                if path.parent() != Some(root.as_path())
                    || !attachment["attachmentId"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("attachment_handoff_"))
                {
                    return Err("application_handoff_attachments_invalid".into());
                }
                remove_file_if_present(&path)
                    .map_err(|_| "application_handoff_attachment_cleanup_failed")?;
            }
        }
        let previous = state.persisted.clone();
        state
            .persisted
            .conversation_session
            .get_mut("applicationHandoffs")
            .and_then(Value::as_array_mut)
            .ok_or("application_handoff_missing")?
            .retain(|h| h["handoffId"] != request.handoff);
        persist_locked_or_restore(&mut state, previous)?;
        state.conversation_session_revision += 1;
        let source = handoff["sourceSessionId"].as_str().map(str::to_owned);
        drop(state);
        if let Some(source) = source {
            self.cleanup_application_conversation(&source)?;
        }
        Ok(json!({"status":"cancelled"}))
    }

    /// # Errors
    /// 当前Profile、来源摘要、目标项目、原生历史或正式存储校验失败时拒绝创建。
    #[expect(
        clippy::too_many_lines,
        reason = "来源、原生历史与正式存储在同一状态锁内核对"
    )]
    pub fn commit_application_handoff(
        &self,
        request: ApplicationHandoffCommitRequest,
    ) -> Result<Conversation, String> {
        let mut state = lock(&self.inner);
        if request.session != state.conversation_session_id {
            return Err("conversation_session_changed".into());
        }
        persist_locked(&mut state)?;
        if let Some(conversation) = completed(&state, &request.handoff) {
            return Ok(conversation.clone());
        }
        let handoff = state
            .persisted
            .conversation_session
            .get("applicationHandoffs")
            .and_then(Value::as_array)
            .and_then(|queue| {
                queue
                    .iter()
                    .find(|entry| entry["handoffId"] == request.handoff)
            })
            .cloned()
            .ok_or("application_handoff_missing")?;
        let output = handoff["output"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or("application_model_output_missing")?;
        if handoff["outputDigest"] != sha256_bytes(output.as_bytes()) {
            return Err("application_handoff_output_changed".into());
        }
        let source_run: SourceRun = serde_json::from_value(
            handoff
                .get("runSnapshot")
                .ok_or("application_handoff_source_missing")?
                .clone(),
        )
        .map_err(|_| "application_handoff_invalid")?;
        let server = state.server.clone().ok_or("ai_runtime_not_initialized")?;
        let service = state
            .codex_threads
            .clone()
            .ok_or("codex_thread_service_unavailable")?;
        let workspace = if let Some(project) = request.project.as_deref() {
            let details = server.request_result("project/get", &json!({"projectId":project}))?;
            details["project"]["primaryWorkspaceId"]
                .as_str()
                .map(str::to_owned)
        } else {
            None
        };
        let mut conversation_context: ConversationContext = serde_json::from_value(
            json!({"workspaceRoot":"","workspaceId":workspace,"projectId":request.project}),
        )
        .map_err(|error| error.to_string())?;
        if let Some(workspace_id) = conversation_context.workspace_id.as_deref() {
            let catalog = server.request_result("workspace/list", &json!({}))?;
            let selected = catalog["workspaces"]
                .as_array()
                .and_then(|workspaces| {
                    workspaces
                        .iter()
                        .find(|workspace| workspace["workspaceId"] == workspace_id)
                })
                .ok_or("conversation_project_workspace_unavailable")?;
            selected["localRoot"]
                .as_str()
                .ok_or("conversation_project_workspaces_invalid")?
                .clone_into(&mut conversation_context.workspace_root);
            conversation_context.workspace_name =
                selected["displayName"].as_str().map(str::to_owned);
        }
        let now = unix_epoch_millis()?;
        let application_name = handoff
            .pointer("/source/applicationName")
            .and_then(Value::as_str)
            .ok_or("application_handoff_invalid")?;
        let original_prompt = handoff
            .pointer("/source/prompt")
            .and_then(Value::as_str)
            .ok_or("application_handoff_invalid")?;
        let topic = original_prompt.trim().chars().take(36).collect::<String>();
        let mut conversation = prepare_conversation(
            &mut state,
            ConversationCreateRequest {
                title: format!("{application_name} · {topic}"),
                conversation_type: if request.project.is_some() {
                    "project"
                } else {
                    "direct"
                }
                .into(),
                mode: "discuss".into(),
                agent_id: None,
                context: conversation_context,
            },
            now,
        )?;
        let mut transfer = handoff.get("transfer").cloned().unwrap_or(Value::Null);
        if transfer.is_null() {
            transfer = json!({"phase":"prepared","projectId":request.project,"conversation":conversation,"userMessageId":format!("message_{}",uuid::Uuid::new_v4()),"assistantMessageId":format!("message_{}",uuid::Uuid::new_v4())});
            transfer_phase(&mut state, &request.handoff, &transfer)?;
        } else {
            if transfer["projectId"] != json!(request.project) {
                return Err("application_handoff_project_locked".into());
            }
            conversation = serde_json::from_value(transfer["conversation"].clone())
                .map_err(|_| "application_handoff_transfer_invalid")?;
        }
        if transfer["phase"] == "deleting" {
            return Err("application_handoff_reconciliation_required".into());
        }
        let attachments = handoff_attachments(&mut state, &handoff, &mut transfer)?;
        transfer["attachments"] = json!(attachments);
        transfer_phase(&mut state, &request.handoff, &transfer)?;
        let page_context: ApplicationAiContextSnapshot = serde_json::from_value(json!({
            "applicationId":handoff["applicationId"],"applicationName":application_name,"version":handoff["source"]["version"],"pageRevision":handoff["source"]["pageRevision"],"contexts":handoff["source"]["contexts"],"actions":[],"toolNames":[],"toolSetRevision":null
        })).map_err(|_| "application_handoff_invalid")?;
        let messages = [
            (
                "user",
                original_prompt.to_owned(),
                Some(page_context),
                source_run.started_at,
            ),
            (
                "assistant",
                output.to_owned(),
                None,
                source_run.completed_at,
            ),
        ]
        .into_iter()
        .map(
            |(role, body, application_context, created_at)| ConversationMessage {
                message_id: transfer[if role == "user" {
                    "userMessageId"
                } else {
                    "assistantMessageId"
                }]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
                conversation_id: conversation.conversation_id.clone(),
                role: role.into(),
                content: body,
                attachments: if role == "user" {
                    attachments.clone()
                } else {
                    vec![]
                },
                referenced_materials: vec![],
                application_context,
                run_id: None,
                created_at,
            },
        )
        .collect::<Vec<_>>();
        conversation.application_handoff = Some(
            json!({"handoffId":request.handoff,"applicationId":handoff["applicationId"],"applicationName":application_name,"runId":handoff["runId"],"runSnapshot":source_run,"userMessageId":messages[0].message_id,"assistantMessageId":messages[1].message_id,"outputDigest":handoff["outputDigest"],"receipt":handoff["receipt"],"createdAt":handoff["createdAt"]}),
        );
        let mut candidate = state.persisted.clone();
        candidate.conversations.push(conversation.clone());
        candidate.messages.extend(messages.clone());
        candidate
            .conversation_session
            .get_mut("applicationHandoffs")
            .and_then(Value::as_array_mut)
            .ok_or("application_handoff_missing")?
            .retain(|entry| entry["handoffId"] != request.handoff);
        let plan = history_migration::plan(
            &candidate,
            &conversation.conversation_id,
            &data_dir_for_state(&state)?,
        )?;
        let thread = if let Some(thread) = transfer["nativeThreadId"].as_str() {
            thread.to_owned()
        } else {
            if transfer["phase"] != "prepared" {
                return Err("application_handoff_creation_unconfirmed".into());
            }
            transfer["phase"] = json!("creating");
            transfer_phase(&mut state, &request.handoff, &transfer)?;
            let created=service.request("thread/start",&json!({"cwd":conversation.context.workspace_root,"approvalPolicy":"never","sandbox":"read-only","ephemeral":false}))?;
            let thread = created["thread"]["id"]
                .as_str()
                .ok_or("native_conversation_create_invalid")?
                .to_owned();
            service
                .import_history(&json!({"threadId":thread,"name":conversation.title,"turns":[]}))?;
            transfer["nativeThreadId"] = json!(thread);
            transfer["phase"] = json!("created");
            transfer_phase(&mut state, &request.handoff, &transfer)?;
            thread
        };
        if !matches!(transfer["phase"].as_str(), Some("verified" | "publishing")) {
            transfer["phase"] = json!("importing");
            transfer_phase(&mut state, &request.handoff, &transfer)?;
            let loaded = service.request("thread/loaded/list", &json!({}))?;
            let loaded = loaded["data"]
                .as_array()
                .ok_or("application_handoff_thread_state_invalid")?;
            if !loaded.iter().any(|id| id.as_str() == Some(thread.as_str())) {
                service.request("thread/resume",&json!({"threadId":thread,"cwd":conversation.context.workspace_root,"excludeTurns":true,"approvalPolicy":"never","sandbox":"read-only"}))?;
            }
            service.import_history(
                &json!({"threadId":thread,"name":conversation.title,"turns":plan["turns"]}),
            )?;
        }
        history_migration::verify(&plan, &service.turns(&thread)?)?;
        transfer["phase"] = json!("verified");
        transfer_phase(&mut state, &request.handoff, &transfer)?;
        service.request(
            "thread/name/set",
            &json!({"threadId":thread,"name":conversation.title}),
        )?;
        transfer["phase"] = json!("publishing");
        transfer_phase(&mut state, &request.handoff, &transfer)?;
        let ledger = state
            .ledger_baseline
            .prepare(&ledger_projection::state(&candidate))?;
        let params = json!({"action":"handoff","payload":{"handoffId":request.handoff,"conversationId":conversation.conversation_id,"nativeThreadId":thread,"title":conversation.title,"messages":plan["messages"],"ledger":ledger.request}});
        state.pending_application_handoff = Some(PendingHandoff {
            ledger,
            params,
            conversation: conversation.clone(),
            messages,
            handoff_id: request.handoff,
        });
        flush(&mut state)?;
        Ok(conversation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(manager: &AiRuntimeManager, id: &str, status: &str) {
        lock(&manager.inner).application_model_runs.insert(id.into(), ApplicationModelRunRecord {
            persistent_session_id: None,
            events: Vec::new(),
            handoff_context: json!({"applicationName":"空气质量热力图","version":"1.0","pageRevision":7,"contexts":[{"contextId":"observations","data":{"mean":113.125}}],"prompt":"当前区域的平均值是多少？"}),
            snapshot: ApplicationModelRunSnapshot {
            event_position: 0,
            model_execution: None,
            reply_usage: None,
            error_code: None,
                run_id:id.into(),application_id:"air".into(),model_id:"test-model".into(),model_label:"测试模型".into(),status:status.into(),partial_output:None,output:Some("算术平均值为 113.125 μg/m³。".into()),error_reason:None,thread_id:None,started_at:1,completed_at:Some(2),write_request:None,model_task:None,skill:None,receipt:None,
            },pid:None,cancel_requested:false,
        });
    }

    #[test]
    fn handoff_bounds_messages_and_bytes_separately_and_keeps_all_source_evidence() {
        let _guard = crate::supervisor::app_server_test_guard();
        let owner = super::super::model_test_fixture::fixture_root("handoff-capacity");
        let root = super::super::model_test_fixture::profile_data_root(&owner);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &root,
                &root.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&root, &server).unwrap();
        seed(&manager, "evidence", "completed");
        let context = "合".repeat(MAX_MESSAGE_CHARS);
        let evidence = "实际工具执行依据".repeat(1000);
        {
            let mut state = lock(&manager.inner);
            let record = state.application_model_runs.get_mut("evidence").unwrap();
            record.handoff_context["contexts"][0]["data"]["fullEvidence"] = json!(context);
            record.events.push(AgentRunEvent {
                summary: evidence.clone(),
                ..Default::default()
            });
        }
        let handoff = manager
            .create_application_handoff("air", "evidence")
            .unwrap();
        assert!(handoff.to_string().chars().count() > MAX_MESSAGE_CHARS);
        assert_eq!(
            handoff["source"]["contexts"][0]["data"]["fullEvidence"],
            context
        );
        assert_eq!(handoff["runSnapshot"]["events"][0]["summary"], evidence);
        let before = lock(&manager.inner).persisted.conversation_session.clone();
        for field in ["prompt", "output"] {
            seed(&manager, field, "completed");
            {
                let mut state = lock(&manager.inner);
                let record = state.application_model_runs.get_mut(field).unwrap();
                if field == "prompt" {
                    record.handoff_context["prompt"] = json!("字".repeat(MAX_MESSAGE_CHARS + 1));
                } else {
                    record.snapshot.output = Some("字".repeat(MAX_MESSAGE_CHARS + 1));
                }
            }
            assert_eq!(
                manager
                    .create_application_handoff("air", field)
                    .unwrap_err(),
                "application_handoff_too_large"
            );
            assert_eq!(lock(&manager.inner).persisted.conversation_session, before);
        }
        for (id, bytes, expected) in [
            (
                "oversize",
                MAX_HANDOFF_BYTES + 1,
                Some("application_handoff_too_large"),
            ),
            ("first", MAX_HANDOFF_BYTES / 2, None),
            (
                "second",
                MAX_HANDOFF_BYTES / 2,
                Some("conversation_session_too_large"),
            ),
        ] {
            seed(&manager, id, "completed");
            lock(&manager.inner)
                .application_model_runs
                .get_mut(id)
                .unwrap()
                .handoff_context["contexts"][0]["data"]["fullEvidence"] = json!("x".repeat(bytes));
            let previous = lock(&manager.inner).persisted.conversation_session.clone();
            let result = manager.create_application_handoff("air", id);
            if let Some(error) = expected {
                assert_eq!(result.unwrap_err(), error);
                assert_eq!(
                    lock(&manager.inner).persisted.conversation_session,
                    previous
                );
            } else {
                assert!(result.is_ok());
            }
        }
        manager.shutdown_checked().unwrap();
        server.shutdown().unwrap();
        fs::remove_dir_all(owner).unwrap();
    }

    #[test]
    fn source_run_keeps_real_events_without_streamed_output_duplicates() {
        let manager = AiRuntimeManager::new();
        seed(&manager, "events", "queued");
        let event = CodexHarnessEvent {
            event_type: "item.started".into(),
            item_id: Some("read-1".into()),
            phase: "reply".into(),
            kind: "mcp".into(),
            status: "started".into(),
            summary: "正在读取观测数据".into(),
            duration_ms: None,
            detail_available: true,
            streamed_output: None,
            steered_input: None,
            context_usage: None,
            read_paths: vec![],
            created_at: 10,
        };
        manager.record_application_model_progress("events", event.clone());
        manager.record_application_model_progress(
            "events",
            CodexHarnessEvent {
                event_type: "item.completed".into(),
                status: "completed".into(),
                summary: "已读取观测数据".into(),
                created_at: 20,
                ..event.clone()
            },
        );
        manager.record_application_model_progress(
            "events",
            CodexHarnessEvent {
                streamed_output: Some("回复片段".into()),
                ..event
            },
        );
        let mut state = lock(&manager.inner);
        let record = state.application_model_runs.get_mut("events").unwrap();
        record.snapshot.completed_at = Some(30);
        let snapshot = SourceRun::from_record(record).unwrap();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].summary, "已读取观测数据");
        assert_eq!(snapshot.events[0].status, "completed");
        assert_eq!(snapshot.events[0].duration_ms, Some(10));
        assert_eq!(record.snapshot.partial_output.as_deref(), Some("回复片段"));
        record.snapshot.completed_at = None;
        assert!(SourceRun::from_record(record).is_err());
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "同一隔离实例中验证项目归属、幂等、取消与重启恢复"
    )]
    fn project_choice_creates_durable_history_once_and_preserves_drafts() {
        let _guard = crate::supervisor::app_server_test_guard();
        let owner = super::super::model_test_fixture::fixture_root("handoff");
        let root = super::super::model_test_fixture::profile_data_root(&owner);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &root,
                &root.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&root, &server).unwrap();
        {
            let mut state = lock(&manager.inner);
            state
                .persisted
                .conversation_session
                .insert("drafts".into(), json!({"new":"保留员工已有草稿"}));
            persist_locked(&mut state).unwrap();
        }
        seed(&manager, "unfinished", "running");
        assert!(
            manager
                .create_application_handoff("air", "unfinished")
                .is_err()
        );
        lock(&manager.inner)
            .application_model_runs
            .remove("unfinished");
        seed(&manager, "direct", "completed");
        assert!(
            manager
                .create_application_handoff("other", "direct")
                .is_err()
        );
        lock(&manager.inner)
            .application_model_runs
            .get_mut("direct")
            .unwrap()
            .events
            .push(AgentRunEvent {
                sequence: 1,
                event_type: "item.completed".into(),
                item_id: Some("read-1".into()),
                kind: "mcp".into(),
                summary: "已读取观测数据".into(),
                created_at: 1,
                ..AgentRunEvent::default()
            });
        let handoff = manager.create_application_handoff("air", "direct").unwrap();
        assert_eq!(handoff["runSnapshot"]["modelLabel"], "测试模型");
        lock(&manager.inner)
            .application_model_runs
            .get_mut("direct")
            .unwrap()
            .snapshot
            .model_label = "后来改名的模型".into();
        manager.shutdown_checked().unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&root, &server).unwrap();
        assert_eq!(
            manager.application_handoff_status("air", "direct").unwrap()["status"],
            "awaiting_project"
        );
        let session = lock(&manager.inner).conversation_session_id.clone();
        let request = |id: &str, project: Option<String>| ApplicationHandoffCommitRequest {
            session: session.clone(),
            handoff: format!("handoff_{id}"),
            project,
        };
        {
            let mut state = lock(&manager.inner);
            state
                .persisted
                .conversation_session
                .get_mut("applicationHandoffs")
                .unwrap()[0]
                .as_object_mut()
                .unwrap()
                .remove("runSnapshot");
            persist_locked(&mut state).unwrap();
        }
        assert_eq!(
            manager
                .commit_application_handoff(request("direct", None))
                .unwrap_err(),
            "application_handoff_source_missing"
        );
        {
            let mut state = lock(&manager.inner);
            assert!(state.persisted.conversations.is_empty());
            state
                .persisted
                .conversation_session
                .get_mut("applicationHandoffs")
                .unwrap()[0]["runSnapshot"] = handoff["runSnapshot"].clone();
            persist_locked(&mut state).unwrap();
        }
        assert!(
            manager
                .commit_application_handoff(ApplicationHandoffCommitRequest {
                    session: "other-profile".into(),
                    handoff: "handoff_direct".into(),
                    project: None
                })
                .is_err()
        );
        assert!(
            manager
                .commit_application_handoff(request("direct", Some("missing-project".into())))
                .is_err()
        );
        assert_eq!(
            manager.application_handoff_status("air", "direct").unwrap()["status"],
            "awaiting_project"
        );
        let direct = manager
            .commit_application_handoff(request("direct", None))
            .unwrap();
        assert!(direct.context.project_id.is_none());
        assert_eq!(
            manager
                .commit_application_handoff(request("direct", None))
                .unwrap(),
            direct
        );
        assert_eq!(
            manager.application_handoff_status("air", "direct").unwrap()["conversationId"],
            direct.conversation_id
        );
        let repeated = manager.create_application_handoff("air", "direct").unwrap();
        assert_eq!(repeated["conversationId"], direct.conversation_id);
        assert_ne!(repeated["requestId"], handoff["requestId"]);
        let folder = owner.join("project-workspace");
        fs::create_dir_all(&folder).unwrap();
        let workspace = server.request_result("workspace/create", &json!({"localRoot":folder,"displayName":"项目工作区","clientOperationId":"handoff-workspace"})).unwrap();
        let workspace_id = workspace["workspace"]["workspaceId"].as_str().unwrap();
        let project=server.request_result("project/create",&json!({"workspaceId":workspace_id,"workspaceIds":[],"displayName":"转交验收项目","goal":"测试","clientOperationId":"handoff-project","requestFingerprint":"handoff-project"})).unwrap();
        seed(&manager, "project", "completed");
        manager
            .create_application_handoff("air", "project")
            .unwrap();
        let associated = manager
            .commit_application_handoff(request(
                "project",
                Some(project["project"]["projectId"].as_str().unwrap().into()),
            ))
            .unwrap();
        assert_eq!(
            associated.context.project_name.as_deref(),
            Some("转交验收项目")
        );
        assert_eq!(
            associated.context.workspace_root,
            folder.canonicalize().unwrap().to_string_lossy()
        );
        assert_eq!(
            associated.context.workspace_id.as_deref(),
            Some(workspace_id)
        );
        assert_eq!(
            associated.context.workspace_name.as_deref(),
            Some("项目工作区")
        );
        seed(&manager, "cancelled", "completed");
        manager
            .create_application_handoff("air", "cancelled")
            .unwrap();
        assert_eq!(
            manager
                .cancel_application_handoff(&request("cancelled", None))
                .unwrap()["status"],
            "cancelled"
        );
        assert_eq!(
            manager
                .application_handoff_status("air", "cancelled")
                .unwrap()["status"],
            "cancelled"
        );
        let restarted = AiRuntimeManager::new();
        restarted.initialize(&root, &server).unwrap();
        let state = lock(&restarted.inner);
        assert_eq!(state.persisted.conversations.len(), 2);
        assert_eq!(state.persisted.messages.len(), 4);
        assert!(state.persisted.runs.is_empty());
        assert_eq!(
            state.persisted.conversation_session["drafts"]["new"],
            "保留员工已有草稿"
        );
        let messages = state
            .persisted
            .messages
            .iter()
            .filter(|message| message.conversation_id == direct.conversation_id)
            .collect::<Vec<_>>();
        assert_eq!(messages[0].content, "当前区域的平均值是多少？");
        assert_eq!(messages[0].created_at, 1);
        assert_eq!(messages[1].created_at, 2);
        let restored = state
            .persisted
            .conversations
            .iter()
            .find(|entry| entry.conversation_id == direct.conversation_id)
            .unwrap();
        let metadata = restored.application_handoff.as_ref().unwrap();
        assert_eq!(metadata["runSnapshot"]["modelId"], "test-model");
        assert_eq!(metadata["runSnapshot"]["modelLabel"], "测试模型");
        assert_eq!(metadata["runSnapshot"]["startedAt"], 1);
        assert_eq!(metadata["runSnapshot"]["completedAt"], 2);
        assert_eq!(
            metadata["runSnapshot"]["events"].as_array().unwrap().len(),
            1
        );
        assert_eq!(
            metadata["runSnapshot"]["events"][0]["summary"],
            "已读取观测数据"
        );
        assert_eq!(metadata["userMessageId"], messages[0].message_id);
        assert_eq!(metadata["assistantMessageId"], messages[1].message_id);
        assert_eq!(messages[1].content, "算术平均值为 113.125 μg/m³。");
        let source = messages[0].application_context.as_ref().unwrap();
        assert_eq!(source.page_revision, 7);
        assert_eq!(source.contexts[0]["data"]["mean"], 113.125);
        assert!(source.tool_names.is_empty());
        assert!(source.write_tools.is_empty());
        drop(state);
        assert_eq!(
            restarted
                .application_handoff_status("air", "direct")
                .unwrap()["conversationId"],
            direct.conversation_id
        );
        manager.shutdown_checked().unwrap();
        restarted.shutdown_checked().unwrap();
        server.shutdown().unwrap();
        fs::remove_dir_all(owner).unwrap();
    }
    #[test]
    fn interrupted_transfer_cancellation_deletes_only_known_unpublished_target() {
        let _guard = crate::supervisor::app_server_test_guard();
        let owner = super::super::model_test_fixture::fixture_root("handoff-cancel");
        let root = super::super::model_test_fixture::profile_data_root(&owner);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &root,
                &root.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&root, &server).unwrap();
        seed(&manager, "cancel-target", "completed");
        manager
            .create_application_handoff("air", "cancel-target")
            .unwrap();
        let service = lock(&manager.inner).codex_threads.clone().unwrap();
        let native = service
            .request(
                "thread/start",
                &json!({"approvalPolicy":"never","sandbox":"read-only","ephemeral":false}),
            )
            .unwrap()["thread"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        service
            .import_history(&json!({"threadId":native,"name":"待取消的转交目标","turns":[]}))
            .unwrap();
        service.shutdown();
        let request = ApplicationHandoffCommitRequest {
            session: lock(&manager.inner).conversation_session_id.clone(),
            handoff: "handoff_cancel-target".into(),
            project: None,
        };
        {
            let mut state = lock(&manager.inner);
            transfer_phase(
                &mut state,
                &request.handoff,
                &json!({"phase":"created","nativeThreadId":native}),
            )
            .unwrap();
        }
        assert_eq!(
            manager.cancel_application_handoff(&request).unwrap()["status"],
            "cancelled"
        );
        assert!(!service.catalog().unwrap().contains_key(&native));
        assert!(lock(&manager.inner).persisted.conversations.is_empty());
        seed(&manager, "unknown-target", "completed");
        manager
            .create_application_handoff("air", "unknown-target")
            .unwrap();
        let request = ApplicationHandoffCommitRequest {
            handoff: "handoff_unknown-target".into(),
            ..request
        };
        {
            let mut state = lock(&manager.inner);
            transfer_phase(&mut state, &request.handoff, &json!({"phase":"creating"})).unwrap();
        }
        assert_eq!(
            manager.cancel_application_handoff(&request).unwrap_err(),
            "application_handoff_creation_unconfirmed"
        );
        assert_eq!(
            manager
                .application_handoff_status("air", "unknown-target")
                .unwrap()["status"],
            "awaiting_project"
        );
        service.shutdown();
        lock(&manager.inner)
            .application_threads
            .as_ref()
            .unwrap()
            .shutdown();
        manager.shutdown_checked().unwrap();
        server.shutdown().unwrap();
        fs::remove_dir_all(owner).unwrap();
    }
    #[test]
    fn cancelling_submitted_writes_preserves_unknown_outcomes_and_known_receipts() {
        let manager = AiRuntimeManager::new();
        for (id, status, expected) in [
            ("waiting", "awaiting_confirmation", "cancelled"),
            ("submitted", "submitting", "outcome_unknown"),
            ("written", "succeeded", "succeeded"),
        ] {
            seed(&manager, id, "running");
            lock(&manager.inner)
                .application_model_runs
                .get_mut(id)
                .unwrap()
                .snapshot
                .write_request = Some(ApplicationModelWriteRequest {
                approval: ApplicationWriteApproval {
                    confirmation_id: id.into(),
                    application_id: "air".into(),
                    application_name: "应用".into(),
                    tool_name: "write".into(),
                    tool_title: "写入".into(),
                    description: String::new(),
                    risk_level: "write".into(),
                    arguments: json!({}),
                    confirmation_view: None,
                    execution_status: Some(status.into()),
                    receipt: Some(json!({"commandId":id})),
                    failure_reason: None,
                },
                requested_at: 1,
                valid_until: 100,
            });
            let result = manager.cancel_application_model_run(id).unwrap();
            let write = result.write_request.unwrap();
            assert_eq!(write.approval.execution_status.as_deref(), Some(expected));
            assert_eq!(write.approval.receipt.unwrap()["commandId"], id);
        }
    }
    #[test]
    fn attachment_copy_failure_keeps_durable_cleanup_intent_and_preserves_source() {
        let _guard = crate::supervisor::app_server_test_guard();
        let owner = super::super::model_test_fixture::fixture_root("handoff-copy");
        let root = super::super::model_test_fixture::profile_data_root(&owner);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &root,
                &root.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&root, &server).unwrap();
        manager
            .open_application_conversation("source", "instance", "air", "authorization")
            .unwrap();
        seed(&manager, "copy", "completed");
        let directory = lock(&manager.inner)
            .data_dir
            .clone()
            .unwrap()
            .join("attachments");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("attachment_source.png");
        fs::write(&source, b"source").unwrap();
        let attachment = MessageAttachment {
            attachment_id: "attachment_source".into(),
            file_name: "source.png".into(),
            mime_type: "image/png".into(),
            media_type: "image".into(),
            size_bytes: 6,
            local_path: source.to_string_lossy().into_owned(),
        };
        let snapshot = {
            let mut state = lock(&manager.inner);
            let record = state.application_model_runs.get_mut("copy").unwrap();
            record.persistent_session_id = Some("source".into());
            record.snapshot.clone()
        };
        AiRuntimeManager::application_store(&server,"reserve",&json!({"sessionId":"source","clientMessageId":"message","contentDigest":"digest","runId":"copy","snapshot":snapshot,"context":{},"attachments":[attachment]})).unwrap();
        let handoff = manager.create_application_handoff("air", "copy").unwrap();
        let target = directory.join(format!(
            "attachment_handoff_{}",
            sha256_bytes(format!("{}:attachment_source", handoff["handoffId"]).as_bytes())
        ));
        fs::write(&target, b"different").unwrap();
        let request = ApplicationHandoffCommitRequest {
            session: lock(&manager.inner).conversation_session_id.clone(),
            handoff: "handoff_copy".into(),
            project: None,
        };
        assert_eq!(
            manager
                .commit_application_handoff(ApplicationHandoffCommitRequest {
                    session: request.session.clone(),
                    handoff: request.handoff.clone(),
                    project: None
                })
                .unwrap_err(),
            "application_handoff_attachment_changed"
        );
        assert_eq!(
            lock(&manager.inner).persisted.conversation_session["applicationHandoffs"][0]["transfer"]
                ["attachments"][0]["localPath"],
            target.to_string_lossy().as_ref()
        );
        manager.cancel_application_handoff(&request).unwrap();
        assert!(!target.exists());
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert!(lock(&manager.inner).persisted.conversations.is_empty());
        manager.create_application_handoff("air", "copy").unwrap();
        fs::write(&target, b"so").unwrap();
        manager.commit_application_handoff(request).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"source");
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(lock(&manager.inner).persisted.conversations.len(), 1);

        manager.shutdown();
        if let Some(service) = lock(&manager.inner).application_threads.take() {
            service.shutdown();
        }
        manager.shutdown_checked().unwrap();
        server.shutdown().unwrap();
        fs::remove_dir_all(owner).unwrap();
    }
}
