//! 只提取终态统计字段；正文、响应标识原值及供应商任意 metadata 不进入账本。
use super::ProxyState;
use contexthub_protocol::{
    ModelPurpose, ModelUsageBeginParams, ModelUsageCost, ModelUsageFinishParams, ModelUsageRecord,
    ModelUsageState, ModelUsageTokens,
};
use serde::Deserialize;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
struct Event<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(borrow)]
    response: Option<Response<'a>>,
}
#[derive(Deserialize)]
struct Response<'a> {
    id: Option<&'a str>,
    #[serde(borrow)]
    usage: Option<Usage<'a>>,
}
#[derive(Deserialize)]
struct Usage<'a> {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    input_tokens_details: Option<InputDetails>,
    output_tokens_details: Option<OutputDetails>,
    #[serde(borrow)]
    cost: Option<&'a RawValue>,
    currency: Option<&'a str>,
}
#[derive(Deserialize)]
struct InputDetails {
    cached_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
}
#[derive(Deserialize)]
struct OutputDetails {
    reasoning_tokens: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Completion {
    state: ModelUsageState,
    response_id: Option<String>,
    tokens: Option<ModelUsageTokens>,
    cost: Option<ModelUsageCost>,
}

#[derive(Default)]
enum CompletionState {
    #[default]
    Empty,
    Observed(Completion),
    Conflict,
}

#[derive(Default)]
struct Observation {
    line: Vec<u8>,
    data: Vec<u8>,
    oversized: bool,
    limit_seen: bool,
    invalid: bool,
    completion: CompletionState,
}
impl Observation {
    fn chunk(&mut self, chunk: &[u8]) {
        for &byte in chunk {
            if byte == b'\n' {
                self.line_complete();
            } else if !self.oversized && self.line.len() + self.data.len() < MAX_EVENT_BYTES {
                self.line.push(byte);
            } else {
                self.oversized = true;
                self.limit_seen = true;
                self.line.clear();
                self.data.clear();
            }
        }
    }
    fn line_complete(&mut self) {
        if self.line.last() == Some(&b'\r') {
            self.line.pop();
        }
        if self.line.is_empty() {
            if !self.oversized {
                self.event_complete();
            }
            self.data.clear();
            self.oversized = false;
        } else if !self.oversized
            && let Some(data) = self.line.strip_prefix(b"data:")
        {
            let data = data.strip_prefix(b" ").unwrap_or(data);
            self.data.extend_from_slice(data);
            self.data.push(b'\n');
        }
        self.line.clear();
    }
    fn event_complete(&mut self) {
        if self.data.is_empty() || self.data.trim_ascii() == b"[DONE]" {
            return;
        }
        let parsed = serde_json::from_slice::<Event<'_>>(&self.data);
        let Ok(event) = parsed else {
            self.invalid = true;
            return;
        };
        let state = match event.kind {
            "response.completed" => ModelUsageState::Completed,
            "response.failed" | "response.incomplete" => ModelUsageState::Failed,
            _ => return,
        };
        if matches!(self.completion, CompletionState::Conflict) {
            return;
        }
        let Some(response) = event.response else {
            self.invalid = true;
            return;
        };
        let response_id = response
            .id
            .map(|id| format!("{:x}", Sha256::digest(id.as_bytes())));
        let (tokens, cost) = response.usage.map_or((None, None), |u| {
            let tokens = ModelUsageTokens {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                total_tokens: u.total_tokens,
                cached_input_tokens: u
                    .input_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens),
                cache_write_input_tokens: u.input_tokens_details.and_then(|d| d.cache_write_tokens),
                reasoning_output_tokens: u.output_tokens_details.and_then(|d| d.reasoning_tokens),
            };
            let tokens = (tokens != ModelUsageTokens::default()).then_some(tokens);
            let cost = u.cost.zip(u.currency).and_then(|(raw, currency)| {
                let amount = decimal(raw.get())?;
                (currency.len() == 3 && currency.bytes().all(|b| b.is_ascii_uppercase())).then(
                    || ModelUsageCost {
                        amount,
                        currency: currency.into(),
                        source: "upstream.responses.usage.cost".into(),
                    },
                )
            });
            (tokens, cost)
        });
        let next = Completion {
            state,
            response_id,
            tokens,
            cost,
        };
        if matches!(&self.completion, CompletionState::Observed(old) if *old != next) {
            self.invalid = true;
            self.completion = CompletionState::Conflict;
            return;
        }
        self.completion = CompletionState::Observed(next);
    }
    fn finish(&mut self, record: &ModelUsageRecord) -> ModelUsageFinishParams {
        if !self.line.is_empty() {
            self.line_complete();
        }
        if !self.data.is_empty() && !self.oversized {
            self.event_complete();
            self.data.clear();
        }
        let (state, response_id_sha256, tokens, cost, reason_code) =
            match std::mem::take(&mut self.completion) {
                CompletionState::Observed(c) => {
                    let reason = c.tokens.is_none().then(|| "usage_not_provided".into());
                    (c.state, c.response_id, c.tokens, c.cost, reason)
                }
                CompletionState::Empty | CompletionState::Conflict => (
                    ModelUsageState::Unconfirmed,
                    None,
                    None,
                    None,
                    Some(
                        if self.limit_seen {
                            "usage_observation_limit"
                        } else if self.invalid {
                            "usage_invalid"
                        } else {
                            "stream_ended_without_completion"
                        }
                        .into(),
                    ),
                ),
            };
        ModelUsageFinishParams {
            request_id: record.request_id.clone(),
            lease_id: record.lease_id.clone(),
            snapshot_digest: record.snapshot_digest.clone(),
            state,
            response_id_sha256,
            tokens,
            cost,
            reason_code,
        }
    }
}

fn decimal(raw: &str) -> Option<String> {
    let text = if raw.starts_with('"') {
        serde_json::from_str::<String>(raw).ok()?
    } else {
        raw.into()
    };
    if text.len() > 80 {
        return None;
    }
    let mut exponent_parts = text.split(['e', 'E']);
    let mantissa = exponent_parts.next()?;
    let exponent = exponent_parts
        .next()
        .map_or(Some(0), |value| value.parse::<i32>().ok())?;
    if exponent_parts.next().is_some() || exponent.unsigned_abs() > 80 {
        return None;
    }
    let mut parts = mantissa.split('.');
    let whole = parts.next()?;
    let fraction = parts.next();
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.is_some_and(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
        || parts.next().is_some()
    {
        return None;
    }
    let digits = format!("{whole}{}", fraction.unwrap_or_default());
    let point = i32::try_from(whole.len()).ok()?.checked_add(exponent)?;
    let value = if point <= 0 {
        format!("0.{}{digits}", "0".repeat(usize::try_from(-point).ok()?))
    } else if usize::try_from(point).ok()? >= digits.len() {
        format!(
            "{digits}{}",
            "0".repeat(usize::try_from(point).ok()? - digits.len())
        )
    } else {
        let point = usize::try_from(point).ok()?;
        format!("{}.{}", &digits[..point], &digits[point..])
    };
    if value.len() > 80 {
        return None;
    }
    let value = value.trim_start_matches('0');
    let value = if value.starts_with('.') {
        format!("0{value}")
    } else if value.is_empty() {
        "0".into()
    } else {
        value.into()
    };
    Some(if value.contains('.') {
        value.trim_end_matches('0').trim_end_matches('.').into()
    } else {
        value
    })
}

pub(super) struct RequestUsage {
    server: crate::supervisor::AppServerSupervisor,
    record: ModelUsageRecord,
    observation: Observation,
    acknowledged: bool,
}
impl RequestUsage {
    pub(super) fn begin(state: &ProxyState, purpose: &'static str) -> Result<Option<Self>, String> {
        if !matches!(purpose, "review" | "memory") {
            return Ok(None);
        }
        let execution = state
            .execution
            .as_ref()
            .ok_or("model_usage_execution_unavailable")?;
        let scope = execution.validate_request(purpose)?;
        let request_id = uuid::Uuid::new_v4().simple().to_string();
        let scope_id = if purpose == "memory" {
            &state
                .memory
                .as_ref()
                .ok_or("model_usage_execution_unavailable")?
                .scope_id
        } else {
            &state.scope_id
        };
        let params = ModelUsageBeginParams {
            request_id,
            lease_id: scope.lease.lease_id.clone(),
            snapshot_digest: scope.lease.snapshot_digest.clone(),
            scope_id: scope_id.clone(),
        };
        let value = scope.server.request_result(
            "modelUsage/begin",
            &serde_json::to_value(params).map_err(|_| "model_usage_request_invalid")?,
        )?;
        let record: ModelUsageRecord =
            serde_json::from_value(value).map_err(|_| "model_usage_response_invalid")?;
        let expected = if purpose == "review" {
            ModelPurpose::Review
        } else {
            ModelPurpose::Memory
        };
        if record.state != ModelUsageState::Started
            || record.lease_id != scope.lease.lease_id
            || record.source_profile_id != scope.lease.snapshot.profile_id
            || record.connection_id != scope.lease.snapshot.connection_id
            || record.model_id != scope.lease.snapshot.model_id
            || record.purpose != expected
        {
            return Err("model_usage_response_invalid".into());
        }
        Ok(Some(Self {
            server: scope.server.clone(),
            record,
            observation: Observation::default(),
            acknowledged: false,
        }))
    }
    pub(super) fn observe(&mut self, chunk: &[u8]) {
        self.observation.chunk(chunk);
    }
    pub(super) fn finish_stream(&mut self) -> Result<(), String> {
        if self.acknowledged {
            return Ok(());
        }
        let p = self.observation.finish(&self.record);
        self.save(&p)
    }
    pub(super) fn finish_rejected(&mut self) -> Result<(), String> {
        self.save(&ModelUsageFinishParams {
            request_id: self.record.request_id.clone(),
            lease_id: self.record.lease_id.clone(),
            snapshot_digest: self.record.snapshot_digest.clone(),
            state: ModelUsageState::Failed,
            response_id_sha256: None,
            tokens: None,
            cost: None,
            reason_code: Some("upstream_http_rejected".into()),
        })
    }
    fn save(&mut self, p: &ModelUsageFinishParams) -> Result<(), String> {
        let value = self.server.request_result(
            "modelUsage/finish",
            &serde_json::to_value(p).map_err(|_| "model_usage_request_invalid")?,
        )?;
        let result: ModelUsageRecord =
            serde_json::from_value(value).map_err(|_| "model_usage_response_invalid")?;
        if result.request_id != p.request_id
            || result.lease_id != p.lease_id
            || result.snapshot_digest != p.snapshot_digest
            || result.source_profile_id != self.record.source_profile_id
            || result.state != p.state
            || result.response_id_sha256 != p.response_id_sha256
            || result.reason_code != p.reason_code
            || result.tokens != p.tokens
            || result.cost != p.cost
        {
            return Err("model_usage_response_invalid".into());
        }
        self.acknowledged = true;
        Ok(())
    }
}
impl Drop for RequestUsage {
    fn drop(&mut self) {
        if !self.acknowledged {
            eprintln!("personal_model_usage_outcome_unconfirmed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn record() -> ModelUsageRecord {
        serde_json::from_value(json!({"requestId":"11111111111141118111111111111111","sourceProfileId":"22222222222242228222222222222222",
            "leaseId":"owned-lease","snapshotDigest":"digest","connectionId":"review-connection","configRevision":2,"authGeneration":3,
            "modelId":"review-model","purpose":"review","scopeId":"run-id","state":"started","responseIdSha256":null,
            "tokens":null,"cost":null,"reasonCode":null,"startedAt":1,"finishedAt":null,"readOnlyAfterRestore":false})).unwrap()
    }
    #[test]
    fn response_usage_ignores_content_and_preserves_each_missing_metric() {
        let mut seen = Observation::default();
        let input=b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"private-id\",\"output\":[{\"text\":\"PRIVATE_REPLY\"}],\"usage\":{\"input_tokens\":7,\"input_tokens_details\":{\"cached_tokens\":0},\"total_tokens\":9}}}\r\n\r\n";
        for part in input.chunks(11) {
            seen.chunk(part);
        }
        let result = seen.finish(&record());
        let tokens = result.tokens.as_ref().unwrap();
        assert_eq!(tokens.input_tokens, Some(7));
        assert_eq!(tokens.cached_input_tokens, Some(0));
        assert_eq!(tokens.output_tokens, None);
        assert_eq!(tokens.reasoning_output_tokens, None);
        let raw = serde_json::to_string(&result).unwrap();
        assert!(!raw.contains("PRIVATE_REPLY"));
        assert!(!raw.contains("private-id"));
        assert_eq!(
            result.response_id_sha256,
            Some(format!("{:x}", Sha256::digest(b"private-id")))
        );
    }
    #[test]
    fn response_usage_cost_requires_currency_and_preserves_exact_decimal_lexemes() {
        for (money, expected) in [
            (
                "\"cost\":0.1234567890123456789012,\"currency\":\"USD\"",
                Some("0.1234567890123456789012"),
            ),
            (
                "\"cost\":1.25e-9,\"currency\":\"EUR\"",
                Some("0.00000000125"),
            ),
            ("\"cost\":0,\"currency\":\"CNY\"", Some("0")),
            ("\"cost\":0", None),
        ] {
            let mut seen = Observation::default();
            seen.chunk(format!("data: {{\"type\":\"response.completed\",\"response\":{{\"usage\":{{\"input_tokens\":1,{money}}}}}}}\n\n").as_bytes());
            let result = seen.finish(&record());
            assert_eq!(result.cost.as_ref().map(|c| c.amount.as_str()), expected);
            assert_eq!(result.tokens.unwrap().input_tokens, Some(1));
        }
    }
    #[test]
    fn response_usage_repeated_completion_is_deduplicated_but_conflicts_are_unconfirmed() {
        let mut seen = Observation::default();
        let one=b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"a\",\"usage\":{\"input_tokens\":1}}}\n\n";
        seen.chunk(one);
        seen.chunk(one);
        assert_eq!(seen.finish(&record()).tokens.unwrap().input_tokens, Some(1));
        let mut seen = Observation::default();
        seen.chunk(one);
        seen.chunk(b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"different\",\"usage\":{\"input_tokens\":9}}}\n\n");
        seen.chunk(one);
        let result = seen.finish(&record());
        assert_eq!(result.state, ModelUsageState::Unconfirmed);
        assert_eq!(result.tokens, None);
    }
    #[test]
    fn response_usage_interrupted_or_oversized_stream_does_not_invent_zero() {
        let mut seen = Observation::default();
        seen.chunk(b"data: [DONE]\n\n");
        let result = seen.finish(&record());
        assert_eq!(result.state, ModelUsageState::Unconfirmed);
        assert_eq!(result.tokens, None);
        let mut seen = Observation::default();
        seen.chunk(&vec![b'x'; MAX_EVENT_BYTES + 1]);
        seen.chunk(b"\n\n");
        let result = seen.finish(&record());
        assert_eq!(
            result.reason_code.as_deref(),
            Some("usage_observation_limit")
        );
        assert_eq!(result.tokens, None);
    }
    #[test]
    fn response_usage_explicit_zero_and_absent_usage_remain_distinct() {
        let mut seen = Observation::default();
        seen.chunk(b"data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":0,\"output_tokens\":0}}}\n\n");
        let result = seen.finish(&record());
        assert_eq!(result.tokens.unwrap().input_tokens, Some(0));
        let mut seen = Observation::default();
        seen.chunk(b"data: {\"type\":\"response.completed\",\"response\":{}}\n\n");
        let result = seen.finish(&record());
        assert_eq!(result.state, ModelUsageState::Completed);
        assert_eq!(result.tokens, None);
        assert_eq!(result.reason_code.as_deref(), Some("usage_not_provided"));
    }
}
