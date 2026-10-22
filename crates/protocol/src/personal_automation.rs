use crate::{ModelExecutionOwner, ModelExecutionPrepareParams};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalAutomationTriggerOutcome {
    Admitted,
    Skipped,
    Completed,
    Failed,
    Cancelled,
    OutcomeUnconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "snake_case")]
pub enum PersonalAutomationFinishOutcome {
    Completed,
    Failed,
    Cancelled,
    OutcomeUnconfirmed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationRuleRecord {
    pub id: String,
    #[ts(type = "number")]
    pub position: i64,
    pub digest: String,
    pub value: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationClaim {
    pub claim_id: String,
    pub profile_id: String,
    pub rule_id: String,
    pub rule_version: u32,
    pub trigger_id: String,
    pub owner: ModelExecutionOwner,
    pub writer_lease_id: String,
    pub run_id: Option<String>,
    pub model_lease_ids: Vec<String>,
    pub released: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationTriggerRecord {
    pub trigger_id: String,
    pub claim_id: Option<String>,
    pub profile_id: String,
    pub rule_id: String,
    pub rule_version: u32,
    #[ts(type = "number | null")]
    pub scheduled_due_at: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number | null")]
    pub finished_at: Option<i64>,
    pub run_id: Option<String>,
    pub outcome: PersonalAutomationTriggerOutcome,
    pub reason_code: Option<String>,
    pub read_only_after_restore: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationAdmitParams {
    pub trigger_id: String,
    pub rule_id: String,
    pub expected_rule_digest: String,
    #[ts(type = "number | null")]
    pub scheduled_due_at: Option<i64>,
    #[ts(type = "number | null")]
    pub next_run_at: Option<i64>,
    pub owner: ModelExecutionOwner,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationAdmitResult {
    pub trigger: PersonalAutomationTriggerRecord,
    pub claim: Option<PersonalAutomationClaim>,
    pub admission_rule_record: PersonalAutomationRuleRecord,
    pub rule_record: PersonalAutomationRuleRecord,
    pub replayed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationPrepareExecutionParams {
    pub claim_id: String,
    pub owner: ModelExecutionOwner,
    pub client_operation_id: String,
    pub execution: ModelExecutionPrepareParams,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationBindParams {
    pub claim_id: String,
    pub owner: ModelExecutionOwner,
    pub run_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationBindResult {
    pub claim: PersonalAutomationClaim,
    pub rule_record: Option<PersonalAutomationRuleRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationFinishParams {
    pub claim_id: String,
    pub owner: ModelExecutionOwner,
    pub outcome: PersonalAutomationFinishOutcome,
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationFinishResult {
    pub claim: PersonalAutomationClaim,
    pub trigger: PersonalAutomationTriggerRecord,
    pub rule_record: Option<PersonalAutomationRuleRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationListTriggersParams {
    pub rule_id: Option<String>,
    pub limit: u32,
    #[ts(type = "number | null")]
    pub before_sequence: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationTriggerItem {
    #[ts(type = "number")]
    pub sequence: i64,
    pub record: PersonalAutomationTriggerRecord,
    pub claim_released: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationTriggerPage {
    pub items: Vec<PersonalAutomationTriggerItem>,
    #[ts(type = "number | null")]
    pub next_before_sequence: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonalAutomationResolveInterruptedParams {
    pub claim_id: String,
}
