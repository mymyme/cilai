use super::{
    AgentRun, AgentRunArtifact, ApplicationAiContextSnapshot, ArtifactVersion, ConversationMessage,
    MessageAttachment, RegenerationDraft,
};

fn application_for_ui(application: &ApplicationAiContextSnapshot) -> ApplicationAiContextSnapshot {
    ApplicationAiContextSnapshot {
        application_id: application.application_id.clone(),
        application_name: application.application_name.clone(),
        application_ids: application.application_ids.clone(),
        restrict_to_applications: application.restrict_to_applications,
        skill_binding: application.skill_binding.clone(),
        model_task: application.model_task.clone(),
        browser_origins: application.browser_origins.clone(),
        report_review: application.report_review,
        local_material_paths: application.local_material_paths.clone(),
        version: application.version.clone(),
        page_revision: application.page_revision,
        contexts: Vec::new(),
        actions: Vec::new(),
        tool_names: application.tool_names.clone(),
        read_tools: Vec::new(),
        write_tools: Vec::new(),
        searchable_applications: Vec::new(),
        tool_set_revision: application.tool_set_revision.clone(),
    }
}

fn attachment_for_ui(attachment: &MessageAttachment) -> MessageAttachment {
    MessageAttachment {
        attachment_id: attachment.attachment_id.clone(),
        file_name: attachment.file_name.clone(),
        mime_type: attachment.mime_type.clone(),
        media_type: attachment.media_type.clone(),
        size_bytes: attachment.size_bytes,
        local_path: String::new(),
    }
}

pub(super) fn message_for_ui(message: &ConversationMessage) -> ConversationMessage {
    ConversationMessage {
        message_id: message.message_id.clone(),
        conversation_id: message.conversation_id.clone(),
        role: message.role.clone(),
        content: message.content.clone(),
        attachments: message.attachments.iter().map(attachment_for_ui).collect(),
        referenced_materials: message.referenced_materials.clone(),
        application_context: message.application_context.as_ref().map(application_for_ui),
        run_id: message.run_id.clone(),
        created_at: message.created_at,
    }
}

pub(super) fn artifact_for_ui(artifact: &AgentRunArtifact) -> AgentRunArtifact {
    AgentRunArtifact {
        version: artifact.version.as_ref().map(|version| ArtifactVersion {
            digest: version.digest.clone(),
            size_bytes: version.size_bytes,
            created_at: version.created_at,
            snapshot_path: String::new(),
        }),
        feedback: artifact.feedback.clone(),
        artifact_id: artifact.artifact_id.clone(),
        artifact_type: artifact.artifact_type.clone(),
        label: artifact.label.clone(),
        path: artifact.path.clone(),
    }
}

pub(super) fn run_for_ui(run: &AgentRun) -> AgentRun {
    AgentRun {
        planning: run.planning,
        run_id: run.run_id.clone(),
        conversation_id: run.conversation_id.clone(),
        agent_id: run.agent_id.clone(),
        input_message_id: run.input_message_id.clone(),
        retry_of_run_id: run.retry_of_run_id.clone(),
        superseded: run.superseded,
        regeneration_draft: run
            .regeneration_draft
            .as_ref()
            .map(|draft| RegenerationDraft {
                content: draft.content.clone(),
                source_run_id: draft.source_run_id.clone(),
                application_context: None,
            }),
        previous_revisions: run.previous_revisions.clone(),
        model_selection: run.model_selection.clone(),
        model_execution: run.model_execution.clone(),
        provider: run.provider.clone(),
        model: run.model.clone(),
        model_label: run.model_label.clone(),
        reasoning_effort: run.reasoning_effort.clone(),
        execution_target: run.execution_target.clone(),
        status: run.status.clone(),
        thread_writer_active: run.thread_writer_active,
        pid: run.pid,
        started_at: run.started_at,
        completed_at: run.completed_at,
        output: run.output.clone(),
        partial_output: run.partial_output.clone(),
        error_reason: run.error_reason.clone(),
        classification_nonce: run.classification_nonce.clone(),
        classification_status: run.classification_status.clone(),
        classification_error: run.classification_error.clone(),
        session_id: run.session_id.clone(),
        harness_protocol_version: run.harness_protocol_version,
        model_session_scope: None,
        memory_preferences: run.memory_preferences,
        skill_ids: run.skill_ids.clone(),
        tool_permissions: run.tool_permissions.clone(),
        permission_request_id: run.permission_request_id.clone(),
        automation_rule_id: run.automation_rule_id.clone(),
        automation_rule_version: run.automation_rule_version,
        sandbox_mode: run.sandbox_mode.clone(),
        approval_mode: run.approval_mode.clone(),
        events: run.events.clone(),
        artifacts: run.artifacts.iter().map(artifact_for_ui).collect(),
        usage: run.usage.clone(),
        log_tail: Vec::new(),
    }
}
