use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use ts_rs::TS;

pub const CONVERSATION_SESSION_KEYS: &[&str] = &[
    "applicationHandoffs",
    "drafts",
    "queuedFollowUps",
    "selectedConversationId",
    "starterAgentId",
    "starterBusinessId",
    "starterFile",
    "starterModelOverride",
    "starterConnectionId",
    "starterReasoningEffort",
    "starterPermissionMode",
    "applicationContextSelections",
    "skillSelections",
    "draftAttachmentGroups",
    "draftMaterialGroups",
    "pendingCreate",
    "initializedSelection",
    "pendingOutgoing",
    "recoveredOutgoing",
];

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSessionSnapshot {
    pub session_id: String,
    #[ts(type = "number")]
    pub revision: u64,
    #[ts(type = "Record<string, unknown>")]
    pub values: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationSessionPatch {
    pub session_id: String,
    pub changes: Vec<ConversationSessionChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct ConversationSessionChange {
    pub key: String,
    #[ts(type = "unknown")]
    pub previous: Value,
    #[ts(type = "unknown")]
    pub value: Value,
}
