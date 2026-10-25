use super::{
    ApplicationAiContextSnapshot, ArtifactFeedback, ArtifactVersion, ConversationMessage,
    MAX_LOG_LINES, PersistedAiState, default_execution_target, default_sandbox_mode,
    next_event_sequence,
};
use crate::codex_harness::CodexHarnessEvent;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    #[serde(default)]
    pub planning: bool,
    pub run_id: String,
    pub conversation_id: String,
    pub agent_id: String,
    pub input_message_id: String,
    pub retry_of_run_id: Option<String>,
    #[serde(default)]
    pub superseded: bool,
    #[serde(default)]
    pub regeneration_draft: Option<RegenerationDraft>,
    #[serde(default)]
    pub previous_revisions: Vec<ConversationRevision>,
    #[serde(default)]
    pub model_selection: Option<contexthub_protocol::ModelSelection>,
    #[serde(default)]
    pub model_execution: Option<contexthub_protocol::ModelExecutionSnapshot>,
    pub provider: String,
    pub model: Option<String>,
    #[serde(default)]
    pub model_label: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default = "default_execution_target")]
    pub execution_target: String,
    pub status: String,
    #[serde(default)]
    pub thread_writer_active: bool,
    pub pid: Option<u32>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub output: Option<String>,
    #[serde(default)]
    pub partial_output: Option<String>,
    pub error_reason: Option<String>,
    #[serde(default)]
    pub classification_nonce: Option<String>,
    #[serde(default)]
    pub classification_status: Option<String>,
    #[serde(default)]
    pub classification_error: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub harness_protocol_version: Option<u32>,
    #[serde(default)]
    pub model_session_scope: Option<String>,
    #[serde(default)]
    pub memory_preferences: Option<crate::memory::MemoryPreferences>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    #[serde(default)]
    pub tool_permissions: Vec<String>,
    #[serde(default)]
    pub permission_request_id: Option<String>,
    #[serde(default)]
    pub automation_rule_id: Option<String>,
    #[serde(default)]
    pub automation_rule_version: Option<u32>,
    #[serde(default = "default_sandbox_mode")]
    pub sandbox_mode: String,
    #[serde(default)]
    pub approval_mode: Option<String>,
    #[serde(default)]
    pub events: Vec<AgentRunEvent>,
    #[serde(default)]
    pub artifacts: Vec<AgentRunArtifact>,
    #[serde(default)]
    pub usage: AgentRunUsage,
    #[serde(default)]
    pub log_tail: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegenerationDraft {
    pub content: String,
    pub source_run_id: String,
    pub application_context: Option<ApplicationAiContextSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRevision {
    pub run_id: String,
    pub user_content: String,
    pub assistant_contents: Vec<String>,
    pub incomplete: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)]
pub struct AgentRunUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub reply: Option<AgentTokenUsage>,
    #[serde(default)]
    pub classification: Option<AgentTokenUsage>,
    #[serde(default)]
    pub context: Option<AgentContextUsage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)]
pub struct AgentTokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
}

impl From<crate::codex_harness::CodexHarnessUsage> for AgentTokenUsage {
    fn from(value: crate::codex_harness::CodexHarnessUsage) -> Self {
        Self {
            input_tokens: value.input,
            cached_input_tokens: value.cached_input,
            output_tokens: value.output,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentContextUsage {
    pub tokens: u64,
    pub window_tokens: Option<u64>,
    #[serde(default)]
    pub composition: Option<AgentContextComposition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_field_names)]
pub struct AgentContextComposition {
    pub conversation_tokens: u64,
    pub system_prompt_tokens: u64,
    pub tool_tokens: u64,
    pub skill_tokens: u64,
    pub mcp_tokens: Option<u64>,
    pub internal_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunEvent {
    pub sequence: u32,
    pub event_type: String,
    #[serde(default)]
    pub item_id: Option<String>,
    #[serde(default = "default_event_phase")]
    pub phase: String,
    #[serde(default = "default_event_kind")]
    pub kind: String,
    #[serde(default = "default_event_status")]
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub detail_available: bool,
    #[serde(default)]
    pub read_paths: Vec<String>,
    pub created_at: i64,
}

fn default_event_phase() -> String {
    "reply".to_owned()
}
fn default_event_kind() -> String {
    "status".to_owned()
}
fn default_event_status() -> String {
    "completed".to_owned()
}

impl Default for AgentRunEvent {
    fn default() -> Self {
        Self {
            sequence: 0,
            event_type: String::new(),
            item_id: None,
            phase: default_event_phase(),
            kind: default_event_kind(),
            status: default_event_status(),
            summary: String::new(),
            duration_ms: None,
            detail_available: false,
            read_paths: Vec::new(),
            created_at: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunEventDetail {
    pub run_id: String,
    pub item_id: String,
    pub phase: String,
    pub kind: String,
    pub status: String,
    pub tool_name: String,
    pub operation: String,
    pub target: Option<String>,
    pub duration_ms: Option<u64>,
    pub parameters: Option<String>,
    pub output: Option<String>,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunArtifact {
    #[serde(default)]
    pub version: Option<ArtifactVersion>,
    #[serde(default)]
    pub feedback: Vec<ArtifactFeedback>,
    pub artifact_id: String,
    pub artifact_type: String,
    pub label: String,
    pub path: Option<String>,
}

#[derive(Eq, PartialEq)]
pub(super) enum ProgressPersistence {
    Transient,
    Required,
}

impl AgentRun {
    pub(super) fn record_steered_input(&mut self, event: CodexHarnessEvent) {
        self.events.push(AgentRunEvent {
            sequence: next_event_sequence(&self.events),
            event_type: event.event_type,
            item_id: event.item_id,
            phase: event.phase,
            kind: event.kind,
            status: event.status,
            summary: event.summary,
            created_at: event.created_at,
            ..AgentRunEvent::default()
        });
    }

    pub(super) fn observe_progress(&mut self, event: CodexHarnessEvent) -> ProgressPersistence {
        if let Some(streamed_output) = event.streamed_output {
            self.partial_output = (!streamed_output.is_empty()).then_some(streamed_output);
            return ProgressPersistence::Transient;
        }
        if event.phase == "reply"
            && let Some((tokens, window_tokens)) = event.context_usage
        {
            self.usage.context = Some(AgentContextUsage {
                tokens,
                window_tokens,
                composition: None,
            });
        }
        self.log_tail.push(event.summary.clone());
        if self.log_tail.len() > MAX_LOG_LINES {
            self.log_tail.remove(0);
        }
        if let Some(existing) = event.item_id.as_deref().and_then(|item_id| {
            self.events.iter_mut().rev().find(|current| {
                current.item_id.as_deref() == Some(item_id) && current.phase == event.phase
            })
        }) {
            existing.event_type = event.event_type;
            existing.kind = event.kind;
            existing.status = event.status;
            existing.summary = event.summary;
            existing.duration_ms = event.duration_ms.or_else(|| {
                u64::try_from(event.created_at.saturating_sub(existing.created_at)).ok()
            });
            existing.detail_available |= event.detail_available;
            existing.read_paths = event.read_paths;
        } else {
            self.events.push(AgentRunEvent {
                sequence: next_event_sequence(&self.events),
                event_type: event.event_type,
                item_id: event.item_id,
                phase: event.phase,
                kind: event.kind,
                status: event.status,
                summary: event.summary,
                duration_ms: event.duration_ms,
                detail_available: event.detail_available,
                read_paths: event.read_paths,
                created_at: event.created_at,
            });
        }
        ProgressPersistence::Required
    }
}

impl PersistedAiState {
    pub(super) fn record_steered_message(
        &mut self,
        run_id: &str,
        message_id: String,
        content: String,
        event: CodexHarnessEvent,
    ) -> bool {
        if self
            .messages
            .iter()
            .any(|message| message.message_id == message_id)
        {
            return false;
        }
        let Some(run) = self.runs.iter_mut().find(|run| run.run_id == run_id) else {
            return false;
        };
        self.messages.push(ConversationMessage {
            message_id,
            conversation_id: run.conversation_id.clone(),
            role: "user".into(),
            content,
            attachments: Vec::new(),
            referenced_materials: Vec::new(),
            application_context: None,
            run_id: Some(run_id.to_owned()),
            created_at: event.created_at,
        });
        if let Some(conversation) = self
            .conversations
            .iter_mut()
            .find(|conversation| conversation.conversation_id == run.conversation_id)
        {
            conversation.updated_at = event.created_at;
        }
        run.record_steered_input(event);
        true
    }
}

#[cfg(test)]
mod tests;
