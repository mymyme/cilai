use super::{
    AgentRun, AgentTokenUsage, AiRuntimeManager, application_usage::StoredApplicationUsage, lock,
};
use contexthub_protocol::{
    ModelUsageListParams, ModelUsageListResult, ModelUsageRecord, ModelUsageState,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MAX_EXACT_UI_INTEGER: u64 = 9_007_199_254_740_991;

fn observed_tokens(usage: Option<&AgentTokenUsage>) -> Value {
    let Some(usage) = usage else {
        return Value::Null;
    };
    if [
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
    ]
    .into_iter()
    .any(|value| value > MAX_EXACT_UI_INTEGER)
    {
        return Value::Null;
    }
    json!({"inputTokens": usage.input_tokens, "cachedInputTokens": usage.cached_input_tokens,
        "outputTokens": usage.output_tokens, "source": "official_runtime_usage"})
}

fn run_rows(run: &AgentRun) -> Vec<Value> {
    let execution = run.model_execution.as_ref();
    let purpose = execution.map_or_else(
        || {
            if run.automation_rule_id.is_some() {
                "automation"
            } else {
                "unknown"
            }
        },
        |snapshot| match snapshot.purpose {
            contexthub_protocol::ModelPurpose::Foreground => "foreground",
            contexthub_protocol::ModelPurpose::Automation => "automation",
            contexthub_protocol::ModelPurpose::Review => "review",
            contexthub_protocol::ModelPurpose::Memory => "memory",
            contexthub_protocol::ModelPurpose::PageAi => "page_ai",
        },
    );
    let mut rows = vec![json!({"runId":run.run_id,"purpose":purpose,"phase":"reply",
        "connectionId":execution.map(|snapshot|&snapshot.connection_id),
        "modelId":execution.map(|snapshot|&snapshot.model_id),"status":run.status,
        "startedAt":run.started_at,"completedAt":run.completed_at,
        "tokens":observed_tokens(run.usage.reply.as_ref()),"cost":null})];
    if let Some(usage) = run.usage.classification.as_ref() {
        rows.push(
            json!({"runId":run.run_id,"purpose":"classification","phase":"classification",
            "connectionId":null,"modelId":null,"status":run.classification_status,
            "startedAt":run.started_at,"completedAt":run.completed_at,
            "tokens":observed_tokens(Some(usage)),"cost":null}),
        );
    }
    rows
}

fn page_row(record: &StoredApplicationUsage, source: &str) -> Value {
    let execution = record.model_execution.as_ref();
    json!({"runId":record.run_id,"purpose":"page_ai","phase":"reply",
        "connectionId":execution.map(|identity| &identity.connection_id),
        "modelId":record.model_id,"status":record.status,
        "sourceProfileId":execution.map(|identity| &identity.profile_id),
        "configRevision":execution.map(|identity| identity.config_revision),
        "authGeneration":execution.map(|identity| identity.auth_generation),
        "startedAt":record.started_at,"completedAt":record.completed_at,
        "tokens":observed_tokens(record.reply_usage.as_ref()),"cost":null,"recordSource":source})
}
fn page_rows(
    stored: Vec<StoredApplicationUsage>,
    current: Vec<StoredApplicationUsage>,
) -> Result<Vec<Value>, String> {
    let mut records = BTreeMap::new();
    for record in stored {
        record.validate()?;
        if records
            .insert(record.run_id.clone(), (record, "persisted_page_metadata"))
            .is_some()
        {
            return Err("application_usage_storage_invalid".into());
        }
    }
    for record in current {
        record.validate()?;
        if let Some((durable, _)) = records.get(&record.run_id) {
            if durable.model_execution != record.model_execution
                || durable.model_id != record.model_id
            {
                return Err("application_usage_identity_changed".into());
            }
            if durable.event_position >= record.event_position {
                continue;
            }
        }
        records.insert(record.run_id.clone(), (record, "current_runtime_page"));
    }
    Ok(records
        .values()
        .map(|(record, source)| page_row(record, source))
        .collect())
}

fn request_row(record: &ModelUsageRecord) -> Value {
    let tokens = record.tokens.as_ref().map(|tokens| {
        let mut value = serde_json::to_value(tokens).expect("usage counters serialize");
        for (_, counter) in value.as_object_mut().expect("usage is an object") {
            if counter
                .as_u64()
                .is_some_and(|count| count > MAX_EXACT_UI_INTEGER)
            {
                *counter = Value::Null;
            }
        }
        value["source"] = json!("upstream_responses_usage");
        value
    });
    json!({"runId":record.request_id,"purpose":record.purpose,"phase":"request",
        "connectionId":record.connection_id,"modelId":record.model_id,"sourceProfileId":record.source_profile_id,
        "configRevision":record.config_revision,"authGeneration":record.auth_generation,
        "readOnlyAfterRestore":record.read_only_after_restore,
        "status":if record.state==ModelUsageState::Started {"unconfirmed"} else {match record.state {
            ModelUsageState::Completed=>"completed",ModelUsageState::Failed=>"failed",_=>"unconfirmed"}},
        "startedAt":record.started_at,"completedAt":record.finished_at,"tokens":tokens,"cost":record.cost,
        "recordSource":"persisted_request_metadata"})
}
fn request_record_rows(mut records: Vec<ModelUsageRecord>) -> Result<Vec<Value>, String> {
    records.sort_by(|a, b| (a.started_at, &a.request_id).cmp(&(b.started_at, &b.request_id)));
    let mut observed = BTreeMap::new();
    let mut rows = Vec::new();
    for record in records {
        let mut row = request_row(&record);
        let key = record.response_id_sha256.as_ref().map(|response| {
            serde_json::to_string(&(
                &record.source_profile_id,
                &record.connection_id,
                record.config_revision,
                record.auth_generation,
                &record.model_id,
                &record.purpose,
                response,
            ))
            .expect("usage identity serializes")
        });
        row["contributesToTotals"] = Value::Bool(true);
        if let Some(key) = key {
            if let Some((tokens, cost, request_id)) = observed.get(&key) {
                if *tokens != record.tokens || *cost != record.cost {
                    return Err("model_usage_response_identity_conflict".into());
                }
                row["contributesToTotals"] = Value::Bool(false);
                row["duplicateOfRequestId"] = json!(request_id);
            } else {
                observed.insert(
                    key,
                    (
                        record.tokens.clone(),
                        record.cost.clone(),
                        record.request_id.clone(),
                    ),
                );
            }
        }
        rows.push(row);
    }
    Ok(rows)
}
fn request_rows(server: &crate::supervisor::AppServerSupervisor) -> Result<Vec<Value>, String> {
    let mut before = None;
    let mut through = None;
    let mut records = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        let params = ModelUsageListParams {
            before_sequence: before,
            through_sequence: through,
            limit: 500,
        };
        let result: ModelUsageListResult = serde_json::from_value(server.request_result(
            "modelUsage/list",
            &serde_json::to_value(params).map_err(|_| "model_usage_request_invalid")?,
        )?)
        .map_err(|_| "model_usage_response_invalid")?;
        if through.is_some_and(|bound| bound != result.snapshot_sequence)
            || result.records.len() > 500
            || (result.next_before_sequence.is_some() && result.records.is_empty())
        {
            return Err("model_usage_response_invalid".into());
        }
        through = Some(result.snapshot_sequence);
        for record in result.records {
            if !seen.insert(record.request_id.clone()) {
                return Err("model_usage_response_invalid".into());
            }
            records.push(record);
            if records.len() > 50_000 {
                return Err("model_usage_limit_exceeded".into());
            }
        }
        let Some(next) = result.next_before_sequence else {
            return request_record_rows(records);
        };
        if next <= 0 || before.is_some_and(|old| next >= old) {
            return Err("model_usage_response_invalid".into());
        }
        before = Some(next);
    }
}

impl AiRuntimeManager {
    /// 只读实际运行用量；不查询供应商，不用默认零值或当前选择填补历史来源。
    /// # Errors
    /// 持久页面回执无法读取或身份不一致时失败，不用空列表替代。
    pub fn usage_view(&self) -> Result<Value, String> {
        let (revision, mut rows, current, server) = {
            let state = lock(&self.inner);
            (
                state.state_revision,
                state
                    .persisted
                    .runs
                    .iter()
                    .flat_map(run_rows)
                    .collect::<Vec<_>>(),
                state
                    .application_model_runs
                    .values()
                    .map(|record| StoredApplicationUsage::from_snapshot(&record.snapshot))
                    .collect::<Vec<_>>(),
                state.server.clone().ok_or("ai_runtime_not_initialized")?,
            )
        };
        let response = server.request_result(
            "aiApplication/execute",
            &json!({"action":"usage","payload":{}}),
        )?;
        if response["source"] != "persisted_page_metadata" {
            return Err("application_usage_storage_invalid".into());
        }
        let stored =
            serde_json::from_value::<Vec<StoredApplicationUsage>>(response["snapshots"].clone())
                .map_err(|_| "application_usage_storage_invalid")?;
        rows.extend(page_rows(stored, current)?);
        rows.extend(request_rows(&server)?);
        Ok(json!({"stateRevision":revision,"rows":rows,
            "coverage":[{"purpose":"foreground","source":"persisted_runtime_records"},
                {"purpose":"automation","source":"persisted_runtime_records"},
                {"purpose":"review","source":"persisted_request_receipts"},
                {"purpose":"memory","source":"persisted_request_receipts"},
                {"purpose":"page_ai","source":"persisted_and_current_page_receipts"}],
            "costSource":null,"priceCheckedAt":null}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run() -> AgentRun {
        serde_json::from_value(json!({"runId":"run-example","conversationId":"conversation",
            "agentId":"agent","inputMessageId":"message","retryOfRunId":null,"provider":"legacy",
            "model":"old-model-without-recorded-connection","status":"completed","pid":null,
            "startedAt":1,"completedAt":2,"output":"PRIVATE_CONTENT","errorReason":null,"sessionId":null})).unwrap()
    }
    #[test]
    fn default_legacy_zero_is_unknown_while_observed_zero_is_real() {
        let run = run();
        assert!(run_rows(&run)[0]["tokens"].is_null());
        assert!(run_rows(&run)[0]["connectionId"].is_null());
        assert!(run_rows(&run)[0]["modelId"].is_null());
        assert_eq!(run_rows(&run)[0]["purpose"], "unknown");
        assert_eq!(
            observed_tokens(Some(&AgentTokenUsage {
                input_tokens: 0,
                cached_input_tokens: 0,
                output_tokens: 0
            }))["outputTokens"],
            0
        );
    }
    #[test]
    fn classification_does_not_become_approval_review_or_inherit_unproven_identity() {
        let mut run = run();
        run.automation_rule_id = Some("rule".into());
        run.usage.classification = Some(AgentTokenUsage {
            input_tokens: 4,
            cached_input_tokens: 0,
            output_tokens: 2,
        });
        let rows = run_rows(&run);
        assert_eq!(rows[0]["purpose"], "automation");
        assert_eq!(rows[1]["purpose"], "classification");
        assert!(rows[1]["connectionId"].is_null());
        assert!(rows[1]["cost"].is_null());
        assert!(
            !serde_json::to_string(&rows)
                .unwrap()
                .contains("PRIVATE_CONTENT")
        );
    }
    #[test]
    fn unsafe_integer_cannot_be_reported_as_a_precise_javascript_count() {
        assert!(
            observed_tokens(Some(&AgentTokenUsage {
                input_tokens: u64::MAX,
                cached_input_tokens: 0,
                output_tokens: 1
            }))
            .is_null()
        );
    }
    fn page(position: u64) -> StoredApplicationUsage {
        serde_json::from_value(json!({"runId":"page-example","modelId":"recorded-page-model","status":"completed",
            "startedAt":1,"completedAt":2,"eventPosition":position,"modelExecution":{"profileId":"source-profile",
            "connectionId":"original-connection","configRevision":2,"authGeneration":3,"modelId":"recorded-page-model"},
            "replyUsage":{"inputTokens":4,"cachedInputTokens":0,"outputTokens":2}})).unwrap()
    }
    #[test]
    fn page_usage_survives_without_a_live_page_and_cost_stays_unknown() {
        let rows = page_rows(vec![page(2)], Vec::new()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["purpose"], "page_ai");
        assert_eq!(rows[0]["connectionId"], "original-connection");
        assert_eq!(rows[0]["sourceProfileId"], "source-profile");
        assert_eq!(rows[0]["tokens"]["inputTokens"], 4);
        assert_eq!(rows[0]["tokens"]["cachedInputTokens"], 0);
        assert!(rows[0]["cost"].is_null());
        assert_eq!(rows[0]["recordSource"], "persisted_page_metadata");
    }
    #[test]
    fn stale_page_cache_cannot_replace_a_newer_durable_usage_receipt() {
        let mut stale = page(1);
        stale.status = "running".into();
        stale.reply_usage = None;
        let rows = page_rows(vec![page(2)], vec![stale]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["status"], "completed");
        assert_eq!(rows[0]["tokens"]["outputTokens"], 2);
    }
    #[test]
    fn changing_connection_does_not_relabel_an_old_page_usage_row() {
        let mut other = page(3);
        other.model_execution.as_mut().unwrap().connection_id = "new-connection".into();
        assert_eq!(
            page_rows(vec![page(2)], vec![other]).unwrap_err(),
            "application_usage_identity_changed"
        );
        let mut legacy = page(1);
        legacy.model_execution = None;
        legacy.reply_usage = None;
        let rows = page_rows(vec![legacy], Vec::new()).unwrap();
        assert!(rows[0]["tokens"].is_null());
        assert!(rows[0]["connectionId"].is_null());
        assert_eq!(rows[0]["modelId"], "recorded-page-model");
    }
    #[test]
    fn request_usage_preserves_independent_purpose_missing_metrics_and_exact_cost() {
        let record:ModelUsageRecord=serde_json::from_value(json!({
            "requestId":"request-identity","sourceProfileId":"source-profile","leaseId":"owned-lease","snapshotDigest":"digest",
            "connectionId":"independent-memory-connection","configRevision":2,"authGeneration":3,"modelId":"independent-memory-model",
            "purpose":"memory","scopeId":"memory-scope","state":"completed","responseIdSha256":null,
            "tokens":{"inputTokens":4,"cachedInputTokens":null,"cacheWriteInputTokens":null,"outputTokens":2,"reasoningOutputTokens":null,"totalTokens":null},
            "cost":{"amount":"0.1234567890123456789012","currency":"USD","source":"upstream.responses.usage.cost"},
            "reasonCode":null,"startedAt":1,"finishedAt":2,"readOnlyAfterRestore":true})).unwrap();
        let row = request_row(&record);
        assert_eq!(row["phase"], "request");
        assert_eq!(row["purpose"], "memory");
        assert_eq!(row["connectionId"], "independent-memory-connection");
        assert_eq!(row["readOnlyAfterRestore"], true);
        assert!(row["tokens"]["cachedInputTokens"].is_null());
        assert_eq!(row["cost"]["amount"], "0.1234567890123456789012");
        assert!(!row.to_string().contains("owned-lease"));
        assert!(!row.to_string().contains("memory-scope"));
        let mut duplicate = record.clone();
        duplicate.request_id = "later-request".into();
        duplicate.started_at = 3;
        let rows = request_record_rows(vec![duplicate.clone(), record.clone()]).unwrap();
        assert_eq!(rows[0]["contributesToTotals"], true);
        assert_eq!(rows[1]["contributesToTotals"], true);
        let mut original = record.clone();
        original.response_id_sha256 = Some("a".repeat(64));
        duplicate.response_id_sha256 = original.response_id_sha256.clone();
        let rows = request_record_rows(vec![duplicate.clone(), original.clone()]).unwrap();
        assert_eq!(rows[1]["contributesToTotals"], false);
        assert_eq!(rows[1]["duplicateOfRequestId"], original.request_id);
        duplicate.tokens.as_mut().unwrap().input_tokens = Some(99);
        assert_eq!(
            request_record_rows(vec![original, duplicate]).unwrap_err(),
            "model_usage_response_identity_conflict"
        );
    }
}
