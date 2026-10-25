use super::{
    AiRuntimeManager, Conversation, ConversationCreateRequest, ConversationMessage,
    MAX_MESSAGE_ATTACHMENTS, MAX_MESSAGE_CHARS, ManagerState, MessageAttachment,
    MessageRegenerationEdit, MessageSendRequest, ModelInfo, automation_lifecycle,
    auxiliary_execution, build_model_classification_instructions, build_model_harness_instructions,
    data_dir_for_state, default_tool_permissions, lock, merge_message_materials,
    model_classification_output_schema, model_info_from_snapshot, model_main_turn_context,
    prepare_harness_attachments, prepare_harness_skills, required_application_harness_skills,
    validate_execution_target, validate_model_permission_mode_for_platform,
    validate_permission_mode,
};
use crate::codex_harness::{CodexHarnessAttachment, CodexHarnessSkill};
use crate::model_connections::{ExecutionScope, ModelExecutionPreflight};
use contexthub_platform_adapter::current_operating_system;
use contexthub_protocol::{ModelExecutionPrepareParams, ModelPurpose};
use serde_json::Value;
use std::sync::{Arc, MutexGuard};

pub(super) fn normalize_request(
    request: &mut MessageSendRequest,
    regenerating: bool,
) -> Result<(), String> {
    for material in &mut request.referenced_materials {
        material.resolved_path = None;
        material.cloud_reference = None;
    }
    if request.content.chars().count() > MAX_MESSAGE_CHARS {
        return Err("ai_message_too_large".into());
    }
    if regenerating && request.content.trim().is_empty() {
        return Err("ai_message_required".into());
    }
    validate_model_permission_mode_for_platform(
        current_operating_system(),
        &request.execution_target,
        &request.permission_mode,
    )?;
    validate_permission_mode(&request.permission_mode)?;
    validate_execution_target(&request.execution_target)?;
    if request.planning && request.execution_target != "model" {
        return Err("ai_planning_requires_model".into());
    }
    if request.application_context.is_some() {
        return Err("ai_application_context_unvalidated".into());
    }
    if request.execution_target != "model" {
        return Err("ai_local_agents_removed".into());
    }
    if !matches!(
        request.permission_mode.as_str(),
        "request" | "auto-review" | "full-access" | "read-only" | "workspace-write"
    ) {
        return Err("ai_permission_mode_invalid".into());
    }
    Ok(())
}

pub(super) struct ModelPreparation {
    pub(super) preflight: ModelExecutionPreflight,
    pub(super) execution: Arc<ExecutionScope>,
    pub(super) model: ModelInfo,
    pub(super) review_execution: Option<Arc<ExecutionScope>>,
    pub(super) memory_execution: Option<Arc<ExecutionScope>>,
}

impl ModelPreparation {
    pub(super) fn prepare(
        manager: &AiRuntimeManager,
        request: &MessageSendRequest,
        ticket: Option<&automation_lifecycle::AutomationTicket>,
        automated: bool,
    ) -> Result<Self, String> {
        let purpose = if automated {
            ModelPurpose::Automation
        } else {
            ModelPurpose::Foreground
        };
        let execution_params = ModelExecutionPrepareParams {
            launch_protocol: 1,
            owner: None,
            purpose,
            binding_id: None,
            selection: request.model_selection.clone(),
            conversation_id: (!request.conversation_id.is_empty())
                .then(|| request.conversation_id.clone()),
        };
        let preflight =
            manager.personal_model_message_preflight(request, execution_params.purpose.clone())?;
        let execution = match ticket {
            Some(ticket) => ticket.prepare_execution(manager, execution_params)?,
            None => ExecutionScope::prepare(manager, execution_params)?,
        };
        if execution.lease.snapshot_digest != preflight.snapshot_digest {
            return Err("model_execution_snapshot_changed".into());
        }
        if request
            .model_override
            .as_ref()
            .is_some_and(|model| model != &execution.lease.snapshot.model_id)
        {
            return Err("model_selection_required".into());
        }
        let model = model_info_from_snapshot(&execution.lease.snapshot)?;
        let review_execution = (request.permission_mode == "auto-review")
            .then(|| {
                let params = ModelExecutionPrepareParams {
                    launch_protocol: 1,
                    owner: None,
                    purpose: ModelPurpose::Review,
                    binding_id: None,
                    selection: None,
                    conversation_id: None,
                };
                match ticket {
                    Some(ticket) => ticket.prepare_execution(manager, params),
                    None => ExecutionScope::prepare(manager, params),
                }
            })
            .transpose()?;
        let memory_enabled = lock(&manager.inner).persisted.memory_preferences.enabled;
        let memory_execution = memory_enabled
            .then(|| {
                let params = ModelExecutionPrepareParams {
                    launch_protocol: 1,
                    owner: None,
                    purpose: ModelPurpose::Memory,
                    binding_id: None,
                    selection: None,
                    conversation_id: None,
                };
                match ticket {
                    Some(ticket) => ticket.prepare_execution(manager, params),
                    None => ExecutionScope::prepare(manager, params),
                }
            })
            .transpose()?;
        for auxiliary in review_execution.iter().chain(memory_execution.iter()) {
            auxiliary_execution::validate(&execution.lease.snapshot, &auxiliary.lease.snapshot)?;
        }
        if execution.lease.snapshot.kind != contexthub_protocol::ModelConnectionKind::Chatgpt {
            execution.credential()?;
        }
        Ok(Self {
            preflight,
            execution,
            model,
            review_execution,
            memory_execution,
        })
    }
}

pub(super) fn prefetch_materials(
    manager: &AiRuntimeManager,
    request: &MessageSendRequest,
    create: Option<&ConversationCreateRequest>,
) -> Result<(), String> {
    let prefetch_context = if let Some(create) = create {
        Some(create.context.clone())
    } else {
        lock(&manager.inner)
            .persisted
            .conversations
            .iter()
            .find(|conversation| conversation.conversation_id == request.conversation_id)
            .map(|conversation| conversation.context.clone())
    };
    if let Some(mut context) = prefetch_context {
        merge_message_materials(&mut context, &request.referenced_materials)?;
        manager.prefetch_context_materials(
            &context,
            if create.is_some() {
                None
            } else {
                Some(&request.conversation_id)
            },
        )?;
    } else {
        manager.prepare_local_materials(&request.referenced_materials)?;
    }
    Ok(())
}

pub(super) struct LockedPreparation<'a> {
    state: &'a ManagerState,
    request: &'a MessageSendRequest,
    model: &'a ModelInfo,
}

impl<'a> LockedPreparation<'a> {
    pub(super) fn new(
        state: &'a MutexGuard<'_, ManagerState>,
        request: &'a MessageSendRequest,
        model: &'a ModelInfo,
    ) -> Self {
        Self {
            state,
            request,
            model,
        }
    }

    pub(super) fn requested_attachments(
        &self,
        retrying: bool,
    ) -> Result<Vec<MessageAttachment>, String> {
        let state = self.state;
        let request = self.request;
        if request.attachment_ids.len() > MAX_MESSAGE_ATTACHMENTS {
            return Err("ai_attachment_count_exceeded".to_owned());
        }
        let requested_attachments = if retrying {
            Vec::new()
        } else {
            request
                .attachment_ids
                .iter()
                .map(|attachment_id| {
                    let pending = state
                        .pending_attachments
                        .get(attachment_id)
                        .ok_or_else(|| "ai_attachment_not_found".to_owned())?;
                    if !pending.complete {
                        return Err("ai_attachment_incomplete".to_owned());
                    }
                    Ok(pending.attachment.clone())
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        Ok(requested_attachments)
    }

    pub(super) fn input_message(
        &self,
        conversation: &Conversation,
        retry_of_run_id: Option<&str>,
        regeneration: Option<&MessageRegenerationEdit>,
        requested_attachments: Vec<MessageAttachment>,
        now: i64,
    ) -> Result<ConversationMessage, String> {
        let state = self.state;
        let request = self.request;
        let model = self.model;
        let mut input_message = if let Some(retry_run_id) = retry_of_run_id {
            let previous = state
                .persisted
                .runs
                .iter()
                .find(|run| run.run_id == retry_run_id)
                .ok_or_else(|| "retry_run_not_found".to_owned())?;
            state
                .persisted
                .messages
                .iter()
                .find(|message| message.message_id == previous.input_message_id)
                .cloned()
                .ok_or_else(|| "retry_input_message_not_found".to_owned())?
        } else {
            let content = request.content.trim();
            if content.is_empty()
                && requested_attachments.is_empty()
                && request.referenced_materials.is_empty()
            {
                return Err("ai_message_required".to_owned());
            }
            ConversationMessage {
                message_id: request
                    .client_message_id
                    .clone()
                    .unwrap_or_else(|| format!("message_{}", uuid::Uuid::new_v4())),
                conversation_id: conversation.conversation_id.clone(),
                role: "user".to_owned(),
                content: content.to_owned(),
                attachments: requested_attachments,
                referenced_materials: request.referenced_materials.clone(),
                application_context: None,
                run_id: None,
                created_at: now,
            }
        };
        if let Some(edit) = regeneration {
            edit.content.trim().clone_into(&mut input_message.content);
            input_message.application_context = None;
        }
        if input_message
            .attachments
            .iter()
            .any(|item| item.media_type != "document")
            && !model.input_modalities.iter().any(|item| item == "image")
        {
            return Err("ai_attachment_not_supported".to_owned());
        }
        Ok(input_message)
    }
}

pub(super) struct RunCapabilities {
    pub(super) run_id: String,
    pub(super) harness_skills: Vec<CodexHarnessSkill>,
    pub(super) classification_nonce: String,
    pub(super) task_candidates: Value,
    pub(super) policy: RunPolicy,
    pub(super) turn_context: String,
    pub(super) model_classification: Option<(String, Value)>,
    pub(super) harness_attachments: Vec<CodexHarnessAttachment>,
}

pub(super) struct RunPolicy {
    pub(super) approval_mode: String,
    pub(super) sandbox_mode: String,
    pub(super) run_tools: Vec<String>,
    pub(super) prompt: String,
}

impl RunPolicy {
    fn prepare(request: &MessageSendRequest, conversation: &Conversation) -> Result<Self, String> {
        let approval_mode = match request.permission_mode.as_str() {
            "read-only" | "workspace-write" => "legacy",
            mode => mode,
        }
        .to_owned();
        let write_allowed = !request.planning && request.permission_mode != "read-only";
        let sandbox_mode = if request.planning {
            "read-only"
        } else if request.permission_mode == "full-access" {
            "danger-full-access"
        } else if write_allowed {
            "workspace-write"
        } else {
            "read-only"
        }
        .to_owned();
        let mut prompt = build_model_harness_instructions(conversation, write_allowed)?;
        if request.planning {
            prompt.push_str("\n本轮为计划模式：仅阅读、分析和提出可供用户审阅的方案。不得执行文件修改、应用业务操作、生图、协作或发布。输出明确步骤和需要用户确认的事项；用户选择执行并发送新消息后才能实施。");
        }
        let mut run_tools = default_tool_permissions();
        if write_allowed && !run_tools.iter().any(|tool| tool == "write-files") {
            run_tools.push("write-files".into());
        }
        Ok(Self {
            approval_mode,
            sandbox_mode,
            run_tools,
            prompt,
        })
    }
}

impl LockedPreparation<'_> {
    pub(super) fn reasoning_effort(
        &self,
        requested: Option<&str>,
    ) -> Result<Option<String>, String> {
        super::resolve_reasoning_effort(self.model, requested)
    }

    pub(super) fn history(
        &self,
        conversation: &Conversation,
        input_message: &ConversationMessage,
        regeneration_target: Option<&ConversationMessage>,
    ) -> Result<Vec<ConversationMessage>, String> {
        let messages = self
            .state
            .persisted
            .messages
            .iter()
            .filter(|message| message.conversation_id == conversation.conversation_id);
        let history = if let Some(target) = regeneration_target {
            let mut prefix = Vec::new();
            for message in messages {
                let mut message = message.clone();
                if message.message_id == target.message_id {
                    message.content.clone_from(&input_message.content);
                    prefix.push(message);
                    break;
                }
                prefix.push(message);
            }
            prefix
        } else {
            messages.cloned().collect()
        };
        if !self
            .model
            .input_modalities
            .iter()
            .any(|modality| modality == "image")
            && history.iter().any(|message| {
                message
                    .attachments
                    .iter()
                    .any(|attachment| attachment.media_type != "document")
            })
        {
            return Err("model_history_incompatible".into());
        }
        Ok(history)
    }

    fn skills(
        &self,
        input_message: &ConversationMessage,
        run_id: &str,
    ) -> Result<Vec<CodexHarnessSkill>, String> {
        let application_skill_binding = input_message
            .application_context
            .as_ref()
            .and_then(|context| context.skill_binding.as_ref());
        let prepared = prepare_harness_skills(
            self.state
                .server
                .as_ref()
                .ok_or("ai_runtime_not_initialized")?,
            self.state
                .managed_skills_root
                .as_deref()
                .ok_or("ai_runtime_not_initialized")?,
            &data_dir_for_state(self.state)?,
            run_id,
            self.request.selected_skill_id.as_deref(),
            application_skill_binding,
        );
        if application_skill_binding.is_some() {
            required_application_harness_skills(prepared)
        } else {
            prepared
        }
    }

    fn task_candidates(&self, conversation: &Conversation) -> Result<Value, String> {
        let Some(project_id) = conversation.context.project_id.as_deref() else {
            return Ok(Value::Array(Vec::new()));
        };
        let result = self
            .state
            .server
            .as_ref()
            .ok_or("ai_runtime_not_initialized")?
            .request_result(
                "aiWork/taskCandidates",
                &serde_json::json!({"projectId":project_id}),
            )?;
        result
            .get("tasks")
            .cloned()
            .ok_or_else(|| "ai_work_task_candidates_invalid".into())
    }

    pub(super) fn capabilities(
        &self,
        conversation: &Conversation,
        input_message: &ConversationMessage,
        history: &[ConversationMessage],
    ) -> Result<RunCapabilities, String> {
        let run_id = format!("run_{}", uuid::Uuid::new_v4());
        let harness_skills = self.skills(input_message, &run_id)?;
        let classification_nonce = format!("classification_{}", uuid::Uuid::new_v4());
        let task_candidates = self.task_candidates(conversation)?;
        let turn_context = model_main_turn_context(conversation, input_message, history)?;
        let policy = RunPolicy::prepare(self.request, conversation)?;
        let model_classification =
            if !self.request.planning && conversation.context.project_id.is_some() {
                Some((
                    build_model_classification_instructions(
                        conversation,
                        &classification_nonce,
                        &task_candidates,
                    )?,
                    model_classification_output_schema(
                        &classification_nonce,
                        conversation.context.task_id.as_deref(),
                    ),
                ))
            } else {
                None
            };
        let harness_attachments = prepare_harness_attachments(
            &input_message.attachments,
            &data_dir_for_state(self.state)?,
            &run_id,
        )?;
        Ok(RunCapabilities {
            run_id,
            harness_skills,
            classification_nonce,
            task_candidates,
            policy,
            turn_context,
            model_classification,
            harness_attachments,
        })
    }
}

#[cfg(test)]
mod tests;
