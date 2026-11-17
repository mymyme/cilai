use super::{
    AgentRun, AiRuntimeManager, ApplicationAiContextSnapshot, ApplicationDiscoveryNotice, HashSet,
    ManagerState, MessageSendRequest, Serialize, Value, application_write_receipt_succeeded,
    bundled_codex_harness_executable, data_dir_for_state, lock, persistence_outcome_unknown,
    persistent_threads, regeneration_application_selection, run_is_active, unix_epoch_millis,
};

pub(super) const CONTINUE_INPUT: &str = "继续原任务。依据当前会话的有效执行历史、已确认工具回执和已登记产物，先核对中断位置，再完成尚未完成的事项。已完成的操作不要重复执行；外部操作结果未知时先核实，无法核实则停止并说明。保留原任务目标和用户补充，不重新执行整段原请求。";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecoveryCheck {
    pub run_id: String,
    pub state: String,
    pub summary: String,
    pub checked_at: i64,
    pub continuation_run_id: Option<String>,
    pub native_input_recorded: bool,
}

fn reply(run: &AgentRun, state: &str, summary: &str) -> Result<RunRecoveryCheck, String> {
    Ok(RunRecoveryCheck {
        run_id: run.run_id.clone(),
        state: state.into(),
        summary: summary.into(),
        checked_at: unix_epoch_millis()?,
        continuation_run_id: None,
        native_input_recorded: false,
    })
}

pub(super) fn continuation_child<'a>(
    state: &'a ManagerState,
    source: &str,
) -> Option<&'a AgentRun> {
    state
        .persisted
        .runs
        .iter()
        .find(|run| run.continuation_of_run_id.as_deref() == Some(source))
}

pub(super) fn prior_write_receipt(
    state: &ManagerState,
    run_id: &str,
    application: &str,
    tool: &str,
    arguments: &Value,
) -> Result<Option<Value>, String> {
    let mut ancestors = HashSet::new();
    let mut current = state
        .persisted
        .runs
        .iter()
        .find(|run| run.run_id == run_id)
        .ok_or("agent_run_not_found")?;
    while let Some(parent) = current.continuation_of_run_id.as_deref() {
        if !ancestors.insert(parent.to_owned()) {
            return Err("ai_recovery_chain_invalid".into());
        }
        current = state
            .persisted
            .runs
            .iter()
            .find(|run| run.run_id == parent)
            .ok_or("ai_recovery_chain_invalid")?;
    }
    for request in state
        .persisted
        .permission_requests
        .iter()
        .rev()
        .filter(|request| ancestors.contains(&request.run_id))
    {
        let Some(write) = request.application_write.as_ref() else {
            continue;
        };
        if write.application_id != application
            || write.tool_name != tool
            || write.arguments != *arguments
        {
            continue;
        }
        if matches!(
            write.execution_status.as_deref(),
            Some("submitting" | "outcome_unknown")
        ) {
            return Err("ai_recovery_tool_outcome_unknown".into());
        }
        if write.execution_status.as_deref() == Some("succeeded") {
            let receipt = write
                .receipt
                .as_ref()
                .filter(|receipt| application_write_receipt_succeeded(receipt))
                .ok_or("ai_recovery_tool_outcome_unknown")?;
            return Ok(Some(receipt.clone()));
        }
    }
    Ok(None)
}

fn local_check(state: &ManagerState, run: &AgentRun) -> Result<Option<RunRecoveryCheck>, String> {
    data_dir_for_state(state)?;
    if state.shutting_down || state.memory_resetting || persistence_outcome_unknown(state) {
        return reply(run, "blocked", "运行状态尚未确认").map(Some);
    }
    if let Some(child) = continuation_child(state, &run.run_id) {
        let mut checked = reply(run, "continued", "已继续原任务")?;
        checked.continuation_run_id = Some(child.run_id.clone());
        return Ok(Some(checked));
    }
    if run_is_active(&run.status)
        || run.thread_writer_active
        || state.harness_processes.contains_key(&run.run_id)
    {
        return reply(run, "running", "原执行尚未结束").map(Some);
    }
    if run.status == "completed" {
        return reply(run, "completed", "原执行已完成").map(Some);
    }
    if run.superseded
        || run.execution_target != "model"
        || !matches!(run.status.as_str(), "failed" | "interrupted" | "terminated")
    {
        return reply(run, "blocked", "当前记录无法继续").map(Some);
    }
    let conversation = state
        .persisted
        .conversations
        .iter()
        .find(|conversation| conversation.conversation_id == run.conversation_id)
        .ok_or("conversation_not_found")?;
    if conversation.archived || conversation.agent_id.as_deref() != Some(&run.agent_id) {
        return reply(run, "blocked", "对话配置已变化").map(Some);
    }
    if state
        .persisted
        .runs
        .iter()
        .rev()
        .find(|item| item.conversation_id == run.conversation_id && !item.superseded)
        .is_none_or(|latest| latest.run_id != run.run_id)
    {
        return reply(run, "blocked", "请查看最新执行").map(Some);
    }
    if super::message_submission::conversation_preparing(state, &run.conversation_id)
        || super::conversation_session::follow_up_sending(
            &state.persisted.conversation_session,
            &run.conversation_id,
        )
    {
        return reply(run, "blocked", "对话仍有消息正在提交").map(Some);
    }
    if state.persisted.permission_requests.iter().any(|request| {
        request.run_id == run.run_id
            && (request.status == "pending"
                || request.application_write.as_ref().is_some_and(|write| {
                    matches!(
                        write.execution_status.as_deref(),
                        Some("submitting" | "outcome_unknown")
                    )
                }))
    }) {
        return reply(run, "blocked", "操作结果需核实").map(Some);
    }
    if let Some(reason) = super::personal_assistant::continuation_block_reason(state, run)? {
        return reply(run, "blocked", reason).map(Some);
    }
    Ok(None)
}

pub(super) fn validate_continuation_locked(
    state: &ManagerState,
    run: &AgentRun,
) -> Result<(), String> {
    if local_check(state, run)?.is_some() {
        return Err("ai_recovery_not_ready".into());
    }
    Ok(())
}

fn native_check(run: &AgentRun, turn: &Value) -> Result<RunRecoveryCheck, String> {
    match turn["status"].as_str() {
        Some("completed") => reply(run, "completed", "原执行已完成"),
        Some("inProgress") => reply(run, "blocked", "原生执行仍在进行，需核实"),
        Some("failed" | "interrupted") => {
            let items = turn["items"]
                .as_array()
                .ok_or("thread_recovery_response_invalid")?;
            let uncertain = items.iter().any(|item| {
                matches!(
                    item["type"].as_str(),
                    Some("mcpToolCall" | "dynamicToolCall" | "fileChange" | "commandExecution")
                ) && !matches!(item["status"].as_str(), Some("completed" | "declined"))
                    && item["exitCode"].as_i64().is_none()
            });
            if !uncertain
                && run.conversation_id == super::personal_assistant::THREAD
                && items.iter().any(assistant_tool_may_have_written)
            {
                return reply(run, "blocked", "本轮可能已变更个人事项，请先核对");
            }
            reply(
                run,
                if uncertain { "blocked" } else { "ready" },
                if uncertain {
                    "工具结果需核实"
                } else {
                    "可继续原任务"
                },
            )
        }
        _ => reply(run, "blocked", "原生执行结果需核实"),
    }
}

fn assistant_tool_may_have_written(item: &Value) -> bool {
    if item["type"] != "dynamicToolCall" || item["status"] == "declined" {
        return false;
    }
    let action = item["arguments"]["action"].as_str();
    match item["tool"].as_str() {
        Some("cilai_personal_assistant") => !matches!(
            action,
            Some("search" | "read" | "status" | "search_memory" | "reply")
        ),
        Some(tool) if super::personal_records::is_record_tool(tool) => {
            !matches!(action, Some("list" | "get"))
        }
        _ => true,
    }
}

impl AiRuntimeManager {
    /// 核对本地执行者、持久回执及原生轮次；此入口不提交模型请求。
    ///
    /// # Errors
    /// 身份、账本、进程或原生记录无法核实时返回错误。
    pub fn check_run_recovery(&self, run_id: &str) -> Result<RunRecoveryCheck, String> {
        let (run, server, service, home, profile) = {
            let state = lock(&self.inner);
            let run = state
                .persisted
                .runs
                .iter()
                .find(|run| run.run_id == run_id)
                .cloned()
                .ok_or("agent_run_not_found")?;
            if let Some(checked) = local_check(&state, &run)? {
                return Ok(checked);
            }
            let profile = state.data_dir.clone().ok_or("ai_runtime_not_initialized")?;
            let snapshot = run
                .model_execution
                .as_ref()
                .ok_or("model_runtime_scope_missing")?;
            let service = super::service_for_runtime_scope(
                &state,
                &crate::model_connections::runtime_scope(
                    snapshot,
                    contexthub_protocol::ModelRuntimeKind::Workbench,
                ),
            )?;
            let home = service.home().to_path_buf();
            (
                run,
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
                service,
                home,
                profile,
            )
        };
        let stored = server.request_result(
            "aiThread/get",
            &serde_json::json!({"conversationId":run.conversation_id}),
        )?;
        let recovered = persistent_threads::recover(
            &server,
            &run.conversation_id,
            stored,
            &bundled_codex_harness_executable()?,
            &home,
        )?;
        persistent_threads::validate_binding(&recovered, &persistent_threads::owner_instance_id())?;
        let operation = recovered["operations"].as_array().and_then(|items| {
            items.iter().rev().find(|operation| {
                operation["request"]["runId"] == run_id
                    && matches!(
                        operation["request"]["kind"].as_str(),
                        Some("send" | "steer")
                    )
            })
        });
        let checked = if let Some(operation) = operation {
            if operation["state"] == "acknowledged" {
                let thread = operation["receipt"]["nativeThreadId"]
                    .as_str()
                    .ok_or("thread_recovery_response_invalid")?;
                let turn_id = operation["receipt"]["nativeTurnId"]
                    .as_str()
                    .ok_or("thread_recovery_response_invalid")?;
                let turns = service.turns(thread)?;
                let turn = turns
                    .iter()
                    .find(|turn| turn["id"] == turn_id)
                    .ok_or("thread_recovery_response_invalid")?;
                let mut checked = native_check(&run, turn)?;
                checked.native_input_recorded = source_input_recorded(Some(&recovered), run_id);
                checked
            } else {
                reply(&run, "blocked", "消息接收结果需核实")?
            }
        } else if run.session_id.is_some() {
            reply(&run, "blocked", "原执行回执需核实")?
        } else {
            reply(&run, "ready", "可继续原任务")?
        };
        let state = lock(&self.inner);
        if state.data_dir.as_ref() != Some(&profile) {
            return Err("ai_profile_changed".into());
        }
        if let Some(current) = local_check(
            &state,
            state
                .persisted
                .runs
                .iter()
                .find(|item| item.run_id == run_id)
                .ok_or("agent_run_not_found")?,
        )? {
            return Ok(current);
        }
        Ok(checked)
    }

    /// # Errors
    /// 原执行尚不可继续或原始消息不可用时返回错误。
    pub fn continuation_request(&self, run_id: &str) -> Result<MessageSendRequest, String> {
        let checked = self.check_run_recovery(run_id)?;
        if checked.state != "ready" {
            return Err("ai_recovery_not_ready".into());
        }
        let state = lock(&self.inner);
        let run = state
            .persisted
            .runs
            .iter()
            .find(|run| run.run_id == run_id)
            .ok_or("agent_run_not_found")?;
        let message = state
            .persisted
            .messages
            .iter()
            .find(|message| message.message_id == run.input_message_id)
            .ok_or("retry_input_message_not_found")?;
        Ok(MessageSendRequest {
            conversation_id: run.conversation_id.clone(),
            content: continuation_content(checked.native_input_recorded, &message.content),
            client_message_id: Some(format!("continue_{run_id}")),
            attachment_ids: Vec::new(),
            referenced_materials: Vec::new(),
            application_context: regeneration_application_selection(message)?,
            selected_skill_id: run.selected_skill_id.clone(),
            retry_of_run_id: Some(run_id.into()),
            expected_work_run_id: None,
            model_override: run.model.clone(),
            model_selection: run.model_selection.clone(),
            history_consent: None,
            reasoning_effort: run.reasoning_effort.clone(),
            execution_target: run.execution_target.clone(),
            permission_mode: run
                .approval_mode
                .clone()
                .filter(|mode| mode != "legacy")
                .unwrap_or_else(|| run.sandbox_mode.clone()),
            planning: run.planning,
        })
    }

    #[must_use]
    pub fn continued_run(&self, source: &str) -> Option<AgentRun> {
        let state = lock(&self.inner);
        if persistence_outcome_unknown(&state) {
            return None;
        }
        continuation_child(&state, source).cloned()
    }

    /// # Errors
    /// 核对、授权、保存或执行启动失败时返回错误。
    pub fn continue_run(
        &self,
        source: &str,
        request: MessageSendRequest,
        context: Option<ApplicationAiContextSnapshot>,
        discovery: Option<ApplicationDiscoveryNotice>,
    ) -> Result<AgentRun, String> {
        if let Some(existing) = self.continued_run(source) {
            return Ok(existing);
        }
        if self.check_run_recovery(source)?.state != "ready" {
            return Err("ai_recovery_not_ready".into());
        }
        self.send_message_with_delivery(
            request,
            None,
            None,
            None,
            context,
            discovery,
            None,
            false,
            Some(source),
        )
    }
}

pub(super) fn source_input_recorded(journal: Option<&Value>, source: &str) -> bool {
    journal
        .and_then(|journal| journal["operations"].as_array())
        .is_some_and(|operations| {
            operations.iter().any(|operation| {
                operation["request"]["runId"] == source
                    && matches!(
                        operation["request"]["kind"].as_str(),
                        Some("send" | "steer")
                    )
                    && operation["state"] == "acknowledged"
            })
        })
}

pub(super) fn continuation_content(native_input_recorded: bool, original: &str) -> String {
    if native_input_recorded {
        "继续".into()
    } else {
        original.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> AgentRun {
        serde_json::from_value(serde_json::json!({"runId":"source","conversationId":"conversation","agentId":"agent","inputMessageId":"message","provider":"gateway","executionTarget":"model","status":"interrupted","startedAt":1})).unwrap()
    }

    #[test]
    fn completed_native_turn_never_creates_another_execution() {
        assert_eq!(
            native_check(&run(), &serde_json::json!({"status":"completed"}))
                .unwrap()
                .state,
            "completed"
        );
        assert_eq!(
            native_check(&run(), &serde_json::json!({"status":"inProgress"}))
                .unwrap()
                .state,
            "blocked"
        );
    }

    #[test]
    fn uncertain_tool_result_blocks_continuation_but_known_command_exit_does_not() {
        for item in [
            serde_json::json!({"type":"mcpToolCall","status":"failed"}),
            serde_json::json!({"type":"commandExecution","status":"inProgress"}),
        ] {
            assert_eq!(
                native_check(
                    &run(),
                    &serde_json::json!({"status":"interrupted","items":[item]})
                )
                .unwrap()
                .state,
                "blocked"
            );
        }
        assert_eq!(native_check(&run(), &serde_json::json!({"status":"failed","items":[{"type":"commandExecution","status":"failed","exitCode":1}]})).unwrap().state, "ready");
    }

    #[test]
    fn continuation_uses_saved_context_without_replaying_original_input() {
        assert_eq!(continuation_content(true, "original task"), "继续");
        assert_eq!(
            continuation_content(false, "original task"),
            "original task"
        );
    }

    #[test]
    fn assistant_recovery_distinguishes_queries_from_personal_writes() {
        let mut source = run();
        source.conversation_id = super::super::personal_assistant::THREAD.into();
        for (tool, action, state) in [
            ("cilai_personal_assistant", "status", "ready"),
            ("cilai_pa_todo", "list", "ready"),
            ("cilai_pa_memo", "get", "ready"),
            ("unknown_tool", "read", "blocked"),
            ("cilai_personal_assistant", "dispatch", "blocked"),
            ("cilai_pa_memo", "save", "blocked"),
        ] {
            let turn = serde_json::json!({"status":"interrupted","items":[{
                "type":"dynamicToolCall","status":"completed","tool":tool,"arguments":{"action":action}
            }]});
            assert_eq!(native_check(&source, &turn).unwrap().state, state);
        }
    }
    #[test]
    fn continuation_identity_survives_ledger_projection_and_blocks_duplicate_creation() {
        let source = run();
        let mut child = source.clone();
        child.run_id = "child".into();
        child.continuation_of_run_id = Some(source.run_id.clone());
        child.selected_skill_id = Some("selected-skill".into());
        let persisted = super::super::ledger_projection::run(&child);
        let restored: AgentRun = serde_json::from_value(persisted).unwrap();
        let mut state = ManagerState::default();
        state.persisted.runs = vec![source, restored];
        assert_eq!(
            continuation_child(&state, "source").unwrap().run_id,
            "child"
        );
        assert_eq!(
            continuation_child(&state, "source")
                .unwrap()
                .selected_skill_id
                .as_deref(),
            Some("selected-skill")
        );
    }

    #[test]
    fn confirmed_write_is_reused_only_in_the_same_continuation_chain() {
        let mut state = ManagerState::default();
        let source = run();
        let mut child = source.clone();
        child.run_id = "child".into();
        child.continuation_of_run_id = Some("source".into());
        state.persisted.runs = vec![source, child];
        state.persisted.permission_requests.push(serde_json::from_value(serde_json::json!({
            "permissionRequestId":"permission", "runId":"source", "agentId":"agent", "workspaceRoot":"/tmp", "directoryScope":"application-write",
            "tools":["save"], "networkAccess":true, "credentialLabels":[], "reason":"confirmed", "riskLevel":"write", "status":"consumed", "requestedAt":1,"validUntil":2,
            "applicationWrite":{"confirmationId":"confirmation", "applicationId":"app", "applicationName":"app", "toolName":"save", "toolTitle":"save", "description":"save", "riskLevel":"write", "arguments":{"id":1},"executionStatus":"succeeded", "receipt":{"status":"succeeded","invocationId":"original-operation","result":{}}}
        })).unwrap());
        let arguments = serde_json::json!({"id":1});
        assert_eq!(
            prior_write_receipt(&state, "child", "app", "save", &arguments)
                .unwrap()
                .unwrap()["invocationId"],
            "original-operation"
        );
        assert!(
            prior_write_receipt(&state, "source", "app", "save", &arguments)
                .unwrap()
                .is_none()
        );
        assert!(
            prior_write_receipt(&state, "child", "app", "save", &serde_json::json!({"id":2}))
                .unwrap()
                .is_none()
        );
        state.persisted.permission_requests[0]
            .application_write
            .as_mut()
            .unwrap()
            .execution_status = Some("outcome_unknown".into());
        assert_eq!(
            prior_write_receipt(&state, "child", "app", "save", &arguments).unwrap_err(),
            "ai_recovery_tool_outcome_unknown"
        );
    }
    #[test]
    fn continuation_keyword_does_not_replace_authoritative_input_receipts() {
        let mut journal = serde_json::json!({"operations":[{"request":{"runId":"other","kind":"send"},"state":"acknowledged"}]});
        assert!(!source_input_recorded(Some(&journal), "source"));
        journal["operations"][0]["request"]["runId"] = "source".into();
        journal["operations"][0]["state"] = "uncertain".into();
        assert!(!source_input_recorded(Some(&journal), "source"));
        journal["operations"][0]["state"] = "acknowledged".into();
        assert!(source_input_recorded(Some(&journal), "source"));
    }
}
