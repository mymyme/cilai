use super::{AppServerServices, ServiceError};
use contexthub_protocol::{AiLedgerApplyParams, AiLedgerApplyResult, AiLedgerGetResult};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(super) fn prepare_ledger_request(params: &AiLedgerApplyParams) -> Result<Value, ServiceError> {
    let mut request =
        serde_json::to_value(params).map_err(|_| ServiceError::new("ai_ledger_request_invalid"))?;
    let mut deliveries = Vec::new();
    for input in &params.delivery_inputs {
        let content = input["content"]
            .as_str()
            .ok_or_else(|| ServiceError::new("ai_ledger_delivery_invalid"))?;
        deliveries.push(json!({"messageId":input["messageId"],"runId":input["runId"],"contentDigest":format!("{:x}",Sha256::digest(content.as_bytes())),"skillDelivery":super::skill_generation::structured_delivery(content),"capabilityDelivery":super::capability_generation::structured_delivery(content)}));
    }
    request
        .as_object_mut()
        .ok_or_else(|| ServiceError::new("ai_ledger_request_invalid"))?
        .remove("deliveryInputs");
    request["deliveries"] = Value::Array(deliveries);
    Ok(request)
}

impl AppServerServices {
    pub(super) fn ai_history(
        &mut self,
        params: &contexthub_protocol::AiHistoryParams,
        now: i64,
    ) -> Result<contexthub_protocol::AiHistoryResult, ServiceError> {
        let mut payload = params.payload.clone();
        if matches!(params.action.as_str(), "recoverText" | "handoff") {
            let parameters =
                serde_json::from_value::<AiLedgerApplyParams>(payload["ledger"].clone())
                    .map_err(|_| ServiceError::new("ai_ledger_request_invalid"))?;
            payload["ledger"] = prepare_ledger_request(&parameters)?;
        }
        if params.action == "activate" {
            let parameters =
                serde_json::from_value::<AiLedgerApplyParams>(payload["ledger"].clone())
                    .map_err(|_| ServiceError::new("ai_ledger_request_invalid"))?;
            payload["ledger"] = prepare_ledger_request(&parameters)?;
            let rows = self
                .persistence
                .as_ref()
                .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
                .ai_history_delivery_sources()
                .map_err(|_| ServiceError::new("ai_history_delivery_invalid"))?;
            payload["legacyDeliveries"] = json!(
                rows.into_iter()
                    .map(|mut record| {
                        let mut source = record["source"].clone();
                        record["sourceDigest"] = json!(format!(
                            "{:x}",
                            Sha256::digest(source.to_string().as_bytes())
                        ));
                        let body = source["message"]["content"].as_str().map(str::to_owned);
                        if let Some(body) = body {
                            let parsed = serde_json::from_str::<Value>(&body).ok();
                            let body = parsed
                                .as_ref()
                                .filter(|value| {
                                    value["schemaVersion"] == 1
                                        && value["classification"].is_object()
                                        && value["nonce"].is_string()
                                })
                                .and_then(|value| value["reply"].as_str())
                                .unwrap_or(&body);
                            source["skillDelivery"] =
                                super::skill_generation::structured_delivery(body);
                            source["capabilityDelivery"] =
                                super::capability_generation::structured_delivery(body);
                            if let Some(message) = source["message"].as_object_mut() {
                                message.remove("content");
                            }
                        }
                        record["source"] = source;
                        record
                    })
                    .collect::<Vec<_>>()
            );
        }
        let value = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .ai_history(&params.action, &payload, now)
            .map_err(|error| ServiceError::new(error.reason().unwrap_or("ai_history_failed")))?;
        Ok(contexthub_protocol::AiHistoryResult { value })
    }
    pub(super) fn ai_ledger_get(&self) -> Result<AiLedgerGetResult, ServiceError> {
        let ledger = self
            .persistence
            .as_ref()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .ai_business_ledger()
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("ai_ledger_read_failed"))
            })?;
        Ok(AiLedgerGetResult { ledger })
    }

    pub(super) fn ai_ledger_apply(
        &mut self,
        params: &AiLedgerApplyParams,
        now: i64,
    ) -> Result<AiLedgerApplyResult, ServiceError> {
        let request = prepare_ledger_request(params)?;
        let result = self
            .persistence
            .as_mut()
            .ok_or_else(|| ServiceError::new("persistence_unavailable"))?
            .apply_ai_business_batch(&request, now)
            .map_err(|error| {
                ServiceError::new(error.reason().unwrap_or("ai_ledger_write_failed"))
            })?;
        serde_json::from_value(result).map_err(|_| ServiceError::new("ai_ledger_receipt_invalid"))
    }
}
