//! 页面模型用量只保存本次执行身份和官方明确提供的统计。
use super::{AgentTokenUsage, ApplicationModelRunSnapshot};
use crate::codex_harness::CodexHarnessResult;
use contexthub_protocol::ModelExecutionSnapshot;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplicationUsageIdentity {
    pub profile_id: String,
    pub connection_id: String,
    pub config_revision: u32,
    pub auth_generation: u32,
    pub model_id: String,
}
impl ApplicationUsageIdentity {
    pub(crate) fn from_execution(execution: &ModelExecutionSnapshot) -> Self {
        Self {
            profile_id: execution.profile_id.clone(),
            connection_id: execution.connection_id.clone(),
            config_revision: execution.config_revision,
            auth_generation: execution.auth_generation,
            model_id: execution.model_id.clone(),
        }
    }
}

pub(super) fn capture(snapshot: &mut ApplicationModelRunSnapshot, result: &CodexHarnessResult) {
    snapshot.reply_usage = result.reply_usage.clone().map(Into::into);
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct StoredApplicationUsage {
    pub run_id: String,
    pub model_id: Option<String>,
    pub status: String,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub event_position: u64,
    pub model_execution: Option<ApplicationUsageIdentity>,
    pub reply_usage: Option<AgentTokenUsage>,
}
impl StoredApplicationUsage {
    pub(super) fn from_snapshot(snapshot: &ApplicationModelRunSnapshot) -> Self {
        Self {
            run_id: snapshot.run_id.clone(),
            model_id: Some(snapshot.model_id.clone()),
            status: snapshot.status.clone(),
            started_at: Some(snapshot.started_at),
            completed_at: snapshot.completed_at,
            event_position: snapshot.event_position,
            model_execution: snapshot.model_execution.clone(),
            reply_usage: snapshot.reply_usage.clone(),
        }
    }
    pub(super) fn validate(&self) -> Result<(), String> {
        if self.run_id.is_empty()
            || !matches!(
                self.status.as_str(),
                "queued"
                    | "running"
                    | "waiting_confirmation"
                    | "submitting"
                    | "terminating"
                    | "recovery_required"
                    | "completed"
                    | "failed"
                    | "cancelled"
            )
            || self.model_execution.as_ref().is_some_and(|execution| {
                execution.profile_id.is_empty()
                    || execution.connection_id.is_empty()
                    || self.model_id.as_deref() != Some(execution.model_id.as_str())
            })
        {
            return Err("application_usage_storage_invalid".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_harness::CodexHarnessUsage;
    use serde_json::json;
    #[test]
    fn missing_reply_receipt_never_uses_aggregate_default_usage() {
        let mut snapshot: ApplicationModelRunSnapshot = serde_json::from_value(json!({
            "runId":"page-run","applicationId":"personal.page","modelId":"recorded-model","modelLabel":"Model",
            "status":"running","partialOutput":null,"output":null,"errorReason":null,"threadId":null,
            "startedAt":1,"completedAt":null,"modelTask":null,"skill":null,"receipt":null
        })).unwrap();
        let mut result = CodexHarnessResult {
            thread_id: "native-page-thread".into(),
            output: "private reply".into(),
            classification_output: None,
            classification_error: None,
            file_changes: Vec::new(),
            usage: CodexHarnessUsage {
                input: 99,
                cached_input: 98,
                output: 97,
            },
            reply_usage: None,
            classification_usage: None,
            context_tokens: None,
            context_window: None,
            context_composition: None,
        };
        capture(&mut snapshot, &result);
        assert!(snapshot.reply_usage.is_none());
        result.reply_usage = Some(CodexHarnessUsage::default());
        capture(&mut snapshot, &result);
        assert_eq!(
            snapshot.reply_usage,
            Some(AgentTokenUsage {
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0
            })
        );
    }
}
