use super::{AgentRun, PersistedAiState};
use serde_json::{Value, json};

pub(super) fn state(state: &PersistedAiState) -> Value {
    let mut projected = state_without_messages(state);
    projected["messages"] = json!(state.messages);
    projected
}

/// 消息正文体量最大且极少变化，保存路径按消息缓存投影，因此这里不序列化消息。
pub(super) fn state_without_messages(state: &PersistedAiState) -> Value {
    let mut projected = json!({
        "agents":state.agents,
        "conversations":state.conversations,
        "messages":[],
        "permissionRequests":state.permission_requests,
        "inboxReadItemIds":state.inbox_read_item_ids,
        "automationRules":state.automation_rules,
        "approvalReviewModel":state.approval_review_model,
        "memoryPreferences":state.memory_preferences,
        "memoryEnableConfirmation":state.memory_enable_confirmation,
        "conversationSession":state.conversation_session,
        "draftAttachments":state.draft_attachments,
        "runs":state.runs.iter().map(run).collect::<Vec<_>>()
    });
    if !state.personal_notifications.is_default() {
        projected["personalNotifications"] = json!(state.personal_notifications);
    }
    projected
}

pub(super) fn run(run: &AgentRun) -> Value {
    let AgentRun {
        planning,
        model_selection,
        model_execution,
        run_id,
        conversation_id,
        agent_id,
        input_message_id,
        retry_of_run_id,
        superseded,
        regeneration_draft: _,
        previous_revisions: _,
        provider,
        model,
        model_label,
        reasoning_effort,
        execution_target,
        status,
        thread_writer_active,
        pid,
        started_at,
        completed_at,
        output: _,
        partial_output: _,
        error_reason,
        classification_nonce,
        classification_status,
        classification_error,
        session_id,
        harness_protocol_version,
        model_session_scope,
        memory_preferences,
        skill_ids,
        tool_permissions,
        permission_request_id,
        automation_rule_id,
        automation_rule_version,
        sandbox_mode,
        approval_mode,
        events,
        artifacts,
        usage,
        log_tail: _,
    } = run;
    let latest = events.iter().rev().find(|event| {
        event.phase == "reply" && event.kind == "mcp" && event.event_type == "item.completed"
    });
    json!({
        "planning":planning,"modelSelection":model_selection,"modelExecution":model_execution,
        "runId":run_id,
        "conversationId":conversation_id,
        "agentId":agent_id,
        "inputMessageId":input_message_id,
        "retryOfRunId":retry_of_run_id,
        "superseded":superseded,
        "provider":provider,
        "model":model,
        "modelLabel":model_label,
        "reasoningEffort":reasoning_effort,
        "executionTarget":execution_target,
        "status":status,
        "threadWriterActive":thread_writer_active,
        "pid":pid,
        "startedAt":started_at,
        "completedAt":completed_at,
        "errorReason":error_reason,
        "classificationNonce":classification_nonce,
        "classificationStatus":classification_status,
        "classificationError":classification_error,
        "sessionId":session_id,
        "harnessProtocolVersion":harness_protocol_version,
        "modelSessionScope":model_session_scope,
        "memoryPreferences":memory_preferences,
        "skillIds":skill_ids,
        "toolPermissions":tool_permissions,
        "permissionRequestId":permission_request_id,
        "automationRuleId":automation_rule_id,
        "automationRuleVersion":automation_rule_version,
        "sandboxMode":sandbox_mode,
        "approvalMode":approval_mode,
        "artifacts":artifacts,
        "usage":usage,
        "events":latest.into_iter().collect::<Vec<_>>()
    })
}
