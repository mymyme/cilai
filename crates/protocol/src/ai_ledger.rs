use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiLedgerGetParams {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiLedgerGetResult {
    #[ts(type = "unknown")]
    pub ledger: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiLedgerRecordMutation {
    pub collection: String,
    pub id: String,
    #[ts(type = "number")]
    pub position: i64,
    pub expected_digest: Option<String>,
    #[ts(type = "unknown")]
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiLedgerApplyParams {
    pub command_id: String,
    pub records: Vec<AiLedgerRecordMutation>,
    #[serde(default)]
    #[ts(type = "unknown[]")]
    pub pending_inputs: Vec<Value>,
    #[serde(default)]
    #[ts(type = "unknown[]")]
    pub delivery_inputs: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiHistoryParams {
    pub action: String,
    #[ts(type = "unknown")]
    pub payload: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiHistoryResult {
    #[ts(type = "unknown")]
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiLedgerRecordReceipt {
    pub collection: String,
    pub id: String,
    pub digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiLedgerApplyResult {
    pub command_id: String,
    pub records: Vec<AiLedgerRecordReceipt>,
}
