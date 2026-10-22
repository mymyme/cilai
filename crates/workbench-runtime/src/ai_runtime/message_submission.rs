//! 对话消息的受理回执。
//!
//! 发送要核对个人模型连接、用途授权和执行框架，耗时以秒计。宿主先在本地
//! 核对身份并登记回执，立即答复界面，再在同一后台线程完成网络准备与启动；界面据回执
//! 保持"正在准备"或显示明确失败，不等待整条链路，也不在失败时静默丢弃输入。
//! 回执只存在于本次进程：桌面重启后，未落库的发送由会话恢复记录承接。

use super::*;

/// 受理回执的状态，随运行时快照交给界面。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSubmission {
    pub client_message_id: String,
    pub conversation_id: String,
    /// `preparing`：已受理，正在准备执行；`failed`：消息未落库，`reason` 给出原因。
    pub status: String,
    pub reason: Option<String>,
    pub accepted_at: i64,
}

/// 受理结果。
pub enum SubmissionAcceptance {
    /// 同一客户端标识的消息已经落库，直接返回其运行，不再重复执行。
    Recorded(Box<AgentRun>),
    /// 已登记回执，调用方须在后台完成发送并调用 [`AiRuntimeManager::finish_message_submission`]。
    Accepted(LocalOperationGuard),
}

fn valid_client_message_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_submission(request: &MessageSendRequest) -> Result<&str, String> {
    if request.application_context.is_some() {
        return Err("enterprise_application_tools_removed".to_owned());
    }
    let id = request
        .client_message_id
        .as_deref()
        .filter(|id| valid_client_message_id(id))
        .ok_or("ai_message_client_id_invalid")?;
    if request.content.chars().count() > MAX_MESSAGE_CHARS {
        return Err("ai_message_too_large".to_owned());
    }
    if request.content.trim().is_empty()
        && request.attachment_ids.is_empty()
        && request.referenced_materials.is_empty()
    {
        return Err("ai_message_required".to_owned());
    }
    validate_execution_target(&request.execution_target)?;
    Ok(id)
}

pub(super) fn conversation_preparing(state: &ManagerState, conversation_id: &str) -> bool {
    state
        .message_submissions
        .values()
        .any(|item| item.conversation_id == conversation_id && item.status == "preparing")
}

impl AiRuntimeManager {
    /// 创建本地对话并受理首条消息；网络准备与运行启动由宿主在回执之后执行。
    ///
    /// # Errors
    /// 参数无效、创建失败或同一消息仍在准备时返回错误。客户端消息标识用于重试去重。
    pub fn accept_conversation_submission(
        &self,
        request: ConversationStartRequest,
    ) -> Result<(Conversation, MessageSendRequest, SubmissionAcceptance), String> {
        let mut message = MessageSendRequest {
            conversation_id: String::new(),
            content: request.content,
            client_message_id: request.client_message_id,
            attachment_ids: request.attachment_ids,
            referenced_materials: request.referenced_materials,
            application_context: request.application_context,
            selected_skill_id: request.selected_skill_id,
            retry_of_run_id: None,
            model_selection: request.model_selection,
            history_consent: request.history_consent,
            model_override: request.model_override,
            reasoning_effort: request.reasoning_effort,
            execution_target: request.execution_target,
            permission_mode: request.permission_mode,
            planning: request.planning,
        };
        validate_submission(&message)?;
        let client_id = message.client_message_id.as_deref().unwrap_or_default();
        let recorded = {
            let state = lock(&self.inner);
            data_dir_for_state(&state)?;
            let existing = state
                .persisted
                .messages
                .iter()
                .find(|item| item.message_id == client_id)
                .map(|item| item.conversation_id.clone())
                .or_else(|| {
                    state
                        .message_submissions
                        .get(client_id)
                        .map(|item| item.conversation_id.clone())
                });
            if let Some(id) = existing {
                message.conversation_id = id;
                Self::check_message_submission_locked(&state, &message)?.is_some()
            } else {
                false
            }
        };
        if !recorded {
            self.personal_model_message_preflight(
                &message,
                contexthub_protocol::ModelPurpose::Foreground,
            )?;
        }
        let mut state = lock(&self.inner);
        data_dir_for_state(&state)?;
        let existing_id = state
            .persisted
            .messages
            .iter()
            .find(|item| item.message_id == client_id)
            .map(|item| item.conversation_id.clone())
            .or_else(|| {
                state
                    .message_submissions
                    .get(client_id)
                    .map(|item| item.conversation_id.clone())
            });
        let conversation = if let Some(id) = existing_id {
            state
                .persisted
                .conversations
                .iter()
                .find(|item| item.conversation_id == id)
                .cloned()
                .ok_or("conversation_not_found")?
        } else {
            let now = unix_epoch_millis()?;
            let mut conversation = prepare_conversation(
                &mut state,
                ConversationCreateRequest {
                    title: request.title,
                    conversation_type: request.conversation_type,
                    mode: request.mode,
                    agent_id: Some(request.agent_id),
                    context: request.context,
                },
                now,
            )?;
            // 首条消息尚未建立运行时，界面也必须能恢复本次选择的审批模式。
            if validate_approval_mode(&message.permission_mode).is_ok() {
                conversation.approval_mode = Some(message.permission_mode.clone());
            }
            state.persisted.conversations.push(conversation.clone());
            if let Err(error) = persist_locked(&mut state) {
                restore_after_save_rejection(&mut state, |persisted| {
                    persisted.conversations.pop();
                });
                return Err(error);
            }
            conversation
        };
        message
            .conversation_id
            .clone_from(&conversation.conversation_id);
        let acceptance = self.accept_message_submission_locked(&mut state, &message)?;
        Ok((conversation, message, acceptance))
    }

    /// 只做本地核对并登记回执，不访问网络。
    ///
    /// # Errors
    /// 客户端标识无效、对话不可用、内容超限或对话已有进行中的运行时返回错误。
    pub fn accept_message_submission(
        &self,
        request: &MessageSendRequest,
    ) -> Result<SubmissionAcceptance, String> {
        {
            let state = lock(&self.inner);
            if let Some(run) = Self::check_message_submission_locked(&state, request)? {
                return Ok(SubmissionAcceptance::Recorded(Box::new(run)));
            }
        }
        self.personal_model_message_preflight(
            request,
            contexthub_protocol::ModelPurpose::Foreground,
        )?;
        let mut state = lock(&self.inner);
        self.accept_message_submission_locked(&mut state, request)
    }

    fn check_message_submission_locked(
        state: &ManagerState,
        request: &MessageSendRequest,
    ) -> Result<Option<AgentRun>, String> {
        let client_message_id = validate_submission(request)?;
        data_dir_for_state(state)?;
        if let Some(message) = state
            .persisted
            .messages
            .iter()
            .find(|message| message.message_id == client_message_id)
        {
            if message.content != request.content.trim()
                || message.conversation_id != request.conversation_id
                || message.referenced_materials != request.referenced_materials
                || message
                    .attachments
                    .iter()
                    .map(|item| &item.attachment_id)
                    .collect::<Vec<_>>()
                    != request.attachment_ids.iter().collect::<Vec<_>>()
            {
                return Err("ai_message_identity_conflict".to_owned());
            }
            return state
                .persisted
                .runs
                .iter()
                .find(|run| Some(&run.run_id) == message.run_id.as_ref())
                .cloned()
                .map(Some)
                .ok_or_else(|| "message_delivery_uncertain".to_owned());
        }
        if !state.persisted.conversations.iter().any(|conversation| {
            conversation.conversation_id == request.conversation_id && !conversation.archived
        }) {
            return Err("conversation_not_found".to_owned());
        }
        if let Some(existing) = state.message_submissions.get(client_message_id)
            && existing.status == "preparing"
        {
            if existing.conversation_id != request.conversation_id {
                return Err("ai_message_identity_conflict".to_owned());
            }
            return Err("message_submission_in_progress".to_owned());
        }
        let busy = state.persisted.runs.iter().any(|run| {
            run.conversation_id == request.conversation_id
                && (run.thread_writer_active || run_is_active(&run.status))
        }) || conversation_preparing(state, &request.conversation_id)
            || super::conversation_session::follow_up_sending(
                &state.persisted.conversation_session,
                &request.conversation_id,
            );
        if busy {
            return Err("conversation_run_already_active".to_owned());
        }
        Ok(None)
    }

    fn accept_message_submission_locked(
        &self,
        state: &mut ManagerState,
        request: &MessageSendRequest,
    ) -> Result<SubmissionAcceptance, String> {
        if let Some(run) = Self::check_message_submission_locked(state, request)? {
            return Ok(SubmissionAcceptance::Recorded(Box::new(run)));
        }
        let client_message_id = validate_submission(request)?;
        let accepted_at = unix_epoch_millis()?;
        state.message_submissions.insert(
            client_message_id.to_owned(),
            MessageSubmission {
                client_message_id: client_message_id.to_owned(),
                conversation_id: request.conversation_id.clone(),
                status: "preparing".to_owned(),
                reason: None,
                accepted_at,
            },
        );
        state.state_revision = state.state_revision.saturating_add(1);
        state.local_operations += 1;
        Ok(SubmissionAcceptance::Accepted(LocalOperationGuard {
            manager: self.clone(),
        }))
    }

    /// 后台发送结束后更新回执。消息已经落库时回执即完成：运行自身的失败由运行记录表达。
    pub fn finish_message_submission(&self, client_message_id: &str, error: Option<&str>) {
        let mut state = lock(&self.inner);
        let recorded = state
            .persisted
            .messages
            .iter()
            .any(|message| message.message_id == client_message_id);
        let Some(submission) = state.message_submissions.get_mut(client_message_id) else {
            return;
        };
        match error {
            Some(reason) if !recorded => {
                "failed".clone_into(&mut submission.status);
                submission.reason = Some(reason.to_owned());
            }
            _ => {
                state.message_submissions.remove(client_message_id);
            }
        }
        state.state_revision = state.state_revision.saturating_add(1);
    }

    /// 界面已恢复或放弃失败的发送后移除回执；进行中的回执不能移除。
    ///
    /// # Errors
    /// 回执仍在准备时返回错误。
    pub fn dismiss_message_submission(&self, client_message_id: &str) -> Result<(), String> {
        let mut state = lock(&self.inner);
        match state.message_submissions.get(client_message_id) {
            Some(item) if item.status == "preparing" => {
                Err("message_submission_in_progress".to_owned())
            }
            Some(_) => {
                state.message_submissions.remove(client_message_id);
                state.state_revision = state.state_revision.saturating_add(1);
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub(super) fn message_submissions_for_ui(state: &ManagerState) -> Vec<MessageSubmission> {
        let mut items: Vec<_> = state.message_submissions.values().cloned().collect();
        items.sort_by(|left, right| {
            left.accepted_at
                .cmp(&right.accepted_at)
                .then_with(|| left.client_message_id.cmp(&right.client_message_id))
        });
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        server: AppServerSupervisor,
        runtime: AiRuntimeManager,
        selection: contexthub_protocol::ModelSelection,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl std::ops::Deref for Fixture {
        type Target = AiRuntimeManager;
        fn deref(&self) -> &Self::Target {
            &self.runtime
        }
    }

    impl Fixture {
        fn request(&self, id: &str) -> MessageSendRequest {
            let mut value = request(id);
            value.model_selection = Some(self.selection.clone());
            value
        }
        fn start_request(&self, id: &str) -> ConversationStartRequest {
            let mut value = start_request(id);
            value.model_selection = Some(self.selection.clone());
            value
        }

        fn disconnect_server(&self) {
            let services = {
                let state = lock(&self.inner);
                [
                    state.codex_threads.clone(),
                    state.application_threads.clone(),
                ]
            };
            for service in services.into_iter().flatten() {
                service.shutdown_checked().unwrap();
            }
            self.server.shutdown().unwrap();
            lock(&self.inner).server = None;
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.runtime.shutdown();
            self.server.shutdown().unwrap();
            fs::remove_dir_all(&self.root).unwrap();
        }
    }

    fn manager() -> Fixture {
        let guard = crate::supervisor::app_server_test_guard();
        let root = super::super::model_test_fixture::fixture_root("message-submission");
        let data = super::super::model_test_fixture::profile_data_root(&root);
        let server = AppServerSupervisor::new();
        server
            .start(
                &crate::supervisor::app_server_test_binary(),
                "test",
                &data,
                &data.join("run"),
            )
            .unwrap();
        let manager = AiRuntimeManager::new();
        manager.initialize(&data, &server).unwrap();
        let selection =
            super::super::model_test_fixture::seed_simulated_connection(&server, "model_1");
        super::super::model_test_fixture::bind_purpose(
            &server,
            &selection,
            contexthub_protocol::ModelPurpose::Review,
        );
        {
            let mut state = lock(&manager.inner);
            let mut conversation = super::super::tests::fixture_conversation("agent_1");
            "conversation_1".clone_into(&mut conversation.conversation_id);
            state.persisted.conversations.push(conversation);
        }
        Fixture {
            root,
            server,
            runtime: manager,
            selection,
            _guard: guard,
        }
    }

    fn request(id: &str) -> MessageSendRequest {
        serde_json::from_value(serde_json::json!({
            "conversationId": "conversation_1", "content": "继续", "clientMessageId": id,
            "retryOfRunId": null, "modelOverride": "model_1", "executionTarget": "model",
            "permissionMode": "auto-review"
        }))
        .unwrap()
    }

    fn run(status: &str) -> AgentRun {
        serde_json::from_value(serde_json::json!({
            "runId": "run_1", "conversationId": "conversation_1", "agentId": "agent_1",
            "inputMessageId": "pending-1", "retryOfRunId": null, "provider": "codex",
            "model": "model_1", "status": status, "pid": null, "startedAt": 1,
            "completedAt": null, "output": null, "errorReason": null
        }))
        .unwrap()
    }

    fn start_request(id: &str) -> ConversationStartRequest {
        serde_json::from_value(serde_json::json!({
            "title": "首条受理", "conversationType": "direct", "mode": "discuss",
            "agentId": "agent_1", "context": super::super::tests::fixture_conversation("agent_1").context,
            "content": "继续", "clientMessageId": id, "modelOverride": "model_1",
            "executionTarget": "model", "permissionMode": "auto-review"
        })).unwrap()
    }

    #[test]
    fn new_conversation_retry_reuses_receipt_without_creating_another_conversation() {
        let manager = manager();
        let SubmissionAcceptance::Accepted(guard) = manager
            .accept_message_submission(&manager.request("first-1"))
            .unwrap()
        else {
            panic!("expected acceptance");
        };
        assert_eq!(
            manager
                .accept_conversation_submission(manager.start_request("first-1"))
                .err()
                .unwrap(),
            "message_submission_in_progress"
        );
        manager.finish_message_submission("first-1", Some("model_connection_unavailable"));
        drop(guard);
        let (conversation, message, acceptance) = manager
            .accept_conversation_submission(manager.start_request("first-1"))
            .unwrap();
        assert_eq!(conversation.conversation_id, "conversation_1");
        assert_eq!(message.conversation_id, conversation.conversation_id);
        assert_eq!(lock(&manager.inner).persisted.conversations.len(), 1);
        drop(acceptance);
    }

    #[test]
    fn invalid_first_message_does_not_create_a_conversation() {
        let manager = manager();
        let mut request = manager.start_request("first-2");
        request.content.clear();
        assert_eq!(
            manager
                .accept_conversation_submission(request)
                .err()
                .unwrap(),
            "ai_message_required"
        );
        assert_eq!(lock(&manager.inner).persisted.conversations.len(), 1);
        assert!(submissions(&manager).is_empty());
    }

    fn submissions(manager: &AiRuntimeManager) -> Vec<MessageSubmission> {
        AiRuntimeManager::message_submissions_for_ui(&lock(&manager.inner))
    }

    #[test]
    fn accepted_submission_blocks_the_conversation_until_it_finishes() {
        let manager = manager();
        let guard = match manager
            .accept_message_submission(&manager.request("pending-1"))
            .unwrap()
        {
            SubmissionAcceptance::Accepted(guard) => guard,
            SubmissionAcceptance::Recorded(_) => panic!("新消息不应已落库"),
        };
        assert_eq!(submissions(&manager)[0].status, "preparing");
        assert!(lock(&manager.inner).persisted.runs.is_empty());
        assert!(
            manager
                .tenant_switch_blockers()
                .contains(&"ai_run_active".to_owned())
        );
        assert_eq!(
            manager
                .accept_message_submission(&manager.request("pending-1"))
                .err()
                .unwrap(),
            "message_submission_in_progress"
        );
        assert_eq!(
            manager
                .accept_message_submission(&manager.request("pending-2"))
                .err()
                .unwrap(),
            "conversation_run_already_active"
        );
        assert!(conversation_preparing(
            &lock(&manager.inner),
            "conversation_1"
        ));
        assert_eq!(
            manager.dismiss_message_submission("pending-1").unwrap_err(),
            "message_submission_in_progress"
        );
        manager.finish_message_submission("pending-1", None);
        drop(guard);
        assert!(submissions(&manager).is_empty());
        assert_eq!(lock(&manager.inner).local_operations, 0);
        assert!(manager.tenant_switch_blockers().is_empty());
    }

    #[test]
    fn failure_before_recording_keeps_an_explicit_receipt_until_dismissed() {
        let manager = manager();
        let SubmissionAcceptance::Accepted(guard) = manager
            .accept_message_submission(&manager.request("pending-1"))
            .unwrap()
        else {
            panic!("新消息应被受理");
        };
        manager.finish_message_submission("pending-1", Some("模型连接已失效，请重新连接。"));
        drop(guard);
        let receipt = &submissions(&manager)[0];
        assert_eq!(receipt.status, "failed");
        assert_eq!(
            receipt.reason.as_deref(),
            Some("模型连接已失效，请重新连接。")
        );
        assert!(!conversation_preparing(
            &lock(&manager.inner),
            "conversation_1"
        ));
        assert!(manager.tenant_switch_blockers().is_empty());
        // 失败后同一标识可以再次受理，即用户点了重试
        assert!(matches!(
            manager
                .accept_message_submission(&manager.request("pending-1"))
                .unwrap(),
            SubmissionAcceptance::Accepted(_)
        ));
        manager.finish_message_submission("pending-1", Some("仍失败"));
        manager.dismiss_message_submission("pending-1").unwrap();
        assert!(submissions(&manager).is_empty());
    }

    #[test]
    fn recorded_message_finishes_the_receipt_and_replays_its_run() {
        let manager = manager();
        let SubmissionAcceptance::Accepted(guard) = manager
            .accept_message_submission(&manager.request("pending-1"))
            .unwrap()
        else {
            panic!("新消息应被受理");
        };
        {
            let mut state = lock(&manager.inner);
            state.persisted.messages.push(ConversationMessage {
                message_id: "pending-1".to_owned(),
                conversation_id: "conversation_1".to_owned(),
                role: "user".to_owned(),
                content: "继续".to_owned(),
                attachments: Vec::new(),
                referenced_materials: Vec::new(),
                application_context: None,
                run_id: Some("run_1".to_owned()),
                created_at: 1,
            });
            state.persisted.runs.push(run("failed"));
        }
        // 运行启动失败由运行记录表达，回执不再重复报失败
        manager.finish_message_submission("pending-1", Some("codex_harness_spawn_failed"));
        drop(guard);
        assert!(submissions(&manager).is_empty());
        manager.disconnect_server();
        let mut replay = manager.request("pending-1");
        replay.model_selection = None;
        match manager.accept_message_submission(&replay).unwrap() {
            SubmissionAcceptance::Recorded(run) => assert_eq!(run.run_id, "run_1"),
            SubmissionAcceptance::Accepted(_) => panic!("已落库的消息不应再次执行"),
        }
        let mut restart = manager.start_request("pending-1");
        restart.model_selection = None;
        let (conversation, message, accepted) =
            manager.accept_conversation_submission(restart).unwrap();
        assert_eq!(conversation.conversation_id, "conversation_1");
        assert_eq!(message.conversation_id, "conversation_1");
        assert!(matches!(accepted, SubmissionAcceptance::Recorded(_)));
        assert_eq!(lock(&manager.inner).persisted.conversations.len(), 1);
        let mut conflicting = replay;
        conflicting.content = "不同正文".into();
        assert_eq!(
            manager
                .accept_message_submission(&conflicting)
                .err()
                .unwrap(),
            "ai_message_identity_conflict"
        );
        assert_eq!(lock(&manager.inner).persisted.messages.len(), 1);
        assert_eq!(lock(&manager.inner).persisted.runs.len(), 1);
        assert_eq!(lock(&manager.inner).local_operations, 0);
    }

    #[test]
    fn active_run_rejects_new_submission_and_invalid_ids_are_refused() {
        let manager = manager();
        lock(&manager.inner).persisted.runs.push(run("running"));
        manager.disconnect_server();
        assert_eq!(
            manager
                .accept_message_submission(&manager.request("pending-9"))
                .err()
                .unwrap(),
            "conversation_run_already_active"
        );
        assert_eq!(
            manager
                .accept_message_submission(&manager.request("bad id"))
                .err()
                .unwrap(),
            "ai_message_client_id_invalid"
        );
    }

    #[test]
    fn missing_or_invalid_selection_is_rejected_without_accepting_or_creating_a_conversation() {
        let manager = manager();
        let mut missing = manager.start_request("missing-selection");
        missing.model_selection = None;
        let error = manager
            .accept_conversation_submission(missing)
            .err()
            .unwrap();
        assert!(error.contains("model_purpose_binding_required"), "{error}");
        assert_eq!(lock(&manager.inner).persisted.conversations.len(), 1);
        assert!(submissions(&manager).is_empty());
        let mut invalid = manager.request("invalid-selection");
        invalid.model_selection.as_mut().unwrap().connection_id =
            "00000000000040008000000000000000".into();
        let error = manager.accept_message_submission(&invalid).err().unwrap();
        assert!(error.contains("model_connection_not_found"), "{error}");
        assert!(submissions(&manager).is_empty());
        assert!(lock(&manager.inner).persisted.messages.is_empty());
        assert!(lock(&manager.inner).persisted.runs.is_empty());
    }

    #[test]
    fn unconfigured_connection_keeps_input_and_completed_attachment_available() {
        let manager = manager();
        let attachment = manager
            .create_attachment(AttachmentCreateRequest {
                file_name: "保留.txt".into(),
                mime_type: "text/plain".into(),
                size_bytes: 3,
            })
            .unwrap();
        manager
            .append_attachment_chunk(AttachmentChunkRequest {
                attachment_id: attachment.attachment_id.clone(),
                offset: 0,
                bytes: b"123".to_vec(),
                complete: true,
            })
            .unwrap();
        let mut pending = manager.start_request("preserve-input");
        pending.content = "保留这段输入".into();
        pending.attachment_ids = vec![attachment.attachment_id.clone()];
        pending.model_selection = None;
        let original = pending.clone();
        assert!(manager.accept_conversation_submission(pending).is_err());
        assert_eq!(original.content, "保留这段输入");
        assert_eq!(
            manager.read_attachment(&attachment.attachment_id).unwrap(),
            b"123"
        );
        assert!(
            lock(&manager.inner)
                .pending_attachments
                .contains_key(&attachment.attachment_id)
        );
        assert_eq!(lock(&manager.inner).persisted.conversations.len(), 1);
        assert!(submissions(&manager).is_empty());
    }
}
