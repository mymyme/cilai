use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiThreadGetParams {
    pub conversation_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiThreadState {
    #[ts(type = "unknown")]
    pub binding: Option<Value>,
    #[ts(type = "unknown[]")]
    pub operations: Vec<Value>,
    #[ts(type = "unknown[]")]
    pub messages: Vec<Value>,
    #[serde(default)]
    #[ts(type = "unknown[]")]
    pub generations: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiThreadMutationResult {
    #[ts(type = "unknown")]
    pub binding: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "unknown")]
    pub operation: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayed: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
pub enum AiThreadOperationKind {
    Create,
    Resume,
    Fork,
    Inject,
    Send,
    Steer,
    Compact,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiThreadApplicationScope {
    pub application_id: String,
    pub session_kind: AiApplicationSessionKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum AiApplicationSessionKind {
    ApplicationPage,
    ApplicationTask,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiThreadPrepareParams {
    pub conversation_id: String,
    pub operation_id: String,
    pub run_id: String,
    pub owner_instance_id: String,
    #[ts(type = "number")]
    pub expected_binding_revision: i64,
    pub protocol_version: u32,
    pub kind: AiThreadOperationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_scope: Option<AiThreadApplicationScope>,
    #[ts(type = "unknown")]
    pub input: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum AiThreadOperationEvent {
    Dispatch,
    Acknowledge,
    Reject,
    Uncertain,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiThreadAcknowledgeParams {
    pub conversation_id: String,
    pub operation_id: String,
    pub run_id: String,
    pub owner_instance_id: String,
    pub event: AiThreadOperationEvent,
    #[ts(type = "unknown")]
    pub receipt: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiThreadReleaseParams {
    pub conversation_id: String,
    pub run_id: String,
    pub owner_instance_id: String,
    pub process_reaped: bool,
    /// 原生线程已空闲且后台任务已收尾，保留写入者身份供同一进程独占交接。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub retain_writer: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiThreadCleanupParams {
    pub conversation_id: String,
    pub native_thread_id: String,
    pub event: AiThreadOperationEvent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiApplicationParams {
    pub action: AiApplicationAction,
    #[ts(type = "unknown")]
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub enum AiApplicationAction {
    Open,
    OpenReport,
    OpenPage,
    ListInstance,
    RestartBlockers,
    Read,
    Usage,
    Reserve,
    Update,
    Close,
    BeginCleanup,
    FinishCleanup,
    ListPending,
    Attach,
    DiscardAttachment,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn application_usage_is_explicit_and_unknown_actions_remain_rejected() {
        let params: AiApplicationParams =
            serde_json::from_value(json!({"action":"usage","payload":{}})).unwrap();
        assert_eq!(params.action, AiApplicationAction::Usage);
        assert!(
            serde_json::from_value::<AiApplicationParams>(
                json!({"action":"usageWrite","payload":{}})
            )
            .is_err()
        );
    }
}
