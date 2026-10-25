use super::ui_projection::{artifact_for_ui, message_for_ui, run_for_ui};
use super::{
    AgentProfile, AgentRun, AgentRunArtifact, AiRuntimeManager, AutomationRule, Conversation,
    ConversationMessage, PermissionRequest, PersistedAiState, data_dir_for_state, lock,
};
use contexthub_protocol::{AiRuntimeQuery, AiRuntimeQueryScope};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunSummary {
    pub run_id: String,
    pub conversation_id: String,
    pub agent_id: String,
    pub input_message_id: String,
    pub retry_of_run_id: Option<String>,
    pub superseded: bool,
    pub provider: String,
    pub model: Option<String>,
    pub model_label: Option<String>,
    pub status: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub error_reason: Option<String>,
    pub automation_rule_id: Option<String>,
    pub automation_rule_version: Option<u32>,
    pub artifacts: Vec<AgentRunArtifact>,
    pub output_preview: Option<String>,
}

fn output_preview(output: &str) -> String {
    #[derive(serde::Deserialize)]
    struct Reply<'a> {
        #[serde(borrow)]
        reply: std::borrow::Cow<'a, str>,
    }
    let envelope = serde_json::from_str::<Reply<'_>>(output).ok();
    let text = envelope
        .as_ref()
        .map_or(output, |value| value.reply.as_ref());
    text.chars().take(240).collect()
}

fn run_summary(run: &AgentRun, include_preview: bool) -> AgentRunSummary {
    AgentRunSummary {
        run_id: run.run_id.clone(),
        conversation_id: run.conversation_id.clone(),
        agent_id: run.agent_id.clone(),
        input_message_id: run.input_message_id.clone(),
        retry_of_run_id: run.retry_of_run_id.clone(),
        superseded: run.superseded,
        provider: run.provider.clone(),
        model: run.model.clone(),
        model_label: run.model_label.clone(),
        status: run.status.clone(),
        started_at: run.started_at,
        completed_at: run.completed_at,
        error_reason: run.error_reason.clone(),
        automation_rule_id: run.automation_rule_id.clone(),
        automation_rule_version: run.automation_rule_version,
        artifacts: run.artifacts.iter().map(artifact_for_ui).collect(),
        output_preview: if include_preview {
            run.output.as_deref().map(output_preview)
        } else {
            None
        },
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRuntimeQuerySnapshot {
    pub scope: AiRuntimeQueryScope,
    pub state_revision: i64,
    pub agents: Vec<AgentProfile>,
    pub conversations: Vec<Conversation>,
    pub messages: Vec<ConversationMessage>,
    pub runs: Vec<AgentRunSummary>,
    pub run_details: Vec<AgentRun>,
    pub permission_requests: Vec<PermissionRequest>,
    pub automation_rules: Vec<AutomationRule>,
}

fn validate_request(request: &AiRuntimeQuery) -> Result<(), String> {
    let valid_id = |id: &str| !id.trim().is_empty() && id.len() <= 160;
    if [
        request.project_id.as_deref(),
        request.conversation_id.as_deref(),
        request.run_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|id| !valid_id(id))
        || request
            .run_ids
            .as_ref()
            .is_some_and(|ids| ids.iter().any(|id| !valid_id(id)))
    {
        return Err("ai_runtime_query_invalid".into());
    }
    let valid = match request.scope {
        AiRuntimeQueryScope::Project | AiRuntimeQueryScope::Work => {
            request.project_id.is_some()
                && request.conversation_id.is_none()
                && request.run_id.is_none()
                && (request.scope == AiRuntimeQueryScope::Work || request.run_ids.is_none())
        }
        AiRuntimeQueryScope::Conversation => {
            request.conversation_id.is_some()
                && request.project_id.is_none()
                && request.run_id.is_none()
                && request.run_ids.is_none()
        }
        AiRuntimeQueryScope::Run => {
            request.run_id.is_some()
                && request.project_id.is_none()
                && request.conversation_id.is_none()
                && request.run_ids.is_none()
        }
        AiRuntimeQueryScope::Automations
        | AiRuntimeQueryScope::Incidents
        | AiRuntimeQueryScope::Archive => {
            request.project_id.is_none()
                && request.conversation_id.is_none()
                && request.run_id.is_none()
                && request.run_ids.is_none()
        }
    };
    if !valid {
        return Err("ai_runtime_query_invalid".into());
    }
    Ok(())
}

fn conversation_summary(conversation: &Conversation) -> Conversation {
    Conversation {
        conversation_id: conversation.conversation_id.clone(),
        title: conversation.title.clone(),
        conversation_type: conversation.conversation_type.clone(),
        mode: conversation.mode.clone(),
        agent_id: conversation.agent_id.clone(),
        approval_mode: conversation.approval_mode.clone(),
        context: conversation.context.clone(),
        work_classification: conversation.work_classification.clone(),
        application_handoff: None,
        archived: conversation.archived,
        created_at: conversation.created_at,
        updated_at: conversation.updated_at,
    }
}

struct Selection<'a> {
    conversations: Vec<&'a Conversation>,
    runs: Vec<&'a AgentRun>,
}

fn select<'a>(
    state: &'a PersistedAiState,
    request: &AiRuntimeQuery,
) -> Result<Selection<'a>, String> {
    let run_target = request
        .run_id
        .as_ref()
        .map(|id| {
            state
                .runs
                .iter()
                .find(|run| &run.run_id == id)
                .ok_or("agent_run_not_found")
        })
        .transpose()?;
    let target_conversation = request
        .conversation_id
        .as_deref()
        .or_else(|| run_target.map(|run| run.conversation_id.as_str()));
    if let Some(id) = target_conversation
        && !state
            .conversations
            .iter()
            .any(|conversation| conversation.conversation_id == id)
    {
        return Err("conversation_not_found".into());
    }
    let conversations = state
        .conversations
        .iter()
        .filter(|conversation| match request.scope {
            AiRuntimeQueryScope::Project | AiRuntimeQueryScope::Work => {
                conversation.context.project_id == request.project_id
            }
            AiRuntimeQueryScope::Archive => conversation.archived,
            AiRuntimeQueryScope::Conversation | AiRuntimeQueryScope::Run => {
                Some(conversation.conversation_id.as_str()) == target_conversation
            }
            AiRuntimeQueryScope::Automations | AiRuntimeQueryScope::Incidents => true,
        })
        .collect::<Vec<_>>();
    let conversation_ids = conversations
        .iter()
        .map(|conversation| conversation.conversation_id.as_str())
        .collect::<HashSet<_>>();
    let requested_runs = request
        .run_ids
        .as_ref()
        .map(|ids| ids.iter().map(String::as_str).collect::<HashSet<_>>());
    let runs = state
        .runs
        .iter()
        .filter(|run| {
            conversation_ids.contains(run.conversation_id.as_str())
                && match request.scope {
                    AiRuntimeQueryScope::Automations => run.automation_rule_id.is_some(),
                    AiRuntimeQueryScope::Work => requested_runs
                        .as_ref()
                        .is_none_or(|ids| ids.contains(run.run_id.as_str())),
                    AiRuntimeQueryScope::Run => {
                        Some(run.run_id.as_str()) == request.run_id.as_deref()
                    }
                    AiRuntimeQueryScope::Archive => false,
                    AiRuntimeQueryScope::Project
                    | AiRuntimeQueryScope::Incidents
                    | AiRuntimeQueryScope::Conversation => true,
                }
        })
        .collect::<Vec<_>>();
    Ok(Selection {
        conversations,
        runs,
    })
}

fn selected_messages(
    state: &PersistedAiState,
    scope: AiRuntimeQueryScope,
    conversation_ids: &HashSet<&str>,
    run_ids: &HashSet<&str>,
    input_ids: &HashSet<&str>,
) -> Vec<ConversationMessage> {
    state
        .messages
        .iter()
        .filter(|message| match scope {
            AiRuntimeQueryScope::Conversation => {
                conversation_ids.contains(message.conversation_id.as_str())
            }
            AiRuntimeQueryScope::Run => message
                .run_id
                .as_deref()
                .is_some_and(|id| run_ids.contains(id)),
            AiRuntimeQueryScope::Work => {
                message.role == "user" && input_ids.contains(message.message_id.as_str())
            }
            _ => false,
        })
        .map(message_for_ui)
        .collect()
}

impl AiRuntimeManager {
    /// # Errors
    /// 查询用途、目标或当前资料不可用时返回明确错误，不扩大查询范围。
    pub fn query(&self, request: &AiRuntimeQuery) -> Result<AiRuntimeQuerySnapshot, String> {
        validate_request(request)?;
        let state = lock(&self.inner);
        data_dir_for_state(&state)?;
        let Selection {
            conversations,
            runs,
        } = select(&state.persisted, request)?;
        let details = matches!(
            request.scope,
            AiRuntimeQueryScope::Conversation | AiRuntimeQueryScope::Run
        );
        let conversation_ids = conversations
            .iter()
            .map(|conversation| conversation.conversation_id.as_str())
            .collect::<HashSet<_>>();
        let run_ids = runs
            .iter()
            .map(|run| run.run_id.as_str())
            .collect::<HashSet<_>>();
        let input_ids = runs
            .iter()
            .map(|run| run.input_message_id.as_str())
            .collect::<HashSet<_>>();
        let messages = selected_messages(
            &state.persisted,
            request.scope,
            &conversation_ids,
            &run_ids,
            &input_ids,
        );
        Ok(AiRuntimeQuerySnapshot {
            scope: request.scope,
            state_revision: state.state_revision,
            agents: if matches!(
                request.scope,
                AiRuntimeQueryScope::Automations | AiRuntimeQueryScope::Incidents
            ) {
                state.persisted.agents.clone()
            } else {
                Vec::new()
            },
            conversations: conversations
                .iter()
                .map(|conversation| conversation_summary(conversation))
                .collect(),
            messages,
            runs: if details {
                Vec::new()
            } else {
                runs.iter()
                    .map(|run| run_summary(run, request.scope == AiRuntimeQueryScope::Work))
                    .collect()
            },
            run_details: if details {
                runs.iter().map(|run| run_for_ui(run)).collect()
            } else {
                Vec::new()
            },
            permission_requests: if details {
                state
                    .persisted
                    .permission_requests
                    .iter()
                    .filter(|permission| run_ids.contains(permission.run_id.as_str()))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            },
            automation_rules: if request.scope == AiRuntimeQueryScope::Automations {
                state.persisted.automation_rules.clone()
            } else {
                Vec::new()
            },
        })
    }
}

#[cfg(test)]
mod tests;
